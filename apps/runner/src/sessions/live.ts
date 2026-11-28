// Who watches a session live (`GET /sessions/{id}/live`, server-sent events): each new step, the words as they're
// typed and the session's head go to every open stream at once. Nothing is polled; a watcher that drops comes back with
// its last step and misses nothing.
import type { ServerResponse } from "node:http";

/** The open live streams, by session. */
export class Watchers {
  private readonly by = new Map<string, Set<ServerResponse>>();

  /** Whether anyone watches the session. */
  has(id: string): boolean {
    return !!this.by.get(id)?.size;
  }
  /** Sends one event to everyone watching the session. */
  send(id: string, payload: object): void {
    const set = this.by.get(id);
    if (!set?.size) return;
    const line = `data: ${JSON.stringify(payload)}\n\n`;
    for (const res of set) res.write(line);
  }
  /** Adds a stream; it's taken off when it closes, and kept open with a comment every 15 s. */
  add(id: string, res: ServerResponse, onClose: () => void = () => {}): void {
    let set = this.by.get(id);
    if (!set) this.by.set(id, (set = new Set()));
    set.add(res);
    const ping = setInterval(() => res.write(": ping\n\n"), 15_000);
    ping.unref();
    res.on("close", () => {
      clearInterval(ping);
      set.delete(res);
      onClose();
    });
  }
  /** Ends every stream. */
  closeAll(): void {
    for (const set of this.by.values()) for (const res of set) res.end();
    this.by.clear();
  }
}
