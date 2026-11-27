// The toolbar icon: its badge (ON while Emberhut drives, II paused, ! when the owner is needed) and its title.

import { clock, pausedNow } from "../pause";
import { attached } from "./held";
import { session, settings } from "./state";
import { ready } from "./wire";

/** Draws the icon as things are now. */
export async function badge(): Promise<void> {
  const { handoff } = await session();
  const paused = pausedNow((await settings()).paused, Date.now());
  const on = attached.size > 0;
  await chrome.action.setBadgeText({ text: handoff ? "!" : paused ? "II" : on ? "ON" : "" });
  await chrome.action.setBadgeBackgroundColor({ color: handoff ? "#f59e0b" : paused ? "#6b7280" : "#2563eb" });
  await chrome.action.setTitle({
    title: !ready()
      ? "Emberhut: not connected"
      : handoff
        ? `Emberhut needs you: ${handoff}`
        : paused
          ? `Emberhut: paused, back by itself at ${clock(paused.until)}`
          : on
            ? "Emberhut is driving in this Chrome"
            : "Emberhut: ready",
  });
}
