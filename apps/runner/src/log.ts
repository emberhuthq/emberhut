// The runner's log: one line per event on stdout, with the time first (the service manager keeps or rotates it).

/** Writes one log line, its parts joined by spaces. */
export function log(...parts: unknown[]): void {
  const text = parts.map((p) => (typeof p === "string" ? p : p instanceof Error ? p.message : JSON.stringify(p))).join(" ");
  process.stdout.write(`${new Date().toISOString()} ${text}\n`);
}

/** Runs a promise nobody waits for, logging its failure under `what` instead of leaving it unhandled. */
export function inBackground(what: string, p: Promise<unknown>): void {
  p.catch((e: unknown) => log(`${what} failed:`, e instanceof Error ? e.message : String(e)));
}

/** The message of a thrown value, whatever was thrown. */
export const messageOf = (e: unknown): string => (e instanceof Error ? e.message : String(e));
