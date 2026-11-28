// The sessions: each one runs the agent (Claude Code, through the Agent SDK) in a folder, keeps its agent session id,
// takes its next turn when the server sends one, can be interrupted or stopped, and reports every change as it happens.
//
// - A turn is one message and its answer. Turns queue one behind the other (never folded into the running one), so each
//   answer belongs to exactly one turn.
// - A session starts on the account with the most room. When a turn ends on a usage limit, the account is marked out
//   and the session moves to the next one and carries on with the same turn. All out: the turn fails, saying when one
//   is free, and the session carries on then by itself.
// - A runner restart (or a crashed agent process) resumes every turn that was running, by session id, with a short
//   "carry on" message; queued turns follow.
// - The owner's own sessions (`meta.direct`) are driven from an app: no work instructions, their own words, and a
//   question (AskUserQuestion) goes to the app as a card while the session waits for the answer.
import { randomBytes, randomUUID } from "node:crypto";
import { existsSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { basename, resolve } from "node:path";
import type { Options, Query, SDKUserMessage } from "@anthropic-ai/claude-agent-sdk";
import { Accounts, LIMIT, PROVIDER_DOWN, type RateLimitInfo, type Work } from "../accounts/index.ts";
import { linkSession, memoryDir } from "../accounts/login.ts";
import { providerOf, modelFor, type Account } from "../accounts/list.ts";
import type { Config } from "../config.ts";
import type { Files } from "../files.ts";
import { mb } from "../files.ts";
import { inBackground, log, messageOf } from "../log.ts";
import type { Reporter } from "../report.ts";
import type { EventRow, SessionRow, Store, TurnRow } from "../store.ts";
import { now } from "../store.ts";
import { clip, diffOf, doing, toolSummary } from "../text.ts";
import { DEFAULT_BY, EFFORTS, EVENT, FROM_ASSISTANT, LIVE, type Outcome } from "../wire.ts";
import { instructionFiles, makeWorktree, Refused, type WorktreeAsk } from "./folders.ts";
import { ASKED, FREE_AGAIN, JOBS_LOST, MOVED, projectNote, RESTARTED, systemAppend } from "./prompts.ts";
import { Watchers } from "./live.ts";

/** The SDK's `query`, or a stand-in with the same shape (the tests'). */
export type QueryFn = (params: { prompt: AsyncIterable<SDKUserMessage>; options?: Options }) => Query;

/** A session with nothing to do closes its agent process after this long; its next turn resumes it. */
const IDLE = 10 * 60_000;

/** The parts of an agent message the runner reads (the SDK's union, loosely). */
type Msg = {
  type: string;
  subtype?: string;
  session_id?: string;
  parent_tool_use_id?: string | null;
  error?: string;
  message?: { content?: Block[] | string };
  event?: {
    type?: string;
    delta?: { type?: string; text?: string; thinking?: string; partial_json?: string };
    content_block?: { type?: string };
    usage?: { output_tokens?: number };
  };
  rate_limit_info?: RateLimitInfo;
  tasks?: { ambient?: boolean }[];
  task_id?: string;
  task_type?: string;
  tool_use_id?: string;
  description?: string;
  summary?: string;
  last_tool_name?: string;
  patch?: { status?: string };
  usage?: {
    total_tokens?: number;
    tool_uses?: number;
    input_tokens?: number;
    output_tokens?: number;
    cache_creation_input_tokens?: number;
    cache_read_input_tokens?: number;
  };
  estimated_tokens?: number;
  result?: string;
  is_error?: boolean;
  errors?: string[];
  queued_turn_count?: number;
  total_cost_usd?: number;
};
type Block = {
  type: string;
  text?: string;
  thinking?: string;
  id?: string;
  name?: string;
  input?: Record<string, unknown>;
  tool_use_id?: string;
  is_error?: boolean;
  content?: unknown;
};

type Todo = { id?: string; content: string; status: string; activeForm?: string };
type Tok = { out: number; thought: number; think: number; chars: number; at: number };
const noTok = (): Tok => ({ out: 0, thought: 0, think: 0, chars: 0, at: 0 });
/** A sub-agent the session runs, for the live head. */
type AgentInfo = {
  id: string;
  description: string;
  tokens: number;
  tools: number;
  last: string | null;
  running: boolean;
  started: string;
  ended: number;
};

/** A session's live state while the runner runs it. */
type Live = {
  id: string;
  q: Query | null;
  open: { v: boolean };
  queue: SDKUserMessage[];
  wake: (() => void) | null;
  turn: string | null;
  limit: { until: string | null; why: string } | null;
  interrupting: boolean;
  stopping: boolean;
  moving: boolean;
  crashes: number;
  lastText: string;
  toldAt: number;
  usedAt: number;
  background: number;
  asked: boolean;
  reopen: boolean;
  doing: string;
  partial: string;
  usage: { tokens: number; cost: number | null } | null;
  thinking: string;
  thinkAt: number;
  tok: Tok;
  todos: Todo[] | null;
  agents: Map<string, AgentInfo>;
};

/** What `POST /sessions` takes. */
export type OpenAsk = {
  cwd?: unknown;
  prompt?: unknown;
  title?: unknown;
  project?: unknown;
  account?: unknown;
  model?: unknown;
  effort?: unknown;
  worktree?: unknown;
  meta?: unknown;
  by?: unknown;
  files?: unknown;
};

/** A file the app shows with a message: its upload on the server. */
type ShownFile = { name: string; url: string; kind: string };

const parse = (s: string | null): Record<string, unknown> => {
  if (!s) return {};
  try {
    const v: unknown = JSON.parse(s);
    return v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : {};
  } catch {
    return {};
  }
};
const text = (v: unknown): string => (typeof v === "string" ? v : typeof v === "number" || typeof v === "boolean" ? String(v) : "");

/** A model id as the agent takes it (an alias, a full id, `[1m]`, a provider's id), or null for none. */
export function modelOf(m: unknown): string | null {
  if (m === undefined || m === null || m === "") return null;
  const s = text(m).trim();
  if (!/^[A-Za-z0-9][\w.:/\-[\]]{0,159}$/.test(s)) throw new Refused(`no model ${JSON.stringify(s).slice(0, 90)}`, 400);
  return s;
}

/** The runner's sessions. */
export class Sessions {
  private readonly lives = new Map<string, Live>();
  /** Who watches each session live. */
  readonly watchers = new Watchers();
  private readonly creating = new Map<string, Todo>();
  private off: { mcp: string[]; skills: string[] } = { mcp: [], skills: [] };
  private timers: NodeJS.Timeout[] = [];
  private readonly store: Store;
  private readonly accounts: Accounts;
  private readonly reporter: Reporter;
  private readonly files: Files;
  private readonly config: Config;
  private readonly query: QueryFn;

  constructor(store: Store, accounts: Accounts, reporter: Reporter, files: Files, config: Config, query: QueryFn) {
    this.store = store;
    this.accounts = accounts;
    this.reporter = reporter;
    this.files = files;
    this.config = config;
    this.query = query;
  }

  // ── Shapes ────────────────────────────────────────────────────────────────────────────────────────────────────────

  /** A session as the protocol carries it. */
  shape(s: SessionRow): Record<string, unknown> {
    return {
      id: s.id,
      title: s.title,
      cwd: s.cwd,
      project: s.project,
      status: s.status,
      account: s.account,
      claude: s.claude,
      worktree: s.worktree,
      branch: s.branch,
      meta: s.meta ? parse(s.meta) : null,
      created: s.created,
      updated: s.updated,
      model: s.model ?? this.config.model,
      effort: s.effort ?? null,
    };
  }
  /** A turn as the protocol carries it. */
  turnShape(t: TurnRow | undefined): Record<string, unknown> | null {
    return t
      ? {
          id: t.id,
          n: t.n,
          prompt: t.prompt,
          by: t.by,
          requested: t.requested,
          started: t.started,
          ended: t.ended,
          outcome: t.outcome,
          last: t.last,
          accounts: t.accounts ? (JSON.parse(t.accounts) as unknown) : [],
        }
      : null;
  }
  /** A stored step as the transcript carries it. */
  static stepOf(e: EventRow): Record<string, unknown> {
    return { seq: e.seq, turn: e.turn, at: e.at, kind: e.kind, ...parse(e.data) };
  }
  /** Whether a turn runs now. */
  running(id: string): boolean {
    return !!this.lives.get(id)?.turn;
  }
  /** How many turns run now. */
  runningCount(): number {
    return [...this.lives.values()].filter((l) => l.turn).length;
  }
  /** What the running turn is typing and thinking now, for a watcher who joins late. */
  typing(id: string): { turn: string; text: string; thinking: string; thinkingSince: string | null } | null {
    const l = this.lives.get(id);
    return l?.turn
      ? { turn: l.turn, text: l.partial, thinking: l.thinking, thinkingSince: l.thinkAt ? new Date(l.thinkAt).toISOString() : null }
      : null;
  }
  /** How many sessions have an agent process open. */
  openCount(): number {
    return [...this.lives.values()].filter((l) => l.q).length;
  }

  /** The session's head: as stored, plus what runs now (its step, since when, tokens, sub-agents, what waits). */
  headOf(id: string): Record<string, unknown> {
    const s = this.store.session(id)!,
      l = this.lives.get(id),
      t = l?.turn ? this.store.turn(l.turn) : undefined;
    return {
      ...this.shape(s),
      running: !!l?.turn,
      doing: l?.turn ? l.doing || null : null,
      since: t?.started ?? null,
      turn: l?.turn ?? null,
      tokens: l?.turn ? this.tokensOf(l) : null,
      agents: this.agentsNow(l),
      queued: this.store.queued(id),
    };
  }
  private pushHead(id: string): void {
    if (this.watchers.has(id)) this.watchers.send(id, { type: LIVE.head, session: this.headOf(id) });
  }
  private isDirect(s: SessionRow | undefined): boolean {
    return !!s && parse(s.meta).direct === true;
  }

  // ── Steps and events ────────────────────────────────────────────────────────────────────────────────────────────────

  /** Writes a transcript step and sends it to whoever watches live. */
  note(session: string, turn: string | null, kind: string, data: Record<string, unknown>): void {
    const clean = Object.fromEntries(Object.entries(data).filter(([, v]) => v !== undefined));
    const seq = this.store.addEvent(session, turn, kind, clean);
    if (this.watchers.has(session)) this.watchers.send(session, { type: LIVE.step, step: Sessions.stepOf(this.store.event(seq)!) });
    if (kind === "text" && !data.parent) {
      const l = this.lives.get(session);
      if (l) l.partial = "";
    }
  }

  /** Reports a change of a session to the server, with the session as it is now. */
  private tell(id: string, kind: string, extra: Record<string, unknown> = {}): void {
    const s = this.store.session(id);
    if (!s) return;
    this.reporter.tell(id, kind, { session: this.shape(s), ...extra });
    this.pushHead(id);
  }

  // ── Live state ──────────────────────────────────────────────────────────────────────────────────────────────────────

  private live(id: string): Live {
    let l = this.lives.get(id);
    if (!l)
      this.lives.set(
        id,
        (l = {
          id,
          q: null,
          open: { v: false },
          queue: [],
          wake: null,
          turn: null,
          limit: null,
          interrupting: false,
          stopping: false,
          moving: false,
          crashes: 0,
          lastText: "",
          toldAt: 0,
          usedAt: Date.now(),
          background: 0,
          asked: false,
          reopen: false,
          doing: "",
          partial: "",
          usage: null,
          thinking: "",
          thinkAt: 0,
          tok: noTok(),
          todos: null,
          agents: new Map(),
        }),
      );
    return l;
  }
  private async *input(l: Live, open: { v: boolean }): AsyncGenerator<SDKUserMessage> {
    while (open.v) {
      while (l.queue.length) yield l.queue.shift()!;
      if (!open.v) return;
      await new Promise<void>((r) => (l.wake = r));
    }
  }
  private say(l: Live, content: string): void {
    l.queue.push({ type: "user", message: { role: "user", content }, parent_tool_use_id: null, session_id: "" });
    l.wake?.();
  }

  // ── Accounts ────────────────────────────────────────────────────────────────────────────────────────────────────────

  private work(s: SessionRow): Work {
    const meta = parse(s.meta),
      model = s.model ?? this.config.model;
    const pin = typeof meta.pin === "string" ? meta.pin : null;
    const only = this.accounts.onlyFor(pin, model);
    return { project: s.project, cwd: s.origin ?? s.cwd, optIn: meta.optIn === true || !!only, only, model };
  }
  private accountFor(s: SessionRow): Account | null {
    return this.accounts.pick({ ...this.work(s), prefer: s.account });
  }
  private nameOf(id: string | null): string {
    return (id && this.accounts.get(id)?.name) || id || "an account";
  }

  /** The MCP servers and skills turned off: the sessions the assistant starts next get them. */
  setOff(o: { mcp?: unknown; skills?: unknown }): { mcp: string[]; skills: string[] } {
    const names = (x: unknown) =>
      (Array.isArray(x) ? x : []).filter((n): n is string => typeof n === "string" && n.length > 0 && n.length <= 120);
    this.off = { mcp: names(o.mcp), skills: names(o.skills) };
    return this.off;
  }
  private offSettings(): Record<string, unknown> {
    return {
      ...(this.off.mcp.length ? { deniedMcpServers: this.off.mcp.map((serverName) => ({ serverName })) } : {}),
      ...(this.off.skills.length ? { skillOverrides: Object.fromEntries(this.off.skills.map((n) => [n, "off"])) } : {}),
    };
  }

  // ── The agent process ───────────────────────────────────────────────────────────────────────────────────────────────

  /** Opens the session's agent process: resumed by its id once it has a transcript, else started with that id. */
  private openQuery(l: Live): { account: Account; resumed: boolean } {
    const s = this.store.session(l.id)!;
    const a = this.accountFor(s);
    if (!a) throw new Error("no account has room now");
    if (a.id !== s.account) this.store.setSession(l.id, { account: a.id });
    const resumed = !!s.claude && linkSession(s.claude, a, this.accounts.list());
    l.open = { v: true };
    l.moving = false;
    l.reopen = false;
    const direct = this.isDirect(s);
    const origin = s.origin ?? s.cwd;
    const want = instructionFiles(origin),
      have = new Set(instructionFiles(s.cwd));
    const missing = want.filter((f) => !have.has(f));
    const model = s.model ?? this.config.model;
    const mcp = this.config.shareMcp ? this.reporter.sharedMcp() : {};
    const options = {
      cwd: s.cwd,
      env: { ...this.accounts.logins.envFor(a, this.config.skillsDir), CLAUDE_CODE_ENABLE_TODO_TOOLS: "1" },
      ...(model ? { model: modelFor(a, model) } : {}),
      ...(s.effort ? { effort: s.effort } : {}),
      permissionMode: "bypassPermissions",
      allowDangerouslySkipPermissions: true,
      systemPrompt: {
        type: "preset",
        preset: "claude_code",
        append: systemAppend(
          this.config.instructions,
          direct,
          this.files.inboxOf(l.id),
          missing.length ? projectNote(origin, missing) : null,
        ),
      },
      settings: {
        autoMemoryDirectory: memoryDir(origin),
        includeCoAuthoredBy: false,
        attribution: { commit: "", pr: "", sessionUrl: false },
        ...(direct ? {} : this.offSettings()),
      },
      ...(resolve(s.cwd) !== resolve(origin) ? { additionalDirectories: [origin] } : {}),
      ...(Object.keys(mcp).length ? { mcpServers: mcp } : {}),
      // A session the assistant runs puts its questions in its answer, where the server reads them.
      disallowedTools: direct ? [] : ["AskUserQuestion"],
      includePartialMessages: true,
      thinking: { type: "adaptive", display: "summarized" },
      ...(resumed ? { resume: s.claude } : { sessionId: s.claude ?? randomUUID() }),
      // The owner's own session: a question goes to the app as a card (the agent offers the tool only with a handler).
      ...(direct
        ? {
            hooks: {
              PreToolUse: [{ matcher: "AskUserQuestion", hooks: [(h: unknown) => Promise.resolve(this.askOwner(l, h as AskHook))] }],
            },
            canUseTool: (name: string, input: Record<string, unknown>) =>
              Promise.resolve(
                name === "AskUserQuestion"
                  ? { behavior: "deny" as const, message: ASKED }
                  : { behavior: "allow" as const, updatedInput: input },
              ),
          }
        : {}),
      stderr: (d: string) => {
        if (/error/i.test(d)) log(`agent (${l.id}):`, d.trim().slice(0, 300));
      },
    } as unknown as Options;
    const q = this.query({ prompt: this.input(l, l.open), options });
    l.q = q;
    inBackground(`session ${l.id}`, this.listen(l, q, a.id));
    return { account: a, resumed };
  }

  /** Reads the agent's messages for as long as its process runs. */
  private async listen(l: Live, q: Query, account: string): Promise<void> {
    let crashed: string | null = null;
    try {
      for await (const raw of q as AsyncIterable<unknown>) {
        if (l.q !== q) break;
        this.onMessage(l, raw as Msg, account);
        if ((raw as Msg).type === "result") this.endOfTurn(l, raw as Msg);
      }
    } catch (e) {
      crashed = messageOf(e);
    }
    if (l.q !== q) return; // replaced on purpose (moved, idle, stopped)
    l.q = null;
    l.open.v = false;
    l.wake?.();
    if (l.moving || !l.turn) return;
    // The process died mid-turn: twice more from where it was, then the turn fails.
    log(`session ${l.id} ended mid-turn: ${crashed ?? "process exited"}`);
    if (++l.crashes <= 2) {
      this.note(l.id, l.turn, "system", { text: `The agent stopped (${clip(crashed ?? "exited", 160)}); carrying on` });
      return this.restartTurn(l, RESTARTED);
    }
    this.finishTurn(l, "failed", `The agent stopped and didn't come back: ${clip(crashed ?? "exited", 300)}`);
  }

  private onMessage(l: Live, m: Msg, account: string): void {
    l.usedAt = Date.now();
    if (m.type === "system" && m.subtype === "init") {
      this.store.setSession(l.id, { materialized: 1, claude: m.session_id ?? null });
      l.crashes = 0;
    }
    if (m.type === "system" && m.subtype === "compact_boundary") this.note(l.id, l.turn, "system", { text: "Conversation compacted" });
    if (m.type === "system" && m.subtype === "background_tasks_changed") {
      l.background = (m.tasks ?? []).filter((t) => !t.ambient).length;
      this.store.setSession(l.id, { background: l.background });
    }
    if (m.type === "rate_limit_event") {
      const info = m.rate_limit_info;
      this.accounts.observe(account, info);
      if (info?.status === "rejected" && info.overageStatus !== "allowed")
        l.limit = {
          until: info.resetsAt ? new Date(info.resetsAt * 1000).toISOString() : null,
          why: `hit its ${info.rateLimitType ?? "usage"} limit`,
        };
    }
    if (m.type === "system" && /^task_(started|progress|notification|updated)$/.test(m.subtype ?? "")) return this.agentSeen(l, m);
    if (m.type === "system" && m.subtype === "thinking_tokens") {
      l.tok.think = m.estimated_tokens ?? 0;
      return this.tokens(l);
    }
    if (m.type === "stream_event" && !m.parent_tool_use_id) return this.streamed(l, m);
    if (m.type === "assistant") this.assistant(l, m, account);
    if (m.type === "user" && Array.isArray(m.message?.content))
      for (const c of m.message.content)
        if (c.type === "tool_result") {
          const out = Array.isArray(c.content)
            ? (c.content as Block[])
                .filter((x) => x.type === "text")
                .map((x) => x.text ?? "")
                .join("\n")
            : text(c.content);
          // Long output keeps its start and its end (a command's last lines say how it went), with its real line count.
          this.note(l.id, l.turn, "result", {
            id: c.tool_use_id,
            error: !!c.is_error,
            text: out.length > 4000 ? `${out.slice(0, 2800)}\n…\n${out.slice(-1200)}` : out,
            lines: out ? out.split("\n").length : 0,
            parent: m.parent_tool_use_id ?? undefined,
          });
          if (!m.parent_tool_use_id && !c.is_error && c.tool_use_id) this.todosSeen(l, "result", c.tool_use_id, null, out);
        }
  }

  /** Words and thinking as they are typed: to live watchers only, never stored. */
  private streamed(l: Live, m: Msg): void {
    const ev = m.event,
      d = ev?.delta,
      t = l.tok;
    if (ev?.type === "content_block_start" && ev.content_block?.type === "text") {
      l.partial = "";
      this.watchers.send(l.id, { type: LIVE.delta, turn: l.turn, start: true, text: "" });
    }
    if (ev?.type === "content_block_delta" && d?.type === "text_delta" && d.text) {
      l.partial += d.text;
      t.chars += d.text.length;
      this.watchers.send(l.id, { type: LIVE.delta, turn: l.turn, text: d.text });
    }
    if (ev?.type === "content_block_start" && ev.content_block?.type === "thinking") {
      t.thought += t.think;
      t.think = 0;
      l.thinking = "";
      l.thinkAt = Date.now();
      this.watchers.send(l.id, { type: LIVE.think, turn: l.turn, start: true, text: "", since: new Date(l.thinkAt).toISOString() });
    }
    if (ev?.type === "content_block_delta" && d?.type === "thinking_delta" && d.thinking) {
      l.thinking += d.thinking;
      this.watchers.send(l.id, { type: LIVE.think, turn: l.turn, text: d.thinking });
    }
    if (ev?.type === "content_block_delta" && d?.type === "input_json_delta") t.chars += d.partial_json?.length ?? 0;
    if (ev?.type === "message_delta" && ev.usage?.output_tokens != null) {
      // The call's real count replaces the estimate.
      l.tok = { ...noTok(), out: t.out + ev.usage.output_tokens, at: t.at };
      this.tokens(l, true);
    } else this.tokens(l);
  }

  private assistant(l: Live, m: Msg, account: string): void {
    if (!l.turn) this.selfTurn(l); // the agent took a turn on its own (a background job it waited on ended)
    const on = this.accounts.get(account),
      perToken = !!on && providerOf(on) !== "claude";
    const later = (min: number) => new Date(Date.now() + min * 60_000).toISOString();
    if (m.error === "rate_limit")
      l.limit ??= perToken ? { until: later(15), why: "was throttled by its provider" } : { until: null, why: "hit its usage limit" };
    if ((m.error === "billing_error" || m.error === "server_error") && perToken)
      l.limit ??= { until: later(15), why: `got ${m.error} from its provider` };
    if (m.error === "authentication_failed" || m.error === "oauth_org_not_allowed" || m.error === "account_on_hold")
      l.limit ??= { until: later(360), why: `login problem (${m.error})` };
    const parent = m.parent_tool_use_id ?? undefined;
    const content = Array.isArray(m.message?.content) ? m.message.content : [];
    for (const c of content) {
      if (c.type === "thinking" && (!parent || c.thinking?.trim())) {
        this.note(l.id, l.turn, "thinking", {
          text: clip(c.thinking ?? "", 8000),
          ms: !parent && l.thinkAt ? Date.now() - l.thinkAt : undefined,
          parent,
        });
        if (!parent) {
          l.thinking = "";
          l.thinkAt = 0;
        }
      }
      if (c.type === "text" && c.text?.trim()) {
        if (!parent) l.lastText = c.text;
        this.note(l.id, l.turn, "text", { text: clip(c.text, 12_000), parent });
      }
      if (c.type === "tool_use" && c.name) {
        const now_ = doing(c.name, c.input);
        this.note(l.id, l.turn, "tool", {
          id: c.id,
          name: c.name,
          summary: toolSummary(c.name, c.input),
          doing: now_,
          parent,
          input: clip(JSON.stringify(c.input ?? {}, null, 1), 4000),
          diff: diffOf(c.name, c.input) ?? undefined,
        });
        if (!parent && c.id) this.todosSeen(l, c.name, c.id, c.input ?? null, null);
        l.doing = parent ? `Helper agent · ${now_}` : now_;
        this.pushHead(l.id);
        if (!parent && Date.now() - l.toldAt > 3_000) {
          l.toldAt = Date.now();
          this.tell(l.id, EVENT.activity, { turn: this.turnShape(this.store.turn(l.turn!)), doing: now_ });
        }
      }
    }
  }

  /** The agent started a turn by itself: it becomes a turn here too, so the server sees the session working. */
  private selfTurn(l: Live): void {
    const s = this.store.session(l.id)!,
      n = s.seq + 1,
      id = `${l.id}:${n}`;
    this.store.run(
      "insert into turns(id, session, n, prompt, by, requested, started, accounts) values (?, ?, ?, ?, 'self', ?, ?, ?)",
      id,
      l.id,
      n,
      "",
      now(),
      now(),
      JSON.stringify([s.account]),
    );
    this.store.setSession(l.id, { seq: n, status: "running" });
    l.turn = id;
    this.tell(l.id, EVENT.turnStarted, { turn: this.turnShape(this.store.turn(id)) });
  }

  private endOfTurn(l: Live, m: Msg): void {
    if (!l.turn) return;
    // A result while this turn's message still waits in the agent's queue isn't this turn's (a resumed session first wraps
    // up a background job a restart cut off).
    if ((m.queued_turn_count ?? 0) > 0) return;
    const answer = text(m.result) || l.lastText,
      u = m.usage ?? {};
    l.usage = {
      tokens: (u.input_tokens ?? 0) + (u.cache_creation_input_tokens ?? 0) + (u.cache_read_input_tokens ?? 0) + (u.output_tokens ?? 0),
      cost: typeof m.total_cost_usd === "number" ? m.total_cost_usd : null,
    };
    const limitText = !!m.is_error && LIMIT.test(answer); // an error result saying so, never an answer that talks about limits
    const on = this.accounts.get(this.store.session(l.id)!.account);
    if (!l.limit && !!m.is_error && on && providerOf(on) !== "claude" && PROVIDER_DOWN.test(answer))
      l.limit = {
        until: new Date(Date.now() + 15 * 60_000).toISOString(),
        why: `was refused by its provider (${clip(answer.replace(/\s+/g, " "), 120)})`,
      };
    if (!l.interrupting && !l.stopping && (l.limit || limitText))
      return this.moveAccount(l, l.limit?.why ?? "hit its usage limit", l.limit?.until ?? null);
    const outcome: Outcome = l.interrupting || l.stopping ? "interrupted" : m.subtype === "success" && !m.is_error ? "completed" : "failed";
    this.finishTurn(
      l,
      outcome,
      answer || (outcome === "failed" ? (m.errors ?? []).join("; ") || `Turn ended: ${m.subtype ?? "error"}` : ""),
    );
  }

  private finishTurn(l: Live, outcome: Outcome, last: string): void {
    const id = l.turn!;
    this.store.run("update turns set ended = ?, outcome = ?, last = ? where id = ?", now(), outcome, last, id);
    // The owner's session that asked a question waits for the answer.
    this.store.setSession(l.id, {
      status: l.asked && outcome === "completed" ? "waiting" : outcome === "cancelled" ? "interrupted" : outcome,
    });
    this.note(l.id, id, "end", { outcome, n: this.store.turn(id)!.n });
    Object.assign(l, {
      turn: null,
      lastText: "",
      limit: null,
      interrupting: false,
      crashes: 0,
      doing: "",
      partial: "",
      asked: false,
      thinking: "",
      thinkAt: 0,
      tok: noTok(),
    });
    this.tell(l.id, EVENT.turnFinished, { turn: this.turnShape(this.store.turn(id)), ...(l.usage ? { usage: l.usage } : {}) });
    l.usage = null;
    if (l.stopping) {
      l.stopping = false;
      this.close(l, "stopped");
      return;
    }
    if (l.reopen) this.close(l, "new settings"); // its next turn opens with the model and effort just set
    this.pump(l.id);
  }

  /** An owner's session asks a question: it goes to the app as a card, the agent ends its turn and the session waits. */
  private askOwner(l: Live, h: AskHook): Record<string, unknown> {
    const qs = h?.tool_input?.questions ?? [];
    this.note(l.id, l.turn, "question", {
      id: h?.tool_use_id,
      questions: qs.map((q) => ({
        question: clip(text(q.question), 600),
        header: q.header ?? null,
        multiSelect: !!q.multiSelect,
        options: (q.options ?? []).map((o) => ({
          label: clip(text(o.label), 80),
          description: o.description ? clip(text(o.description), 300) : null,
        })),
      })),
    });
    l.asked = true;
    this.tell(l.id, EVENT.question, {
      turn: this.turnShape(this.store.turn(l.turn ?? "")),
      text: qs.map((q) => text(q.question)).join(" / "),
    });
    return { hookSpecificOutput: { hookEventName: "PreToolUse", permissionDecision: "deny", permissionDecisionReason: ASKED } };
  }

  /**
   * The account ran out mid-turn: it's marked out, and the session moves to the account with the most room and carries on
   * with the same turn. None left: the turn fails, and the session carries on when the first account is free again.
   */
  private moveAccount(l: Live, why: string, until: string | null): void {
    const s = this.store.session(l.id)!,
      from = s.account;
    if (from) this.accounts.markLimited(from, until, why);
    const next = this.accounts.pick({ ...this.work(s), exclude: from ? [from] : [] });
    if (!next) {
      const free = this.accounts.nextFree(this.work(s));
      this.store.setSession(l.id, { resume_at: free ?? new Date(Date.now() + 3_600_000).toISOString() });
      this.note(l.id, l.turn, "system", {
        text: `${this.nameOf(from)} ${why}; every other account is out too. Carrying on ${free ? `at ${free}` : "in an hour"}.`,
      });
      this.tell(l.id, EVENT.account, {
        turn: this.turnShape(this.store.turn(l.turn!)),
        from,
        to: null,
        why,
        resumeAt: free,
        fromName: this.nameOf(from),
      });
      return this.finishTurn(
        l,
        "failed",
        `Stopped: ${this.nameOf(from)} ${why} and every other account is out too. It carries on by itself ${free ? `at ${free}` : "in an hour"}.`,
      );
    }
    l.moving = true;
    const q = l.q;
    l.q = null;
    l.open.v = false;
    l.wake?.();
    q?.close();
    this.store.setSession(l.id, { account: next.id });
    const t = this.store.turn(l.turn!)!;
    this.store.run(
      "update turns set accounts = ? where id = ?",
      JSON.stringify([...(JSON.parse(t.accounts ?? "[]") as string[]), next.id]),
      t.id,
    );
    this.note(l.id, l.turn, "system", { text: `Moved from ${this.nameOf(from)} to ${next.name}: ${this.nameOf(from)} ${why}` });
    this.tell(l.id, EVENT.account, { turn: this.turnShape(t), from, to: next.id, why, fromName: this.nameOf(from), toName: next.name });
    log(`session ${l.id}: ${from ?? "?"} ${why}; moved to ${next.id}`);
    l.limit = null;
    this.restartTurn(l, MOVED);
  }

  /** The same turn, carried on in a new agent process (after a move, a restart or a crash). */
  private restartTurn(l: Live, why: string): void {
    try {
      const { resumed } = this.openQuery(l);
      // No transcript yet (it stopped before its first message was written): the turn's own message again.
      this.say(l, resumed ? why : (this.store.turn(l.turn!)!.prompt ?? why));
    } catch (e) {
      this.finishTurn(l, "failed", messageOf(e));
    }
  }

  /**
   * Between turns an open session goes where it should be now: onto the default account when one is set, off one nearly
   * full. Its process closes and the next turn resumes the same session there, as a move at a usage limit does. Not while
   * its background jobs or sub-agents run (they live in that process).
   */
  private rehome(l: Live): void {
    const s = this.store.session(l.id);
    if (!s || !l.q || l.background || [...l.agents.values()].some((a) => a.running)) return;
    const to = this.accountFor(s);
    if (!to || to.id === s.account) return;
    const why =
      to.id === this.accounts.defaultAccount()
        ? `${to.name} is the default account`
        : `${s.account ? this.accounts.lowWhy(s.account) : "no account"}`;
    this.close(l, `moving to ${to.id}`);
    this.note(l.id, null, "system", { text: `Moved from ${this.nameOf(s.account)} to ${to.name}: ${why}` });
    this.tell(l.id, EVENT.account, { from: s.account, to: to.id, why, fromName: this.nameOf(s.account), toName: to.name });
  }

  /** The next queued turn starts when nothing runs. */
  private pump(id: string): void {
    const l = this.live(id);
    if (l.turn) return;
    const t = this.store.nextTurn(id);
    if (!t) {
      this.timers.push(setTimeout(() => this.idleClose(id), IDLE + 1000).unref());
      return;
    }
    try {
      this.rehome(l);
    } catch (e) {
      log(`session ${id}: rehome: ${messageOf(e)}`);
    }
    l.turn = t.id;
    this.store.run("update turns set started = ? where id = ?", now(), t.id);
    try {
      if (!l.q) this.openQuery(l);
    } catch (e) {
      return this.finishTurn(l, "failed", messageOf(e));
    }
    this.store.run("update turns set accounts = ? where id = ?", JSON.stringify([this.store.session(id)!.account]), t.id);
    this.store.setSession(id, { status: "running" });
    this.say(l, t.prompt ?? "");
    this.tell(id, EVENT.turnStarted, { turn: this.turnShape(this.store.turn(t.id)) });
  }

  private close(l: Live, why: string): void {
    const q = l.q;
    if (!q) return;
    l.moving = true;
    l.q = null;
    l.open.v = false;
    l.wake?.();
    q.close();
    log(`session ${l.id} closed (${why})`);
  }
  private idleClose(id: string): void {
    const l = this.lives.get(id);
    if (l && l.q && !l.turn && !l.background && Date.now() - l.usedAt >= IDLE) this.close(l, "idle");
  }

  // ── Sub-agents, tokens and to-dos (for the live head) ──────────────────────────────────────────────────────────────

  private agentsNow(l: Live | undefined): Record<string, unknown>[] {
    return [...(l?.agents.values() ?? [])]
      .filter((x) => x.running || Date.now() - x.ended < 600_000)
      .map((x) => ({
        id: x.id,
        description: x.description,
        tokens: x.tokens,
        tools: x.tools,
        last: x.last,
        running: x.running,
        started: x.started,
      }));
  }
  private agentSeen(l: Live, m: Msg): void {
    if (m.task_type && !/agent|workflow/.test(m.task_type)) return; // a background shell command
    const k = String(m.task_id);
    let i = l.agents.get(k);
    if (!i && m.subtype === "task_started" && m.tool_use_id)
      l.agents.set(
        k,
        (i = { id: m.tool_use_id, description: "", tokens: 0, tools: 0, last: null, running: true, started: now(), ended: 0 }),
      );
    if (!i) return;
    if (m.description && (m.subtype === "task_started" || !i.description)) i.description = clip(m.description, 120);
    if (m.usage) {
      i.tokens = m.usage.total_tokens ?? i.tokens;
      i.tools = m.usage.tool_uses ?? i.tools;
    }
    if (m.subtype === "task_progress" && (m.summary || m.description || m.last_tool_name))
      i.last = clip(m.summary || m.description || m.last_tool_name, 120);
    if (
      i.running &&
      (m.subtype === "task_notification" || (m.subtype === "task_updated" && m.patch?.status && m.patch.status !== "running"))
    ) {
      i.running = false;
      i.ended = Date.now();
    }
    this.pushHead(l.id);
  }
  private tokensOf(l: Live): number {
    return l.tok.out + l.tok.thought + l.tok.think + Math.round(l.tok.chars / 4);
  }
  /** The turn's output tokens so far, to live watchers at most every 400 ms (and at once when a call's real count lands). */
  private tokens(l: Live, at = false): void {
    if (!l.turn || !this.watchers.has(l.id) || (!at && Date.now() - l.tok.at < 400)) return;
    l.tok.at = Date.now();
    this.watchers.send(l.id, { type: LIVE.tokens, turn: l.turn, tokens: this.tokensOf(l) });
  }
  /**
   * Its to-do list, as a `todos` step each time it changes: TodoWrite gives the whole list; TaskCreate adds one (its id
   * comes back in the result: "Task #3 created…"), TaskUpdate changes one. After a restart it carries on from the last one.
   */
  private todosSeen(l: Live, name: string, call: string, input: Record<string, unknown> | null, result: string | null): void {
    l.todos ??= (() => {
      const e = this.store.one<{ data: string }>(
        "select data from events where session = ? and kind = 'todos' order by seq desc limit 1",
        l.id,
      );
      return e ? ((parse(e.data).todos as Todo[] | undefined) ?? []) : [];
    })();
    let list = l.todos;
    if (name === "TodoWrite" && Array.isArray(input?.todos))
      list = (input.todos as Record<string, unknown>[]).map((t) => ({
        content: clip(t.content, 300),
        status: text(t.status),
        ...(t.activeForm ? { activeForm: clip(t.activeForm, 120) } : {}),
      }));
    else if (name === "TaskCreate") {
      this.creating.set(call, {
        content: clip(input?.subject, 300),
        status: "pending",
        ...(input?.activeForm ? { activeForm: clip(input.activeForm, 120) } : {}),
      });
      return;
    } else if (name === "TaskUpdate" && input?.taskId != null) {
      const id = text(input.taskId);
      list =
        input.status === "deleted"
          ? list.filter((t) => t.id !== id)
          : list.map((t) =>
              t.id !== id
                ? t
                : {
                    ...t,
                    ...(input.status ? { status: text(input.status) } : {}),
                    ...(input.subject ? { content: clip(input.subject, 300) } : {}),
                    ...(input.activeForm ? { activeForm: clip(input.activeForm, 120) } : {}),
                  },
            );
    } else if (name === "result" && this.creating.has(call)) {
      const t = this.creating.get(call)!;
      this.creating.delete(call);
      const tid = result?.match(/#(\w+)/)?.[1];
      list = [...list, { ...t, ...(tid ? { id: tid } : {}) }];
    } else return;
    l.todos = list;
    this.note(l.id, l.turn, "todos", { todos: list });
  }

  // ── Actions ─────────────────────────────────────────────────────────────────────────────────────────────────────────

  /** Opens a session: in a folder (and its own worktree when asked), on an account that may take its work, with its first turn. */
  open(b: OpenAsk): Record<string, unknown> {
    const cwd = text(b.cwd).replace(/^~(?=\/|$)/, homedir());
    if (!cwd || !existsSync(cwd) || !statSync(cwd).isDirectory())
      throw new Refused(`no folder ${cwd || "(none given)"} on ${this.config.name}`, 400);
    const meta = b.meta && typeof b.meta === "object" && !Array.isArray(b.meta) ? (b.meta as Record<string, unknown>) : {};
    const direct = meta.direct === true;
    const model = modelOf(b.model);
    const effort = b.effort === undefined || b.effort === null || b.effort === "" ? null : text(b.effort);
    if (effort && !(EFFORTS as readonly string[]).includes(effort)) throw new Refused(`no effort ${effort}`, 400);
    const project = b.project === undefined || b.project === null || b.project === "" ? null : text(b.project);
    const prompt0 = text(b.prompt);
    // A session may open with no message yet: the server puts its files in first, then sends the first message.
    const refused = this.accounts.refusal(project, cwd);
    if (refused) throw new Refused(refused, 409);
    const id = `r-${randomBytes(4).toString("hex")}`;
    const title = text(
      b.title ||
        (direct ? prompt0.trim().split("\n")[0]!.slice(0, 70) || basename(cwd) : prompt0.trim().split("\n")[0]!.slice(0, 70) || "Session"),
    ).slice(0, 120);
    let prompt = prompt0,
      dir = cwd,
      wt: { dir: string; branch: string; top: string } | null = null;
    if (b.worktree) {
      const ask: WorktreeAsk = b.worktree === true || typeof b.worktree !== "object" ? true : b.worktree;
      wt = makeWorktree(cwd, ask, title, this.config.worktreesDir, this.config.branchPrefix);
      if (resolve(wt.top) === resolve(cwd)) dir = wt.dir;
      else
        prompt += `\n\nYour git worktree for this work: ${wt.dir} (branch ${wt.branch}, from ${wt.top}). Make your changes there, not in the main checkout.`;
    }
    const want = b.account ? this.accounts.get(text(b.account)) : null;
    if (b.account && !want) throw new Refused(`no account ${text(b.account)} on ${this.config.name}`, 400);
    // An opt-in account asked for by id pins the session there; a model only one provider runs keeps it there. Either
    // one that can't take it now is refused, never started quietly elsewhere.
    const pin = want?.optIn ? want.id : null,
      only = this.accounts.onlyFor(pin, model);
    if (want && only && !only.includes(want.id)) throw new Refused(`${model ?? "that model"} doesn't run on ${want.name}`, 409);
    const w: Work = { project, cwd, optIn: !!only, only, model: model ?? this.config.model };
    const a = want && this.accounts.ranked(w).some((p) => p.account.id === want.id) ? want : this.accounts.pick(w);
    const free = this.accounts.nextFree(w);
    if (!a && only)
      throw new Refused(
        `${only.map((x) => this.nameOf(x)).join(", ") || "no listed account"} can't take work now: not started on another account`,
        409,
      );
    if (!a) throw new Refused(`no account has room now${free ? ` (the first is free at ${free})` : ""}`, 409);
    const keep = { ...meta, ...(only ? { optIn: true } : {}), ...(pin ? { pin } : {}) };
    this.store.run(
      "insert into sessions(id, title, cwd, origin, project, model, effort, account, claude, worktree, branch, meta, created, updated) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
      id,
      title,
      dir,
      cwd,
      project,
      model,
      effort,
      a.id,
      randomUUID(),
      wt?.dir ?? null,
      wt?.branch ?? null,
      Object.keys(keep).length ? JSON.stringify(keep) : null,
      now(),
      now(),
    );
    log(`session ${id} "${title}" in ${dir} on ${a.id}${model ? ` (${model})` : ""}${direct ? " (the owner's)" : ""}`);
    if (!prompt.trim()) {
      this.store.setSession(id, { status: "idle" });
      this.tell(id, EVENT.session);
      return { ...this.shape(this.store.session(id)!), turn: null };
    }
    this.tell(id, EVENT.session);
    const turn = this.send(id, prompt, b.by ?? DEFAULT_BY, b.files);
    return { ...this.shape(this.store.session(id)!), turn: turn.turn };
  }

  /** Queues a turn: it runs when the one before it ends. Files that arrived since the last turn are named in it. */
  send(id: string, said: string, by: unknown = DEFAULT_BY, files?: unknown): { turn: string; queued: boolean } {
    const s = this.store.session(id);
    if (!s) throw new Refused(`no session ${id} on ${this.config.name}`, 404);
    if (!said.trim()) throw new Refused("nothing to send", 400);
    const who = text(by) || DEFAULT_BY;
    let prompt = who === "assistant" && this.isDirect(s) ? FROM_ASSISTANT + said : said; // the agent knows it isn't the owner typing
    const arrived = this.files.arrived(id);
    if (arrived.length)
      prompt += `\n\n(Files sent to you since your last turn, in ${this.files.inboxOf(id)}: ${arrived.map((f) => `${f.path} (${mb(f.size)}, from ${f.by})`).join(", ")})`;
    const n = s.seq + 1,
      tid = `${id}:${n}`;
    this.store.run("insert into turns(id, session, n, prompt, by, requested) values (?, ?, ?, ?, ?, ?)", tid, id, n, prompt, who, now());
    const queued = !!this.live(id).turn;
    this.store.setSession(id, { seq: n, resume_at: null, ...(queued ? {} : { status: "queued" }) });
    const shown: ShownFile[] = Array.isArray(files)
      ? (files as Record<string, unknown>[])
          .filter((f) => f && typeof f.url === "string")
          .slice(0, 20)
          .map((f) => ({ name: text(f.name) || "file", url: text(f.url), kind: text(f.kind) || "file" }))
      : [];
    this.note(id, tid, "user", {
      text: clip(this.isDirect(s) ? said : prompt, 12_000),
      by: who,
      ...(shown.length ? { files: shown } : {}),
    });
    this.tell(id, EVENT.turnQueued, { turn: this.turnShape(this.store.turn(tid)) });
    this.pump(id);
    return { turn: tid, queued };
  }

  /** Ends the running turn; with `stop` the queued turns go too and the process closes after it. */
  async interrupt(id: string, stop = false): Promise<{ ok: true; running: boolean }> {
    const l = this.live(id),
      s = this.store.session(id);
    if (!s) throw new Refused(`no session ${id} on ${this.config.name}`, 404);
    if (stop) {
      for (const t of this.store.all<{ id: string }>("select id from turns where session = ? and started is null and ended is null", id)) {
        this.store.run("update turns set ended = ?, outcome = 'cancelled' where id = ?", now(), t.id);
        this.tell(id, EVENT.turnFinished, { turn: this.turnShape(this.store.turn(t.id)) });
      }
      this.store.setSession(id, { resume_at: null });
    }
    if (!l.turn) {
      if (stop) this.close(l, "stopped");
      return { ok: true, running: false };
    }
    if (stop) l.stopping = true;
    else l.interrupting = true;
    if (l.q) await l.q.interrupt().catch(() => undefined);
    else this.finishTurn(l, "interrupted", "Stopped");
    return { ok: true, running: true };
  }

  /** The owner changes a session's model, effort, title or account: its next turn opens with them (a running turn ends first). */
  settings(id: string, b: Record<string, unknown>): Record<string, unknown> {
    const s = this.store.session(id);
    if (!s) throw new Refused(`no session ${id} on ${this.config.name}`, 404);
    const effort = b.effort === undefined ? undefined : b.effort === null ? null : text(b.effort);
    if (effort && !(EFFORTS as readonly string[]).includes(effort)) throw new Refused(`no effort ${effort}`, 400);
    const model = b.model === undefined ? undefined : modelOf(b.model);
    const to = b.account ? this.moveTo(s, text(b.account), model ?? s.model) : null;
    this.store.setSession(id, {
      ...(model !== undefined ? { model } : {}),
      ...(effort !== undefined ? { effort } : {}),
      ...(b.title ? { title: text(b.title).slice(0, 120) } : {}),
    });
    if (to) {
      const meta = parse(s.meta);
      delete meta.optIn;
      delete meta.pin;
      this.store.setSession(id, { account: to.id, meta: JSON.stringify({ ...meta, ...(to.optIn ? { optIn: true, pin: to.id } : {}) }) });
      this.note(id, null, "system", { text: `Moved from ${this.nameOf(s.account)} to ${to.name}: asked for` });
      this.tell(id, EVENT.account, { from: s.account, to: to.id, why: "asked for", fromName: this.nameOf(s.account), toName: to.name });
    }
    const l = this.live(id);
    if (l.turn) l.reopen = true;
    else this.close(l, "new settings");
    this.tell(id, EVENT.note, { text: "Settings changed" });
    return this.headOf(id);
  }

  private moveTo(s: SessionRow, id: string, model: string | null): Account {
    const a = this.accounts.get(id);
    if (!a) throw new Refused(`no account ${id} on ${this.config.name}`, 400);
    const only = this.accounts.onlyFor(a.optIn ? a.id : null, model);
    if (only && !only.includes(a.id)) throw new Refused(`${model ?? "that model"} doesn't run on ${a.name}`, 409);
    if (!this.accounts.ranked({ ...this.work(s), optIn: !!a.optIn, only: [a.id] }).length)
      throw new Refused(`${a.name} can't take this session now (${this.accounts.lowWhy(a.id)})`, 409);
    return a;
  }

  // ── Start and timers ────────────────────────────────────────────────────────────────────────────────────────────────

  /** Carries on what a restart cut off: running turns resume, sessions that waited on background jobs hear they stopped, queued turns run. */
  boot(): void {
    for (const t of this.store.all<TurnRow>("select * from turns where started is not null and ended is null order by session, n")) {
      const l = this.live(t.session);
      if (l.turn) continue;
      l.turn = t.id;
      this.note(t.session, t.id, "system", { text: "The runner restarted; carrying on" });
      this.tell(t.session, EVENT.note, {
        turn: this.turnShape(t),
        text: "The runner restarted mid-turn; the session carries on from where it was.",
      });
      log(`session ${t.session}: resuming turn ${t.n} after a restart`);
      this.restartTurn(l, RESTARTED);
    }
    for (const s of this.store.all<{ id: string }>("select id from sessions where background > 0")) {
      this.store.setSession(s.id, { background: 0 });
      if (!this.lives.get(s.id)?.turn) this.send(s.id, JOBS_LOST, "runner");
    }
    for (const s of this.store.all<{ session: string }>("select distinct session from turns where started is null and ended is null"))
      this.pump(s.session);
    // Every minute: sessions waiting for a free account carry on when one is.
    const resume = setInterval(() => {
      for (const s of this.store.all<SessionRow>("select * from sessions where resume_at is not null and resume_at <= ?", now())) {
        if (!this.accounts.pick(this.work(s))) continue;
        this.store.setSession(s.id, { resume_at: null });
        this.send(s.id, FREE_AGAIN, "runner");
      }
    }, 60_000);
    resume.unref();
    this.timers.push(resume);
  }

  /** What the machine sample says of the sessions. */
  summary(): { running: number; open: number; queued: number; list: Record<string, unknown>[] } {
    const rows = this.store.all<SessionRow>(
      "select id, title, status, account from sessions where status in ('running', 'queued', 'waiting') or id in (select session from turns where ended is null)",
    );
    return {
      running: this.runningCount(),
      open: this.openCount(),
      queued: Number(this.store.one<{ n: number }>("select count(*) as n from turns where started is null and ended is null")?.n ?? 0),
      list: rows
        .slice(0, 40)
        .map((r) => ({ id: r.id, title: clip(r.title ?? "", 120), status: r.status, account: r.account, running: this.running(r.id) })),
    };
  }

  /** Closes every agent process and stops the timers (running turns resume at the next start). */
  shutdown(): void {
    for (const t of this.timers) clearTimeout(t);
    for (const l of this.lives.values()) this.close(l, "shutting down");
    this.watchers.closeAll();
  }
}

/** What the AskUserQuestion hook receives. */
type AskHook = {
  tool_use_id?: string;
  tool_input?: {
    questions?: { question?: unknown; header?: string; multiSelect?: boolean; options?: { label?: unknown; description?: unknown }[] }[];
  };
};
