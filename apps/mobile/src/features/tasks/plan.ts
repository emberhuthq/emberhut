import {
  isServerPath,
  STEP_STATE,
  type EmberhutState,
  type Issue,
  type Plan,
  type PlanStep,
  type PlanSummary,
  type RunInfo,
  type RunnerSession,
  type StepState,
  type TaskPage,
} from "@/api";
import { parseInstant, relative } from "@/format/time";
import { allSessions, isRunning } from "@/features/sessions/sessionList";
import { runStatus, type RunStatus } from "./taskPage";

// Tasks with plans (state.ui.taskPlans): each task carries the plan its agent ticks as it goes, and tasks, not sessions,
// are the work running. Everything here is used only while the server says it keeps plans.

/** Whether the server keeps plans. An older server sends no such switch: off, and the app is as it was. */
export const plansOn = (state: EmberhutState | undefined): boolean => state?.ui?.taskPlans === true;

const known = new Set<string>(Object.values(STEP_STATE));

/** A step's state as the app draws it: one it doesn't know reads as not started. */
export const stepState = (state: string): StepState => (known.has(state) ? (state as StepState) : STEP_STATE.todo);

/** No steps yet. */
export const isEmptyPlan = (summary: PlanSummary): boolean => summary.total <= 0;

/** How far along, 0…1; null while there is no plan. */
export function planFraction(summary: PlanSummary): number | null {
  if (isEmptyPlan(summary)) return null;
  return Math.min(Math.max(summary.done, 0) / summary.total, 1);
}

/** "4/9", the ticks at a glance. */
export const planTicks = (summary: PlanSummary): string => `${summary.done}/${summary.total}`;

export type CurrentKind = "doing" | "next" | "blocked" | "finished" | "none";

/** What a running task is on, in one line: the step it works on, what blocks it, or what's next. */
export function currentLine(summary: PlanSummary): { kind: CurrentKind; text: string } {
  if (isEmptyPlan(summary)) return { kind: "none", text: "No plan yet" };
  const current = summary.current;
  if (summary.blocker) return { kind: "blocked", text: `Blocked: ${summary.blocker}` };
  if (current && stepState(current.state) === STEP_STATE.blocked) return { kind: "blocked", text: `Blocked: ${current.title}` };
  if (current && stepState(current.state) === STEP_STATE.doing) return { kind: "doing", text: current.title };
  if (current) return { kind: "next", text: `Next: ${current.title}` };
  if (summary.done >= summary.total) return { kind: "finished", text: "Every step done" };
  return { kind: "none", text: `${summary.done} of ${summary.total} done` };
}

/** The plan's header line: "4 of 9 done · ETA about 2 hours · updated 3 min ago". */
export function summaryLine(summary: PlanSummary, now: Date = new Date()): string {
  if (isEmptyPlan(summary)) return "No plan yet";
  const updated = parseInstant(summary.updated);
  return [
    `${summary.done} of ${summary.total} done`,
    summary.eta && summary.done < summary.total ? `ETA ${summary.eta}` : null,
    updated ? `updated ${relative(updated, now)}` : null,
  ]
    .filter(Boolean)
    .join(" · ");
}

/** The quiet line under "No plan yet" when a turn of work ended without writing one. */
export function unplannedLine(summary: PlanSummary, now: Date = new Date()): string | null {
  const at = parseInstant(summary.unplannedTurn);
  if (!isEmptyPlan(summary) || !at) return null;
  return `A turn of work ended ${relative(at, now)} without a plan.`;
}

export interface PlanRow {
  step: PlanStep;
  state: StepState;
  /** 1 for a sub-step, drawn indented under its step. */
  depth: 0 | 1;
  /** The step the task is on now. */
  current: boolean;
}

/**
 * The checklist: each top-level step, then its sub-steps. A sub-step whose step isn't in the plan (or is itself a
 * sub-step: plans nest one level) is listed as a step of its own, where it came.
 */
export function planRows(plan: Plan): PlanRow[] {
  const tops = new Set(plan.steps.filter((s) => s.parent == null).map((s) => s.id));
  const isSub = (s: PlanStep) => s.parent != null && tops.has(s.parent);
  const currentId = plan.summary.current?.id ?? plan.steps.find((s) => stepState(s.state) === STEP_STATE.doing)?.id;
  const row = (step: PlanStep, depth: 0 | 1): PlanRow => ({ step, state: stepState(step.state), depth, current: step.id === currentId });
  return plan.steps
    .filter((s) => !isSub(s))
    .flatMap((top) => [row(top, 0), ...(tops.has(top.id) ? plan.steps.filter((s) => s.parent === top.id).map((s) => row(s, 1)) : [])]);
}

const stateWords: Record<StepState, string> = {
  todo: "not started",
  doing: "in progress",
  done: "done",
  blocked: "blocked",
  skipped: "skipped",
};

/** What a screen reader says for a step: "Sub-step, Build the Mac app, in progress, current step". */
export function stepLabel(row: PlanRow): string {
  return [
    row.depth === 1 ? "Sub-step" : null,
    row.step.title,
    stateWords[row.state],
    row.current && row.state !== STEP_STATE.done ? "current step" : null,
    row.step.note ? row.step.note : null,
  ]
    .filter(Boolean)
    .join(", ");
}

/** Whether a step's link opens outside the app: only web links do. */
export const isWebLink = (link: string | null | undefined): boolean => /^https?:\/\//i.test(link ?? "");

/**
 * What a step's link is and how it reads: a web page (opened in the browser), a file on the server (opened full screen,
 * with the session), or anything else, shown as text and not opened.
 */
export function stepLink(link: string | null | undefined): { kind: "web" | "server" | "text"; text: string } | null {
  const given = link?.trim();
  if (!given) return null;
  if (isWebLink(given)) {
    const [, host = "", path = ""] = /^https?:\/\/(?:www\.)?([^/?#]+)([^?#]*)/i.exec(given) ?? [];
    const parts = path.split("/").filter(Boolean);
    const shown = parts.length <= 2 ? parts.join("/") : `…/${parts.slice(-2).join("/")}`;
    return { kind: "web", text: shown ? `${host}/${shown}` : host };
  }
  if (isServerPath(given)) {
    // A file's name when the path ends in one; an artifact's bare number says nothing.
    const last = given.split(/[/?#]/).filter(Boolean).pop() ?? "";
    return { kind: "server", text: /\.[A-Za-z0-9]+$/.test(last) ? last : "The file" };
  }
  return { kind: "text", text: given };
}

/**
 * The page's whole plan. The page's `plan` is the whole plan; a summary alone in its place (the task's own field) still
 * reads, as a plan with no steps listed.
 */
export function pagePlan(page: TaskPage | undefined): Plan | null {
  const plan: unknown = page?.plan;
  if (!plan || typeof plan !== "object") return null;
  if (Array.isArray((plan as Plan).steps) && (plan as Plan).summary) return plan as Plan;
  if (typeof (plan as PlanSummary).total === "number") return { steps: [], summary: plan as PlanSummary };
  return null;
}

/** The page as a task, its plan at a glance in the task's own field. */
export function pageTask(page: TaskPage): Issue {
  return { ...page, plan: pagePlan(page)?.summary ?? null };
}

/** A run is going on it now, or is about to start. */
export const isRunningTask = (task: Issue): boolean => task.live?.state === "running" || task.live?.state === "queued";

/** How long a finished session with no task stays in the Tasks list. */
const sessionShownMs = 24 * 3_600_000;

/**
 * Sessions with no task (yours and Emberhut's, once each), so nothing that runs leaves the Tasks list: running ones,
 * ones waiting on you, and the rest for a day after they last moved. Running first, then waiting, then the newest.
 */
export function tasklessSessions(state: EmberhutState | undefined, now: Date = new Date()): RunnerSession[] {
  const recent = (s: RunnerSession) => now.getTime() - (parseInstant(s.updatedAt)?.getTime() ?? 0) < sessionShownMs;
  const rank = (s: RunnerSession) => (isRunning(s.status) ? 0 : s.status === "waiting" ? 1 : 2);
  return allSessions(state)
    .filter((s) => s.tasks.length === 0 && (isRunning(s.status) || s.status === "waiting" || recent(s)))
    .sort((a, b) => rank(a) - rank(b) || b.updatedAt.localeCompare(a.updatedAt));
}

/** A text's first line that says something, without Markdown's heading and list marks, at most `max` characters. */
export function firstLine(text: string | null | undefined, max = 160): string | null {
  const line = (text ?? "")
    .split("\n")
    .map((l) => l.replace(/^\s*(#+|[-*>]|\d+\.)\s+/, "").trim())
    .find((l) => l !== "");
  if (!line) return null;
  return line.length > max ? `${line.slice(0, max - 1).trimEnd()}…` : line;
}

export interface ActivityItem {
  run: RunInfo;
  /** Still going. */
  going: boolean;
  /** What it was asked, its first line. */
  asked: string | null;
  /** What it answered, its first line. */
  said: string | null;
  /** Its STATUS line. */
  status: RunStatus | null;
  /** How it ended, when not as it should have. */
  failed: string | null;
  /** Its session, when the page lists it: one tap opens it. */
  session: RunnerSession | null;
}

/** The turns of work on a task as a timeline, newest first: what each was asked and what came of it, in a line each. */
export function activity(runs: readonly RunInfo[], sessions: readonly RunnerSession[]): ActivityItem[] {
  return runs.map((run) => ({
    run,
    going: run.ended == null,
    asked: firstLine(run.asked),
    said: firstLine(run.answer),
    status: runStatus(run.status),
    failed: run.outcome && run.outcome !== "completed" && run.ended != null ? run.outcome : null,
    session: sessions.find((s) => s.id === run.thread) ?? null,
  }));
}

/** The task's sessions to open: running ones first, then the newest. */
export function taskSessions(sessions: readonly RunnerSession[]): RunnerSession[] {
  return [...sessions].sort((a, b) => Number(isRunning(b.status)) - Number(isRunning(a.status)) || b.updatedAt.localeCompare(a.updatedAt));
}

/**
 * What says the open page is out of date: the task changed, its plan moved, or its run started or ended. The state
 * carries these on every poll; the page is fetched again when they change.
 */
export function pageStamp(task: Issue | undefined): string | undefined {
  if (!task) return undefined;
  const plan = task.plan;
  return [
    task.updated,
    plan?.updated ?? "",
    plan ? `${plan.done}/${plan.total}` : "",
    task.live?.state ?? "",
    task.live?.last?.ended ?? "",
  ].join("|");
}
