// The server's calls into this Chrome (its relay turns the browser tools' DevTools commands into these). Every call
// is checked here, on the owner's machine: only Emberhut's own tabs, never a blocked site, never DevTools commands that
// reach past the page, never a password typed. A final step (submit, send, pay…) is the server's to hold: its step
// gate asks the owner first, and nothing here acts except on the server's own calls.

import { callRefusal, keyRefusal, refusal, worldRefusal } from "../cdp-rules";
import { messageOf } from "../log";
import { BY_ASSISTANT } from "../protocol";
import { clock, pausedNow } from "../pause";
import { GROUP_TITLE, hostOf } from "../scope";
import { badge } from "./badge";
import { refuseSecretInput } from "./guard";
import { attached, closing, counters, dropped, framing, made, reported, urls } from "./held";
import { awake, pill, pointTo } from "./pill";
import { FRAMED, framed } from "./recovery";
import { settings, step } from "./state";
import { blockedTab, blockedUrl, isEmberhut, joinGroup, leaveBlocked, slim, within, emberhutGroups } from "./tabs";

const record = (value: unknown): Record<string, unknown> =>
  typeof value === "object" && value !== null ? (value as Record<string, unknown>) : {};

/** Answers one call from the server: its result, or a thrown error whose words the server gets. */
export async function request(method: string, args: unknown): Promise<unknown> {
  const paused = pausedNow((await settings()).paused, Date.now());
  if (paused)
    throw new Error(`The owner paused Emberhut in this Chrome (${paused.why}): it comes back by itself at ${clock(paused.until)}`);
  const [a, b, c] = Array.isArray(args) ? (args as unknown[]) : [];
  switch (method) {
    case "chrome.debugger.attach":
      return attach(Number(record(a).tabId));
    case "chrome.debugger.sendCommand":
      return command(record(a), String(b), c);
    case "chrome.tabs.create":
      return create(record(a).url);
    case "chrome.tabs.remove":
      return remove(Number(a));
  }
  throw new Error(`${method} isn't something Emberhut can do here`);
}

async function attach(tabId: number): Promise<object> {
  const tab = await chrome.tabs.get(tabId);
  if (!reported.has(tabId) || !(await isEmberhut(tab))) {
    const groups = (await emberhutGroups()).join(",") || "none";
    throw new Error(
      `Not one of Emberhut's tabs (${reported.has(tabId) ? "" : "not given to Emberhut; "}group ${tab.groupId}, ` +
        `the "${GROUP_TITLE}" groups ${groups}, ${made.has(tabId) ? "opened by Emberhut" : "not opened by Emberhut"})`,
    );
  }
  if (await blockedUrl(tab.url || tab.pendingUrl)) throw new Error("A blocked site: hand it to the owner");
  if (attached.has(tabId)) return {};
  const since = counters.letGoes;
  try {
    await chrome.debugger.attach({ tabId }, "1.3");
  } catch (e) {
    if (FRAMED.test(messageOf(e))) framed(tab).catch(() => undefined);
    throw e;
  }
  if (since !== counters.letGoes) {
    await chrome.debugger.detach({ tabId }).catch(() => undefined);
    throw new Error("Emberhut let go of its tabs here meanwhile");
  }
  framing.delete(tabId);
  attached.add(tabId);
  await awake(tabId);
  await within(5_000, pill(tabId)).catch(() => undefined);
  await badge();
  return {};
}

async function command(debuggee: Record<string, unknown>, method: string, params: unknown): Promise<unknown> {
  const tabId = Number(debuggee.tabId);
  const sessionId = typeof debuggee.sessionId === "string" ? debuggee.sessionId : undefined;
  if (!attached.has(tabId)) {
    const why = dropped.get(tabId);
    throw new Error(`Not one of Emberhut's tabs (not attached${why ? `: Chrome dropped it, ${why}` : ""})`);
  }
  const refused = refusal(method, sessionId) ?? worldRefusal(method, params) ?? keyRefusal(method, params) ?? callRefusal(method, params);
  if (refused) throw new Error(refused);
  // Where the tab is, and where it is going, on every command: a tab that reached a blocked site is let go first.
  const tab = await chrome.tabs.get(tabId);
  if (await blockedTab(tab)) {
    await leaveBlocked(tab);
    throw new Error(`That tab went to a blocked site (${hostOf(tab.pendingUrl || tab.url || "")}): hand it to the owner`);
  }
  if (method === "Page.bringToFront") return {}; // the owner's tab and window stay in front: Emberhut works where its tab is
  const url = record(params).url;
  if (method === "Page.navigate" && typeof url === "string" && (await blockedUrl(url))) {
    throw new Error(`${hostOf(url)} is blocked in this Chrome: hand it to the owner`);
  }
  await refuseSecretInput(tabId, method, params);
  const result = await chrome.debugger.sendCommand({ tabId }, method, record(params));
  if (method === "Input.dispatchMouseEvent") pointTo(tabId, params);
  return result;
}

async function create(url: unknown): Promise<object> {
  const address = typeof url === "string" && url !== "" ? url : "about:blank";
  if (await blockedUrl(address)) throw new Error(`${hostOf(address)} is blocked in this Chrome`);
  // In the window that already has Emberhut's group (one per profile), else the owner's last one; behind the owner's own
  // tab, never in front, never a new window.
  const [group] = await chrome.tabGroups.query({ title: GROUP_TITLE });
  const tab = await chrome.tabs.create({ url: address, active: false, ...(group ? { windowId: group.windowId } : {}) });
  if (tab.id === undefined) throw new Error("Chrome gave the new tab no id");
  made.add(tab.id);
  urls.set(tab.id, tab.pendingUrl || tab.url || "");
  await joinGroup(tab.id).catch((e: unknown) => step(`Couldn't put a tab in Emberhut's group: ${messageOf(e)}`));
  reported.add(tab.id);
  return slim(await chrome.tabs.get(tab.id));
}

async function remove(tabId: number): Promise<object> {
  const tab = await chrome.tabs.get(tabId).catch(() => null);
  if (!tab || tab.id === undefined || !reported.has(tab.id)) throw new Error("Not one of Emberhut's tabs");
  closing.set(tab.id, { by: BY_ASSISTANT, at: Date.now() });
  await chrome.tabs.remove(tab.id);
  return {};
}
