// Where a failure nobody waits for goes: work started from a Chrome event or a timer has no caller to hand its
// rejection to.

/** What a failure is written to: the service worker's console, or a stand-in in the tests. */
export type LogSink = (where: string, cause: unknown) => void;

// eslint-disable-next-line no-console -- the extension's one way to its service worker's console (chrome://extensions, Inspect)
const toConsole: LogSink = (where, cause) => console.warn(`[emberhut] ${where}:`, cause);

let sink: LogSink = toConsole;

/** Writes down a failure that has no one to show it to, with where it happened. */
export function logError(where: string, cause: unknown): void {
  sink(where, cause);
}

/** Starts work nobody awaits (an event, a timer): its rejection is logged, never left unhandled. */
export function inBackground(work: Promise<unknown>, where: string): void {
  work.catch((cause: unknown) => logError(where, cause));
}

/** The tests' way to see what was logged; returns the sink to put back. */
export function setLogSink(next: LogSink): LogSink {
  const before = sink;
  sink = next;
  return before;
}

/** An error's words. */
export function messageOf(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}
