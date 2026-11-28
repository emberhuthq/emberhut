import { describe, expect, it } from "vitest";
import {
  ASSISTANT,
  taskKey,
  type EmberhutState,
  type Issue,
  type Plan,
  type PlanStep,
  type RunInfo,
  type RunnerSession,
  type TaskPage,
} from "@/api";
import { planLine, taskLine } from "./model";
import {
  activity,
  currentLine,
  firstLine,
  isWebLink,
  pagePlan,
  pageStamp,
  pageTask,
  planFraction,
  planRows,
  plansOn,
  runningTasks,
  stepLabel,
  stepLink,
  stepState,
  summaryLine,
  taskSessions,
  tasklessSessions,
  unplannedLine,
} from "./plan";

const now = new Date("2026-11-28T12:00:00Z");

function issue(id: number, fields: Partial<Issue> = {}): Issue {
  return { id, key: taskKey(id), title: `Task ${id}`, owner: ASSISTANT, status: "todo", priority: 0, today: false, updated: "", ...fields };
}

function step(id: number, state: string, fields: Partial<PlanStep> = {}): PlanStep {
  return { id, title: `Step ${id}`, state, updated: "2026-11-28T11:00:00Z", ...fields };
}

function session(id: string, fields: Partial<RunnerSession> = {}): RunnerSession {
  return {
    id,
    machine: "m1",
    place: "the PC",
    title: `Session ${id}`,
    project: "",
    status: "completed",
    updatedAt: "2026-11-28T11:00:00Z",
    doing: null,
    account: null,
    accountName: null,
    cwd: null,
    branch: null,
    ask: null,
    last: null,
    statusLine: null,
    since: null,
    direct: true,
    model: null,
    effort: null,
    question: null,
    tasks: [],
    room: 1,
    ...fields,
  };
}

function run(id: number, fields: Partial<RunInfo> = {}): RunInfo {
  return {
    id,
    kind: "thread",
    machine: "m1",
    thread: "r-1",
    title: null,
    asked: null,
    answer: null,
    status: null,
    requested: null,
    started: null,
    ended: null,
    outcome: null,
    ...fields,
  };
}

describe("whether plans are on", () => {
  it("is off unless the server says it keeps plans", () => {
    expect(plansOn(undefined)).toBe(false);
    expect(plansOn({ room: 1 } as EmberhutState)).toBe(false);
    expect(plansOn({ room: 1, ui: { hideProjects: true } } as EmberhutState)).toBe(false);
    expect(plansOn({ room: 1, ui: { taskPlans: true } } as EmberhutState)).toBe(true);
  });
});

describe("a plan at a glance", () => {
  it("says no plan yet for an empty one", () => {
    expect(planLine({ done: 0, total: 0 })).toBe("No plan yet");
    expect(planFraction({ done: 0, total: 0 })).toBeNull();
    expect(currentLine({ done: 0, total: 0 })).toEqual({ kind: "none", text: "No plan yet" });
    expect(summaryLine({ done: 0, total: 0 })).toBe("No plan yet");
  });

  it("counts the ticks and how far along", () => {
    expect(planLine({ done: 4, total: 9 })).toBe("4/9 steps");
    expect(planFraction({ done: 3, total: 4 })).toBe(0.75);
    expect(planFraction({ done: 5, total: 4 })).toBe(1);
    expect(planLine(undefined)).toBeNull();
  });

  it("shows an open task's ticks in its row only with plans on", () => {
    const task = issue(1, { plan: { done: 4, total: 9 } });
    expect(taskLine(task, null, true)).toBe("Emberhut · 4/9 steps");
    expect(taskLine(task, null, false)).toBe("Emberhut");
    expect(taskLine(issue(2, { plan: { done: 0, total: 0 } }), null, true)).toBe("Emberhut · No plan yet");
    expect(taskLine(issue(3, { status: "done", plan: { done: 9, total: 9 } }), null, true)).toBe("Emberhut");
  });

  it("names the step it is on, what blocks it first", () => {
    expect(currentLine({ done: 2, total: 5, current: { id: 3, title: "Build the Mac app", state: "doing" } })).toEqual({
      kind: "doing",
      text: "Build the Mac app",
    });
    expect(currentLine({ done: 2, total: 5, current: { id: 3, title: "Sign it", state: "todo" } }).text).toBe("Next: Sign it");
    expect(currentLine({ done: 2, total: 5, current: { id: 3, title: "Sign it", state: "doing" }, blocker: "No signing key" })).toEqual({
      kind: "blocked",
      text: "Blocked: No signing key",
    });
    expect(currentLine({ done: 2, total: 5, current: { id: 3, title: "Sign it", state: "blocked" } }).text).toBe("Blocked: Sign it");
    expect(currentLine({ done: 5, total: 5 }).kind).toBe("finished");
  });

  it("reads the summary line with the estimate and when it moved", () => {
    expect(summaryLine({ done: 4, total: 9, eta: "about 2 hours", updated: "2026-11-28T11:57:00Z" }, now)).toBe(
      "4 of 9 done · ETA about 2 hours · updated 3 min ago",
    );
    expect(summaryLine({ done: 9, total: 9, eta: "about 2 hours" }, now)).toBe("9 of 9 done");
  });

  it("says quietly when a turn ended with no plan", () => {
    expect(unplannedLine({ done: 0, total: 0, unplannedTurn: "2026-11-28T11:00:00Z" }, now)).toBe(
      "A turn of work ended 1 h ago without a plan.",
    );
    expect(unplannedLine({ done: 0, total: 0 }, now)).toBeNull();
    expect(unplannedLine({ done: 1, total: 2, unplannedTurn: "2026-11-28T11:00:00Z" }, now)).toBeNull();
  });
});

describe("the checklist", () => {
  const plan: Plan = {
    steps: [
      step(1, "done", { link: "https://example.com/pull/7" }),
      step(2, "doing"),
      step(3, "done", { parent: 2 }),
      step(4, "doing", { parent: 2 }),
      step(5, "blocked", { note: "No signing key" }),
      step(6, "skipped"),
      step(7, "someday"),
      step(8, "todo", { parent: 99 }),
      step(9, "todo", { parent: 3 }),
    ],
    summary: { done: 2, total: 6, current: { id: 4, title: "Step 4", state: "doing" } },
  };

  it("puts sub-steps under their step, one level deep", () => {
    expect(planRows(plan).map((r) => [r.step.id, r.depth])).toEqual([
      [1, 0],
      [2, 0],
      [3, 1],
      [4, 1],
      [5, 0],
      [6, 0],
      [7, 0],
      [8, 0],
      [9, 0],
    ]);
  });

  it("marks the current step, and reads an unknown state as not started", () => {
    const rows = planRows(plan);
    expect(rows.filter((r) => r.current).map((r) => r.step.id)).toEqual([4]);
    expect(rows.find((r) => r.step.id === 7)?.state).toBe("todo");
    expect(stepState("blocked")).toBe("blocked");
  });

  it("takes the first step being worked on when the summary names none", () => {
    const rows = planRows({ ...plan, summary: { done: 2, total: 6 } });
    expect(rows.filter((r) => r.current).map((r) => r.step.id)).toEqual([2]);
  });

  it("says each step for a screen reader", () => {
    const rows = planRows(plan);
    expect(stepLabel(rows[3]!)).toBe("Sub-step, Step 4, in progress, current step");
    expect(stepLabel(rows[4]!)).toBe("Step 5, blocked, No signing key");
    expect(stepLabel(rows[5]!)).toBe("Step 6, skipped");
  });

  it("reads a step's link short, and how it opens", () => {
    expect(stepLink("https://github.com/example/app/pull/7")).toEqual({ kind: "web", text: "github.com/…/pull/7" });
    expect(stepLink("https://www.example.com/")).toEqual({ kind: "web", text: "example.com" });
    expect(stepLink("https://example.com/docs/plan?x=1")).toEqual({ kind: "web", text: "example.com/docs/plan" });
    expect(stepLink("/api/artifacts/12")).toEqual({ kind: "server", text: "12" });
    expect(stepLink("//elsewhere.example.com/x")).toEqual({ kind: "text", text: "//elsewhere.example.com/x" });
    expect(stepLink("  ")).toBeNull();
  });

  it("opens only web links", () => {
    expect(isWebLink("https://example.com/pull/7")).toBe(true);
    expect(isWebLink("javascript:alert(1)")).toBe(false);
    expect(isWebLink(null)).toBe(false);
  });
});

describe("the task page's plan", () => {
  const page = (plan: unknown): TaskPage => ({ ...issue(1), plan }) as TaskPage;

  it("reads the whole plan, or a summary alone as a plan with no steps", () => {
    const whole: Plan = { steps: [step(1, "todo")], summary: { done: 0, total: 1 } };
    expect(pagePlan(page(whole))).toEqual(whole);
    expect(pagePlan(page({ done: 0, total: 0 }))).toEqual({ steps: [], summary: { done: 0, total: 0 } });
    expect(pagePlan(page(undefined))).toBeNull();
    expect(pagePlan(undefined)).toBeNull();
  });

  it("gives the page as a task with its summary", () => {
    expect(pageTask(page({ steps: [], summary: { done: 1, total: 3 } })).plan).toEqual({ done: 1, total: 3 });
    expect(pageTask(page(undefined)).plan).toBeNull();
  });

  it("changes its stamp when the plan or the run moves", () => {
    const task = issue(1, { updated: "a", plan: { done: 1, total: 3, updated: "x" }, live: { state: "running", alarm: false } });
    const stamp = pageStamp(task);
    expect(pageStamp({ ...task })).toBe(stamp);
    expect(pageStamp({ ...task, plan: { done: 2, total: 3, updated: "x" } })).not.toBe(stamp);
    expect(pageStamp({ ...task, plan: { done: 1, total: 3, updated: "y" } })).not.toBe(stamp);
    expect(pageStamp({ ...task, live: { state: "stalled", alarm: false } })).not.toBe(stamp);
    expect(pageStamp(undefined)).toBeUndefined();
  });
});

describe("the Tasks list with plans", () => {
  it("lists the open tasks a run is going on", () => {
    const state = {
      room: 1,
      issues: [
        issue(1, { status: "working", live: { state: "running", alarm: false } }),
        issue(2, { status: "working", live: { state: "queued", alarm: false } }),
        issue(3, { status: "working", live: { state: "stalled", alarm: false } }),
        issue(4, { status: "done", live: { state: "running", alarm: false } }),
        issue(5, { status: "working", live: { state: "running", alarm: false } }),
      ],
    } as EmberhutState;
    expect(runningTasks(state, new Set([5])).map((t) => t.id)).toEqual([1, 2]);
  });

  it("keeps sessions with no task: running, waiting, and finished ones for a day", () => {
    const state = {
      room: 1,
      mySessions: [
        session("r-old", { updatedAt: "2026-11-26T11:00:00Z" }),
        session("r-new", { updatedAt: "2026-11-28T10:00:00Z" }),
        session("r-task", { status: "running", tasks: [12] }),
      ],
      sessions: [
        session("r-new", { updatedAt: "2026-11-28T10:00:00Z" }),
        session("r-run", { status: "running", updatedAt: "2026-11-28T09:00:00Z" }),
        session("r-ask", { status: "waiting", updatedAt: "2026-11-20T09:00:00Z" }),
      ],
    } as EmberhutState;
    expect(tasklessSessions(state, now).map((s) => s.id)).toEqual(["r-run", "r-ask", "r-new"]);
  });
});

describe("a task's activity", () => {
  it("rolls each turn into a line: asked, what came of it, its session", () => {
    const items = activity(
      [
        run(2, { thread: "r-2", asked: "Ship the beta", started: "2026-11-28T11:00:00Z" }),
        run(1, {
          thread: "r-1",
          asked: "## Build it\nthen test",
          answer: "\n- Built the Mac app.\nMore detail here.",
          status: "STATUS: needs emberhut: pick the cluster",
          ended: "2026-11-28T10:00:00Z",
          outcome: "completed",
        }),
        run(0, { thread: "w-9", ended: "2026-11-28T09:00:00Z", outcome: "failed" }),
      ],
      [session("r-1")],
    );
    expect(items[0]).toMatchObject({ going: true, asked: "Ship the beta", said: null, session: null });
    expect(items[1]).toMatchObject({ going: false, asked: "Build it", said: "Built the Mac app.", failed: null });
    expect(items[1]!.status?.text).toBe("Needs you: pick the cluster");
    expect(items[1]!.session?.id).toBe("r-1");
    expect(items[2]!.failed).toBe("failed");
  });

  it("cuts a long first line", () => {
    expect(firstLine("x".repeat(200), 10)).toBe(`${"x".repeat(9)}…`);
    expect(firstLine("  \n ")).toBeNull();
  });

  it("lists the task's sessions running first, then the newest", () => {
    const sorted = taskSessions([
      session("a", { updatedAt: "2026-11-28T09:00:00Z" }),
      session("b", { updatedAt: "2026-11-28T10:00:00Z" }),
      session("c", { status: "running", updatedAt: "2026-11-28T08:00:00Z" }),
    ]);
    expect(sorted.map((s) => s.id)).toEqual(["c", "b", "a"]);
  });
});
