// The words of the runner protocol, in one place: the server's routes the runner calls, the event kinds it reports,
// the statuses, step kinds and live events it sends. They must match the server's contract (crates/emberhut-api's
// runner types and crates/emberhut-server's runner routes); test/contract.test.ts holds them to the shared fixtures.

/** The server's routes a runner calls, each with its runner token. */
export const SERVER = {
  /** Every change of a session, one event each (`POST`). */
  events: "/api/runner/events",
  /** A sample of the machine, every 20 s (`POST`). */
  machine: "/api/runner/machine",
  /** The MCP servers the runner's sessions get too (`GET`). */
  mcp: "/api/runner/mcp",
  /** Trading a pairing code for the runner's name and token (`POST`, no token yet). */
  pair: "/api/runner/pair",
} as const;

/** The kinds of event a runner reports. Each carries the session as it is now, so taking one twice changes nothing. */
export const EVENT = {
  /** A session was opened. */
  session: "session",
  /** A turn was asked for (`turn.prompt`, `turn.by`). */
  turnQueued: "turn_queued",
  /** A turn started (`turn.started`). */
  turnStarted: "turn_started",
  /** What the running turn does now (`doing`). */
  activity: "activity",
  /** A turn ended (`turn.outcome`, `turn.last`). */
  turnFinished: "turn_finished",
  /** The session asks its owner something (`text`). */
  question: "question",
  /** A line for the activity log (`text`). */
  note: "note",
  /** The session moved to another account, or every account is out (`from`, `to`, `why`, `resumeAt`). */
  account: "account",
} as const;

/** A session's status. `completed`, `failed`, `interrupted`, `waiting` and `cancelled` end a turn for the server. */
export type SessionStatus = "queued" | "running" | "completed" | "failed" | "interrupted" | "waiting" | "idle";

/** How a turn ended. */
export type Outcome = "completed" | "failed" | "interrupted" | "cancelled";

/** Who asked for a turn: the owner (from an app), the assistant, the agent itself (`self`) or this runner. */
export type By = "owner" | "assistant" | "self" | "runner";
/** Who asked, when the server says nothing: a session it opens for work comes from the assistant. */
export const DEFAULT_BY: By = "assistant";

/** The kinds of step in a session's transcript. */
export type StepKind = "user" | "text" | "tool" | "result" | "end" | "system" | "question" | "thinking" | "todos";

/** The events of a session's live stream (server-sent events, one `data:` line each). */
export const LIVE = {
  /** The session's head: status, current step, running since. */
  head: "head",
  /** A new step of the transcript. */
  step: "step",
  /** Words being typed (`start`: a new block). */
  delta: "delta",
  /** Thinking being written (`start`: a new block, `since`: when it began). */
  think: "think",
  /** The running turn's output tokens so far. */
  tokens: "tokens",
} as const;

/** Efforts a session may run at. */
export const EFFORTS = ["low", "medium", "high", "xhigh", "max"] as const;

/** A session id: `r-` and 8 lowercase hex digits (the server accepts `r-` and lowercase hex only in a runner's path). */
export const SESSION_ID = /^r-[0-9a-f]{8}$/;
/** A turn id: its session's id, `:` and its number. */
export const TURN_ID = /^r-[0-9a-f]+:\d+$/;

/** The label put before a message the assistant writes into the owner's own session, which the server strips back off. */
export const FROM_ASSISTANT = "(From the assistant, writing in this session for its owner)\n\n";
