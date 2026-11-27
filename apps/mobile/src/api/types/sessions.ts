import type { ChatFile } from "./chat";
import type { Instant } from "./common";

/** A runner machine's id, as GET /api/machines lists it (`Machine.id`); its words are that machine's `name`. */
export type RunnerMachine = string;

/** A Claude Code session on Emberhut's runner ("r-…"), or an Emberhut worker ("w-<task>"). */
export interface RunnerSession {
  id: string;
  machine: RunnerMachine;
  /** The machine in words, as the server names it. */
  place: string;
  title: string;
  project: string;
  /** queued, running, completed, failed, interrupted, idle, waiting. */
  status: string;
  updatedAt: Instant;
  doing: string | null;
  account: string | null;
  accountName: string | null;
  cwd: string | null;
  branch: string | null;
  ask: string | null;
  last: string | null;
  statusLine: string | null;
  /** Only while running. */
  since: Instant | null;
  /** Your own session, opened and driven from the app. */
  direct: boolean;
  model: string | null;
  effort: string | null;
  /** A question (AskUserQuestion) waiting for you. */
  question: string | null;
  tasks: number[];
  room: number;
}

/** A thread another coding app runs, still running, shown read-only. */
export interface ExternalThread {
  id: string;
  machine: string;
  title: string;
  project: string;
  status: string;
  updatedAt: Instant;
  ask: string | null;
  last: string | null;
  statusLine: string | null;
  tasks: number[];
}

/** GET /api/t3/activity: an external thread's latest activity, read-only. */
export interface ExternalThreadActivity {
  title: string | null;
  status: string;
  updatedAt: Instant | null;
  items: { type: string; title?: string | null; text: string; at?: Instant | null }[];
}

/** An agent running now across your tasks (state.agentsNow). */
export interface AgentNow {
  /** "lead-<task>" or "agent-<id>". */
  id: string;
  task: number;
  key: string;
  title: string | null;
  name: string | null;
  line: string | null;
  started: Instant | null;
  tokens: number | null;
  viewer: string | null;
  parent: string | null;
  lead: boolean;
  /** Who runs it (newer servers): worker, session, lead (an ultra run's), direct (your own) or helper (a sub-agent). */
  kind?: string | null;
  /** The runner machine it's on (an id of GET /api/machines); place says it in words. */
  machine?: string | null;
  place?: string | null;
  account?: string | null;
  accountName?: string | null;
}

export interface TodoItem {
  id?: string;
  content: string;
  status: "pending" | "in_progress" | "completed";
  activeForm?: string;
}

export interface LiveAgent {
  id: string;
  description?: string;
  tokens?: number;
  tools?: number;
  last?: string | null;
  running?: boolean;
  started?: Instant;
}

export interface SessionQuestion {
  question: string;
  header?: string | null;
  multiSelect?: boolean;
  options: { label: string; description?: string | null }[];
}

/** One step of a session's transcript. */
export interface SessionStep {
  seq: number;
  /** "r-xxxx:n". */
  turn: string | null;
  at: Instant | null;
  /** user, text, tool, result, end, system, question, thinking, todos. */
  kind: string;
  text?: string;
  /** emberhut, emberhut, runner, self. */
  by?: string;
  name?: string;
  summary?: string;
  doing?: string;
  input?: string;
  error?: boolean;
  outcome?: string;
  n?: number;
  /** The sub-agent call this step happened inside. */
  parent?: string;
  /** The tool call's id. */
  id?: string;
  diff?: string;
  files?: Omit<ChatFile, "path">[];
  questions?: SessionQuestion[];
  ms?: number;
  lines?: number;
  todos?: TodoItem[];
}

export interface SessionHead {
  id: string;
  title: string;
  status: string;
  running?: boolean;
  cwd?: string | null;
  project?: string | null;
  account?: string | null;
  accountName?: string | null;
  branch?: string | null;
  worktree?: string | null;
  meta?: { direct?: boolean } | null;
  model?: string | null;
  effort?: string | null;
  created?: Instant;
  updated?: Instant;
  place?: string;
  tasks?: number[];
  doing?: string | null;
  since?: Instant | null;
  /** Live "head" events only. */
  turn?: string | null;
  tokens?: number | null;
  agents?: LiveAgent[];
  queued?: number;
}

/** GET /api/runner/sessions/<id>/transcript. */
export interface SessionTranscript {
  session: SessionHead;
  events: SessionStep[];
  earlier?: boolean;
}

/** One data payload of /api/runner/sessions/<id>/live. Workers ("w-") only send head and step. */
export type SessionLiveEvent =
  | { type: "head"; session: SessionHead }
  | { type: "step"; step: SessionStep }
  /** Text as it streams; `start` begins a new block (replace the buffer). */
  | { type: "delta"; turn: string; text: string; start?: true }
  | { type: "think"; turn: string; text: string; start?: true; since?: Instant }
  | { type: "tokens"; turn: string; tokens: number };

export type Effort = "low" | "medium" | "high" | "xhigh" | "max";

/** POST /api/runner/sessions: one of your own sessions. */
export interface NewSessionRequest {
  machine: RunnerMachine;
  cwd: string;
  text: string;
  model?: string;
  effort?: Effort;
  files: ChatFile[];
  account?: string;
  /** A new worktree ({}), or one on an existing branch. */
  worktree?: { branch?: string };
  title?: string;
}

export interface NewSessionResult {
  ok: boolean;
  id: string;
  turn: string;
  session: SessionHead;
}

export interface SessionSendResult {
  ok: boolean;
  turn?: string;
  queued?: boolean | number;
  to?: string;
  session?: string;
}

export interface SessionSettings {
  model?: string;
  effort?: Effort | null;
  title?: string;
}

export interface FolderEntry {
  path: string;
  name: string;
  git: boolean;
}

/** GET /api/runner/folders without a path: where to start. */
export interface FolderRoots {
  machine: string;
  home: string;
  recent: (FolderEntry & { used: Instant })[];
  documents: string;
  projects: { name: string; path: string }[];
}

/** GET /api/runner/folders?path=: one folder's folders. */
export interface FolderListing {
  machine: string;
  home: string;
  path: string;
  git: boolean;
  folders: FolderEntry[];
}

export interface Branches {
  repo: string;
  current: string;
  branches: string[];
}
