// What the runner tells its server: every change of a session as an event, in order. An event waits in the outbox
// until the server has taken it, so a server restart (or no server yet) loses nothing; a newer "what it's doing" line
// replaces an older one not yet sent. It also asks the server, every 10 minutes, for the MCP servers its sessions get.
import { inBackground, log } from "./log.ts";
import type { Store } from "./store.ts";
import { now } from "./store.ts";
import { EVENT, SERVER } from "./wire.ts";

/** How the runner reaches its server: its address and the runner's token, read when needed (pairing may set them later). */
export type ServerLink = { server: () => string | null; token: () => string | null; name: string; fetch?: typeof fetch };

/** Sends the outbox to the server, oldest first. */
export class Reporter {
  private flushing = false;
  private backoff = 0;
  private timer: NodeJS.Timeout | null = null;
  private mcp: Record<string, unknown> = {};
  private readonly store: Store;
  private readonly link: ServerLink;

  constructor(store: Store, link: ServerLink) {
    this.store = store;
    this.link = link;
  }

  private get fetch(): typeof fetch {
    return this.link.fetch ?? fetch;
  }

  /** Queues one event: the session as it is now (`session`), its kind, and what that kind carries. */
  tell(session: string, kind: string, body: Record<string, unknown>): void {
    if (kind === EVENT.activity) this.store.run("delete from outbox where session = ? and kind = ?", session, kind);
    this.store.run(
      "insert into outbox(session, kind, body) values (?, ?, ?)",
      session,
      kind,
      JSON.stringify({ machine: this.link.name, kind, at: now(), ...body }),
    );
    inBackground("sending events", this.flush());
  }

  /** How many events wait for the server. */
  pending(): number {
    return Number(this.store.one<{ n: number }>("select count(*) as n from outbox")?.n ?? 0);
  }

  /** Sends what waits; on a failure tries again later, waiting twice as long each time (30 s at most). */
  async flush(): Promise<void> {
    if (this.flushing) return;
    const server = this.link.server(),
      token = this.link.token();
    if (!server || !token) return;
    this.flushing = true;
    try {
      for (let e = this.next(); e; e = this.next()) {
        const r = await this.fetch(`${server}${SERVER.events}`, {
          method: "POST",
          body: e.body,
          signal: AbortSignal.timeout(15_000),
          headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
        }).catch(() => null);
        if (r && r.status === 400) {
          // The server will never take this one (not this runner's, or not an event): kept in the log, not retried forever.
          log(`the server refused an event (${await r.text().catch(() => "")}); dropped`);
        } else if (!r?.ok) {
          this.backoff = Math.min(30_000, this.backoff ? this.backoff * 2 : 2_000);
          this.retry();
          return;
        }
        this.backoff = 0;
        this.store.run("delete from outbox where id = ?", e.id);
      }
    } finally {
      this.flushing = false;
    }
  }

  private next() {
    return this.store.one<{ id: number; body: string }>("select id, body from outbox order by id limit 1");
  }
  private retry(): void {
    if (this.timer) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      inBackground("sending events", this.flush());
    }, this.backoff);
    this.timer.unref();
  }

  /** The MCP servers sessions get, as last heard from the server (kept in memory only). */
  sharedMcp(): Record<string, unknown> {
    return this.mcp;
  }

  /** Asks the server for the MCP servers sessions get; keeps the last answer when it can't be reached. */
  async loadMcp(): Promise<void> {
    const server = this.link.server(),
      token = this.link.token();
    if (!server || !token) return;
    const r = await this.fetch(`${server}${SERVER.mcp}`, {
      headers: { Authorization: `Bearer ${token}` },
      signal: AbortSignal.timeout(15_000),
    }).catch(() => null);
    if (!r?.ok) return;
    const body = (await r.json().catch(() => null)) as { mcpServers?: unknown } | null;
    if (body?.mcpServers && typeof body.mcpServers === "object" && !Array.isArray(body.mcpServers))
      this.mcp = body.mcpServers as Record<string, unknown>;
  }

  /** Stops the retry timer. */
  stop(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }
}
