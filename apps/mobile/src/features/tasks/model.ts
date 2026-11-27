import { OWNER, taskKey, type AreaTree, type Issue, type EmberhutState, type Project, type TaskRollup } from "@/api";
import { parseInstant, relative, shortDate } from "@/format/time";
import { needsRows, rowWhen } from "@/features/needsYou/model";

// Tasks as the app reads them: where each stands and the lists by project.
// A project's areas are in laneModel.ts, a task's own page in taskPage.ts.

/** What is happening on it (its runs), not the label Emberhut last set: "working" only while a run is open. */
export function taskState(task: Issue): string {
  return task.live?.state ?? (task.status === "working" ? "stalled" : task.status);
}

/** Still to be done: what Not now and Archive act on. */
export const isOpen = (task: Issue): boolean => task.status === "todo" || task.status === "working" || task.status === "waiting";

export interface TodayLists {
  /** What you have to decide today; the rest waits in Needs you. */
  decide: Issue[];
  /** The server's ranked list for today, without what waits on you. */
  ranked: Issue[];
}

/** The Today page's two lists; More counts the same tasks. */
export function todayLists(state: EmberhutState | undefined, gone: ReadonlySet<number>, now: Date = new Date()): TodayLists {
  const open = (state?.issues ?? []).filter((t) => isOpen(t) && !gone.has(t.id));
  const due = new Set(needsRows(state, gone).flatMap((r) => (r.task != null && rowWhen(r, now) === "today" ? [r.task] : [])));
  return {
    decide: open.filter((t) => t.status === "waiting" && due.has(t.id)),
    ranked: (state?.today ?? []).flatMap((id) => open.filter((t) => t.id === id && t.status !== "waiting")),
  };
}

export const isMine = (task: Issue): boolean => task.owner === OWNER;

export type StateTone = "needsYou" | "working" | "done" | "danger" | "muted";

export function stateTone(task: Issue): StateTone {
  switch (taskState(task)) {
    case "waiting":
      return "needsYou";
    case "running":
    case "queued":
    case "scheduled":
      return "working";
    case "done":
      return "done";
    case "stalled":
      return task.live?.alarm ? "danger" : "muted";
    default:
      return "muted";
  }
}

export function statusName(task: Issue): string {
  switch (taskState(task)) {
    case "waiting":
      return "Needs you";
    case "running":
      return "Emberhut is working";
    case "queued":
      return "Queued";
    case "stalled":
      return "Nothing running";
    case "scheduled":
      return task.live?.checking ? "Checking now" : "Scheduled";
    case "parked":
      return "Parked";
    case "held":
      return "On hold";
    case "done":
      return "Done";
    case "canceled":
      return "Archived";
    default:
      return task.live ? "Not started" : "To do";
  }
}

/** 240 → "4 h", 90 → "1 h 30 min", 1440 → "day". */
function interval(minutes: number): string {
  if (minutes % 1440 === 0) return minutes === 1440 ? "day" : `${minutes / 1440} days`;
  if (minutes < 60) return `${minutes} min`;
  return minutes % 60 === 0 ? `${minutes / 60} h` : `${Math.floor(minutes / 60)} h ${minutes % 60} min`;
}

/** One plain line on what's happening: where it runs and since when, or what ran last and when. */
export function liveLine(task: Issue, now: Date = new Date()): string | null {
  const live = task.live;
  if (!live) return null;
  const ago = (at: string | null | undefined) => {
    const date = parseInstant(at);
    return date ? relative(date, now) : null;
  };
  switch (live.state) {
    case "scheduled": {
      const next = parseInstant(live.watch?.nextAt ?? live.checkin?.at);
      if (!next) return null;
      const every = live.watch ? `every ${interval(live.watch.every)}` : null;
      return [live.checking ? "Checking now" : `Next check ${relative(next, now)}`, every].filter(Boolean).join(" · ");
    }
    case "parked": {
      const until = parseInstant(live.until);
      return until ? `Until ${shortDate(until)}` : null;
    }
    case "held":
      return live.by ? `Waits on ${taskKey(live.by)}, which needs you` : null;
    case "running":
    case "queued": {
      if (live.state === "running" && live.runner) {
        const runner = live.runner;
        if (runner === "browser") return "Emberhut's browser is on it";
        if (runner.startsWith("watch:")) return `Emberhut's watch #${runner.slice(6)} is on it`;
        return `Waiting on ${runner.slice(5)}`;
      }
      const run = live.run;
      if (!run) return null;
      // The server's words for the machine, else its id: the app has no names of its own for anyone's machines.
      const place = run.place ?? run.machine;
      const kind = run.machine.startsWith("runner") ? "session" : "thread";
      const what = run.worker ? "Emberhut's worker" : run.title ? `${run.title} ${kind}` : `A ${kind}`;
      const started = ago(run.started);
      const when = live.state === "running" ? (started ? `started ${started}` : null) : "waiting behind another turn";
      return [`${what} on ${place}`, when, live.inferred ? "task guessed" : null].filter(Boolean).join(" · ");
    }
    case "stalled": {
      const ended = ago(live.last?.ended);
      return [ended ? `last turn ended ${ended}` : "no turn yet", live.last?.line].filter(Boolean).join(": ");
    }
    default:
      return null;
  }
}

/** "7/21 done · 160/300 episodes", or "12/21 done · 144 episodes left" when not every subtask has a count. */
export function rollupSummary(rollup: TaskRollup): string {
  const unit = rollup.unit ? ` ${rollup.unit}` : "";
  const counts =
    rollup.of != null ? ` · ${rollup.progress ?? 0}/${rollup.of}${unit}` : rollup.left != null ? ` · ${rollup.left}${unit} left` : "";
  return `${rollup.done}/${rollup.total} done${counts}`;
}

/** A counted task: "3/10 episodes". */
export function countText(task: Issue): string | null {
  if (task.total == null) return null;
  const done = task.status === "done" ? task.total : Math.min(task.progress ?? 0, task.total);
  return `${done}/${task.total}${task.unit ? ` ${task.unit}` : ""}`;
}

/** How far along: a parent by its counts (or subtasks done), a counted task by its count. */
export function fraction(task: Issue): number | null {
  const rollup = task.sub;
  if (rollup) return rollup.of != null ? (rollup.progress ?? 0) / Math.max(rollup.of, 1) : rollup.done / Math.max(rollup.total, 1);
  if (task.total == null) return null;
  return task.status === "done" ? 1 : Math.min(task.progress ?? 0, task.total) / Math.max(task.total, 1);
}

const sizes: Record<string, string> = { S: "1 h", M: "3 h", L: "1 day", XL: "days" };

/** The gray line under a task in a list. */
export function taskLine(task: Issue, project?: string | null): string {
  return [
    task.status === "canceled" ? "Archived" : null,
    project,
    isMine(task) || task.parent != null ? null : "Emberhut",
    task.status === "waiting" ? (task.action ?? task.decision?.question) : null,
    countText(task),
    task.sub ? rollupSummary(task.sub) : null,
    task.sub && task.sub.waiting.length > 0 ? `${task.sub.waiting.length} needs you` : null,
    task.size ? `${task.size} · ${sizes[task.size] ?? ""}` : null,
  ]
    .filter(Boolean)
    .join(" · ");
}

// The list of tasks by project (ProjectsList).

export interface ProjectGroup {
  project: Project;
  /** Its active areas, one row each, instead of every task in them. */
  areas: AreaTree[];
  tasks: Issue[];
}

export interface TaskOverview {
  /** Today's tasks, ranked by the server; what needs you has a tab of its own. */
  today: Issue[];
  groups: ProjectGroup[];
  /** Active projects with nothing open. */
  clear: Project[];
  /** Open tasks in no project. */
  loose: Issue[];
  paused: Project[];
  archived: Project[];
}

/** Your work by project: open top-level tasks (a parent counts once; its subtasks open under it), waiting ones left out. */
export function taskOverview(state: EmberhutState | undefined, gone: ReadonlySet<number>): TaskOverview {
  const projects = state?.projects ?? [];
  const open = (state?.issues ?? []).filter((t) => t.parent == null && isOpen(t) && !gone.has(t.id));
  const shown = open.filter((t) => t.status !== "waiting" && t.area == null);
  const today = (state?.today ?? []).flatMap((id) => open.filter((t) => t.id === id && t.status !== "waiting"));
  const areas = (state?.areas ?? []).filter((a) => a.status === "active");
  const active = projects.filter((p) => p.status === "active");
  const groups = active
    .map((project) => ({
      project,
      areas: areas.filter((a) => a.pid === project.id),
      tasks: shown.filter((t) => t.pid === project.id),
    }))
    .filter((g) => g.areas.length > 0 || g.tasks.length > 0);
  return {
    today,
    groups,
    clear: active.filter((p) => !groups.some((g) => g.project.id === p.id)),
    loose: shown.filter((t) => t.pid == null),
    paused: projects.filter((p) => p.status === "paused"),
    archived: projects.filter((p) => p.status === "archived"),
  };
}

/**
 * Projects hidden (state.ui): your tasks as one list, what needs you first, then Today, then the rest in the order of
 * their projects; tasks of paused and archived projects wait.
 */
export function flatTasks(state: EmberhutState | undefined, gone: ReadonlySet<number>): { needs: Issue[]; today: Issue[]; rest: Issue[] } {
  const order = (state?.projects ?? []).filter((p) => p.status === "active").map((p) => p.id);
  const open = (state?.issues ?? []).filter(
    (t) => t.parent == null && isOpen(t) && !gone.has(t.id) && (t.pid == null || order.includes(t.pid)),
  );
  const today = (state?.today ?? []).flatMap((id) => open.filter((t) => t.id === id && t.status !== "waiting"));
  const rank = (t: Issue) => (t.pid == null ? order.length : order.indexOf(t.pid));
  const rest = open
    .filter((t) => t.status !== "waiting" && !today.includes(t))
    .map((t, i) => ({ t, i }))
    .sort((a, b) => rank(a.t) - rank(b.t) || a.i - b.i)
    .map(({ t }) => t);
  return { needs: open.filter((t) => t.status === "waiting"), today, rest };
}

/** The open subtasks of a parent, in their order. */
export function subtasksOf(parent: number, state: EmberhutState | undefined, gone: ReadonlySet<number>): Issue[] {
  return (state?.issues ?? []).filter((t) => t.parent === parent && t.status !== "canceled" && !gone.has(t.id)).sort((a, b) => a.id - b.id);
}
