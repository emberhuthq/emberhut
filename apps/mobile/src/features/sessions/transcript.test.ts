import { describe, expect, it } from "vitest";
import type { SessionStep } from "@/api";
import { applyEvent, emptyView, loaded } from "./sessionLive";
import {
  brief,
  foldedLabel,
  insideItems,
  isQueued,
  liveTodos,
  liveWord,
  runningTool,
  stepItems,
  toolLine,
  turns,
  turnNumber,
} from "./transcript";

const s = (seq: number, kind: string, extra: Partial<SessionStep> = {}): SessionStep => ({ seq, turn: "r-a:1", at: null, kind, ...extra });

describe("turns", () => {
  it("groups steps by their turn", () => {
    const t = turns([s(1, "user", { turn: null }), s(2, "text"), s(3, "user", { turn: "r-a:2" }), s(4, "text", { turn: "r-a:2" })]);
    expect(t.map((x) => [x.id, x.steps.map((y) => y.seq)])).toEqual([
      ["start-1", [1]],
      ["r-a:1", [2]],
      ["r-a:2", [3, 4]],
    ]);
  });
});

describe("stepItems", () => {
  it("hangs results and a helper's steps off their call, and skips quiet tools", () => {
    const steps = [
      s(1, "user", { by: "owner", text: "go" }),
      s(2, "thinking", { ms: 6000 }),
      s(3, "tool", { name: "Bash", id: "c1", summary: "npm test" }),
      s(4, "tool", { name: "TodoWrite", id: "c2" }),
      s(5, "tool", { name: "Agent", id: "c3", summary: "Explore" }),
      s(6, "text", { parent: "c3", text: "found it" }),
      s(7, "result", { id: "c1", text: "ok", lines: 3 }),
      s(8, "result", { id: "c3", text: "done" }),
      s(9, "todos", { todos: [{ content: "a", status: "pending" }] }),
      s(10, "text", { text: "All green." }),
      s(11, "end", { n: 1, outcome: "completed" }),
    ];
    const items = stepItems(steps);
    expect(items.map((i) => i.kind)).toEqual(["said", "tools", "todos", "said", "note"]);
    const tools = items[1];
    if (tools?.kind !== "tools") throw new Error("no tools");
    expect(tools.calls.map((c) => [c.call.seq, c.result?.seq ?? null, c.inside.length])).toEqual([
      [2, null, 0],
      [3, 7, 0],
      [5, 8, 1],
    ]);
    expect(insideItems(tools.calls[2]!)).toEqual([
      { kind: "said", step: { seq: 6, turn: "r-a:1", at: null, kind: "text", text: "found it" } },
    ]);
    expect(foldedLabel(tools.calls)).toEqual({ label: "2 steps", last: null, failed: 0 });
  });

  it("asks as a question card, not a tool", () => {
    expect(stepItems([s(1, "tool", { name: "AskUserQuestion" }), s(2, "question", { questions: [] })]).map((i) => i.kind)).toEqual([
      "question",
    ]);
  });
});

describe("lines", () => {
  it("reads a call as one line", () => {
    expect(toolLine(s(1, "tool", { name: "Bash", summary: "cd /home/ada/x && npm test" }))).toBe("$ npm test");
    expect(toolLine(s(1, "tool", { name: "Read", doing: "Reading src/a.ts" }))).toBe("Reading src/a.ts");
    expect(toolLine(s(1, "tool", { name: "Grep", doing: "Searching", summary: "TODO" }))).toBe("Searching TODO");
    expect(
      foldedLabel([{ call: s(1, "tool", { doing: "Running: npm test" }), result: s(2, "result", { error: true }), inside: [] }]),
    ).toEqual({
      label: "1 step",
      last: "npm test",
      failed: 1,
    });
  });

  it("cuts Emberhut's STATUS instruction from a brief", () => {
    expect(brief("Fix it.\n\nEnd every reply with one last line: STATUS")).toBe("Fix it.");
  });

  it("knows a queued message by its turn number", () => {
    expect(turnNumber("r-ab:12")).toBe(12);
    expect(turnNumber(null)).toBeNull();
    expect(isQueued("r-a:3", "r-a:2")).toBe(true);
    expect(isQueued("r-a:2", "r-a:2")).toBe(false);
    expect(isQueued("r-a:3", null)).toBe(false);
  });

  it("finds the word, the running call and the live to-do list", () => {
    expect(liveWord({ thinking: false, typing: false, tool: "Bash", todo: null }, 1)).toBe("Executing");
    expect(liveWord({ thinking: true, typing: false, tool: null, todo: null }, 4)).toBe("Thinking");
    expect(liveWord({ thinking: false, typing: false, tool: null, todo: "Fixing the bug" }, 0)).toBe("Fixing the bug");
    const steps = [s(1, "tool", { name: "Bash", id: "a" }), s(2, "result", { id: "a" }), s(3, "tool", { name: "Read", id: "b" })];
    expect(runningTool(steps)?.seq).toBe(3);
    expect(runningTool(steps.slice(0, 2))).toBeNull();
    const todos = [{ content: "x", status: "in_progress" as const }];
    expect(liveTodos([s(4, "todos", { todos })], "r-a:1")).toEqual(todos);
    expect(liveTodos([s(4, "todos", { todos })], "r-a:2")).toBeNull();
  });
});

describe("applyEvent", () => {
  const head = { id: "r-a", title: "T", status: "running", running: true, turn: "r-a:1" };
  const start = loaded({ session: head, events: [s(1, "user"), s(2, "text")], earlier: true });

  it("adds a step once, and a finished block ends the words being typed", () => {
    let v = applyEvent(start, { type: "delta", turn: "r-a:1", text: "Hel", start: true });
    v = applyEvent(v, { type: "delta", turn: "r-a:1", text: "lo" });
    expect(v.typing).toBe("Hello");
    v = applyEvent(v, { type: "step", step: s(2, "text") });
    expect(v.steps).toHaveLength(2);
    v = applyEvent(v, { type: "step", step: s(3, "text", { text: "Hello" }) });
    expect([v.steps.length, v.seen, v.typing, v.earlier]).toEqual([3, 3, "", true]);
  });

  it("follows the thinking and the tokens, and clears them when the turn ends", () => {
    let v = applyEvent(start, { type: "think", turn: "r-a:1", text: "Hmm", start: true }, () => "2026-11-24T10:00:00Z");
    v = applyEvent(v, { type: "think", turn: "r-a:1", text: " ok" });
    expect([v.thinking, v.thinkingSince]).toEqual(["Hmm ok", "2026-11-24T10:00:00Z"]);
    v = applyEvent(v, { type: "tokens", turn: "r-a:1", tokens: 900 });
    v = applyEvent(v, { type: "head", session: { ...head, tokens: 400 } });
    expect(v.tokens).toBe(900);
    v = applyEvent(v, { type: "step", step: s(3, "thinking", { ms: 2000 }) });
    expect(v.thinking).toBeNull();
    v = applyEvent(v, { type: "head", session: { ...head, running: false, status: "completed" } });
    expect([v.tokens, v.typing, v.head?.status]).toEqual([null, "", "completed"]);
    expect(emptyView.steps).toEqual([]);
  });
});
