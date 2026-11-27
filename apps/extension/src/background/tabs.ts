// Emberhut's own tabs: the "Emberhut" tab group in any window (whoever put a tab there), a tab Emberhut opened, or a popup
// one of them opened. Dragged out of the group, or the group renamed, a tab is the owner's again.

import { Msg } from "../protocol";
import { GROUP_COLOR, GROUP_TITLE, hostOf, isBlocked, isTabBlocked } from "../scope";
import { pausedNow } from "../pause";
import { UNPILL } from "../page-scripts";
import { badge } from "./badge";
import { adopted, attached, counters, framing, joining, made, pillWorld, reported, urls } from "./held";
import { settings, step } from "./state";
import { maybeReload } from "./update";
import { emit, ready } from "./wire";

/** A tab as the server sees it. */
export interface SlimTab {
  id: number;
  url: string;
  title: string;
  windowId: number;
  active: boolean;
}

/** A tab as the server sees it. */
export function slim(tab: chrome.tabs.Tab): SlimTab {
  return { id: tab.id ?? -1, url: tab.url || tab.pendingUrl || "", title: tab.title || "", windowId: tab.windowId, active: tab.active };
}

/** A page that doesn't answer (its script never yields, or it's frozen) holds up nothing: each call gives up. */
export function within<T>(ms: number, work: Promise<T>): Promise<T> {
  return Promise.race([work, new Promise<T>((_, no) => setTimeout(() => no(new Error(`no answer within ${ms / 1000} s`)), ms))]);
}

/** The ids of every "Emberhut" group. */
export async function emberhutGroups(): Promise<number[]> {
  return (await chrome.tabGroups.query({ title: GROUP_TITLE })).map((g) => g.id);
}

/** Whether `tab` is Emberhut's. A tab seen in the group stops counting as merely opened by Emberhut. */
export async function isEmberhut(tab: chrome.tabs.Tab): Promise<boolean> {
  if (tab.id === undefined) return false;
  const grouped = tab.groupId !== -1 && (await emberhutGroups()).includes(tab.groupId);
  if (grouped) made.delete(tab.id);
  return adopted.has(tab.id) || grouped || made.has(tab.id);
}

/** Whether `url` is out of bounds, with the owner's own list. */
export async function blockedUrl(url: string | undefined): Promise<boolean> {
  return isBlocked(url, (await settings()).blocked);
}

/** Whether a tab is out of bounds where it is or where it is going, with the owner's own list. */
export async function blockedTab(tab: chrome.tabs.Tab): Promise<boolean> {
  return isTabBlocked(tab, (await settings()).blocked);
}

/** Lets go of one of Emberhut's tabs that is going to (or reached) a blocked site, and says so. */
export async function leaveBlocked(tab: { id?: number | undefined }, url?: string): Promise<void> {
  if (tab.id === undefined || (!attached.has(tab.id) && !reported.has(tab.id))) return;
  const where =
    url ??
    (await chrome.tabs.get(tab.id).then(
      (t) => t.pendingUrl || t.url || "",
      () => "",
    ));
  await letGo(tab.id, "emberhut_scope");
  emit(Msg.blocked, { tabId: tab.id, host: hostOf(where) });
  await step(`Left a blocked site (${hostOf(where)})`);
}

/** Puts a tab in its window's "Emberhut" group, making the group when there is none. */
export async function joinGroup(tabId: number): Promise<void> {
  joining.add(tabId);
  try {
    const tab = await chrome.tabs.get(tabId);
    const [group] = await chrome.tabGroups.query({ title: GROUP_TITLE, windowId: tab.windowId });
    if (group) {
      await chrome.tabs.group({ tabIds: [tabId], groupId: group.id });
      return;
    }
    const id = await chrome.tabs.group({ tabIds: [tabId], createProperties: { windowId: tab.windowId } });
    await chrome.tabGroups.update(id, { title: GROUP_TITLE, color: GROUP_COLOR });
  } finally {
    joining.delete(tabId);
  }
}

/** Tells the server about a tab, or takes it away, as it is now: Emberhut's, on an allowed site, and not paused. */
export async function sync(tab: chrome.tabs.Tab): Promise<void> {
  if (tab.id === undefined || joining.has(tab.id)) return;
  const url = tab.url || tab.pendingUrl || "";
  const ok = ready() && !pausedNow((await settings()).paused, Date.now()) && (await isEmberhut(tab)) && !(await blockedUrl(url));
  if (ok && !reported.has(tab.id)) {
    reported.add(tab.id);
    emit("chrome.tabs.onCreated", [slim(tab)]);
  }
  if (ok && url) urls.set(tab.id, url);
  if (!ok && reported.has(tab.id)) {
    await letGo(tab.id, "emberhut_scope");
    if (url && (await blockedUrl(url)) && (await isEmberhut(tab))) {
      emit(Msg.blocked, { tabId: tab.id, host: hostOf(url) });
      await step(`Left a blocked site (${hostOf(url)})`);
    }
  }
}

/** Every tab synced again. */
export async function syncAll(): Promise<void> {
  for (const tab of await chrome.tabs.query({})) await sync(tab);
}

/** Lets go of one tab: its pill gone, the debugger off, the server told. The tab stays open. */
export async function letGo(tabId: number, reason: string): Promise<void> {
  if (attached.has(tabId)) {
    const contextId = pillWorld.get(tabId);
    const where = contextId === undefined ? {} : { contextId };
    await within(1_000, chrome.debugger.sendCommand({ tabId }, "Runtime.evaluate", { expression: UNPILL, ...where })).catch(
      () => undefined,
    );
  }
  if (attached.delete(tabId)) {
    await chrome.debugger.detach({ tabId }).catch(() => undefined);
    emit("chrome.debugger.onDetach", [{ tabId }, reason]);
  }
  framing.delete(tabId);
  if (reported.delete(tabId)) emit("chrome.tabs.onRemoved", [tabId]);
  await badge();
}

/** Lets go of all of Emberhut's tabs at once; then an update waiting for that may reload. */
export async function letGoAll(reason: string): Promise<void> {
  counters.letGoes++;
  await Promise.all([...new Set([...attached, ...reported])].map((id) => letGo(id, reason)));
  await maybeReload();
}
