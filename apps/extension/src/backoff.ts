// When to connect again, and when a socket that looks open is dead. A socket that never got the server's hello makes
// the next try wait 20 s, 40 s, 80 s, then every 2 minutes, so a server that can't be reached (the computer just
// woke, the server restarting) isn't hammered; once in, the next try is at once.

/** The socket's ping cadence: it also keeps the service worker alive while connected. */
export const TICK_MS = 20_000;

/** When the next try may start after `fails` failed ones in a row (0: now). */
export function retryAt(fails: number, now: number): number {
  return fails ? now + Math.min(120_000, 10_000 * 2 ** fails) : 0;
}

/** A socket as the back-off sees it. */
export interface SocketLife {
  /** Still connecting, or open. */
  open: boolean;
  /** When it was made. */
  since: number;
  /** It answered a ping at least once (counting misses means something). */
  answers: boolean;
  /** Pings unanswered in a row. */
  missed: number;
}

/**
 * Whether a socket's close will never come and it must be let go: connecting for a minute, or three pings unanswered
 * by a server that answers them. Counted in pings, not time, so a computer that slept doesn't drop a socket that's fine.
 */
export function isStale(life: SocketLife, now: number): boolean {
  return life.open ? life.answers && life.missed >= 3 : now - life.since > 60_000;
}

/**
 * Whether the computer slept since the last tick (the tick came a minute late). The socket's connection died with it
 * while Chrome still calls it open, and its close may not come for many minutes.
 */
export function slept(lastTick: number, now: number): boolean {
  return now - lastTick > 60_000;
}

/**
 * Whether a socket is old enough to be dropped after a sleep: only one from before it. The timer only runs while the
 * computer is awake, up to a tick after it wakes, so a socket opened since is fine, and the assistant may be using it.
 */
export function fromBeforeSleep(since: number, now: number): boolean {
  return now - since > TICK_MS;
}
