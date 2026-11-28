import { describe, expect, it } from "vitest";
import { ASSISTANT, taskKey, type EmberhutState, type Issue, type Plan, type PlanStep, type RunInfo, type RunnerSession } from "@/api";
import { planRows } from "./plan";
import {
  activityFeed,
  countLine,
  currentPath,
  foldChecklist,
  nextSteps,
  planLists,
  planSegments,
  quietMinutes,
  quietText,
  soFar,
  stepResults,
  summarySegments,
} from "./planView";

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
    title: id,
    project: "",
    status: "running",
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
    direct: false,
    model: null,
    effort: null,
    question: null,
    tasks: [],
    room: 1,
    ...fields,
  };
}

const running = { state: "running", alarm: false };

describe("the Tasks list's groups", () => {
  it("counts what needs you, and puts a blocked task that runs with the running ones", () => {
    const state = {
      room: 1,
      issues: [
        issue(1, { status: "waiting", live: running }),
        issue(2, { status: "working", live: running, plan: { done: 1, total: 4, blocker: "No key" } }),
        issue(3, { status: "working", live: running }),
        issue(4, { status: "working", live: { state: "stalled", alarm: false }, plan: { done: 1, total: 4, blocker: "No key" } }),
        issue(5, { plan: { done: 0, total: 0 } }),
      ],
    } as EmberhutState;
    const lists = planLists(state, new Set());
    expect(lists.needs).toBe(1);
    expect(lists.running.map((t) => t.id)).toEqual([2, 3]);
    expect(lists.blocked.map((t) => t.id)).toEqual([4]);
    expect(lists.open.map((t) => t.id)).toEqual([5]);
  });
});

describe("the segmented bar", () => {
  it("draws a row's steps from its summary: done, then where it is, then the rest", () => {
    expect(summarySegments({ done: 2, total: 5, current: { id: 3, title: "x", state: "doing" } })).toEqual([
      "done",
      "done",
      "doing",
      "todo",
      "todo",
    ]);
    expect(summarySegments({ done: 1, total: 3, blocker: "No key" })).toEqual(["done", "blocked", "todo"]);
    expect(summarySegments({ done: 0, total: 0 })).toBeNull();
    expect(summarySegments({ done: 3, total: 13 })).toBeNull();
  });

  it("draws the page's from its steps, sub-steps in place of theirs, skipped ones left out", () => {
    const plan: Plan = {
      steps: [
        step(1, "done"),
        step(2, "doing"),
        step(3, "done", { parent: 2 }),
        step(4, "doing", { parent: 2 }),
        step(5, "skipped"),
        step(6, "blocked"),
      ],
      summary: { done: 2, total: 4 },
    };
    expect(planSegments(plan)).toEqual(["done", "done", "doing", "blocked"]);
    expect(countLine(plan)).toBe("2 of 4 done · 1 blocked · 1 skipped");
  });
});

describe("where the plan is", () => {
  const plan: Plan = {
    steps: [
      step(1, "done"),
      step(2, "doing", { title: "Build the Windows installer" }),
      step(3, "done", { parent: 2 }),
      step(4, "doing", { parent: 2, title: "Sign the installer" }),
      step(5, "todo", { parent: 2 }),
      step(6, "skipped"),
      step(7, "todo"),
      step(8, "todo"),
    ],
    summary: { done: 2, total: 5, current: { id: 4, title: "Sign the installer", state: "doing" } },
  };

  it("names the current step with its step", () => {
    expect(currentPath(plan)).toBe("Build the Windows installer › Sign the installer");
  });

  it("names the next steps still to do", () => {
    expect(nextSteps(plan).map((s) => s.id)).toEqual([5, 7]);
  });

  it("gives only the current step a duration", () => {
    expect(soFar(step(4, "doing", { started: "2026-11-28T11:52:00Z" }), now)).toBe("8m so far");
    expect(soFar(step(4, "done", { started: "2026-11-28T11:52:00Z" }), now)).toBeNull();
    expect(soFar(step(4, "doing"), now)).toBeNull();
  });

  it("lists what the steps produced", () => {
    expect(stepResults({ ...plan, steps: [step(1, "done", { link: " https://example.com/pull/7 " }), step(2, "todo")] })).toEqual([
      { step: expect.objectContaining({ id: 1 }), link: "https://example.com/pull/7" },
    ]);
  });
});

describe("a quiet running task", () => {
  const task = issue(1, {
    status: "working",
    live: {
      state: "running",
      alarm: false,
      since: "2026-11-28T11:00:00Z",
      run: { machine: "m", thread: "r-1", worker: false, started: "2026-11-28T11:00:00Z" },
    },
    plan: { done: 1, total: 3, updated: "2026-11-28T11:35:00Z" },
  });

  it("is quiet after 20 minutes with neither its plan nor its session moving", () => {
    expect(quietMinutes(task, [session("r-1", { updatedAt: "2026-11-28T11:30:00Z" })], now)).toBe(25);
    expect(quietMinutes(task, [session("r-1", { updatedAt: "2026-11-28T11:50:00Z" })], now)).toBeNull();
    expect(quietMinutes(task, [session("r-2", { tasks: [1], updatedAt: "2026-11-28T11:45:00Z" })], now)).toBeNull();
    expect(quietMinutes({ ...task, live: { state: "queued", alarm: false } }, [], now)).toBeNull();
  });

  it("says how long", () => {
    expect(quietText(25)).toBe("25m");
    expect(quietText(65)).toBe("1h 05m");
  });
});

describe("the checklist folded around the current step", () => {
  const long = (states: string[], current: number): PlanRow[] =>
    planRows({
      steps: states.map((s, i) => step(i + 1, s)),
      summary: { done: 0, total: states.length, current: { id: current, title: "", state: "doing" } },
    });
  type PlanRow = ReturnType<typeof planRows>[number];
  const shape = (rows: PlanRow[]) => foldChecklist(rows).map((e) => (e.kind === "fold" ? e.label : e.row.step.id));

  it("leaves a short plan as it is", () => {
    expect(shape(long(["done", "doing", "todo"], 2))).toEqual([1, 2, 3]);
  });

  it("shows the last done step, the current one and the next two, the rest folded", () => {
    const states = [...Array<string>(11).fill("done"), "doing", "todo", "todo", "todo", "todo", "todo", "todo"];
    expect(shape(long(states, 12))).toEqual(["10 done", 11, 12, 13, 14, "4 more"]);
  });

  it("never folds a blocked step away", () => {
    const states = ["done", "done", "done", "done", "doing", "todo", "todo", "todo", "blocked", "todo"];
    expect(shape(long(states, 5))).toEqual(["3 done", 4, 5, 6, 7, "1 more", 9, "1 more"]);
  });

  it("opens whole", () => {
    expect(foldChecklist(long(Array<string>(10).fill("todo"), 1), true)).toHaveLength(10);
  });

  it("keeps a shown sub-step's step above it", () => {
    const rows = planRows({
      steps: [
        ...Array.from({ length: 8 }, (_, i) => step(i + 1, "done")),
        step(9, "doing"),
        step(10, "done", { parent: 9 }),
        step(11, "doing", { parent: 9 }),
        step(12, "todo"),
      ],
      summary: { done: 9, total: 11, current: { id: 11, title: "", state: "doing" } },
    });
    expect(shape(rows)).toEqual(["8 done", 9, 10, 11, 12]);
  });
});

describe("the activity feed", () => {
  const run = (id: number, fields: Partial<RunInfo>): RunInfo => ({
    id,
    kind: "thread",
    machine: "m",
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
  });
  const runs = [run(1, { started: "2026-11-28T10:00:00Z", ended: "2026-11-28T10:30:00Z", answer: "Built it" })];
  const history = [
    { at: "2026-11-28T10:30:00Z", who: "worker", text: "Built it · STATUS: done" },
    { at: "2026-11-28T10:20:00Z", who: "worker", text: "Ticked: Build the Mac app" },
    { at: "2026-11-28T09:00:00Z", who: "owner", text: "Created" },
  ];
  const artifacts = [{ id: 3, kind: "link", name: "PR", url: "https://example.com/pull/7", at: "2026-11-28T10:25:00Z" }];

  it("puts turns, history and files in one feed, newest first, a turn's own history line left out", () => {
    expect(activityFeed(runs, [], history, artifacts).map((f) => f.kind)).toEqual(["file", "moment", "turn", "moment"]);
  });

  it("filters by turns, plan events or files", () => {
    expect(activityFeed(runs, [], history, artifacts, "turns").map((f) => f.kind)).toEqual(["turn"]);
    expect(activityFeed(runs, [], history, artifacts, "plan").map((f) => (f.kind === "moment" ? f.moment.text : ""))).toEqual([
      "Ticked: Build the Mac app",
    ]);
    expect(activityFeed(runs, [], history, artifacts, "files").map((f) => f.kind)).toEqual(["file"]);
  });

  it("keeps the turn lines from an older server's history when it sends no runs", () => {
    expect(activityFeed([], [], history, []).filter((f) => f.kind === "moment")).toHaveLength(3);
  });
});
