// The Emberhut extension's service worker, one per Chrome profile: it wires Chrome's events to the modules beside it.
// What Emberhut may touch is decided here, on the owner's machine: only its own tabs (the "Emberhut" tab group, a tab it
// opened, or any popup one of them opened), never a blocked site, never DevTools commands that reach past the page,
// never a password typed or filled in. Once paired, Emberhut may work here any time; the owner's brake is Pause (the pill, the
// popup, the app), which lets go of every tab and comes back by itself after 10 minutes, or when the owner hands
// back. Cancel on Chrome's own "started debugging this browser" bar is the owner saying stop, so it pauses Emberhut
// the same way.

import { aboutOwnWorld, forwards, stripCookies } from "../cdp-rules";
import { inBackground, messageOf } from "../log";
import { CLOSING_BINDING, PILL_BINDING, PILL_WORLD } from "../page-scripts";
import { BY_ASSISTANT } from "../protocol";
import { hostOf, isNewTabPage } from "../scope";
import type { Answer, PopupAsk } from "../status";
import { badge } from "./badge";
import { adopted, attached, closing, dropped, forget, mainFrame, noteOwnContext, ownContexts, pillWorld, reported, urls } from "./held";
import { TICK, connect, tick } from "./link";
import { pairFromTab, pairTabs } from "./pairing-flow";
import { UNPAUSE_ALARM, pause, tidyPause, unpause } from "./pause-control";
import { onPopup } from "./popup-api";
import { cut } from "./recovery";
import { step } from "./state";
import { blockedUrl, joinGroup, leaveBlocked, sync } from "./tabs";
import { maybeReload } from "./update";
import { emit } from "./wire";

const RECONNECT_ALARM = "reconnect";

chrome.debugger.onEvent.addListener((source, method, params) => {
  const p = (params ?? {}) as { name?: string; context?: { id: number; name: string; auxData?: { frameId?: string } } };
  const tabId = source.tabId;
  if (tabId === undefined) return;
  if (method === "Runtime.bindingCalled" && p.name === PILL_BINDING) {
    inBackground(pause("you pressed Pause on the page"), "pill pause");
    return;
  }
  if (method === "Runtime.bindingCalled" && p.name === CLOSING_BINDING) {
    if (!source.sessionId) closing.set(tabId, { by: "page", at: Date.now() });
    return;
  }
  if (method === "Inspector.targetCrashed" && !source.sessionId) closing.set(tabId, { by: "crash", at: Date.now() });
  if (method === "Runtime.executionContextCreated" && !source.sessionId && p.context?.name === PILL_WORLD) {
    noteOwnContext(tabId, p.context.id);
    if (p.context.auxData?.frameId === mainFrame.get(tabId)) pillWorld.set(tabId, p.context.id); // the page navigated
  }
  // Only the page's own events go back, none about Emberhut's own world, and never a cookie in them (cdp-rules.ts).
  const own = ownContexts.get(tabId) ?? new Set<number>();
  if (attached.has(tabId) && forwards(method, source.sessionId) && !aboutOwnWorld(method, params, PILL_WORLD, own)) {
    emit("chrome.debugger.onEvent", [{ tabId }, method, stripCookies(params)]);
  }
});

// A navigation of one of Emberhut's tabs (or a frame in it) to a blocked site: Emberhut lets go before it commits. A frame
// another extension puts in the page (a password manager's menu) isn't a site: Chrome cuts the link for it, and the
// tab is offered again (recovery.ts).
chrome.webNavigation.onBeforeNavigate.addListener((details) => {
  if (!attached.has(details.tabId) && !reported.has(details.tabId)) return;
  if (details.frameId !== 0 && !/^(https?|data|blob):/i.test(details.url)) return;
  inBackground(
    blockedUrl(details.url).then((blocked) => (blocked ? leaveBlocked({ id: details.tabId }, details.url) : undefined)),
    "blocked navigation",
  );
});

chrome.debugger.onDetach.addListener((source, reason) => {
  const tabId = source.tabId;
  if (tabId === undefined) return;
  dropped.set(tabId, reason);
  if (!attached.delete(tabId)) return;
  emit("chrome.debugger.onDetach", [source, reason]); // its session is gone either way
  inBackground(badge(), "badge");
  if (reason === "canceled_by_user") inBackground(pause("you pressed Cancel on Chrome's debugging bar"), "debugging bar");
  if (reason === "target_closed") inBackground(cut(tabId), "cut");
});

// A popup (or any tab) one of Emberhut's tabs opened is Emberhut's too, a sign-in window among them; not a new tab the owner
// opened while one of Emberhut's was in front (Chrome gives it that tab as its opener too).
chrome.tabs.onCreated.addListener((tab) => {
  if (tab.id === undefined || tab.openerTabId === undefined || !reported.has(tab.openerTabId)) return;
  if (isNewTabPage(tab.pendingUrl || tab.url || "")) return;
  const tabId = tab.id;
  adopted.add(tabId);
  inBackground(
    joinGroup(tabId)
      .catch(() => undefined) // a popup window has no tab strip: it stays adopted only
      .then(() => chrome.tabs.get(tabId))
      .then(sync),
    "adopt",
  );
});

// Any tab in Emberhut's group is Emberhut's, whoever put it there; dragged out, or the group renamed, it's the owner's again.
chrome.tabs.onUpdated.addListener((_id, change, tab) => {
  if (change.url || "groupId" in change) inBackground(sync(tab), "sync");
  if (change.url) inBackground(pairFromTab(tab), "pairing link");
});
chrome.tabs.onAttached.addListener((id) => inBackground(chrome.tabs.get(id).then(sync), "sync"));
const syncGroup = async (group: chrome.tabGroups.TabGroup): Promise<void> => {
  for (const tab of await chrome.tabs.query({ groupId: group.id })) await sync(tab);
};
chrome.tabGroups.onCreated.addListener((group) => inBackground(syncGroup(group), "group"));
chrome.tabGroups.onUpdated.addListener((group) => inBackground(syncGroup(group), "group"));

// Who closed one of Emberhut's tabs, so the assistant doesn't open again a tab the page keeps closing: the page itself,
// Emberhut, a crash, its window, or Chrome (the owner, or another extension).
chrome.tabs.onRemoved.addListener((id, info) => {
  const said = closing.get(id);
  const url = urls.get(id) ?? "";
  const by = said && Date.now() - said.at < 5_000 ? said.by : info.isWindowClosing ? "window" : "chrome";
  forget(id);
  if (!reported.delete(id)) return;
  emit("chrome.tabs.onRemoved", [id, { by, url }]);
  if (by !== BY_ASSISTANT) {
    inBackground(step(`${by === "page" ? "The page closed Emberhut's tab itself" : "Emberhut's tab was closed"} (${hostOf(url)})`), "step");
  }
});

chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === RECONNECT_ALARM) {
    inBackground(connect(), "connect");
    inBackground(maybeReload(), "update");
  }
  if (alarm.name === UNPAUSE_ALARM) inBackground(unpause("10 minutes are up"), "unpause");
});

chrome.runtime.onMessage.addListener((ask: PopupAsk, _sender, reply: (answer: Answer) => void) => {
  onPopup(ask).then(reply, (e: unknown) => reply({ refused: messageOf(e) }));
  return true;
});

chrome.runtime.onStartup.addListener(() => inBackground(connect(), "connect"));
// Added from the pairing page: it's open in a tab.
chrome.runtime.onInstalled.addListener(() => inBackground(pairTabs().then(connect), "installed"));

inBackground(chrome.alarms.create(RECONNECT_ALARM, { periodInMinutes: 0.5 }), "alarm");
setInterval(tick, TICK);
inBackground(tidyPause().then(pairTabs).then(connect).then(badge), "start");
