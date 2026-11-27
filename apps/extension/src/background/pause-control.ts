// The owner's pause, as the worker applies it: every tab let go at once, the server told, and back by itself.

import { Msg } from "../protocol";
import { PAUSE_MS, clock, newPause, pausedNow } from "../pause";
import { badge } from "./badge";
import { session, setSettings, settings, step } from "./state";
import { letGoAll, syncAll } from "./tabs";
import { emit } from "./wire";

/** The alarm that ends a pause. */
export const UNPAUSE_ALARM = "unpause";

/** Pauses Emberhut in this Chrome for `ms`, saying `why`. */
export async function pause(why: string, ms = PAUSE_MS): Promise<void> {
  const now = Date.now();
  const was = pausedNow((await settings()).paused, now);
  const { handoff } = await session();
  const paused = newPause(why, ms, now, handoff !== null, was);
  await setSettings({ paused });
  await chrome.alarms.create(UNPAUSE_ALARM, { when: paused.until });
  await letGoAll("paused");
  emit(Msg.stopped, paused);
  await step(`Paused Emberhut (${why}): back by itself at ${clock(paused.until)}`);
  await badge();
}

/** Ends the pause (its time is up, or the owner handed back), saying `why`. */
export async function unpause(why: string): Promise<void> {
  const { paused } = await settings();
  await chrome.alarms.clear(UNPAUSE_ALARM);
  if (!paused) return;
  await setSettings({ paused: null });
  emit(Msg.stopped, null);
  await step(`Emberhut is back: ${why}`);
  await syncAll();
  await badge();
}

/** At start: a pause past its time is over; one still on gets its alarm again. */
export async function tidyPause(): Promise<void> {
  const { paused } = await settings();
  if (paused && paused.until <= Date.now()) await setSettings({ paused: null });
  else if (paused) await chrome.alarms.create(UNPAUSE_ALARM, { when: paused.until });
}
