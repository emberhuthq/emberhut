import {
  PLAN_HISTORY_OPENINGS,
  STEP_STATE,
  type EmberhutState,
  type Issue,
  type Plan,
  type PlanStep,
  type PlanSummary,
  type RunnerSession,
  type TaskArtifact,
  type TaskMoment,
  type RunInfo,
} from "@/api";
import { parseInstant } from "@/format/time";
import { flatTasks } from "./model";
import { activity, isRunningTask, stepState, type ActivityItem, type PlanRow } from "./plan";

// How the Tasks list and a task's page lay a plan out: the list's groups, each row's segmented bar and quiet clock, the
// page's counts, the checklist folded around the current step, and the activity feed with its filter.

/** The Tasks list while the server keeps plans. What needs you is counted only: it has a tab of its own. */
export interface PlanLists {
  /** Open tasks waiting on you (the Needs you tab lists them). */
  needs: number;
  /** A run is going on them; a blocked one stays here, with its red mark. */
  running: Issue[];
  /** Blocked with no run going. */
  blocked: Issue[];
  /** The rest: Today's first, then the others in their projects' order. */
  open: Issue[];
}

/** Whether a task's plan is stuck: a step is blocked. */
export const isBlocked = (task: Issue): boolean =>
  Boolean(task.plan?.blocker) || (task.plan?.current != null && stepState(task.plan.current.state) === STEP_STATE.blocked);

export function planLists(state: EmberhutState | undefined, gone: ReadonlySet<number>): PlanLists {
  const { needs, today, rest } = flatTasks(state, gone);
  const others = [...today, ...rest];
  return {
    needs: needs.length,
    running: others.filter(isRunningTask),
    blocked: others.filter((t) => !isRunningTask(t) && isBlocked(t)),
    open: others.filter((t) => !isRunningTask(t) && !isBlocked(t)),
  };
}

export type Segment = "done" | "doing" | "blocked" | "todo";

/** Past this many steps a bar is plain: segments would be too thin to read. */
export const maxSegments = 12;

/**
 * A row's bar from the plan at a glance: a segment per step it counts, done ones first, then the step it is on (or the
 * blocked one), then the rest. Null: no plan, or too many steps for segments.
 */
export function summarySegments(summary: PlanSummary | null | undefined): Segment[] | null {
  if (!summary || summary.total <= 0 || summary.total > maxSegments) return null;
  const done = Math.min(Math.max(summary.done, 0), summary.total);
  const current = summary.current ? stepState(summary.current.state) : null;
  const now: Segment | null = summary.blocker || current === STEP_STATE.blocked ? "blocked" : current === STEP_STATE.doing ? "doing" : null;
  const segments: Segment[] = Array.from({ length: summary.total }, (_, i) => (i < done ? "done" : "todo"));
  if (now && done < summary.total) segments[done] = now;
  return segments;
}

/** The steps a plan counts: steps without sub-steps, and sub-steps in place of theirs; skipped ones left out. */
export function countedSteps(plan: Plan): PlanStep[] {
  const parents = new Set(plan.steps.flatMap((s) => (s.parent != null ? [s.parent] : [])));
  return plan.steps.filter((s) => !parents.has(s.id) && stepState(s.state) !== STEP_STATE.skipped);
}

/** The page's bar: a segment per counted step, in the plan's order. Null past `maxSegments` (a plain bar then). */
export function planSegments(plan: Plan): Segment[] | null {
  const counted = countedSteps(plan);
  if (counted.length === 0 || counted.length > maxSegments) return null;
  return counted.map((s) => {
    const state = stepState(s.state);
    return state === STEP_STATE.done ? "done" : state === STEP_STATE.doing ? "doing" : state === STEP_STATE.blocked ? "blocked" : "todo";
  });
}

/** "3 of 6 done · 1 skipped": skipped steps don't count, a step with sub-steps counts as its sub-steps. */
export function countLine(plan: Plan): string {
  const { done, total } = plan.summary;
  const parents = new Set(plan.steps.flatMap((s) => (s.parent != null ? [s.parent] : [])));
  const skipped = plan.steps.filter((s) => !parents.has(s.id) && stepState(s.state) === STEP_STATE.skipped).length;
  const blocked = countedSteps(plan).filter((s) => stepState(s.state) === STEP_STATE.blocked).length;
  return [`${done} of ${total} done`, blocked > 0 ? `${blocked} blocked` : null, skipped > 0 ? `${skipped} skipped` : null]
    .filter(Boolean)
    .join(" · ");
}

/** The current step with the step it is part of: "Build the Windows installer › Sign the installer". */
export function currentPath(plan: Plan): string | null {
  const id = plan.summary.current?.id;
  const step = plan.steps.find((s) => s.id === id);
  if (!step) return plan.summary.current?.title ?? null;
  const parent = step.parent != null ? plan.steps.find((s) => s.id === step.parent) : undefined;
  return parent ? `${parent.title} › ${step.title}` : step.title;
}

/** The steps after the current one still to do, at most `count`: what comes next. */
export function nextSteps(plan: Plan, count = 2): PlanStep[] {
  const counted = countedSteps(plan);
  const at = counted.findIndex((s) => s.id === plan.summary.current?.id);
  return counted
    .slice(at + 1)
    .filter((s) => stepState(s.state) === STEP_STATE.todo)
    .slice(0, count);
}

/** How long it has gone without a word: "25m", "1h 05m". */
export function quietText(minutes: number): string {
  if (minutes < 60) return `${minutes}m`;
  return `${Math.floor(minutes / 60)}h ${String(minutes % 60).padStart(2, "0")}m`;
}

/** A running task is quiet after this long with neither its plan nor its session moving. */
export const quietAfterMinutes = 20;

/**
 * How many minutes a running task has gone with neither its plan nor its session moving, once that is 20 or more; null
 * while it moves, or when it isn't running. Its session: the run's thread, else a session that names the task.
 */
export function quietMinutes(task: Issue, sessions: readonly RunnerSession[], now: Date = new Date()): number | null {
  if (!isRunningTask(task) || task.live?.state !== "running") return null;
  const thread = task.live.run?.thread;
  const times = [
    task.plan?.updated,
    task.live.run?.started,
    task.live.since,
    ...sessions.filter((s) => s.id === thread || s.tasks.includes(task.id)).map((s) => s.updatedAt),
  ].flatMap((at) => {
    const date = parseInstant(at);
    return date ? [date.getTime()] : [];
  });
  if (times.length === 0) return null;
  const minutes = Math.floor((now.getTime() - Math.max(...times)) / 60_000);
  return minutes >= quietAfterMinutes ? minutes : null;
}

/** The current step's time so far: "8m so far". Only the current step shows a duration. */
export function soFar(step: PlanStep, now: Date = new Date()): string | null {
  const started = parseInstant(step.started);
  if (!started || stepState(step.state) !== STEP_STATE.doing) return null;
  const minutes = Math.max(0, Math.floor((now.getTime() - started.getTime()) / 60_000));
  return minutes < 1 ? "just started" : `${quietText(minutes)} so far`;
}

/** Past this many rows, the checklist folds around the current step. */
export const foldAfter = 8;

export type ChecklistEntry = { kind: "step"; row: PlanRow } | { kind: "fold"; place: "before" | "after"; rows: PlanRow[]; label: string };

/**
 * The checklist, folded once it has more than `foldAfter` rows so what needs you stays near the top: the done steps
 * before the last one fold into "11 done", then the last done step, the current one and the next two, then the rest
 * into "5 more". A blocked step is never folded away, and a shown sub-step keeps its step above it. `open`: everything.
 */
export function foldChecklist(rows: readonly PlanRow[], open = false): ChecklistEntry[] {
  if (open || rows.length <= foldAfter) return rows.map((row) => ({ kind: "step", row }));
  let current = rows.findIndex((r) => r.current);
  if (current < 0) current = rows.findIndex((r) => r.state !== STEP_STATE.done && r.state !== STEP_STATE.skipped);
  if (current < 0) current = rows.length - 1;
  const shown = new Set<number>();
  const lastDone = rows.slice(0, current).findLastIndex((r) => r.state === STEP_STATE.done);
  if (lastDone >= 0) shown.add(lastDone);
  for (let i = current; i <= current + 2 && i < rows.length; i++) shown.add(i);
  rows.forEach((r, i) => {
    if (r.state === STEP_STATE.blocked) shown.add(i);
  });
  // A sub-step shown keeps its step above it, so it reads in place.
  for (const i of [...shown]) {
    const parent = rows[i]!.step.parent;
    if (rows[i]!.depth === 1 && parent != null) {
      const at = rows.findIndex((r) => r.step.id === parent);
      if (at >= 0) shown.add(at);
    }
  }
  const entries: ChecklistEntry[] = [];
  let hidden: PlanRow[] = [];
  const flush = (place: "before" | "after") => {
    if (hidden.length === 0) return;
    const done = hidden.filter((r) => r.state === STEP_STATE.done).length;
    const label =
      place === "before" && done === hidden.length
        ? `${done} done`
        : place === "before"
          ? `${hidden.length} earlier`
          : `${hidden.length} more`;
    entries.push({ kind: "fold", place, rows: hidden, label });
    hidden = [];
  };
  rows.forEach((row, i) => {
    if (shown.has(i)) {
      flush(i < current ? "before" : "after");
      entries.push({ kind: "step", row });
    } else hidden.push(row);
  });
  flush("after");
  return entries;
}

/** What a step produced, for Files and results: each with the step that made it. */
export function stepResults(plan: Plan | null): { step: PlanStep; link: string }[] {
  return (plan?.steps ?? []).flatMap((step) => (step.link?.trim() ? [{ step, link: step.link.trim() }] : []));
}

export type FeedFilter = "all" | "turns" | "plan" | "files";

export type FeedItem =
  | { kind: "turn"; at: number; item: ActivityItem }
  | { kind: "moment"; at: number; moment: TaskMoment; plan: boolean }
  | { kind: "file"; at: number; artifact: TaskArtifact };

/**
 * A history line about the plan: a step ticked, started, skipped or blocked, the plan written or changed. Read by how the
 * server's plan lines start, so a message that only mentions a plan or a step isn't one.
 */
export const isPlanMoment = (text: string): boolean => PLAN_HISTORY_OPENINGS.some((opening) => text.startsWith(opening));

/**
 * The task's activity, newest first: its turns of work (each in a line or two), its history and what it produced, in one
 * feed. `filter` keeps only turns, plan events (ticks, skips, blocks, plan changes) or files.
 */
export function activityFeed(
  runs: readonly RunInfo[],
  sessions: readonly RunnerSession[],
  history: readonly TaskMoment[],
  artifacts: readonly TaskArtifact[],
  filter: FeedFilter = "all",
): FeedItem[] {
  const time = (...at: (string | null | undefined)[]) => {
    for (const a of at) {
      const date = parseInstant(a);
      if (date) return date.getTime();
    }
    return 0;
  };
  // A finished turn also leaves a history line with its answer and STATUS: the turn says it already.
  const turnLines = history.filter((m) => m.text.includes(" · STATUS:")).length;
  const turns: FeedItem[] = activity(runs, sessions).map((item) => ({
    kind: "turn",
    at: time(item.run.started, item.run.requested, item.run.ended) || Number.MAX_SAFE_INTEGER,
    item,
  }));
  const moments: FeedItem[] = history
    .filter((m) => !(runs.length > 0 && turnLines > 0 && m.text.includes(" · STATUS:")))
    .map((moment) => ({ kind: "moment", at: time(moment.at), moment, plan: isPlanMoment(moment.text) }));
  const files: FeedItem[] = artifacts.map((artifact) => ({ kind: "file", at: time(artifact.at), artifact }));
  const all = [...turns, ...moments, ...files].sort((a, b) => b.at - a.at);
  switch (filter) {
    case "turns":
      return all.filter((f) => f.kind === "turn");
    case "plan":
      return all.filter((f) => f.kind === "moment" && f.plan);
    case "files":
      return all.filter((f) => f.kind === "file");
    default:
      return all;
  }
}

/**
 * What a task's row in the Tasks list draws, as one string: two rows with the same key look the same, so a new state
 * that left a task as it was draws nothing again. Its title, state, ask and its plan at a glance.
 */
export function taskRowKey(task: Issue): string {
  const plan = task.plan;
  const current = plan?.current;
  return [
    task.id,
    task.title,
    task.status,
    task.action ?? task.decision?.question ?? "",
    task.live?.state ?? "",
    plan ? `${plan.done}/${plan.total}` : "-",
    current ? `${current.id}:${current.state}:${current.title}` : "",
    plan?.blocker ?? "",
  ].join("\u0000");
}
