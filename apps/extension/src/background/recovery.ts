// When Chrome cuts Emberhut's link to a tab that's still there, or won't let Emberhut in yet, the tab is offered to the server
// again (which attaches it afresh) a few times, and the server hears why.

import { Msg } from "../protocol";
import { hostOf } from "../scope";
import { pausedNow } from "../pause";
import { attached, framing, reported } from "./held";
import { settings, step } from "./state";
import { isEmberhut, slim } from "./tabs";
import { emit, ready } from "./wire";

const wait = (ms: number): Promise<void> => new Promise((done) => setTimeout(done, ms));

/** Chrome's words when another extension's frame is in the page (a password manager's fill-in menu, say). */
export const FRAMED = /chrome-extension:\/\/|different extension/i;

/** One of Emberhut's tabs, not attached, offered to the server again as it is now. */
export async function offer(tabId: number): Promise<boolean> {
  const tab = await chrome.tabs.get(tabId).catch(() => null);
  if (!tab || !ready() || attached.has(tabId) || !reported.has(tabId) || pausedNow((await settings()).paused, Date.now())) return false;
  if (!(await isEmberhut(tab))) return false;
  emit("chrome.tabs.onCreated", [slim(tab)]);
  return true;
}

const reattached = new Map<number, number[]>();

/**
 * Chrome cut the link to a tab that is still there (another extension's frame came into the page): the tab is offered
 * again, at most 3 times a minute; past that it is attached again when the assistant next uses it. (Cancel on Chrome's
 * debugging bar is not this: it pauses Emberhut, main.ts.)
 */
export async function reattach(tabId: number): Promise<void> {
  const now = Date.now();
  const times = (reattached.get(tabId) ?? []).filter((t) => now - t < 60_000);
  if (times.length >= 3) {
    await step("Chrome keeps cutting Emberhut's link to its tab: it attaches again when Emberhut next uses the tab");
    return;
  }
  reattached.set(tabId, [...times, now]);
  await wait(500 * (times.length + 1));
  if (await offer(tabId)) await step("Chrome cut Emberhut's link to its tab: attached again");
}

/**
 * "target_closed" with the tab still there: Chrome cut the link, the tab didn't close (another extension's frame came
 * into the page). The server hears the tab is still open, and it is offered again as the frame may go in a moment.
 */
export async function cut(tabId: number): Promise<void> {
  await wait(500); // a tab that's closing is gone by then (its onRemoved says who)
  const tab = await chrome.tabs.get(tabId).catch(() => null);
  if (!tab || !reported.has(tabId)) return;
  const url = tab.url || tab.pendingUrl || "";
  emit(Msg.cut, { tabId, url });
  await step(`Chrome cut Emberhut's link to its tab (${hostOf(url)}): another extension's frame in the page`);
  for (const ms of [0, 1_000, 2_500]) {
    await wait(ms);
    if (attached.has(tabId)) return;
    await reattach(tabId);
  }
}

/**
 * Chrome lets no extension attach to a page holding another one's frame. The server hears the tab is there and why;
 * it's offered again after 1, 2.5, 5 and 10 s, then on every tick, until Chrome lets Emberhut in, it closes, or it leaves
 * the group.
 */
export async function framed(tab: chrome.tabs.Tab): Promise<void> {
  if (tab.id === undefined || framing.has(tab.id)) return;
  const tabId = tab.id;
  framing.set(tabId, Date.now());
  const url = tab.url || tab.pendingUrl || "";
  emit(Msg.framed, { tabId, url });
  await step(`Chrome won't let Emberhut into its tab yet (${hostOf(url)}): another extension's frame in the page`);
  for (const ms of [1_000, 2_500, 5_000, 10_000]) {
    await wait(ms);
    if (!framing.has(tabId) || attached.has(tabId)) return;
    await offer(tabId);
  }
}
