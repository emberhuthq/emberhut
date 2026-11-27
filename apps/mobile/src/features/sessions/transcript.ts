import { ASSISTANT, OWNER, type SessionStep, type TodoItem } from "@/api";

// A session's transcript as the page lays it out: turn by turn; in each, what was said, runs of tool calls (each with its
// result and, for a helper agent, the agent's own steps), notes, questions and its to-do list.

export interface ToolCall {
  call: SessionStep;
  result: SessionStep | null;
  /** A helper agent's own steps: its words, calls and results. */
  inside: SessionStep[];
}

export type StepItem =
  | { kind: "said"; step: SessionStep }
  | { kind: "tools"; calls: ToolCall[] }
  | { kind: "note"; step: SessionStep }
  | { kind: "question"; step: SessionStep }
  | { kind: "todos"; step: SessionStep };

export interface Turn {
  id: string;
  steps: SessionStep[];
}

/** Calls the page doesn't list: its to-do list shows them, and loading a tool says nothing. */
const quietTools = new Set(["TodoWrite", "TaskCreate", "TaskUpdate", "TaskList", "TaskGet", "ToolSearch"]);

export const isAgentCall = (name: string | undefined): boolean => name === "Agent" || name === "Task";

export function turns(steps: readonly SessionStep[]): Turn[] {
  const out: Turn[] = [];
  for (const s of steps) {
    const last = out.at(-1);
    if (s.turn && last?.id !== s.turn) out.push({ id: s.turn, steps: [s] });
    else if (!last) out.push({ id: `start-${s.seq}`, steps: [s] });
    else last.steps.push(s);
  }
  return out;
}

export function stepItems(steps: readonly SessionStep[]): StepItem[] {
  const inside = new Map<string, SessionStep[]>();
  const results = new Map<string, SessionStep>();
  for (const s of steps) {
    if (s.parent) inside.set(s.parent, [...(inside.get(s.parent) ?? []), s]);
    if (s.kind === "result" && s.id && !results.has(s.id)) results.set(s.id, s);
  }
  const out: StepItem[] = [];
  let run: ToolCall[] = [];
  const flush = () => {
    if (run.length > 0) out.push({ kind: "tools", calls: run });
    run = [];
  };
  for (const s of steps) {
    if (s.parent) continue;
    if (s.kind === "tool") {
      if (s.name === "AskUserQuestion" || quietTools.has(s.name ?? "")) continue;
      run.push({ call: s, result: (s.id && results.get(s.id)) || null, inside: (s.id && inside.get(s.id)) || [] });
    } else if (s.kind === "thinking") {
      run.push({ call: s, result: null, inside: [] });
    } else if (s.kind !== "result") {
      flush();
      out.push({
        kind: s.kind === "user" || s.kind === "text" ? "said" : s.kind === "question" ? "question" : s.kind === "todos" ? "todos" : "note",
        step: s,
      });
    }
  }
  flush();
  return out;
}

/** A helper agent's steps as their own transcript (no parent: they're the view's own). */
export const insideItems = (call: ToolCall): StepItem[] => stepItems(call.inside.map(({ parent: _parent, ...s }) => s));

/** A call as one line: the command without its cd into the session's own folder, what it did, what a search looked for. */
export function toolLine(c: SessionStep): string {
  if (c.name === "Bash" && c.summary) return `$ ${c.summary.replace(/^cd\s+\S+\s*(&&|;)\s*/, "")}`;
  const what = (c.doing ?? c.name ?? "").replace("Running: ", "");
  return c.name === "Grep" || c.name === "Glob" ? `${what} ${c.summary ?? ""}`.trimEnd() : what;
}

/** A finished run folded: "12 steps", "Thought" for thinking alone. */
export function foldedLabel(calls: readonly ToolCall[]): { label: string; last: string | null; failed: number } {
  const n = calls.filter((c) => c.call.kind === "tool").length;
  const last = calls.findLast((c) => c.call.kind === "tool")?.call.doing ?? null;
  return {
    label: n === 0 ? "Thought" : `${n} step${n === 1 ? "" : "s"}`,
    last: last === null ? null : last.replace("Running: ", ""),
    failed: calls.filter((c) => c.result?.error).length,
  };
}

/** A message as it reads: without the STATUS-line instruction Emberhut adds to every brief. */
export function brief(text: string): string {
  let t = text;
  for (const cut of ["\n\nEnd every reply with one last line", "\n\nEnd your reply with its STATUS line"]) {
    const at = t.indexOf(cut);
    if (at >= 0) t = t.slice(0, at);
  }
  return t;
}

export function whoSaid(by: string | undefined): string {
  switch (by) {
    case OWNER:
      return "You";
    case ASSISTANT:
      return "From Emberhut";
    case "runner":
      return "Emberhut's runner";
    case "self":
      return "Carried on by itself";
    default:
      return "Emberhut";
  }
}

/** "r-ab12:4" is turn 4. */
export function turnNumber(turn: string | null | undefined): number | null {
  const n = Number(turn?.split(":").at(-1));
  return turn && Number.isInteger(n) ? n : null;
}

/** A message sent while a turn runs waits behind it: its turn's number is past the running one's. */
export function isQueued(turn: string, running: string | null | undefined): boolean {
  const now = turnNumber(running);
  const n = turnNumber(turn);
  return now !== null && n !== null && n > now;
}

/** The word for what it's doing: its to-do's own when one is in progress, else one that fits, changing with `tick`. */
export function liveWord(at: { thinking: boolean; typing: boolean; tool: string | null; todo: string | null }, tick: number): string {
  if (at.todo && at.todo.trim() !== "") return at.todo;
  const words = at.thinking
    ? ["Thinking", "Reasoning", "Weighing it up", "Working it out"]
    : at.typing
      ? ["Writing", "Putting it into words"]
      : toolWords(at.tool);
  return words[tick % words.length]!;
}

function toolWords(tool: string | null): string[] {
  switch (tool) {
    case "Bash":
      return ["Running", "Executing", "Crunching"];
    case "Read":
    case "Grep":
    case "Glob":
      return ["Reading", "Exploring", "Tracing the code"];
    case "Edit":
    case "Write":
    case "MultiEdit":
    case "NotebookEdit":
      return ["Editing", "Shaping the code", "Writing code"];
    case "Agent":
    case "Task":
      return ["Orchestrating", "Coordinating agents", "Delegating"];
    case "WebSearch":
    case "WebFetch":
      return ["Researching", "Reading the web"];
    default:
      return ["Working", "Building", "Tinkering", "Figuring it out"];
  }
}

/** The call running now (its result not in yet), for the bar above the reply box. */
export function runningTool(steps: readonly SessionStep[]): SessionStep | null {
  const last = steps.findLast((s) => s.kind === "tool" && !s.parent && !quietTools.has(s.name ?? ""));
  if (!last) return null;
  return steps.some((s) => s.kind === "result" && s.id === last.id) ? null : last;
}

/** The to-do list of the turn running now: the latest, if it changed in this turn. */
export function liveTodos(steps: readonly SessionStep[], turn: string | null | undefined): TodoItem[] | null {
  const t = steps.findLast((s) => s.kind === "todos" && !s.parent);
  return t && t.turn === turn && t.todos && t.todos.length > 0 ? t.todos : null;
}

/** "12.4k" tokens. */
export const tokenCount = (n: number): string => (n >= 1000 ? `${(n / 1000).toFixed(n >= 10_000 ? 0 : 1)}k` : String(n));

/** A long answer is kept short until "Show more". */
export const isLong = (text: string): boolean => text.length > 1200 || text.split("\n").length > 18;
