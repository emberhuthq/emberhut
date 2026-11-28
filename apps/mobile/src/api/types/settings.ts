import type { Day, Instant } from "./common";
import type { AssistantAccount, UsageRow } from "./state";

/**
 * POST /api/settings. One body may carry several of these; the server runs them in a fixed order and answers
 * `{ ok: true, ...state }` for `room` (or Main), or `{ ok: false, error }`.
 */
export interface SettingsChange {
  room?: number;
  /** Pause or resume Emberhut. */
  on?: boolean;
  /** Stop the answer Emberhut is writing in `room`; its background jobs keep running. */
  interrupt?: true;
  cancelCheckin?: Instant;
  stopWatch?: number;
  /** A background job's id. */
  stopTask?: string;
  todoDone?: number;
  done?: boolean;
  /** Not now: parked until `until`. */
  todoPark?: number;
  until?: Instant | null;
  taskArchive?: number;
  taskUnarchive?: number;
  /** Your answer to an ask, with `choice`. "Leave it", "Not now" and "No" cancel the task; "Done" marks it done. */
  taskDecide?: number;
  choice?: string;
  ui?: { hideGoals?: boolean; hideProjects?: boolean };
  /** Sign a device out. */
  authRevoke?: number;
  /** An X reply you posted, with `option` (0-based); also marks the task done. */
  xPosted?: number;
  option?: number;
  factSet?: { id?: string; category: string; title: string; text: string; project?: string | null };
  personSet?: { id: string; important: boolean };
  /** Forget a fact everywhere: the fact, what was said about it in your ember's record, and a tombstone so it isn't kept again. */
  factDrop?: string;
  /** Forget a fact everywhere, with the pieces of the record you picked (all when left out). */
  factForget?: { id: string; pieces?: number[] };
  /** Confirm a fact that waits for you. */
  factConfirm?: string;
  /** Pin a fact in what your ember always knows, hide it from that, or neither (null). */
  factProfile?: { id: string; profile: "pin" | "hide" | null };
  /** The nightly memory review on or off. */
  memoryNightly?: boolean;
  trialDrop?: number;
  /** Keep a lesson on trial now, as a standing rule, instead of waiting for the weekly review. */
  trialKeep?: number;
  /** Stop a task's worker. */
  taskStop?: number;
  /** A new task's title, with `goal` or `project`. */
  taskCreate?: string;
  goal?: number;
  project?: number;
  projectSet?: {
    id: number;
    priority?: number;
    deadline?: Day | null;
    hard?: boolean;
    /** paused (with pausedUntil), archived, active, done. */
    status?: string;
    pausedUntil?: Day;
  };
  taskSet?: {
    id: number;
    deadline?: Day | null;
    hard?: boolean;
    size?: string | null;
    project?: number;
    decideBy?: Day | null;
    body?: string;
    ultra?: boolean;
  };
}

/** GET /api/switches: Emberhut's MCP servers and skills, each on or off. */
export interface SwitchPage {
  at: Instant;
  account: string | null;
  live: boolean;
  mcp: {
    name: string;
    state: "connected" | "failed" | "needs-auth" | "pending" | "disabled" | "off" | "unknown";
    error: string | null;
    source: string | null;
    kind: string | null;
    on: boolean;
    /** Emberhut's own server: no switch. */
    fixed: boolean;
    note: string | null;
  }[];
  skills: { name: string; description: string; source: string; on: boolean }[];
  note: string | null;
}

export type SwitchAction = { kind: "mcp" | "skill"; name: string; on: boolean } | { kind: "reconnect"; name: string };

export interface Environment {
  id: string;
  /** connections, runners, workers, browser, or the external threads' group (EXTERNAL_THREADS_GROUP in wire.ts). */
  group: string;
  name: string;
  icon: string;
  tint: string;
  state: "ok" | "warn" | "down" | "off";
  status: string;
  detail: string;
  at?: Instant;
  legacy?: boolean;
  facts: { label: string; value: string; state?: string }[];
  lists: { title: string; items: { name: string; value?: string; note?: string; state?: string }[]; empty?: string }[];
  error?: string;
  hint?: string;
  actions: { id: string; label: string }[];
  lastError?: { error: string; at: Instant };
}

/** GET /api/environments: the sources and machines Emberhut depends on. */
export interface EnvironmentsPage {
  at: Instant;
  summary: { ok: number; warn: number; down: number };
  groups: { id: string; title: string; footer?: string; items: Environment[] }[];
  note?: string;
  link?: string;
}

/** POST /api/accounts/use. */
export interface AccountUseResult extends AssistantAccount {
  ok: boolean;
  usage?: UsageRow[];
  error?: string;
}
