import { TASK_KEY_PATTERN, type Decision, type Issue, type NeedKind, type EmberhutState, type Project, type WorkRow } from "@/api";
import { approvalsWaitingOnYou } from "@/features/approvals/model";
import { daysFromToday, parseDay, parseInstant } from "@/format/time";

// What waits on you, as Needs you reads it: the state's working rows that wait on you,
// grouped by project (the default) or, by your pick, by when you have to decide, the cheap ones first.

/** When you have to decide: Today (overdue too), This week (7 days), This month (30), Later (folded, counted nowhere). */
export type When = "today" | "week" | "month" | "later";

const whenOrder: readonly When[] = ["today", "week", "month", "later"];

export const whenNames: Record<When, string> = { today: "Today", week: "This week", month: "This month", later: "Later" };

/** An ask with no date yet is This week, the server's default. */
export function whenOf(day: string | null | undefined, now: Date = new Date()): When {
  const date = parseDay(day);
  if (!date) return "week";
  const days = daysFromToday(date, now);
  return days <= 0 ? "today" : days <= 6 ? "week" : days <= 30 ? "month" : "later";
}

/** A paused project's asks wait with Later. */
export function rowWhen(row: WorkRow, now: Date = new Date()): When {
  return row.paused ? "later" : whenOf(row.decideBy, now);
}

/** What it costs you, as the small label on its card. */
export const kindNames: Record<NeedKind, string> = { approve: "Quick yes", choose: "Read & decide", do: "Only you" };

const kindRank: Record<NeedKind, number> = { approve: 0, choose: 1, do: 2 };

const isWaiting = (row: WorkRow): boolean => row.state === "waiting";

const staleMs = 12 * 3_600_000;

/** Not re-checked in 12 hours: it may be outdated (Emberhut re-checks it on its own). */
export function isStale(row: WorkRow, now: Date = new Date()): boolean {
  const checked = parseInstant(row.checked ?? row.at);
  return isWaiting(row) && checked !== null && now.getTime() - checked.getTime() > staleMs;
}

/** An ask's own check: not re-checked in 12 hours, so it may be outdated. */
export function askIsStale(decision: Decision, updated: string, now: Date = new Date()): boolean {
  const checked = parseInstant(decision.checked ?? decision.asked ?? updated);
  return checked !== null && now.getTime() - checked.getTime() > staleMs;
}

/** A task key in an action's words, not run on from a word before it. */
const keyInAction = new RegExp(`(?:^|[^\\p{L}\\p{N}_])${TASK_KEY_PATTERN}`, "u");

/**
 * The task whose ask a task stands for. A task without a decision whose action names another task's key ("One yes on <key>…")
 * stands for that one's ask, or for nothing once that one no longer waits.
 */
export function askOf(task: Issue, issues: readonly Issue[]): Issue | null {
  if (task.decision) return task;
  const match = keyInAction.exec(task.action ?? "");
  const id = match ? Number(match[1]) : task.id;
  const other = id === task.id ? undefined : issues.find((i) => i.id === id);
  if (!other) return task;
  return other.status === "waiting" && (other.decision || other.action) ? other : null;
}

/** Waiting on you for real: not just acted on, and not standing for an ask that no longer waits. */
function waitsOnHim(task: number | null | undefined, issues: readonly Issue[], gone: ReadonlySet<number>): boolean {
  if (task == null || gone.has(task)) return false;
  const issue = issues.find((i) => i.id === task);
  return issue ? askOf(issue, issues) !== null : true;
}

/** A row standing for another row's ask: that row answers it, so this one would be the same ask twice. */
function isCopy(row: WorkRow, rows: readonly WorkRow[], issues: readonly Issue[]): boolean {
  const issue = issues.find((i) => i.id === row.task);
  const ask = issue ? askOf(issue, issues) : null;
  return ask !== null && ask.id !== row.task && rows.some((r) => r.task === ask.id);
}

/** What waits on you, Later included, copies left out. */
export function needsRows(state: EmberhutState | undefined, gone: ReadonlySet<number>): WorkRow[] {
  const issues = state?.issues ?? [];
  const rows = (state?.working ?? []).filter((r) => isWaiting(r) && waitsOnHim(r.task, issues, gone));
  return rows.filter((r) => !isCopy(r, rows, issues));
}

/**
 * The tab's badge: the Needs you page's sections without Later, counted from the same groups (as the reference client
 * counts), and the approval requests waiting on you (the page's Approvals row).
 */
export function needsCount(state: EmberhutState | undefined, gone: ReadonlySet<number>, now: Date = new Date()): number {
  const asks = needsGroups(needsRows(state, gone), now)
    .filter((g) => g.when !== "later")
    .reduce((n, g) => n + g.rows.length, 0);
  return asks + approvalsWaitingOnYou(state);
}

export interface NeedsGroup {
  when: When;
  rows: WorkRow[];
}

/** How Needs you is grouped: by project (the default) or by when you have to decide. */
export type NeedsGrouping = "project" | "when";

/** One project's asks: its name ("Other" for those with no project) and its rows, most urgent first. */
export interface ProjectGroup {
  pid: number | null;
  name: string;
  rows: WorkRow[];
}

const whenRank = (row: WorkRow, now: Date) => whenOrder.indexOf(rowWhen(row, now));

/** By code point, as the reference client compares: "~" (no date) after every day ("localeCompare" puts it first). */
const order = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

/** Most urgent first: by when you have to decide (overdue first), then the cheap ones, then the newest ask. */
export function byUrgency(rows: readonly WorkRow[], now: Date = new Date()): WorkRow[] {
  const rank = (r: WorkRow) => kindRank[r.need ?? "choose"];
  return [...rows].sort(
    (a, b) =>
      whenRank(a, now) - whenRank(b, now) ||
      order(a.decideBy ?? "~", b.decideBy ?? "~") ||
      rank(a) - rank(b) ||
      order(b.asked ?? "", a.asked ?? ""),
  );
}

/**
 * Needs you by project, Later left out (it folds on its own): each project's asks most urgent first, the projects in the
 * order of their most urgent ask, Other (no project, or one the state doesn't have) last. Their counts add up to the badge.
 */
export function projectGroups(rows: readonly WorkRow[], projects: readonly Project[], now: Date = new Date()): ProjectGroup[] {
  const names = new Map(projects.map((p) => [p.id, p.name]));
  const groups: ProjectGroup[] = [];
  for (const row of byUrgency(rows, now).filter((r) => rowWhen(r, now) !== "later")) {
    const pid = row.pid != null && names.has(row.pid) ? row.pid : null;
    const group = groups.find((g) => g.pid === pid);
    if (group) group.rows.push(row);
    else groups.push({ pid, name: pid == null ? "Other" : names.get(pid)!, rows: [row] });
  }
  return [...groups.filter((g) => g.pid != null), ...groups.filter((g) => g.pid == null)];
}

/** By when you have to decide; in each, a quick yes first, then a read and a pick, then what only you can do; then the soonest. */
export function needsGroups(rows: readonly WorkRow[], now: Date = new Date()): NeedsGroup[] {
  const rank = (r: WorkRow) => kindRank[r.need ?? "choose"];
  const sorted = [...rows].sort(
    (a, b) => rank(a) - rank(b) || order(a.decideBy ?? "~", b.decideBy ?? "~") || order(b.asked ?? "", a.asked ?? ""),
  );
  return whenOrder.map((when) => ({ when, rows: sorted.filter((r) => rowWhen(r, now) === when) })).filter((group) => group.rows.length > 0);
}

/** The line in bold on a card: what you should do, or the ask of the task it stands for. */
export function cardAction(row: WorkRow, issues: readonly Issue[]): string {
  const issue = issues.find((i) => i.id === row.task);
  const target = issue ? askOf(issue, issues) : null;
  const other = target && target.id !== row.task ? (target.action ?? (target.decision ? askText(target.decision).summary : null)) : null;
  return other ?? row.action ?? row.line ?? "Open it to see what Emberhut needs";
}

export interface CardAnswers {
  /** The task the answer goes to: the row's own, or the one its action names. */
  task: number;
  options: string[];
  decision: Decision | null;
}

/**
 * The answers right on a card: only for a quick yes or something only you can do, checked lately; a pick between options
 * opens on its page, to read first, and so does an X reply. No options and no decision: you say you did it ("Done").
 */
export function cardAnswers(row: WorkRow, issues: readonly Issue[], now: Date = new Date()): CardAnswers | null {
  if (!isWaiting(row) || row.need === "choose" || isStale(row, now) || !row.action || row.task == null) return null;
  // An X reply is posted from its page (the post, the replies to pick from), never closed with "I did it" from here.
  if (row.reply?.x != null) return null;
  const own = issues.find((i) => i.id === row.task);
  const target = own ? askOf(own, issues) : null;
  if (own && !target) return null;
  const task = target?.id ?? row.task;
  const decision = target?.decision ?? null;
  const options = decision?.options ?? (task === row.task ? (row.options ?? []) : []);
  if (options.length === 0 && decision !== null) return null;
  return { task, options, decision };
}

/** The message an answer starting "Send" puts on the clipboard: the draft Emberhut prepared, when the ask has one. */
export function draftToCopy(choice: string, decision: Decision | null): string | null {
  if (!decision || !choice.trim().toLowerCase().startsWith("send")) return null;
  return askText(decision).draft;
}

/** "Done" on something only you can do (a login, a call) reads "I did it"; the server still gets "Done". */
export function answerLabel(option: string): string {
  return option.trim().toLowerCase() === "done" ? "I did it" : option;
}

/** Emberhut's question split for reading: what it is about, the draft it prepared (the quoted message), anything after. */
export interface AskText {
  summary: string;
  draftLabel: string | null;
  draft: string | null;
  after: string;
}

export function askText(decision: Decision): AskText {
  if (decision.draft) return { summary: decision.question, draftLabel: null, draft: decision.draft, after: "" };
  return splitAsk(decision.question);
}

/**
 * The draft starts after a "Draft reply to Abdullah:" line, or at a quote that opens a line, and runs to the last closing
 * quote (quotes inside it are fine). A short quoted phrase in the summary is not a draft.
 */
export function splitAsk(question: string): AskText {
  const whole: AskText = { summary: question, draftLabel: null, draft: null, after: "" };
  const label = /^([^\n]{0,60}[Dd]raft[^\n]{0,60}):[ \t]*$/m.exec(question);
  const from = label ? label.index + label[0].length : /^[ \t]*["“]/m.exec(question)?.index;
  if (from === undefined) return whole;
  const openAt = question.slice(from).search(/["“]/);
  if (openAt < 0) return whole;
  const open = from + openAt;
  const close = Math.max(question.lastIndexOf('"'), question.lastIndexOf("”"));
  if (close - open <= 40) return whole;
  return {
    summary: question.slice(0, label ? label.index : from).trim(),
    draftLabel: label?.[1] ?? null,
    draft: question.slice(open + 1, close),
    after: question.slice(close + 1).trim(),
  };
}
