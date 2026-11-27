// What the extension keeps: in local storage what outlives Chrome (the pairing, the owner's blocked sites, a pause, a
// pending update), in session storage what lasts while Chrome runs (a hand-off, a notice, the recent steps, a pairing
// offer). A restarted service worker finds both again.

import { isPause, type Pause } from "../pause";
import type { Offer, Step } from "../status";
import { isTried, type Tried } from "../version";

/** The local settings. */
export interface Settings {
  /** The server's address, from pairing; none before. */
  server: string | null;
  token: string | null;
  name: string | null;
  label: string | null;
  blocked: string[];
  paused: Pause | null;
  /** The version the folder holds, as the server said (emberhut.update). */
  update: string | null;
  /** The last reload tried for an update. */
  tried: Tried | null;
}

/** The session's state. */
export interface Session {
  handoff: string | null;
  notice: Step | null;
  steps: Step[];
  offer: Offer | null;
}

const text = (value: unknown): string | null => (typeof value === "string" && value !== "" ? value : null);
const texts = (value: unknown): string[] => (Array.isArray(value) ? value.filter((v): v is string => typeof v === "string") : []);
const isStep = (value: unknown): value is Step =>
  typeof value === "object" && value !== null && typeof (value as Step).at === "number" && typeof (value as Step).text === "string";
const isOffer = (value: unknown): value is Offer =>
  typeof value === "object" && value !== null && typeof (value as Offer).server === "string" && typeof (value as Offer).code === "string";

/** The local settings as kept. */
export async function settings(): Promise<Settings> {
  const s = await chrome.storage.local.get(["server", "token", "name", "label", "blocked", "paused", "update", "tried"]);
  return {
    server: text(s.server),
    token: text(s.token),
    name: text(s.name),
    label: text(s.label),
    blocked: texts(s.blocked),
    paused: isPause(s.paused) ? s.paused : null,
    update: text(s.update),
    tried: isTried(s.tried) ? s.tried : null,
  };
}

/** Changes local settings. */
export async function setSettings(change: Partial<Settings>): Promise<void> {
  await chrome.storage.local.set(change);
}

/** The session's state as kept. */
export async function session(): Promise<Session> {
  const s = await chrome.storage.session.get(["handoff", "notice", "steps", "offer"]);
  return {
    handoff: text(s.handoff),
    notice: isStep(s.notice) ? s.notice : null,
    steps: Array.isArray(s.steps) ? s.steps.filter(isStep) : [],
    offer: isOffer(s.offer) ? s.offer : null,
  };
}

/** Changes the session's state. */
export async function setSession(change: Partial<Session>): Promise<void> {
  await chrome.storage.session.set(change);
}

/** Adds a line to the popup's recent steps (the newest 40 are kept). */
export async function step(line: string): Promise<void> {
  const { steps } = await session();
  await setSession({ steps: [{ at: Date.now(), text: line.slice(0, 200) }, ...steps].slice(0, 40) });
}
