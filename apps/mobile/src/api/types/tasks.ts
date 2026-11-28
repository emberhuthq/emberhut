import type { ChatFile } from "./chat";
import type { Day, Instant } from "./common";
import type { RunnerSession } from "./sessions";
import type { ASSISTANT, Author, StepState } from "../wire";

/** todo, working (a worker is on it), waiting (needs your decision), done, canceled (shown as Archived). */
export type TaskStatus = "todo" | "working" | "waiting" | "done" | "canceled";

/** What a need costs you: a yes or no, read and pick, or something only you can do. */
export type NeedKind = "approve" | "choose" | "do";

/** What is actually happening on a task, from the server's runs; the label comes from this, not from status. */
export interface TaskLive {
  /** running, queued, scheduled, waiting, stalled, todo, done, canceled. */
  state: string;
  since?: Instant | null;
  /** Emberhut was woken twice and changed nothing. */
  alarm: boolean;
  inferred?: boolean | null;
  run?: {
    machine: string;
    place?: string | null;
    thread: string;
    title?: string | null;
    worker: boolean;
    requested?: Instant | null;
    started?: Instant | null;
  } | null;
  /** What does the work when it isn't a turn: "unit:…" or "watch:31". */
  runner?: string | null;
  /** Parked until. */
  until?: Instant | null;
  /** Held by this parent, which waits on you. */
  by?: number | null;
  last?: { ended?: Instant | null; line?: string | null } | null;
  watch?: { id: number; title?: string | null; nextAt: Instant; every: number; note?: string | null; noteAt?: Instant | null } | null;
  checkin?: { at: Instant; why: string } | null;
  /** A scheduled look is on now. */
  checking?: boolean | null;
}

/** An option shown as a card: `label` is the answer sent; the rest describes it. */
export interface DecisionChoice {
  label: string;
  title?: string | null;
  summary?: string | null;
  fields?: { k: string; v: string }[] | null;
  /** A short number: a score, a price. */
  badge?: string | null;
  /** Emberhut's reason when it is the pick. */
  why?: string | null;
  /** What your tap starts. */
  then?: string | null;
  more?: string | null;
  /** Artifact ids of its pictures and videos. */
  media?: number[] | null;
}

export interface Decision {
  question: string;
  options: string[];
  /** The exact plain text you would send. */
  draft?: string | null;
  kind?: NeedKind | null;
  brief?: string[] | null;
  /** What each answer does, in the order of `options`. */
  notes?: string[] | null;
  /** Emberhut's pick: an index into `options`. */
  recommend?: number | null;
  asked?: Instant | null;
  /** When Emberhut last made sure it still holds. */
  checked?: Instant | null;
  context?: string | null;
  /** One card per option, in the same order. */
  choices?: DecisionChoice[] | null;
}

export interface XReplyOption {
  text: string;
  why: string | null;
  label?: string;
  style?: string;
  intent_url?: string;
}

/** An X post to reply to and the replies Emberhut suggests (artifact kind "x_reply"). Keys are snake_case on the wire. */
export interface XReply {
  post_url: string;
  post_id: string;
  author: string | null;
  author_name: string | null;
  followers?: number;
  posted_at: Instant | null;
  post_text: string | null;
  metrics?: { likes?: number; replies?: number; reposts?: number; views?: number };
  replies: XReplyOption[];
  /** What to verify before posting. */
  check: string | null;
  room?: {
    winners_pattern: string | null;
    top_replies: { author: string; text: string; likes: number | null; url: string | null }[];
  };
  posted?: { option: number; text: string; at: Instant };
}

/** What a task produced: pictures and videos (with a small thumb), pdf and md notes, links, X replies. */
export interface TaskArtifact {
  id: number;
  kind: string;
  name: string;
  caption?: string | null;
  /** /api/artifacts/<id>, or an https link. */
  url: string;
  /** /api/artifacts/<id>/thumb for pictures and videos. */
  thumb?: string | null;
  at: Instant;
  x?: XReply | null;
}

/** A parent's subtasks rolled up. */
export interface TaskRollup {
  done: number;
  total: number;
  /** Subtasks waiting on you. */
  waiting: number[];
  progress?: number | null;
  of?: number | null;
  left?: number | null;
  unit?: string | null;
}

/** The work that moves a task: threads, tickets, chats, each with its latest word. */
export interface TaskLink {
  kind: string;
  ref: string;
  machine?: string | null;
  title?: string | null;
  status?: string | null;
  at?: Instant | null;
  line?: string | null;
}

export interface TaskMoment {
  at: Instant;
  who: string;
  text: string;
}

/** One turn of work on a task. */
export interface RunInfo {
  id: number;
  kind: string;
  machine: string;
  thread: string;
  title: string | null;
  asked: string | null;
  answer: string | null;
  /** Its STATUS line. */
  status: string | null;
  requested: Instant | null;
  started: Instant | null;
  ended: Instant | null;
  outcome: string | null;
}

/** A task's sub-agents at a glance (ultra mode). */
export interface AgentsLine {
  done: number;
  total: number;
  running: number;
}

export interface UltraAgent {
  id: number;
  name: string | null;
  kind: "part" | "verify" | "lane" | "other" | null;
  part: number | null;
  /** running, queued, done, failed. */
  state: string | null;
  last: string | null;
  tokens: number | null;
  tools: number | null;
  account: string | null;
  accountName: string | null;
  machine: string | null;
  run: string;
  started: Instant | null;
  ended: Instant | null;
  workflow: boolean;
  /** The session to open for its transcript: "w-<task>" or "r-…". */
  viewer: string | null;
  parent: string | null;
}

export interface UltraLead {
  viewer: string;
  name: string;
  place: string;
  state: string;
  running: boolean;
  doing: string | null;
  started: Instant | null;
  ended: Instant | null;
  tokens: number | null;
  cost: number | null;
}

/** An ultra run's plan, caps and each sub-agent (GET /api/tasks/<id>). */
export interface UltraRun {
  on: boolean;
  state: "planning" | "asking" | "running" | "capped" | null;
  parallel: number;
  budget: number;
  spent: number;
  cost: number | null;
  leadTokens: number | null;
  account: string | null;
  plan: { parts: unknown[]; agents: number; tokens: number; accounts: string[]; note: string | null } | null;
  done: number;
  total: number;
  running: number;
  agents: UltraAgent[];
  lead: UltraLead | null;
}

/** One step of a task's plan. Steps nest one level: a step with a `parent` is a sub-step of that top-level step. */
export interface PlanStep {
  /** Stable for the life of its plan, unique within its task. */
  id: number;
  /** The top-level step it is a sub-step of; none for a top-level step. */
  parent?: number | null;
  /** One short line ("Build the Mac app"). */
  title: string;
  /** A string, so a state the app doesn't know still reads (as not started). */
  state: StepState | (string & {});
  /** What it came to, or what blocks it. */
  note?: string | null;
  /** What it produced: a pull request, a page, a file. */
  link?: string | null;
  started?: Instant | null;
  /** When it was done or skipped. */
  finished?: Instant | null;
  updated: Instant;
}

/** The step a task is on: the first one being worked on, else the first not started. */
export interface CurrentStep {
  id: number;
  title: string;
  /** doing, todo or blocked. */
  state: StepState | (string & {});
}

/**
 * A plan at a glance, on every task of a server that keeps plans. Sub-steps count in place of the step they make up;
 * skipped steps count in neither number. `total` 0: no plan yet.
 */
export interface PlanSummary {
  done: number;
  total: number;
  /** The step being worked on, or next. */
  current?: CurrentStep | null;
  /** What blocks the plan: the first blocked step's note, else its title. */
  blocker?: string | null;
  /** The agent's estimate of what is left, in words ("about 2 hours"). */
  eta?: string | null;
  /** When the plan last changed; none while it is empty and never written. */
  updated?: Instant | null;
  /** A turn of work on the task ended while its plan was still empty: when the last one did. */
  unplannedTurn?: Instant | null;
}

/** A task's whole plan (GET /api/tasks/<id>'s `plan`). */
export interface Plan {
  /** Every step in order: each top-level step, then its sub-steps. */
  steps: PlanStep[];
  summary: PlanSummary;
}

/** A task (its key built by `taskKey`), yours or Emberhut's. Fields marked "fetched alone" come only from GET /api/tasks/<id>. */
export interface Issue {
  id: number;
  /** Its key, as `taskKey(212)` builds it. */
  key: string;
  title: string;
  body?: string | null;
  /** The owner (OWNER) or Emberhut (ASSISTANT); a string, so an unknown one still reads. */
  owner: string;
  status: TaskStatus;
  goal?: number | null;
  /** Its project's name. */
  project?: string | null;
  priority: number;
  due?: Day | null;
  today: boolean;
  /** Its project's id. */
  pid?: number | null;
  hard?: boolean | null;
  /** S, M, L, XL. */
  size?: string | null;
  /** Board only (state.issues): its own deadline or its project's. */
  deadline?: Day | null;
  decideBy?: Day | null;
  /** Board only (state.issues). */
  hardDeadline?: boolean | null;
  /** Board only (state.issues): the server's ranking score. */
  score?: number | null;
  decision?: Decision | null;
  /** When it needs you: what you should do, one short line. */
  action?: string | null;
  updated: Instant;
  doneAt?: Instant | null;
  last?: string | null;
  artifacts?: TaskArtifact[] | null;
  live?: TaskLive | null;
  parent?: number | null;
  progress?: number | null;
  total?: number | null;
  unit?: string | null;
  sub?: TaskRollup | null;
  /** Its area in the project. */
  area?: number | null;
  /** "finding" for a scan's result filed under it. */
  kind?: string | null;
  /** A finding's state: landed, partly, not_landed, superseded, unchecked. */
  landed?: string | null;
  fixedBy?: number | null;
  /** Emberhut's estimate, free text ("about 2 hours"). */
  eta?: string | null;
  /** Ultra mode: its run fans out to sub-agents. */
  ultra?: boolean | null;
  agents?: AgentsLine | null;
  /** Fetched alone. */
  history?: TaskMoment[] | null;
  subtasks?: Issue[] | null;
  findings?: Issue[] | null;
  links?: TaskLink[] | null;
  runs?: RunInfo[] | null;
  sessions?: RunnerSession[] | null;
  ultraRun?: UltraRun | null;
  /** Its plan at a glance, on a server that keeps plans (state.ui.taskPlans); `total` 0: no plan yet. */
  plan?: PlanSummary | null;
}

/** GET /api/tasks/<id>: the task with everything its page shows; its `plan` is the whole plan, step by step. */
export interface TaskPage extends Omit<Issue, "plan"> {
  /** On a server that keeps plans. */
  plan?: Plan | null;
}

/** GET /api/tasks. Statuses are the server's raw ones, comma separated. */
export interface TasksQuery {
  /** A task key (`taskKey(212)`), a bare number ("212") or words of its title and body. */
  q?: string;
  /** At most 200; the server's default is 50. */
  limit?: number;
  /** A project id, or "none" for tasks without one. */
  project?: number | "none";
  /** Only done and canceled ones. */
  closed?: boolean;
  /** Only canceled ones. */
  archived?: boolean;
  area?: number;
  status?: TaskStatus[];
  owner?: Author;
}

export interface TasksPage {
  tasks: Issue[];
}

/** POST /api/tasks/<id>/message: your reply on a task's page goes to its worker, its session or Emberhut in its room. */
export interface TaskMessageRequest {
  text: string;
  files: ChatFile[];
}

export interface TaskMessageResult {
  ok: boolean;
  to?: typeof ASSISTANT | "worker" | "session";
  session?: string;
  queued?: boolean;
  error?: string;
}
