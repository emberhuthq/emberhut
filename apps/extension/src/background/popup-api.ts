// The popup's requests: everything it shows and does goes through the service worker.

import { readPairing } from "../pairing";
import { pausedNow } from "../pause";
import { BUILT_IN_BLOCKED, hostOf } from "../scope";
import type { PopupAsk, Status } from "../status";
import { Msg } from "../protocol";
import { badge } from "./badge";
import { attached, reported } from "./held";
import { linkError, pairWith, unpair } from "./link";
import { acceptOffer, dismissOffer } from "./pairing-flow";
import { pause, unpause } from "./pause-control";
import { session, setSession, setSettings, settings, step } from "./state";
import { blockedUrl, joinGroup, letGoAll, slim, syncAll } from "./tabs";
import { emit, ready } from "./wire";

const VERSION = chrome.runtime.getManifest().version;

/** Does what the popup asked, then answers with the status. */
export async function onPopup(ask: PopupAsk): Promise<Status> {
  switch (ask.kind) {
    case "pair": {
      const pair = readPairing(ask.code, ask.server);
      if (typeof pair === "string") throw new Error(pair);
      await pairWith(pair);
      await step(`Paired with ${pair.server}`);
      break;
    }
    case "accept":
      await acceptOffer();
      break;
    case "dismiss":
      await dismissOffer();
      break;
    case "unpair":
      await letGoAll("unpaired");
      await setSession({ handoff: null });
      await unpair();
      break;
    case "pause":
      await pause("you pressed Pause");
      break;
    case "handback":
      await setSession({ handoff: null, notice: null });
      await chrome.notifications.clear("handoff");
      emit(Msg.handback, {});
      await step("Handed back to Emberhut");
      await unpause("you handed back");
      await badge();
      break;
    case "share": {
      const [tab] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
      if (tab?.id === undefined) throw new Error("No tab in front");
      if (await blockedUrl(tab.url)) throw new Error("This site is on the blocked list");
      await joinGroup(tab.id);
      await step(`Shared a tab: ${hostOf(tab.url ?? "")}`);
      break;
    }
    case "settings":
      await setSettings({ blocked: ask.blocked });
      await syncAll();
      break;
    case "status":
      break;
  }
  return status();
}

/** The popup's view of things now. */
async function status(): Promise<Status> {
  const s = await settings();
  const x = await session();
  const tabs = await Promise.all([...reported].map((id) => chrome.tabs.get(id).catch(() => null)));
  return {
    paired: s.token !== null,
    name: s.name,
    label: s.label ?? s.name,
    server: s.server,
    version: VERSION,
    connected: ready(),
    error: ready() ? "" : linkError(),
    paused: pausedNow(s.paused, Date.now()),
    driving: attached.size > 0,
    handoff: x.handoff,
    notice: x.notice && Date.now() - x.notice.at < 10 * 60_000 ? x.notice.text : null, // Emberhut took it back: for a while
    steps: x.steps,
    blocked: s.blocked,
    defaults: BUILT_IN_BLOCKED,
    tabs: tabs.flatMap((tab) => (tab ? [slim(tab)] : [])).map(({ title, url }) => ({ title, url })),
    offer: x.offer,
  };
}
