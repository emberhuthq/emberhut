// The runner over its HTTP API, as its server drives it, with a stand-in agent and a stand-in server: sessions and
// turns, the transcript and live stream, stop, interrupt and settings, account moves, the owner's questions, files,
// the pickers, the accounts and the token.
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import type { Runner } from "../src/runner.ts";
import { standInAgent, standInServer, tempHome, testRunner, until, type Got } from "./support.ts";

const TOKEN = "test-token-0123456789abcdef";
type Json = Record<string, unknown>;

let cleanups: (() => Promise<void> | void)[] = [];
afterEach(async () => {
  for (const c of cleanups.reverse()) await c();
  cleanups = [];
});

/** A runner, its stand-in server and agent, and calls to its API with its token. */
async function setup(o: { config?: Json; accounts?: unknown[] } = {}) {
  const server = await standInServer(TOKEN);
  const h = tempHome({ server: server.url, token: TOKEN, ...o });
  const agent = standInAgent();
  const runner: Runner = await testRunner(h, agent.query);
  cleanups.push(h.cleanup, server.close, runner.close);
  const base = `http://127.0.0.1:${runner.port}`;
  const call = async (method: string, path: string, body?: unknown, token = TOKEN) => {
    const r = await fetch(base + path, {
      method,
      headers: { Authorization: `Bearer ${token}`, ...(body === undefined ? {} : { "Content-Type": "application/json" }) },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
    return { status: r.status, body: (await r.json().catch(() => ({}))) as Json };
  };
  const events = (kind?: string) => server.got.filter((g) => g.path === "/api/runner/events" && (!kind || g.body.kind === kind));
  const finished = (id: string, n = 1) => events("turn_finished").filter((e) => (e.body.session as Json).id === id).length >= n;
  return { server, h, agent, runner, base, call, events, finished };
}

describe("the token", () => {
  it("lets /health answer without it and refuses everything else without it", async () => {
    const t = await setup();
    const health = await fetch(`${t.base}/health`);
    expect(health.status).toBe(200);
    expect(await health.json()).toMatchObject({ ok: true, machine: "test-runner" });
    expect((await t.call("GET", "/accounts", undefined, "wrong")).status).toBe(401);
    const none = await fetch(`${t.base}/accounts`);
    expect(none.status).toBe(401);
    expect(((await none.json()) as Json).error).toBeTypeOf("string");
  });
});

describe("a session", () => {
  it("opens, runs its first turn and reports each change with the session as it is now", async () => {
    const t = await setup();
    const open = await t.call("POST", "/sessions", {
      title: "Fix the build",
      cwd: t.h.work,
      prompt: "The build fails.",
      project: null,
      effort: "high",
      meta: { task: 12 },
    });
    expect(open.status).toBe(200);
    const id = open.body.id as string;
    expect(id).toMatch(/^r-[0-9a-f]{8}$/);
    expect(open.body.turn).toBe(`${id}:1`);
    expect(open.body).toMatchObject({ title: "Fix the build", account: "a-1", cwd: t.h.work, effort: "high", meta: { task: 12 } });
    await until(() => t.finished(id), "the turn to end");

    const kinds = t.events().map((e) => e.body.kind);
    expect(kinds.slice(0, 3)).toEqual(["session", "turn_queued", "turn_started"]);
    expect(kinds).toContain("activity");
    const every = t.events();
    for (const e of every) {
      expect(e.auth).toBe(`Bearer ${TOKEN}`);
      expect(e.body).toMatchObject({ machine: "test-runner", session: { id } });
      expect(Date.parse(e.body.at as string)).not.toBeNaN();
    }
    const queued = t.events("turn_queued")[0]!.body;
    expect(queued.turn).toMatchObject({ id: `${id}:1`, by: "assistant", prompt: "The build fails." });
    const end = t.events("turn_finished")[0]!.body;
    expect(end.turn).toMatchObject({ outcome: "completed", last: "All green.\nSTATUS: done" });
    expect(end.session).toMatchObject({ status: "completed", account: "a-1", meta: { task: 12 } });
    expect((t.events("activity")[0]!.body as Json).doing).toBe("Running: Run the tests");
  });

  it("keeps a transcript of every step, newest last, paged by position", async () => {
    const t = await setup();
    const { body } = await t.call("POST", "/sessions", { title: "T", cwd: t.h.work, prompt: "Go" });
    const id = body.id as string;
    await until(() => t.finished(id));
    const tr = await t.call("GET", `/sessions/${id}/transcript?after=&limit=`);
    expect(tr.status).toBe(200);
    const steps = tr.body.events as Json[];
    expect(steps.map((s) => s.kind)).toEqual([
      "user",
      "thinking",
      "text",
      "tool",
      "result",
      "tool",
      "result",
      "tool",
      "todos",
      "result",
      "end",
    ]);
    expect(steps.find((s) => s.kind === "tool" && s.name === "Edit")).toMatchObject({ diff: "- old\n+ new", id: "call-2" });
    expect(steps.find((s) => s.kind === "result")).toMatchObject({ id: "call-1", error: false, lines: 2, text: "ok\n2 passed" });
    expect(steps.at(-1)).toMatchObject({ kind: "end", outcome: "completed", n: 1 });
    expect(tr.body.earlier).toBe(false);
    expect((tr.body.session as Json).id).toBe(id);
    const after = await t.call("GET", `/sessions/${id}/transcript?after=${steps[8]!.seq as number}`);
    expect((after.body.events as Json[]).map((s) => s.kind)).toEqual(["result", "end"]);
    const last2 = await t.call("GET", `/sessions/${id}/transcript?limit=2`);
    expect(last2.body.earlier).toBe(true);
  });

  it("queues a second turn behind the first and says so", async () => {
    const t = await setup();
    const { body } = await t.call("POST", "/sessions", { title: "T", cwd: t.h.work, prompt: "SLOW" });
    const id = body.id as string;
    await until(() => t.events("turn_started").length > 0);
    const sent = await t.call("POST", `/sessions/${id}/send`, { prompt: "Then this", by: "owner" });
    expect(sent.body).toMatchObject({ turn: `${id}:2`, queued: true });
    const stopped = await t.call("POST", `/sessions/${id}/interrupt`);
    expect(stopped.body).toMatchObject({ running: true });
    await until(() => t.finished(id, 2), "both turns");
    const ends = t.events("turn_finished").map((e) => (e.body.turn as Json).outcome);
    expect(ends).toEqual(["interrupted", "completed"]);
  });

  it("drops what waits when stopped", async () => {
    const t = await setup();
    const { body } = await t.call("POST", "/sessions", { title: "T", cwd: t.h.work, prompt: "SLOW" });
    const id = body.id as string;
    await until(() => t.events("turn_started").length > 0);
    await t.call("POST", `/sessions/${id}/send`, { prompt: "Never", by: "owner" });
    expect((await t.call("POST", `/sessions/${id}/stop`)).body).toMatchObject({ running: true });
    await until(() => t.finished(id, 2));
    expect(t.events("turn_finished").map((e) => (e.body.turn as Json).outcome)).toEqual(["cancelled", "interrupted"]);
    expect(t.agent.started[0]!.messages).not.toContain("Never");
  });

  it("moves to the next account when one runs out, and carries on with the same turn", async () => {
    const t = await setup();
    const { body } = await t.call("POST", "/sessions", { title: "T", cwd: t.h.work, prompt: "LIMIT" });
    const id = body.id as string;
    await until(() => t.finished(id));
    const move = t.events("account")[0]!.body;
    expect(move).toMatchObject({ from: "a-1", to: "a-2", fromName: "First account", toName: "Second account" });
    expect(String(move.why)).toContain("limit");
    expect(t.events("turn_finished")[0]!.body).toMatchObject({ turn: { outcome: "completed" }, session: { account: "a-2" } });
    expect(t.agent.started[1]!.messages[0]).toContain("moved to another account");
    const acc = await t.call("GET", "/accounts");
    const first = (acc.body.accounts as Json[]).find((a) => a.id === "a-1")!;
    expect(first.limitedUntil).toBeTypeOf("string");
  });

  it("fails the turn when every account is out, saying when it carries on", async () => {
    const t = await setup({ accounts: [{ id: "only", name: "Only account", configDir: null, enabled: false }] });
    // No account can take work: the session is refused, said plainly.
    const r = await t.call("POST", "/sessions", { title: "T", cwd: t.h.work, prompt: "Go" });
    expect(r.status).toBe(409);
    expect(r.body.error).toMatch(/no account has room/);
  });

  it("opens with no message, takes files into its inbox, and names them in its next turn", async () => {
    const t = await setup();
    const { body } = await t.call("POST", "/sessions", { cwd: t.h.work, meta: { direct: true }, by: "owner", title: "plan.txt" });
    const id = body.id as string;
    expect(body.turn).toBeNull();
    expect(body.status).toBe("idle");
    const put = await fetch(`${t.base}/sessions/${id}/files?name=plan.txt&by=owner&size=8`, {
      method: "PUT",
      headers: { Authorization: `Bearer ${TOKEN}`, "Content-Type": "application/octet-stream" },
      body: "the plan",
    });
    const landed = (await put.json()) as Json;
    expect(landed).toMatchObject({ size: 8 });
    expect(readFileSync(landed.path as string, "utf8")).toBe("the plan");
    const sent = await t.call("POST", `/sessions/${id}/send`, {
      prompt: "(see the files)",
      by: "owner",
      files: [{ name: "plan.txt", url: "/uploads/1a2b3c4d-plan.txt", kind: "file" }],
    });
    expect(sent.body).toMatchObject({ turn: `${id}:1`, queued: false });
    await until(() => t.finished(id));
    expect(t.agent.started[0]!.messages[0]).toContain(landed.path as string);
    const tr = await t.call("GET", `/sessions/${id}/transcript`);
    expect((tr.body.events as Json[]).find((s) => s.kind === "user")).toMatchObject({
      by: "owner",
      text: "(see the files)",
      files: [{ name: "plan.txt", kind: "file" }],
    });
  });

  it("refuses a file over the limit and one outside the sessions' folders", async () => {
    const t = await setup({ config: { maxFileBytes: 4 } });
    const { body } = await t.call("POST", "/sessions", { cwd: t.h.work, meta: { direct: true } });
    const put = await fetch(`${t.base}/sessions/${body.id as string}/files?name=big.bin&size=10`, {
      method: "PUT",
      headers: { Authorization: `Bearer ${TOKEN}` },
      body: "0123456789",
    });
    expect(put.status).toBe(413);
    writeFileSync(join(t.h.work, ".env"), "SECRET=1");
    expect((await t.call("GET", `/files?path=${encodeURIComponent(join(t.h.work, ".env"))}&stat=1`)).status).toBe(403);
    expect((await t.call("GET", `/files?path=${encodeURIComponent("/etc/hostname")}&stat=1`)).status).toBe(403);
    writeFileSync(join(t.h.work, "out.txt"), "ok");
    const stat = await t.call("GET", `/files?path=${encodeURIComponent(join(t.h.work, "out.txt"))}&stat=1&by=owner`);
    expect(stat.body).toMatchObject({ size: 2 });
    expect(stat.body.mtime).toBeTypeOf("string");
  });

  it("puts the owner's question in front of them and waits", async () => {
    const t = await setup();
    const { body } = await t.call("POST", "/sessions", { cwd: t.h.work, meta: { direct: true }, by: "owner", prompt: "ASK" });
    const id = body.id as string;
    await until(() => t.finished(id));
    expect(t.events("question")[0]!.body).toMatchObject({ text: "Which one?" });
    expect(t.events("turn_finished")[0]!.body).toMatchObject({ session: { status: "waiting" } });
    const tr = await t.call("GET", `/sessions/${id}/transcript`);
    expect((tr.body.events as Json[]).find((s) => s.kind === "question")).toMatchObject({
      questions: [
        { question: "Which one?", header: "Pick", options: [{ label: "The first" }, { label: "The second", description: "Slower" }] },
      ],
    });
  });

  it("labels what the assistant writes in the owner's session", async () => {
    const t = await setup();
    const { body } = await t.call("POST", "/sessions", { cwd: t.h.work, meta: { direct: true } });
    const id = body.id as string;
    await t.call("POST", `/sessions/${id}/send`, { prompt: "Carry on", by: "assistant" });
    await until(() => t.finished(id));
    expect(t.agent.started[0]!.messages[0]).toMatch(/^\(From the assistant/);
    expect((t.events("turn_queued")[0]!.body.turn as Json).by).toBe("assistant");
  });

  it("changes its settings and answers with its bare head", async () => {
    const t = await setup();
    const { body } = await t.call("POST", "/sessions", { cwd: t.h.work, meta: { direct: true } });
    const r = await t.call("POST", `/sessions/${body.id as string}/settings`, { model: "model-b", effort: "max" });
    expect(r.body).toMatchObject({ id: body.id, model: "model-b", effort: "max", running: false, queued: 0 });
    expect(r.body.ok).toBeUndefined();
    expect((await t.call("POST", `/sessions/${body.id as string}/settings`, { effort: "loud" })).status).toBe(400);
  });

  it("starts the agent with the account's folder, the instructions, the shared MCP servers and the switched-off ones", async () => {
    const t = await setup({ config: { instructions: { work: "Work note.", machine: "Machine note." } } });
    await t.call("POST", "/off", { mcp: ["noisy"], skills: ["old-skill"] });
    const { body } = await t.call("POST", "/sessions", { title: "T", cwd: t.h.work, prompt: "Go", model: "claude-test" });
    await until(() => t.finished(body.id as string));
    const o = t.agent.started[0]!.options as Json & { env: Json; systemPrompt: Json; settings: Json };
    expect(o.env.CLAUDE_CODE_ENABLE_TODO_TOOLS).toBe("1");
    expect(String(o.env.CLAUDE_CONFIG_DIR)).toMatch(/acct-1$/);
    expect(o.model).toBe("claude-test");
    expect(o.systemPrompt.append).toContain("Work note.");
    expect(o.systemPrompt.append).toContain("Machine note.");
    expect(o.mcpServers).toEqual({ docs: { type: "http", url: "https://mcp.example.com" } });
    expect(o.settings).toMatchObject({
      deniedMcpServers: [{ serverName: "noisy" }],
      skillOverrides: { "old-skill": "off" },
      includeCoAuthoredBy: false,
    });
    expect(o.disallowedTools).toEqual(["AskUserQuestion"]);
  });

  it("refuses a missing folder, an unknown account and a model id it can't take", async () => {
    const t = await setup();
    expect((await t.call("POST", "/sessions", { cwd: join(t.h.work, "nope"), prompt: "x" })).status).toBe(400);
    expect((await t.call("POST", "/sessions", { cwd: t.h.work, prompt: "x", account: "nobody" })).status).toBe(400);
    expect((await t.call("POST", "/sessions", { cwd: t.h.work, prompt: "x", model: "bad model!" })).status).toBe(400);
    expect((await t.call("GET", "/sessions/r-00000000/transcript")).status).toBe(404);
  });
});

describe("work groups", () => {
  it("keeps a group's work to its accounts and refuses it where none is", async () => {
    const t = await setup({
      config: { groups: { client: { folders: "client-work" } } },
      accounts: [{ id: "own", name: "Own", configDir: null, order: 1 }],
    });
    mkdirSync(join(t.h.work, "client-work"));
    const r = await t.call("POST", "/sessions", { cwd: join(t.h.work, "client-work"), prompt: "x" });
    expect(r.status).toBe(409);
    expect(r.body.error).toMatch(/client work/);
  });
});

describe("the pickers", () => {
  it("lists recent folders, a folder's folders and a repository's branches", async () => {
    const t = await setup();
    const repo = join(t.h.work, "repo");
    mkdirSync(join(repo, "src"), { recursive: true });
    execFileSync("git", ["init", "-q", "-b", "main", repo]);
    execFileSync("git", [
      "-C",
      repo,
      "-c",
      "user.email=t@example.com",
      "-c",
      "user.name=T",
      "commit",
      "-q",
      "--allow-empty",
      "-m",
      "first",
    ]);
    execFileSync("git", ["-C", repo, "branch", "example-fix"]);
    const { body } = await t.call("POST", "/sessions", { cwd: repo, meta: { direct: true } });
    const roots = await t.call("GET", "/folders");
    expect(roots.body).toMatchObject({ machine: "test-runner", recent: [{ path: repo, name: "repo", git: true }] });
    const b = await t.call("GET", `/branches?cwd=${encodeURIComponent(repo)}`);
    expect(b.body).toMatchObject({ current: "main" });
    expect((b.body.branches as string[]).sort()).toEqual(["example-fix", "main"]);
    expect((await t.call("GET", `/branches?cwd=${encodeURIComponent(t.h.work)}`)).body.error).toBe("not a git folder");
    expect(body.id).toBeTruthy();
  });

  it("opens a session in its own worktree on a new branch", async () => {
    const t = await setup();
    const repo = join(t.h.work, "repo");
    mkdirSync(repo);
    execFileSync("git", ["init", "-q", "-b", "main", repo]);
    execFileSync("git", [
      "-C",
      repo,
      "-c",
      "user.email=t@example.com",
      "-c",
      "user.name=T",
      "commit",
      "-q",
      "--allow-empty",
      "-m",
      "first",
    ]);
    const { body } = await t.call("POST", "/sessions", { cwd: repo, title: "Tidy", worktree: { branch: "tidy" }, meta: { direct: true } });
    expect(body).toMatchObject({ branch: "tidy" });
    expect(String(body.cwd)).toMatch(/worktrees[/\\]repo[/\\]tidy$/);
    expect(body.worktree).toBe(body.cwd);
  });
});

describe("the accounts", () => {
  it("lists them as the server reads them, without logins or keys", async () => {
    const t = await setup();
    const r = await t.call("GET", "/accounts");
    const list = r.body.accounts as Json[];
    expect(list.map((a) => a.id)).toEqual(["a-1", "a-2"]);
    expect(list[0]).toMatchObject({ name: "First account", scope: "any", order: 1, enabled: true, loggedIn: true, provider: "claude" });
    for (const a of list) expect(Object.keys(a)).not.toContain("login");
  });
});

describe("reporting", () => {
  it("keeps events while the server is away and sends them in order when it's back", async () => {
    const t = await setup();
    const outbox = t.runner.reporter;
    t.runner.config.server = null; // as before pairing
    const { body } = await t.call("POST", "/sessions", { title: "T", cwd: t.h.work, prompt: "Go" });
    await until(() => outbox.pending() >= 4, "events to wait");
    const before = t.events().length;
    t.runner.config.server = t.server.url;
    await outbox.flush();
    const sent: Got[] = t.events().slice(before);
    expect(sent.map((e) => e.body.kind)[0]).toBe("session");
    expect(sent.every((e) => (e.body.session as Json).id === body.id)).toBe(true);
    expect(outbox.pending()).toBe(0);
  });
});
