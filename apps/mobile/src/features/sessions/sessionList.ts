import { taskKey, type AgentNow, type EmberhutState, type RunnerSession, type ExternalThread } from "@/api";
import type { MachineNames } from "@/features/machines/machineNames";
import {
  agentFacts,
  keeps,
  sessionFacts,
  sessionPicks,
  externalThreadFacts,
  type SessionFilter,
  type SessionPicks,
} from "./sessionFilters";

// The Sessions tab's lists: your own sessions and Emberhut's, every room's, once each; the agents working on your tasks.

export const isRunning = (status: string): boolean => status === "running" || status === "queued";

/** Your sessions and Emberhut's, once each (your own list knows what a waiting one asks). */
export function allSessions(state: EmberhutState | undefined): RunnerSession[] {
  const mine = state?.mySessions ?? [];
  const ids = new Set(mine.map((s) => s.id));
  return [...mine, ...(state?.sessions ?? []).filter((s) => !ids.has(s.id))];
}

export interface SessionGroups {
  agents: AgentNow[];
  running: RunnerSession[];
  recent: RunnerSession[];
  external: ExternalThread[];
  /** The Worker and Environment choices, each with its count (sessionFilters.ts). */
  picks: SessionPicks;
  /** Rows the filter leaves out. */
  hidden: number;
}

/**
 * Running ones (without those an agent row already shows), then the rest: waiting on you first, then the newest. The
 * filter (none: all) narrows every list; the picks count what each choice would show.
 */
export function groupSessions(state: EmberhutState | undefined, filter: SessionFilter = {}, names?: MachineNames): SessionGroups {
  const allAgents = state?.agentsNow ?? [];
  const leads = new Set(allAgents.flatMap((a) => (a.lead && a.viewer ? [a.viewer] : [])));
  // A running lead's session is its Working now row, whether the filter keeps that row or not.
  const all = allSessions(state).filter((s) => !(isRunning(s.status) && leads.has(s.id)));
  const allExternal = state?.t3Running ?? [];
  const agents = allAgents.filter((a) => keeps(filter, agentFacts(a)));
  const shown = all.filter((s) => keeps(filter, sessionFacts(s)));
  const external = allExternal.filter((t) => keeps(filter, externalThreadFacts(t)));
  const running = shown.filter((s) => isRunning(s.status));
  const recent = shown
    .filter((s) => !isRunning(s.status))
    .sort((a, b) => Number(b.status === "waiting") - Number(a.status === "waiting") || b.updatedAt.localeCompare(a.updatedAt));
  const picks = sessionPicks(
    filter,
    [...allAgents.map(agentFacts), ...all.map(sessionFacts), ...allExternal.map(externalThreadFacts)],
    names,
  );
  const hidden = allAgents.length + all.length + allExternal.length - agents.length - shown.length - external.length;
  return { agents, running, recent, external, picks, hidden };
}

/** Agents by the task they work on, each task's lead first. */
export function agentsByTask(agents: readonly AgentNow[]): AgentNow[][] {
  const tasks = new Map<number, AgentNow[]>();
  for (const a of agents) tasks.set(a.task, [...(tasks.get(a.task) ?? []), a]);
  return [...tasks.values()].map((group) => [...group].sort((a, b) => Number(b.lead) - Number(a.lead)));
}

export const shortTitle = (title: string): string => title.replace(/^Emberhut · /, "");

export function sessionStatusText(status: string): string {
  switch (status) {
    case "running":
      return "Running";
    case "queued":
      return "Starting";
    case "completed":
      return "Finished its turn";
    case "failed":
      return "Failed";
    case "interrupted":
      return "Stopped";
    case "waiting":
      return "Waiting for you";
    case "idle":
      return "Ready";
    default:
      return status.charAt(0).toUpperCase() + status.slice(1);
  }
}

/** Its tasks, project, machine (the server's `place`) and account, muted: its task keys, then "Website · Studio · Personal". */
export function sessionMeta(s: RunnerSession): string {
  return [s.tasks.map(taskKey).join(", "), s.project, s.place, s.accountName ?? s.account].filter((x): x is string => !!x).join(" · ");
}

/** A session's words for a preview: no markdown marks, never the English coach's line an answer can open with. */
export function plainSessionText(s: string | null | undefined): string | null {
  if (!s) return null;
  const text = s.replace(/^\s*💬 (More natural: ".*?" — \d+\/10( · \[[^\]]*\])?|Nicely phrased — \d+\/10)\s*/, "").replace(/[*`]/g, "");
  return text.trim() === "" ? null : text;
}

/** What it said last (or asks you, while it waits), whole: a reply cut near the server's 1,500 ends at a word. */
export function sessionSummary(s: RunnerSession): string | null {
  const text = plainSessionText(s.status === "waiting" ? (s.question ?? s.last) : (s.last ?? s.statusLine));
  if (text === null) return null;
  const whole = /[.!?)"]$/.test(text) || /STATUS: [^.]*$/.test(text);
  const cut = text.lastIndexOf(" ");
  return text.length >= 1400 && !whole && cut > 0 ? `${text.slice(0, cut)} …` : text;
}

/** A folder in a home folder as ~, on Linux or macOS: "~/Documents/emberhut". */
export const shortPath = (p: string): string => p.replace(/^\/(home|Users)\/[^/]+\//, "~/");

/** "0:42", "12:05", "1:02:13": how long since `since`, ticking every second. */
export function elapsed(since: number, now: number): string {
  const s = Math.max(0, Math.floor((now - since) / 1000));
  const pad = (n: number) => String(n).padStart(2, "0");
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return h > 0 ? `${h}:${pad(m)}:${pad(s % 60)}` : `${m}:${pad(s % 60)}`;
}

export interface SessionModel {
  id: string;
  name: string;
}

/** Only for a server that names no models (GET /api/parties' `models` missing or empty): what it took when written. */
export const fallbackModels: readonly SessionModel[] = [
  { id: "claude-opus-5-5", name: "Opus 5.5" },
  { id: "claude-fable-5-1", name: "Fable 5.1" },
  { id: "claude-sonnet-5-5", name: "Sonnet 5.5" },
  { id: "claude-haiku-5-5", name: "Haiku 5.5" },
];

/** The models a session can run: the server's list (GET /api/parties' `models`), else the fallback. */
export const sessionModels = (server: readonly SessionModel[] | null | undefined): readonly SessionModel[] =>
  server && server.length > 0 ? server.map(({ id, name }) => ({ id, name: name || id })) : fallbackModels;

export const efforts = ["low", "medium", "high", "xhigh", "max"] as const;

export const modelName = (id: string | null | undefined, models: readonly SessionModel[] = fallbackModels): string | null =>
  id ? (models.find((m) => m.id === id)?.name ?? id) : null;
export const effortName = (e: string): string => (e === "xhigh" ? "Extra high" : e.charAt(0).toUpperCase() + e.slice(1));
