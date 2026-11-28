// The runner's HTTP API, which its server calls with the runner's token: open a session, send it a turn, stop or
// interrupt it, change its settings, read its transcript or watch it live, the folders and branches for a new
// session, the accounts and their use, the background jobs, files in and out, and the switched-off MCP servers and
// skills. `/health` answers without the token. A refusal is a status and `{ "error": "<words for a person>" }`.
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import { timingSafeEqual } from "node:crypto";
import { platform } from "node:os";
import type { Accounts } from "./accounts/index.ts";
import type { Files } from "./files.ts";
import * as jobs from "./jobs.ts";
import { log, messageOf } from "./log.ts";
import { branches, foldersIn, Refused, startFolders } from "./sessions/folders.ts";
import { Sessions } from "./sessions/engine.ts";
import type { Store, EventRow } from "./store.ts";
import { VERSION } from "./version.ts";
import { LIVE } from "./wire.ts";

/** What the API needs. */
export type ApiDeps = {
  name: string;
  token: () => string | null;
  store: Store;
  sessions: Sessions;
  accounts: Accounts;
  files: Files;
  machine: () => Record<string, unknown> | null;
  jobLabels: string | null;
  started: string;
};

const send = (res: ServerResponse, code: number, body: unknown): void => {
  res.writeHead(code, { "Content-Type": "application/json" }).end(JSON.stringify(body));
};
const same = (a: string, b: string) => a.length === b.length && timingSafeEqual(Buffer.from(a), Buffer.from(b));
/** The largest JSON body taken (a prompt with a long brief fits many times over). */
const MAX_BODY = 4 * 1024 * 1024;

async function body(req: IncomingMessage): Promise<Record<string, unknown>> {
  let raw = "";
  for await (const c of req as AsyncIterable<Buffer>) {
    raw += c.toString("utf8");
    if (raw.length > MAX_BODY) throw new Refused("the request is too big", 413);
  }
  if (!raw.trim()) return {};
  let v: unknown;
  try {
    v = JSON.parse(raw);
  } catch {
    throw new Refused("not JSON", 400);
  }
  if (!v || typeof v !== "object" || Array.isArray(v)) throw new Refused("not a JSON object", 400);
  return v as Record<string, unknown>;
}

/** A positive integer from a query value, or the default (empty, missing or not a number). */
const int = (v: string | null, fallback: number): number => {
  const n = Math.trunc(Number(v));
  return v && Number.isFinite(n) && n > 0 ? n : fallback;
};

/** Answers one request. */
export async function handle(d: ApiDeps, req: IncomingMessage, res: ServerResponse): Promise<void> {
  const url = new URL(req.url ?? "/", "http://runner"),
    p = url.pathname,
    m = req.method ?? "GET";
  if (p === "/health") return send(res, 200, { ok: true, machine: d.name, version: VERSION, running: d.sessions.runningCount() });
  const token = d.token();
  const given = req.headers.authorization?.replace(/^Bearer\s+/i, "") ?? "";
  if (!token || !same(given, token)) return send(res, 401, { error: token ? "not this runner's token" : "this runner isn't paired yet" });
  try {
    if (m === "GET" && p === "/info")
      return send(res, 200, {
        machine: d.name,
        version: VERSION,
        started: d.started,
        node: process.version,
        platform: platform(),
        running: d.sessions.runningCount(),
      });
    if (m === "POST" && p === "/sessions") return send(res, 200, d.sessions.open(await body(req)));
    if (m === "GET" && p === "/sessions")
      return send(res, 200, {
        sessions: d.store
          .all<{ id: string }>(
            "select id from sessions order by updated desc limit ?",
            Math.min(500, int(url.searchParams.get("limit"), 100)),
          )
          .map((s) => d.sessions.headOf(s.id)),
      });
    if (m === "GET" && p === "/machine") return send(res, 200, d.machine() ?? {});
    if (m === "POST" && p === "/off") return send(res, 200, { ok: true, ...d.sessions.setOff(await body(req)) });
    if (m === "GET" && p === "/accounts") return send(res, 200, { machine: d.name, accounts: d.accounts.summary() });
    if (m === "POST" && p === "/accounts/check") {
      await d.accounts.probeAll();
      return send(res, 200, { accounts: d.accounts.summary() });
    }
    if (m === "GET" && p === "/jobs") return send(res, 200, await jobs.scan(d.jobLabels));
    if (m === "POST" && p === "/jobs/stop") {
      const pid = Number((await body(req)).pid),
        job = (await jobs.runs()).find((x) => x.pid === pid);
      if (!job) return send(res, 404, { error: "no such run here (it ended, or it isn't a codex exec or claude -p run)" });
      process.kill(pid, "SIGTERM");
      return send(res, 200, { ok: true });
    }
    if (m === "GET" && p === "/files") return d.files.serve(url, res, send);
    if (m === "GET" && p === "/folders") {
      const path = url.searchParams.get("path");
      return send(res, 200, path ? foldersIn(path, d.name) : startFolders(d.store, d.name));
    }
    if (m === "GET" && p === "/branches") return send(res, 200, branches(url.searchParams.get("cwd") ?? ""));
    const s = p.match(/^\/sessions\/(r-[0-9a-f]+)(?:\/([a-z][a-z-]*))?$/);
    if (s) return await session(d, s[1]!, s[2], m, url, req, res);
    return send(res, 404, { error: "not found" });
  } catch (e) {
    if (!res.headersSent) send(res, e instanceof Refused ? e.code : 400, { error: messageOf(e) });
    else res.destroy();
  }
}

async function session(
  d: ApiDeps,
  id: string,
  action: string | undefined,
  m: string,
  url: URL,
  req: IncomingMessage,
  res: ServerResponse,
): Promise<void> {
  const s = d.store.session(id);
  if (!s) return send(res, 404, { error: `no session ${id} on ${d.name}` });
  if (m === "GET" && !action)
    return send(res, 200, {
      ...d.sessions.headOf(id),
      turns: d.store.all("select * from turns where session = ? order by n desc limit 50", id).map((t) => d.sessions.turnShape(t as never)),
    });
  if (m === "GET" && action === "transcript") {
    // after 0 (or empty): the newest `limit` steps and whether older ones exist; else the steps after it.
    const after = int(url.searchParams.get("after"), 0),
      limit = Math.min(1000, int(url.searchParams.get("limit"), 400));
    const rows = after
      ? d.store.all<EventRow>("select * from events where session = ? and seq > ? order by seq limit ?", id, after, limit)
      : d.store.all<EventRow>("select * from (select * from events where session = ? order by seq desc limit ?) order by seq", id, limit);
    const first = rows[0];
    return send(res, 200, {
      session: { ...d.sessions.shape(s), running: d.sessions.running(id) },
      events: rows.map((e) => Sessions.stepOf(e)),
      earlier: !after && !!first && !!d.store.one("select 1 from events where session = ? and seq < ?", id, first.seq),
    });
  }
  if (m === "GET" && action === "live") return live(d, id, url, res);
  if (m === "PUT" && action === "files") {
    const got = await d.files.receive(s, url, req);
    d.sessions.note(id, null, "system", { text: `File from ${url.searchParams.get("by") || "the server"}: ${got.path}` });
    return send(res, 200, got);
  }
  if (m === "POST" && action === "send") {
    const b = await body(req);
    return send(res, 200, d.sessions.send(id, typeof b.prompt === "string" ? b.prompt : "", b.by, b.files));
  }
  if (m === "POST" && action === "settings") return send(res, 200, d.sessions.settings(id, await body(req)));
  if (m === "POST" && action === "interrupt") return send(res, 200, await d.sessions.interrupt(id));
  if (m === "POST" && action === "stop") return send(res, 200, await d.sessions.interrupt(id, true));
  return send(res, 404, { error: "not found" });
}

/** The live stream: the head, the steps after the watcher's last, what's being typed and thought, then every change. */
function live(d: ApiDeps, id: string, url: URL, res: ServerResponse): void {
  res.writeHead(200, { "Content-Type": "text/event-stream", "Cache-Control": "no-cache", "X-Accel-Buffering": "no" });
  const out = (x: object) => res.write(`data: ${JSON.stringify(x)}\n\n`);
  out({ type: LIVE.head, session: d.sessions.headOf(id) });
  for (const e of d.store.all<EventRow>(
    "select * from events where session = ? and seq > ? order by seq limit 2000",
    id,
    int(url.searchParams.get("after"), 0),
  ))
    out({ type: LIVE.step, step: Sessions.stepOf(e) });
  const typed = d.sessions.typing(id);
  if (typed?.text) out({ type: LIVE.delta, turn: typed.turn, start: true, text: typed.text });
  if (typed?.thinkingSince) out({ type: LIVE.think, turn: typed.turn, start: true, text: typed.thinking, since: typed.thinkingSince });
  d.sessions.watchers.add(id, res);
}

/** Starts the API on each listen address; resolves once all of them listen. */
export async function listen(d: ApiDeps, hosts: string[], port: number): Promise<Server[]> {
  const servers = hosts.map(() =>
    createServer({ requestTimeout: 30 * 60_000 }, (req, res) => {
      handle(d, req, res).catch((e: unknown) => {
        if (!res.headersSent) send(res, 500, { error: messageOf(e) });
      });
    }),
  );
  await Promise.all(
    servers.map(
      (s, i) =>
        new Promise<void>((ok, no) => {
          s.once("error", no);
          s.listen(port, hosts[i], () => {
            log(
              `emberhut-runner ${VERSION} (${d.name}) on http://${hosts[i]!.includes(":") ? `[${hosts[i]!}]` : hosts[i]!}:${(s.address() as { port: number }).port}`,
            );
            ok();
          });
        }),
    ),
  );
  return servers;
}
