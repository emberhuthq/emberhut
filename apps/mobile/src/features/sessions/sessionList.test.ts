import { describe, expect, it } from "vitest";
import { taskKey, type AgentNow, type EmberhutState, type RunnerSession } from "@/api";
import {
  agentsByTask,
  effortName,
  elapsed,
  modelName,
  fallbackModels,
  groupSessions,
  sessionModels,
  plainSessionText,
  sessionMeta,
  sessionSummary,
  shortPath,
  shortTitle,
} from "./sessionList";

const session = (id: string, status: string, updatedAt: string, extra: Partial<RunnerSession> = {}): RunnerSession =>
  ({
    id,
    status,
    updatedAt,
    title: `Emberhut · ${id}`,
    tasks: [],
    direct: false,
    machine: "machine-a",
    place: "Studio",
    project: "",
    room: 1,
    ...extra,
  }) as RunnerSession;
const agent = (id: string, task: number, lead: boolean, viewer: string | null = null): AgentNow =>
  ({
    id,
    task,
    lead,
    viewer,
    key: taskKey(task),
    title: null,
    name: null,
    line: null,
    started: null,
    tokens: null,
    parent: null,
  }) as AgentNow;

describe("groupSessions", () => {
  it("lists each session once, running apart, waiting ones first among the rest", () => {
    const state = {
      mySessions: [session("a", "waiting", "2026-11-24T08:00:00Z"), session("b", "running", "2026-11-24T09:00:00Z")],
      sessions: [
        session("b", "running", "2026-11-24T09:00:00Z"),
        session("c", "completed", "2026-11-24T10:00:00Z"),
        session("d", "queued", "2026-11-24T10:00:00Z"),
      ],
      agentsNow: [agent("lead-5", 5, true, "d")],
    } as unknown as EmberhutState;
    const g = groupSessions(state);
    expect(g.running.map((s) => s.id)).toEqual(["b"]);
    expect(g.recent.map((s) => s.id)).toEqual(["a", "c"]);
    expect(groupSessions(undefined)).toMatchObject({ agents: [], running: [], recent: [], external: [], hidden: 0 });
  });

  it("narrows every list by the filter and counts what it leaves out", () => {
    const state = {
      mySessions: [session("mine", "running", "2026-11-24T09:00:00Z", { direct: true, machine: "machine-b", accountName: "AWS Bedrock" })],
      sessions: [
        session("emberhut", "running", "2026-11-24T09:00:00Z", { accountName: "Personal Claude" }),
        session("old", "completed", "2026-11-24T08:00:00Z", { accountName: "AWS Bedrock" }),
        session("lead", "running", "2026-11-24T09:00:00Z"),
      ],
      agentsNow: [
        { ...agent("lead-5", 5, true, "lead"), kind: "lead", machine: "machine-a", accountName: "AWS Bedrock" },
        { ...agent("agent-1", 5, false, "lead"), kind: "helper", machine: "machine-a", accountName: "AWS Bedrock" },
      ],
    } as unknown as EmberhutState;
    const g = groupSessions(state, { workers: ["helper", "direct"] });
    expect([g.agents.map((a) => a.id), g.running.map((s) => s.id), g.recent.map((s) => s.id)]).toEqual([["agent-1"], ["mine"], []]);
    expect(g.hidden).toBe(3); // an ultra run's lead and Emberhut's two sessions; that agent's own session is its agent row, never counted twice
    const bedrock = groupSessions(state, { accounts: ["AWS Bedrock"], machines: ["machine-a"] });
    expect([bedrock.agents.length, bedrock.running.length, bedrock.recent.map((s) => s.id)]).toEqual([2, 0, ["old"]]);
    expect(bedrock.picks.machines.map((p) => [p.key, p.count])).toEqual([
      ["machine-a", 3],
      ["machine-b", 1],
    ]);
  });

  it("puts each task's lead first", () => {
    expect(agentsByTask([agent("x", 1, false), agent("y", 2, false), agent("lead", 1, true)]).map((g) => g.map((a) => a.id))).toEqual([
      ["lead", "x"],
      ["y"],
    ]);
  });
});

describe("rows", () => {
  it("reads short", () => {
    expect(shortTitle("Emberhut · Fix the build")).toBe("Fix the build");
    expect(
      sessionMeta(session("a", "completed", "", { tasks: [12, 13], project: "Garden", place: "Studio", accountName: "Personal" })),
    ).toBe(`${taskKey(12)}, ${taskKey(13)} · Garden · Studio · Personal`);
    expect(shortPath("/home/ada/Documents/emberhut")).toBe("~/Documents/emberhut");
    expect(shortPath("/Users/someone/code")).toBe("~/code");
    expect(shortPath("/srv/emberhut")).toBe("/srv/emberhut");
  });

  it("shows what it asks while it waits, else what it said, plain", () => {
    expect(sessionSummary(session("a", "waiting", "", { question: "Which **one**?", last: "x" }))).toBe("Which one?");
    expect(sessionSummary(session("a", "completed", "", { last: null, statusLine: "STATUS: done" }))).toBe("STATUS: done");
    expect(plainSessionText('💬 More natural: "a b" — 7/10 The `fix` is in.')).toBe("The fix is in.");
    const long = `${"word ".repeat(300)}and more`;
    expect(sessionSummary(session("a", "completed", "", { last: long }))?.endsWith("and …")).toBe(true);
  });
});

describe("elapsed", () => {
  it("reads like a stopwatch", () => {
    expect(elapsed(0, 42_000)).toBe("0:42");
    expect(elapsed(0, 725_000)).toBe("12:05");
    expect(elapsed(0, 3_733_000)).toBe("1:02:13");
    expect(elapsed(10_000, 0)).toBe("0:00");
  });
});

describe("models", () => {
  it("names a model and an effort", () => {
    expect(modelName("claude-sonnet-5-5")).toBe("Sonnet 5.5");
    expect(modelName("claude-x")).toBe("claude-x");
    expect(modelName(null)).toBeNull();
    expect(modelName("claude-next-6", [{ id: "claude-next-6", name: "Next 6" }])).toBe("Next 6");
  });

  it("offers the server's models, and the fallback only when it names none", () => {
    const server = [{ id: "claude-next-6", name: "Next 6", in: 3, out: 15, note: "" }];
    expect(sessionModels(server)).toEqual([{ id: "claude-next-6", name: "Next 6" }]);
    expect(sessionModels([{ id: "claude-x", name: "" }])).toEqual([{ id: "claude-x", name: "claude-x" }]);
    expect(sessionModels([])).toBe(fallbackModels);
    expect(sessionModels(undefined)).toBe(fallbackModels);
    expect(effortName("xhigh")).toBe("Extra high");
    expect(effortName("max")).toBe("Max");
  });
});
