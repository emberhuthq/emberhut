// Where a failure nobody waits for goes: work started from a tap or an effect has no caller to hand its rejection to.

/** What a failure is written to: the developer console in the app, a stand-in in the tests. */
export type LogSink = (where: string, cause: unknown) => void;

const toConsole: LogSink = (where, cause) => {
  // Release builds keep nothing: the app has no crash reporting, and its logs would carry the user's data.
  if (typeof __DEV__ !== "undefined" && __DEV__) {
    // eslint-disable-next-line no-console -- the app's one way to the developer console (Metro shows it), debug builds only
    console.warn(`[emberhut] ${where}:`, cause);
  }
};

let sink: LogSink = toConsole;

/** Writes down a failure that has no one to show it to, with where it happened. */
export function logError(where: string, cause: unknown): void {
  sink(where, cause);
}

/**
 * Starts work nobody awaits (a tap, an effect, a timer): its rejection is logged, never left unhandled. Work that has a
 * screen to tell handles its own failure first; this is the net under it.
 */
export function inBackground(work: Promise<unknown>, where: string): void {
  work.catch((cause: unknown) => logError(where, cause));
}

/** The tests' way to see what was logged; returns the sink to put back. */
export function setLogSink(next: LogSink): LogSink {
  const before = sink;
  sink = next;
  return before;
}
