// The owner's brake: Pause lets go of every tab at once and keeps Emberhut out for 10 minutes (or less, from the app), or
// until the owner hands back; then it comes back by itself. Nothing has to be pressed to resume.

/** The longest pause, and the one the popup and the page's pill give. */
export const PAUSE_MS = 10 * 60_000;

/** A pause as kept (across Chrome restarts): why, until when (ms), and whether it was pressed during the owner's turn. */
export interface Pause {
  why: string;
  until: number;
  /** Pressed while the owner had the browser from a hand-off: it ends when Emberhut takes the browser back too. */
  turn: boolean;
}

/** The pause still on at `now`, or null. */
export function pausedNow(pause: Pause | null | undefined, now: number): Pause | null {
  return pause && pause.until > now ? pause : null;
}

/** A new pause for `ms` from `now`; a pause pressed during the owner's turn (or over one that was) keeps that. */
export function newPause(why: string, ms: number, now: number, handoff: boolean, was: Pause | null): Pause {
  return { why, until: now + ms, turn: handoff || (was?.turn ?? false) };
}

/** A pause the app asked for, in ms: 1 to 10 minutes, 10 when it said none. */
export function pauseLength(minutes: unknown): number {
  const asked = Number(minutes);
  const whole = Number.isFinite(asked) && asked > 0 ? asked : 10;
  return Math.min(10, Math.max(1, whole)) * 60_000;
}

/** A time of day, as the popup and the steps show it. */
export function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/** Whether a stored value is a pause. */
export function isPause(value: unknown): value is Pause {
  if (typeof value !== "object" || value === null) return false;
  const p = value as Record<string, unknown>;
  return typeof p.why === "string" && typeof p.until === "number" && typeof p.turn === "boolean";
}
