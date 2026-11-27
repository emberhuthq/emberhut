import { OWNER, type Issue, type RunInfo, type TaskMoment } from "@/api";
import { askOf } from "@/features/needsYou/model";
import { isMine, isOpen } from "./model";

// A task's page: its runs, where it stands, the exchange and its buttons.

/** Its runs; an older server sends none, so they are rebuilt from the history lines a finished turn leaves. */
export function timeline(task: Issue): RunInfo[] {
  if (task.runs) return task.runs;
  return (task.history ?? [])
    .flatMap((moment, i): RunInfo[] => {
      const at = moment.text.indexOf(" · STATUS:");
      if (at < 0) return [];
      return [
        {
          id: -i,
          kind: "thread",
          machine: "",
          thread: "",
          title: null,
          asked: null,
          answer: moment.text.slice(0, at),
          status: moment.text.slice(at + 3),
          requested: null,
          started: null,
          ended: moment.at,
          outcome: "completed",
        },
      ];
    })
    .reverse();
}

export type RunStatusKind = "done" | "working" | "blocked" | "needs";

export interface RunStatus {
  kind: RunStatusKind;
  /** "Needs you: pick the cluster". */
  text: string;
  /** What follows the label: "pick the cluster". */
  rest: string | null;
}

const runLabels: Record<RunStatusKind, string> = { done: "Done", working: "Working", blocked: "Blocked", needs: "Needs you" };

/** "STATUS: needs Emberhut: pick the cluster" → needs, "Needs you: pick the cluster". */
export function runStatus(line: string | null | undefined): RunStatus | null {
  const match = /STATUS:\s*(done|working|blocked|needs emberhut)\b:?\s*(.*)/i.exec(line ?? "");
  if (!match) return null;
  const word = match[1]!.toLowerCase();
  const kind: RunStatusKind = word.startsWith("needs") ? "needs" : (word as RunStatusKind);
  const rest = match[2]!.trim();
  return { kind, text: rest ? `${runLabels[kind]}: ${rest}` : runLabels[kind], rest: rest || null };
}

/** Where it stands, from the latest run that answered, and what's left (its STATUS line), unless it's done. */
export function standing(task: Issue, runs: readonly RunInfo[]): { line: string; left: string | null } | null {
  const answered = runs.find((r) => (r.answer ?? "").trim() !== "");
  if ((task.runs == null && task.live?.last?.line) || !answered?.answer) {
    const last = task.live?.last?.line;
    return last ? { line: last, left: null } : null;
  }
  const status = runStatus(answered.status);
  return { line: answered.answer, left: status?.kind === "done" || task.status === "done" ? null : (status?.rest ?? null) };
}

/** Steps that only change a task's state: left out of the exchange above the reply box. */
const quietSteps = new Set([
  "Started",
  "Back to to do",
  "Waiting on the owner",
  "Done",
  "Canceled",
  "Archived by the owner",
  "Unarchived by the owner",
  "Created",
  "Created by Emberhut",
  "A worker started on it",
  "Picked up again with the owner's answer",
  "Picked up again with its new brief",
]);

/** Your last message on the task and what came back since, at most four, so the answer is read where it was asked. */
export function exchange(history: readonly TaskMoment[]): TaskMoment[] {
  let last = -1;
  history.forEach((m, i) => {
    if (m.who === OWNER && !quietSteps.has(m.text) && !m.text.startsWith("Chose: ")) last = i;
  });
  if (last < 0) return [];
  return history
    .slice(last)
    .filter((m) => !quietSteps.has(m.text))
    .slice(0, 4);
}

/** Where your reply on a task went, in a line. */
export function replyNote(result: { to?: string; queued?: boolean }): string {
  switch (result.to) {
    case "worker":
      return result.queued ? "Sent. Emberhut's worker reads it when this run ends." : "Sent. Emberhut's worker picks it up now.";
    case "session":
      return result.queued ? "Sent. The session takes it after the turn it's on." : "Sent to the task's session as its next run.";
    default:
      return "Sent. Emberhut answers here on the task.";
  }
}

// The task's own buttons (TaskActions).

export type PrimaryAction = { kind: "answer"; choice: string } | { kind: "didIt" } | { kind: "markDone" } | { kind: "unarchive" };

export interface TaskActionPlan {
  /** The task an answer goes to: this one, or the one its action names. */
  target: number;
  primary: PrimaryAction | null;
  /** The other answers. */
  others: string[];
  notNow: boolean;
  archive: boolean;
}

/**
 * The answers and actions pinned under a task's page: the first answer as the main button and the rest beside it; "I did
 * it" for something only you can do; "Mark done" for your own; then Not now and Archive. An archived one: Unarchive.
 * `answersInPage`: the answers are cards on the page already.
 */
export function taskActions(task: Issue, issues: readonly Issue[], answersInPage: boolean): TaskActionPlan {
  const other = task.decision ? null : askOf(task, issues);
  const stands = other && other.id !== task.id ? other : null;
  const decision = stands?.decision ?? task.decision ?? null;
  const target = stands?.id ?? task.id;
  const options = answersInPage ? [] : (decision?.options ?? []);
  const plan: TaskActionPlan = {
    target,
    primary: null,
    others: options.slice(1),
    notNow: task.status === "todo" || task.status === "waiting",
    archive: isOpen(task),
  };
  const xReply = (task.artifacts ?? []).some((a) => a.x != null);
  if (task.status === "canceled") plan.primary = { kind: "unarchive" };
  else if (options[0] !== undefined) plan.primary = { kind: "answer", choice: options[0] };
  else if (xReply) plan.primary = null;
  else if (task.status === "waiting" && task.action && !answersInPage && !decision && askOf(task, issues) !== null)
    plan.primary = { kind: "didIt" };
  else if (isMine(task) && task.status !== "done") plan.primary = { kind: "markDone" };
  return plan;
}
