import type { AreaState, AreaTree, Issue, EmberhutState, Project } from "@/api";
import { isOpen } from "./model";

// A project's work by area: the area trees and lanes.

/** A rolled-up state's name, as the area trees say it. */
export const areaStateNames: Record<AreaState, string> = {
  done: "Done",
  working: "Working",
  needs_you: "Needs you",
  not_started: "Not started",
};

/** A task status in the area trees' words. */
function areaStateOf(status: string): AreaState {
  switch (status) {
    case "waiting":
    case "needs_you":
      return "needs_you";
    case "todo":
    case "not_started":
      return "not_started";
    case "done":
      return "done";
    default:
      return "working";
  }
}

/** "8 of 14 done · 2 working · 1 needs you". */
export function areaProgress(counts: AreaTree["counts"]): string {
  const total = counts.done + counts.working + counts.needsYou + counts.notStarted;
  return [
    total === 0 ? "No tasks yet" : `${counts.done} of ${total} done`,
    counts.working > 0 ? `${counts.working} working` : null,
    counts.needsYou > 0 ? `${counts.needsYou} needs you` : null,
    counts.notStarted > 0 ? `${counts.notStarted} not started` : null,
  ]
    .filter(Boolean)
    .join(" · ");
}

export const areaTotal = (counts: AreaTree["counts"]): number => counts.done + counts.working + counts.needsYou + counts.notStarted;

/** A task in a lane, with its steps under it. */
export interface WorkItem {
  id: number;
  title: string;
  action: string | null;
  day: string | null;
  hard: boolean;
  state: AreaState | "scheduled";
  owner: string;
  kids: WorkItem[];
}

export interface Lane {
  key: string;
  title: string;
  area: AreaTree | null;
  items: WorkItem[];
}

/**
 * Its active areas, each a lane of its tree (done work too), then the tasks in no area. An area's own task ("… (area)",
 * the work that stands for the area) isn't a row of its own: its steps sit in the lane. With `area`, only that one.
 */
export function projectLanes(
  state: EmberhutState | undefined,
  project: number,
  options: { area?: number; closed?: readonly Issue[]; gone?: ReadonlySet<number> } = {},
): Lane[] {
  const issues = state?.issues ?? [];
  const gone = options.gone ?? new Set<number>();
  const scheduled = new Set(issues.filter((t) => t.live?.state === "scheduled").map((t) => t.id));
  const fromNode = (n: AreaTree["tree"][number]): WorkItem => ({
    id: n.id,
    title: n.title,
    action: n.action ?? null,
    day: n.due ?? null,
    hard: false,
    state: scheduled.has(n.id) && n.state === "working" ? "scheduled" : n.state,
    owner: n.owner,
    kids: n.kids.map(fromNode),
  });
  const fromTask = (t: Issue): WorkItem => ({
    id: t.id,
    title: t.title,
    action: t.action ?? null,
    day: t.decideBy ?? t.due ?? null,
    hard: t.hardDeadline === true,
    state: t.live?.state === "scheduled" ? "scheduled" : areaStateOf(t.status),
    owner: t.owner,
    kids: issues.filter((k) => k.parent === t.id && k.status !== "canceled").map(fromTask),
  });
  const unwrap = (items: WorkItem[]) => items.flatMap((i) => (i.title.endsWith("(area)") && i.kids.length > 0 ? i.kids : [i]));
  const areas = (state?.areas ?? []).filter(
    (a) => a.pid === project && (options.area === undefined ? a.status === "active" : a.id === options.area),
  );
  const lanes: Lane[] = areas.map((a) => ({ key: `a${a.id}`, title: a.name, area: a, items: unwrap(a.tree.map(fromNode)) }));
  if (options.area === undefined) {
    const loose = (t: Issue) => t.pid === project && t.area == null && t.parent == null && t.kind !== "finding";
    const open = issues.filter((t) => loose(t) && isOpen(t) && !gone.has(t.id));
    const done = (options.closed ?? issues).filter((t) => loose(t) && t.status === "done");
    if (open.length > 0 || done.length > 0) {
      lanes.push({
        key: "none",
        title: areas.length === 0 ? "Tasks" : "Not in an area",
        area: null,
        items: [...open, ...done].map(fromTask),
      });
    }
  }
  return lanes;
}

/** Open work first; done work folds at the bottom. */
export function splitDone(items: readonly WorkItem[]): { open: WorkItem[]; done: WorkItem[] } {
  return { open: items.filter((i) => i.state !== "done"), done: items.filter((i) => i.state === "done") };
}

/** A parent's leaves done of all, at any depth. */
export function leaves(item: WorkItem): { done: number; total: number } {
  if (item.kids.length === 0) return { done: item.state === "done" ? 1 : 0, total: 1 };
  return item.kids.map(leaves).reduce((a, b) => ({ done: a.done + b.done, total: a.total + b.total }), { done: 0, total: 0 });
}

/** A project's header: its done work this week and in its areas, of all of it. */
export function projectProgress(project: Project, areas: readonly AreaTree[]): { done: number; total: number } {
  const own = areas.filter((a) => a.pid === project.id && a.status === "active");
  const done = own.reduce((n, a) => n + a.counts.done, 0) + project.doneWeek;
  const rest = own.reduce((n, a) => n + areaTotal(a.counts) - a.counts.done, 0);
  return { done, total: done + project.open + rest };
}
