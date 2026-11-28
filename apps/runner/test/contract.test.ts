// The runner's side of the server's contract. Two checks on what a real runner sends (over HTTP, with a stand-in agent):
//
// 1. Its shapes against the server's fixtures (crates/emberhut-api/tests/fixtures/runner_*.json, machines.json,
//    jobs.json): every key it writes is one the contract knows for that kind of value, with a value of the same type.
// 2. A record of it, made stable (times, ids and folders fixed) and kept beside them in fixtures/runner/, which the Rust
//    contract tests (crates/emberhut-api/tests/contract/runner_recorded.rs) read with the server's own types. A change
//    to what the runner sends shows here as a difference; `EMBERHUT_BLESS=1` writes the record again.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { sample } from "../src/machine.ts";
import type { Runner } from "../src/runner.ts";
import { EVENT, LIVE, SESSION_ID } from "../src/wire.ts";
import { FIXTURES, fixture, standInAgent, standInServer, tempHome, testRunner, until, type Got } from "./support.ts";

type Json = Record<string, unknown>;
const TOKEN = "contract-token";
const RECORDED = new URL("runner/", FIXTURES);

// ── The shape check ───────────────────────────────────────────────────────────────────────────────────────────────

/** A JSON value's kind: null, a number, text, true/false, a list or an object. */
const kindOf = (v: unknown) => (v === null ? "null" : Array.isArray(v) ? "array" : typeof v);

/** The keys of `examples` (objects) and the kinds each key's values have. */
function keysOf(examples: Json[]): Map<string, Set<string>> {
  const m = new Map<string, Set<string>>();
  for (const e of examples) for (const [k, v] of Object.entries(e)) (m.get(k) ?? m.set(k, new Set()).get(k)!).add(kindOf(v));
  return m;
}

/** Every key of `ours` is one the examples have (or one of `alsoKnown`), with a value of a kind they have (null always fits). */
function expectShape(ours: Json, examples: Json[], what: string, alsoKnown: string[] = []): void {
  const known = keysOf(examples);
  for (const [k, v] of Object.entries(ours)) {
    if (alsoKnown.includes(k)) continue;
    expect(known.has(k), `${what}: "${k}" isn't a key the contract has`).toBe(true);
    const kinds = known.get(k)!;
    if (v !== null)
      expect(kinds.has(kindOf(v)) || kinds.has("null"), `${what}.${k}: a ${kindOf(v)}, the contract has ${[...kinds].join("/")}`).toBe(
        true,
      );
  }
}

// ── Making a record stable ────────────────────────────────────────────────────────────────────────────────────────

/** Fixes what changes from run to run: times, ids, folders, durations. */
function stable(v: unknown, folders: [string, string][]): unknown {
  if (typeof v === "string") {
    let s = v.replace(/\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?Z/g, "2026-01-01T00:00:00.000Z");
    s = s
      .replace(/r-[0-9a-f]{8}/g, "r-1a2b3c4d")
      .replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g, "00000000-0000-4000-8000-000000000000");
    for (const [from, to] of folders) s = s.split(from).join(to);
    return s;
  }
  if (Array.isArray(v)) return v.map((x) => stable(x, folders));
  if (v && typeof v === "object")
    return Object.fromEntries(
      Object.entries(v).map(([k, x]) => [
        k,
        ["ms", "tokens"].includes(k) && typeof x === "number" ? 0 : k === "seq" || k === "after" ? x : stable(x, folders),
      ]),
    );
  return v;
}

/** The record matches the committed one (or is written, with EMBERHUT_BLESS=1). */
function expectRecorded(name: string, value: unknown): void {
  const file = new URL(name, RECORDED);
  const text = JSON.stringify(value, null, 2) + "\n";
  if (process.env.EMBERHUT_BLESS === "1" || !existsSync(file)) {
    mkdirSync(new URL(".", file), { recursive: true });
    writeFileSync(file, text);
    if (process.env.EMBERHUT_BLESS !== "1") throw new Error(`${name} was missing and is written now; check it in`);
    return;
  }
  expect(JSON.parse(text), `${name} differs from the record (EMBERHUT_BLESS=1 writes it again)`).toEqual(
    JSON.parse(readFileSync(file, "utf8")),
  );
}

// ── One run of a runner, recorded ─────────────────────────────────────────────────────────────────────────────────

let runner: Runner;
let got: Got[];
let cleanup: () => Promise<void>;
let folders: [string, string][];
const seen: {
  transcript?: Json;
  live?: Json[];
  open?: Json;
  send?: Json;
  folders?: Json;
  inside?: Json;
  branches?: Json;
  settings?: Json;
} = {};
let call: (method: string, path: string, body?: unknown) => Promise<Json>;

beforeAll(async () => {
  const server = await standInServer(TOKEN);
  const h = tempHome({ server: server.url, token: TOKEN });
  const agent = standInAgent();
  runner = await testRunner(h, agent.query);
  got = server.got;
  const base = `http://127.0.0.1:${runner.port}`;
  call = async (method, path, body) => {
    const r = await fetch(base + path, {
      method,
      headers: { Authorization: `Bearer ${TOKEN}`, ...(body === undefined ? {} : { "Content-Type": "application/json" }) },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
    return (await r.json()) as Json;
  };
  const repo = join(h.work, "app");
  mkdirSync(join(repo, "docs"), { recursive: true });
  execFileSync("git", ["init", "-q", "-b", "main", repo]);
  execFileSync("git", ["-C", repo, "-c", "user.email=t@example.com", "-c", "user.name=T", "commit", "-q", "--allow-empty", "-m", "first"]);
  folders = [
    [h.home, "/home/example/.local/share/emberhut-runner"],
    [h.work, "/home/example/work"],
    [process.env.HOME ?? "/nonexistent", "/home/example"],
  ];

  // The owner's session, opened with no message (as with files), watched live from its start, then sent a turn.
  const opened = await call("POST", "/sessions", {
    cwd: repo,
    meta: { direct: true },
    by: "owner",
    title: "Tidy",
    account: "a-2",
    effort: "high",
  });
  const id = opened.id as string;
  seen.open = { ok: true, id, turn: opened.turn, session: opened };
  const live: Json[] = [];
  const ctrl = new AbortController();
  const stream = await fetch(`${base}/sessions/${id}/live?after=0`, { headers: { Authorization: `Bearer ${TOKEN}` }, signal: ctrl.signal });
  const reading = (async () => {
    const dec = new TextDecoder();
    let buf = "";
    try {
      for await (const chunk of stream.body as unknown as AsyncIterable<Uint8Array>) {
        buf += dec.decode(chunk, { stream: true });
        for (let i = buf.indexOf("\n\n"); i >= 0; i = buf.indexOf("\n\n")) {
          const line = buf.slice(0, i);
          buf = buf.slice(i + 2);
          if (line.startsWith("data: ")) live.push(JSON.parse(line.slice(6)) as Json);
        }
      }
    } catch {
      // Aborted at the end.
    }
  })();
  // A file first, as the server sends one with a message: it lands in the inbox and the transcript notes it.
  await fetch(`${base}/sessions/${id}/files?name=readme.md&by=owner&size=6`, {
    method: "PUT",
    headers: { Authorization: `Bearer ${TOKEN}` },
    body: "# Tidy",
  });
  seen.send = { ok: true, ...(await call("POST", `/sessions/${id}/send`, { prompt: "Tidy the readme", by: "owner" })) };
  await until(() => got.some((g) => g.body.kind === EVENT.turnFinished));
  await until(() => live.some((e) => e.type === LIVE.step && (e.step as Json).kind === "end"), "the end step on the live stream");
  ctrl.abort();
  await reading;
  seen.live = live;
  // A question and a note too, so the transcript has every kind of step.
  await call("POST", `/sessions/${id}/send`, { prompt: "ASK", by: "owner" });
  await until(() => got.filter((g) => g.body.kind === EVENT.turnFinished).length >= 2);
  seen.transcript = await call("GET", `/sessions/${id}/transcript?after=&limit=`);
  seen.settings = await call("POST", `/sessions/${id}/settings`, { effort: "max" });
  seen.folders = await call("GET", "/folders");
  seen.inside = await call("GET", `/folders?path=${encodeURIComponent(repo)}`);
  seen.branches = await call("GET", `/branches?cwd=${encodeURIComponent(repo)}`);
  cleanup = async () => {
    await runner.close();
    await server.close();
    h.cleanup();
  };
});
afterAll(async () => cleanup());

describe("against the server's fixtures", () => {
  it("writes each kind of step with the contract's keys", () => {
    const theirs = fixture<{ session: Json; events: Json[] }>("runner_transcript.json");
    const ours = seen.transcript!.events as Json[];
    const kinds = new Set(ours.map((s) => s.kind));
    for (const k of ["user", "thinking", "text", "tool", "result", "todos", "question", "end", "system"])
      expect(kinds, `a ${k} step`).toContain(k);
    for (const step of ours) {
      const examples = theirs.events.filter((e) => e.kind === step.kind);
      expect(examples.length, `the fixture has a ${String(step.kind)} step`).toBeGreaterThan(0);
      // `parent` (inside a sub-agent) and `id` (the tool call) are keys of every step in the contract; the fixture shows
      // them on some.
      expectShape(step, examples, `step ${String(step.kind)}`, ["parent", "id"]);
    }
  });

  it("writes the transcript's head and the live head with the contract's keys", () => {
    const theirs = fixture<{ session: Json }>("runner_transcript.json").session;
    const liveHead = (fixture<Json[]>("runner_live.json").find((e) => e.type === LIVE.head)!.session as Json) ?? {};
    expectShape(seen.transcript!.session as Json, [theirs, liveHead], "transcript head");
    for (const e of seen.live!.filter((x) => x.type === LIVE.head)) expectShape(e.session as Json, [theirs, liveHead], "live head");
    expectShape(seen.settings!, [theirs, liveHead], "settings answer");
    expect(seen.transcript!.earlier).toBe(false);
  });

  it("sends only the live stream's kinds of event, each with the contract's keys", () => {
    const theirs = fixture<Json[]>("runner_live.json");
    const types = new Set(theirs.map((e) => e.type));
    for (const e of seen.live!) {
      expect(types, `live event ${String(e.type)}`).toContain(e.type);
      expectShape(
        e,
        theirs.filter((x) => x.type === e.type),
        `live ${String(e.type)}`,
      );
    }
    expect(new Set(seen.live!.map((e) => e.type))).toEqual(new Set([LIVE.head, LIVE.step, LIVE.delta, LIVE.think, LIVE.tokens]));
  });

  it("answers a new session, a reply and the pickers in the contract's shapes", () => {
    const open = fixture<Json>("runner_open_reply.json");
    expect(seen.open!.id).toMatch(SESSION_ID);
    expect(seen.open!.turn).toBeNull();
    expectShape(
      seen.open!.session as Json,
      [open.session as Json, fixture<{ session: Json }>("runner_transcript.json").session],
      "open answer",
    );
    expectShape(seen.send!, [fixture<Json>("runner_send_reply.json")], "send answer");
    expect(seen.send!.turn).toMatch(/^r-[0-9a-f]{8}:1$/);
    expectShape(seen.folders!, [fixture<Json>("runner_folders.json")], "folders");
    expectShape((seen.folders!.recent as Json[])[0]!, (fixture<Json>("runner_folders.json").recent as Json[]) ?? [], "recent folder");
    expectShape(seen.inside!, [fixture<Json>("runner_folders_inside.json")], "a folder's folders");
    expectShape(seen.branches!, [fixture<Json>("runner_branches.json")], "branches");
  });

  it("reports events the server reads: its name, the session as it is, and each kind's fields", () => {
    const events = got.filter((g) => g.path === "/api/runner/events").map((g) => g.body);
    for (const e of events) {
      expect(e.machine).toBe("test-runner");
      expect((e.session as Json).id).toMatch(SESSION_ID);
      expect(Object.values(EVENT)).toContain(e.kind);
    }
    const finished = events.find((e) => e.kind === EVENT.turnFinished)!;
    expect(finished.turn).toMatchObject({ outcome: "completed", last: expect.stringContaining("STATUS: done") as unknown });
    const queued = events.find((e) => e.kind === EVENT.turnQueued)!.turn as Json;
    expect(queued.by).toBe("owner");
    expect(queued.prompt).toMatch(/^Tidy the readme\n\n\(Files sent to you since your last turn/);
    expect(events.find((e) => e.kind === EVENT.question)!.text).toBe("Which one?");
  });

  it("samples the machine in the shapes the Machines page reads", async () => {
    const sampled = await sample("test-runner", runner.sessions.summary());
    const page = fixture<{ machines: Json[] }>("machines.json").machines[0]!;
    if (sampled.cpu) expectShape(sampled.cpu as Json, [page.cpu as Json], "cpu", ["load5", "load15"]); // the server reads load5 for its rules
    if (sampled.memory) expectShape(sampled.memory as Json, [page.memory as Json], "memory", ["psi"]);
    for (const d of (sampled.disks as Json[] | undefined) ?? []) expectShape(d, page.disks as Json[], "disk");
  });
});

describe("the record the Rust contract tests read", () => {
  it("is what the runner sends now", () => {
    expectRecorded("transcript.json", stable(seen.transcript, folders));
    expectRecorded("live.json", stable(seen.live, folders));
    expectRecorded("open_reply.json", stable(seen.open, folders));
    expectRecorded("send_reply.json", stable(seen.send, folders));
    expectRecorded("settings_reply.json", stable(seen.settings, folders));
    expectRecorded("folders.json", stable({ ...seen.folders, projects: [] }, folders));
    expectRecorded("folders_inside.json", stable(seen.inside, folders));
    expectRecorded("branches.json", stable(seen.branches, folders));
    expectRecorded(
      "events.json",
      stable(
        got.filter((g) => g.path === "/api/runner/events").map((g) => g.body),
        folders,
      ),
    );
  });
});
