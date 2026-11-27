import { describe, expect, it } from "vitest";
import { ASSISTANT, taskKey, type AreaNode, type AreaTree, type Issue, type EmberhutState, type Project } from "@/api";
import { areaProgress, leaves, projectLanes, splitDone } from "./laneModel";
import { flatTasks, fraction, rollupSummary, statusName, taskOverview, todayLists } from "./model";
import { exchange, runStatus, standing, taskActions, timeline } from "./taskPage";

function issue(id: number, fields: Partial<Issue> = {}): Issue {
  return { id, key: taskKey(id), title: `Task ${id}`, owner: ASSISTANT, status: "todo", priority: 0, today: false, updated: "", ...fields };
}

function project(id: number, fields: Partial<Project> = {}): Project {
  return {
    id,
    name: `P${id}`,
    priority: 0,
    kind: "deliverable",
    hard: false,
    status: "active",
    open: 0,
    doneWeek: 0,
    hoursLeft: 0,
    ...fields,
  };
}

function node(id: number, state: AreaNode["state"], kids: AreaNode[] = [], fields: Partial<AreaNode> = {}): AreaNode {
  return { id, title: `Node ${id}`, owner: ASSISTANT, status: state === "done" ? "done" : "todo", state, kids, ...fields };
}

function area(id: number, pid: number, tree: AreaNode[], fields: Partial<AreaTree> = {}): AreaTree {
  const findings = { total: 0, landed: 0, partly: 0, notLanded: 0, superseded: 0, unchecked: 0, unlanded: [] };
  return {
    id,
    pid,
    name: `Area ${id}`,
    status: "active",
    state: "working",
    counts: { done: 1, working: 1, needsYou: 0, notStarted: 2 },
    questions: [],
    findings,
    tree,
    ...fields,
  };
}

describe("where a task stands", () => {
  it("reads its live state, not the label Emberhut last set", () => {
    expect(statusName(issue(1, { status: "working" }))).toBe("Nothing running");
    expect(statusName(issue(1, { status: "working", live: { state: "running", alarm: false } }))).toBe("Emberhut is working");
    expect(statusName(issue(1, { status: "waiting" }))).toBe("Needs you");
    expect(statusName(issue(1, { live: { state: "todo", alarm: false } }))).toBe("Not started");
    expect(statusName(issue(1))).toBe("To do");
  });

  it("rolls a parent's subtasks up into a summary and a fraction", () => {
    expect(rollupSummary({ done: 7, total: 21, waiting: [], progress: 160, of: 300, unit: "episodes" })).toBe(
      "7/21 done · 160/300 episodes",
    );
    expect(rollupSummary({ done: 12, total: 21, waiting: [], left: 144, unit: "episodes" })).toBe("12/21 done · 144 episodes left");
    expect(fraction(issue(1, { sub: { done: 1, total: 4, waiting: [] } }))).toBe(0.25);
    expect(fraction(issue(1, { total: 10, progress: 15 }))).toBe(1);
    expect(fraction(issue(1))).toBeNull();
  });
});

describe("the task list", () => {
  it("groups open top-level tasks by project, Today first, areas as rows, what needs you left to its tab", () => {
    const state = {
      projects: [project(1), project(2), project(3, { status: "paused" })],
      areas: [area(5, 1, [])],
      today: [11, 12],
      issues: [
        issue(11, { pid: 1 }),
        issue(12, { pid: 1, status: "waiting" }),
        issue(13, { pid: 1, area: 5 }),
        issue(14, { pid: 1, parent: 11 }),
        issue(15, { pid: 1, status: "done" }),
        issue(16),
        issue(17, { pid: 1 }),
      ],
    } as EmberhutState;
    const overview = taskOverview(state, new Set([17]));
    expect(overview.today.map((t) => t.id)).toEqual([11]);
    expect(overview.groups.map((g) => [g.project.id, g.areas.map((a) => a.id), g.tasks.map((t) => t.id)])).toEqual([[1, [5], [11]]]);
    expect(overview.clear.map((p) => p.id)).toEqual([2]);
    expect(overview.loose.map((t) => t.id)).toEqual([16]);
    expect(overview.paused.map((p) => p.id)).toEqual([3]);
  });
});

describe("the task list with projects hidden", () => {
  it("puts what needs you first, then Today, then the rest by project order; paused projects' tasks wait", () => {
    const state = {
      projects: [project(2), project(1), project(3, { status: "paused" })],
      today: [14],
      issues: [
        issue(10, { pid: 1 }),
        issue(11, { pid: 2 }),
        issue(12, { status: "waiting", pid: 1 }),
        issue(13),
        issue(14, { pid: 1 }),
        issue(15, { pid: 3 }),
      ],
    } as EmberhutState;
    const flat = flatTasks(state, new Set());
    expect([flat.needs, flat.today, flat.rest].map((list) => list.map((t) => t.id))).toEqual([[12], [14], [11, 10, 13]]);
  });
});

describe("a project's work by area", () => {
  it("makes a lane per active area from its tree, opens an area's own task into its steps, then the tasks in no area", () => {
    const tree = [node(20, "working", [node(21, "done"), node(22, "needs_you")], { title: "Billing (area)" }), node(23, "done")];
    const state = {
      areas: [area(5, 1, tree), area(6, 1, [node(30, "done")], { status: "paused" })],
      issues: [
        issue(40, { pid: 1 }),
        issue(41, { pid: 1, status: "done" }),
        issue(42, { pid: 1, parent: 40, status: "done" }),
        issue(43, { pid: 2 }),
      ],
    } as EmberhutState;
    const lanes = projectLanes(state, 1);
    expect(lanes.map((l) => [l.key, l.title, l.items.map((i) => i.id)])).toEqual([
      ["a5", "Area 5", [21, 22, 23]],
      ["none", "Not in an area", [40, 41]],
    ]);
    const { open, done } = splitDone(lanes[0]!.items);
    expect([open.map((i) => i.id), done.map((i) => i.id)]).toEqual([[22], [21, 23]]);
    expect(lanes[1]!.items[0]).toMatchObject({ state: "not_started", kids: [{ id: 42, state: "done" }] });
    expect(projectLanes(state, 1, { area: 6 }).map((l) => l.key)).toEqual(["a6"]);
  });

  it("counts a parent's leaves at any depth", () => {
    const state = {
      areas: [area(5, 1, [node(1, "working", [node(2, "done"), node(3, "working", [node(4, "done"), node(5, "not_started")])])])],
    } as EmberhutState;
    expect(leaves(projectLanes(state, 1)[0]!.items[0]!)).toEqual({ done: 2, total: 3 });
    expect(areaProgress({ done: 8, working: 2, needsYou: 1, notStarted: 3 })).toBe(
      "8 of 14 done · 2 working · 1 needs you · 3 not started",
    );
  });
});

describe("a task's page", () => {
  it("rebuilds runs from the history an older server sends, newest first", () => {
    const task = issue(1, {
      history: [
        { at: "2026-11-23T10:00:00Z", who: ASSISTANT, text: "Looked at it · STATUS: working: halfway" },
        { at: "2026-11-23T11:00:00Z", who: "owner", text: "Thanks" },
        { at: "2026-11-24T10:00:00Z", who: ASSISTANT, text: "Shipped · STATUS: done" },
      ],
    });
    expect(timeline(task).map((r) => [r.answer, r.status])).toEqual([
      ["Shipped", "STATUS: done"],
      ["Looked at it", "STATUS: working: halfway"],
    ]);
  });

  it("reads a STATUS line and what's left", () => {
    expect(runStatus("All good. STATUS: needs Emberhut: pick the cluster")).toEqual({
      kind: "needs",
      text: "Needs you: pick the cluster",
      rest: "pick the cluster",
    });
    expect(runStatus("STATUS: done")).toEqual({ kind: "done", text: "Done", rest: null });
    expect(runStatus("no status")).toBeNull();
    const task = issue(1, { runs: [] });
    const run = {
      id: 1,
      kind: "thread",
      machine: "",
      thread: "",
      title: null,
      asked: null,
      requested: null,
      started: null,
      ended: null,
      outcome: null,
    };
    expect(standing(task, [{ ...run, answer: "Half done", status: "STATUS: working: the tests" }])).toEqual({
      line: "Half done",
      left: "the tests",
    });
    expect(standing(issue(1, { live: { state: "stalled", alarm: false, last: { line: "It stopped" } } }), [])).toEqual({
      line: "It stopped",
      left: null,
    });
  });

  it("shows your last message and what came back, without state steps", () => {
    const history = [
      { at: "1", who: "owner", text: "First" },
      { at: "2", who: ASSISTANT, text: "Answer" },
      { at: "3", who: "owner", text: "Second" },
      { at: "4", who: ASSISTANT, text: "Started" },
      { at: "5", who: ASSISTANT, text: "On it" },
      { at: "6", who: "owner", text: "Chose: Approve" },
    ];
    expect(exchange(history).map((m) => m.text)).toEqual(["Second", "On it", "Chose: Approve"]);
    expect(exchange([{ at: "1", who: ASSISTANT, text: "Created by Emberhut" }])).toEqual([]);
  });

  it("plans the bar's buttons: the first answer, I did it, Mark done, Unarchive", () => {
    const ask = issue(1, { status: "waiting", decision: { question: "Ship?", options: ["Ship", "Wait", "No"] } });
    expect(taskActions(ask, [ask], false)).toMatchObject({
      target: 1,
      primary: { kind: "answer", choice: "Ship" },
      others: ["Wait", "No"],
      notNow: true,
      archive: true,
    });
    expect(taskActions(ask, [ask], true)).toMatchObject({ primary: null, others: [] });
    const todo = issue(2, { status: "waiting", action: "Call the bank" });
    expect(taskActions(todo, [todo], false).primary).toEqual({ kind: "didIt" });
    const named = issue(3, { status: "waiting", action: `One yes on ${taskKey(1)}` });
    expect(taskActions(named, [named, ask], false)).toMatchObject({ target: 1, primary: { kind: "answer", choice: "Ship" } });
    expect(taskActions(issue(4, { owner: "owner" }), [], false).primary).toEqual({ kind: "markDone" });
    expect(taskActions(issue(5, { status: "canceled" }), [], false)).toMatchObject({
      primary: { kind: "unarchive" },
      notNow: false,
      archive: false,
    });
  });
});

describe("Today", () => {
  it("lists what to decide today, then the ranked tasks still open, and More counts the same", () => {
    const now = new Date(2026, 10, 24, 10);
    const state = {
      issues: [issue(1, { status: "waiting" }), issue(2), issue(3, { status: "done" }), issue(4, { status: "waiting" }), issue(5)],
      working: [
        { id: "t1", kind: "task", task: 1, title: "Task 1", who: "Emberhut", state: "waiting", action: "Say yes", decideBy: "2026-11-24" },
        { id: "t4", kind: "task", task: 4, title: "Task 4", who: "Emberhut", state: "waiting", action: "Say yes", decideBy: "2026-12-20" },
      ],
      today: [2, 3, 4, 5],
    } as unknown as EmberhutState;
    const lists = todayLists(state, new Set([5]), now);
    expect(lists.decide.map((t) => t.id)).toEqual([1]);
    expect(lists.ranked.map((t) => t.id)).toEqual([2]);
  });
});
