// The server's named messages once the profile is known (the hello and the pairing are link.ts's).

import { BY_ASSISTANT, INITIALIZED, Msg } from "../protocol";
import { pauseLength, pausedNow } from "../pause";
import { badge } from "./badge";
import { pause, unpause } from "./pause-control";
import { setSession, setSettings, settings, step } from "./state";
import { letGoAll, syncAll } from "./tabs";
import { noteUpdate } from "./update";
import { emit } from "./wire";

const ICON = "icons/emberhut-128.png";

const text = (value: unknown): string | null => (typeof value === "string" && value !== "" ? value : null);

/** Acts on one named message from the server, `params` as sent. */
export async function onServerMessage(method: string, params: unknown): Promise<void> {
  const p = (typeof params === "object" && params !== null ? params : {}) as Record<string, unknown>;
  switch (method) {
    case Msg.start: // the server's relay is fresh: Emberhut's tabs again, from the start
      await letGoAll("restart");
      await syncAll();
      emit(INITIALIZED, {});
      return;
    case Msg.resume: // the server's side: its time is up, or the owner's turn ended
      return unpause(text(p.why) ?? "the pause is over");
    case Msg.handoff:
      return handoff(text(p.reason) ?? "It needs you");
    case Msg.handback:
      return handback(text(p.by), text(p.reason));
    case Msg.step:
      return step(text(p.text) ?? "");
    case Msg.name: // the owner renamed it in the app
      return setSettings({ name: text(p.name), label: text(p.label) ?? text(p.name) });
    case Msg.pause: // from the app; minutes: a shorter pause (1-10)
      return pause(text(p.why) ?? "you paused it from the Emberhut app", pauseLength(p.minutes));
    case Msg.update:
      return noteUpdate(p.version);
  }
}

async function handoff(reason: string): Promise<void> {
  await setSession({ handoff: reason, notice: null });
  await chrome.notifications.create("handoff", {
    type: "basic",
    iconUrl: ICON,
    title: "Emberhut needs you",
    message: `${reason}. When you're done, hand back from Emberhut's icon.`,
    priority: 2,
    requireInteraction: true,
  });
  await step(`Handed to you: ${reason}`);
  await badge();
}

/** The owner handed back from the app, or Emberhut took the browser back (by: the assistant). */
async function handback(by: string | null, reason: string | null): Promise<void> {
  await setSession({ handoff: null });
  await chrome.notifications.clear("handoff");
  const tookBack = by === BY_ASSISTANT;
  const paused = pausedNow((await settings()).paused, Date.now());
  if (paused && (!tookBack || paused.turn)) await unpause(tookBack ? "your turn ended" : "you handed back");
  if (tookBack) {
    const line = `Emberhut took it back${reason ? `: ${reason}` : ""}`;
    await setSession({ notice: { at: Date.now(), text: line } });
    await chrome.notifications.create("takeback", {
      type: "basic",
      iconUrl: ICON,
      title: "Emberhut took it back",
      message: reason || "It carries on from here.",
      priority: 0,
    });
    await step(line);
  }
  await badge();
}
