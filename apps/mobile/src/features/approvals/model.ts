import { ApiError, type ActionKind, type AgentRow, type Approval, type ApprovalFile, type ApprovalState, type EmberhutState } from "@/api";
import { fileSize } from "@/format/size";

// Approvals as the app reads them: what waits on you (Approve or Reject on the exact payload), what waits on an
// approver (shown, never decided here), and the agents whose requests they are.

/**
 * Unicode's format characters (general category Cf, as of Unicode 16), as code point ranges: the same table the server
 * writes out on its signed-link page and in Slack.
 */
const formatCharacters: readonly (readonly [number, number])[] = [
  [0x00ad, 0x00ad],
  [0x0600, 0x0605],
  [0x061c, 0x061c],
  [0x06dd, 0x06dd],
  [0x070f, 0x070f],
  [0x0890, 0x0891],
  [0x08e2, 0x08e2],
  [0x180e, 0x180e],
  [0x200b, 0x200f],
  [0x202a, 0x202e],
  [0x2060, 0x2064],
  [0x2066, 0x206f],
  [0xfeff, 0xfeff],
  [0xfff9, 0xfffb],
  [0x110bd, 0x110bd],
  [0x110cd, 0x110cd],
  [0x13430, 0x1343f],
  [0x1bca0, 0x1bca3],
  [0x1d173, 0x1d17a],
  [0xe0001, 0xe0001],
  [0xe0020, 0xe007f],
];

/**
 * Whether a code point hides in text: a format character (bidi overrides and isolates, zero-width characters, the
 * byte-order mark…), a control character other than a line break or a tab, or the line and paragraph separators.
 */
function hides(code: number): boolean {
  if (code === 0x0a || code === 0x09) return false;
  if (code < 0x20 || (code >= 0x7f && code <= 0x9f) || code === 0x2028 || code === 0x2029) return true;
  return formatCharacters.some(([first, last]) => code >= first && code <= last);
}

/**
 * A request's words as you are shown them: exactly as they are, but with every character that would change how they read
 * without showing itself written out as `[U+202E]`, so a target or a text can't show one thing and say another.
 */
export function visible(text: string): string {
  let out = "";
  for (const char of text) {
    const code = char.codePointAt(0) ?? 0;
    out += hides(code) ? `[U+${code.toString(16).toUpperCase().padStart(4, "0")}]` : char;
  }
  return out;
}

/** The default agent's id: the assistant in Main and every room no other agent owns. */
export const mainAgent = "main";

/** The fixed core: kinds only you ever approve, and only in the apps, whatever an agent's policy says. */
const fixedCore: ReadonlySet<string> = new Set(["payment", "credential", "secret", "account_security", "as_owner"]);

export function isFixedCore(kind: ActionKind): boolean {
  return fixedCore.has(kind);
}

const kindNames: Record<string, string> = {
  post: "Post",
  send: "Message",
  merge: "Merge",
  deploy: "Deploy",
  create: "Create",
  other: "Other",
  payment: "Payment",
  credential: "Password or code",
  secret: "Secret",
  account_security: "Account security",
  as_owner: "In your name",
};

/** What a request is for, as its chip says it; a kind this app doesn't know, as the server spells it (nothing hidden). */
export function kindName(kind: ActionKind): string {
  return kindNames[kind] ?? visible(kind.replace(/_/g, " "));
}

const stateNames: Record<string, string> = {
  pending: "Waiting",
  approved: "Approved",
  rejected: "Rejected",
  used: "Used",
  expired: "Expired",
  withdrawn: "Withdrawn",
};

export function stateName(state: ApprovalState): string {
  return stateNames[state] ?? state;
}

/** A SHA-256 as the app shows it: its first 12 hex digits, enough to tell two files or payloads apart by eye. */
export function shortHash(hex: string): string {
  return hex.slice(0, 12).toLowerCase();
}

/** One file's line: its name, size and short hash ("cover.png · 12.3 KB · 3f2a9c0b1d4e"). */
export function fileLine(file: ApprovalFile): string {
  return [visible(file.name), file.size == null ? "size unknown" : fileSize(file.size), shortHash(file.sha256)].join(" · ");
}

/**
 * A payment's amount, exactly: whole cents, the thousands grouped, the currency's code after ("1,234.50 EUR"). Never
 * rounded or put in a local style: this is what you say yes to.
 */
export function amountText(cents: number, currency?: string | null): string {
  const whole = Math.floor(Math.abs(cents) / 100);
  const rest = String(Math.abs(cents) % 100).padStart(2, "0");
  const grouped = String(whole).replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  return `${cents < 0 ? "-" : ""}${grouped}.${rest}${currency ? ` ${currency}` : ""}`;
}

/** The agent's name, nothing in it hidden: the request's own, the state's agent's, or its id. */
export function agentName(approval: Approval, agents: readonly AgentRow[] = []): string {
  return visible(approval.agentName ?? agents.find((a) => a.id === approval.agent)?.name ?? approval.agent);
}

/** Who decides it, as a role: you, or the approver by name. */
export function approverRole(approval: Approval): string {
  if (approval.approver === "owner") return "You";
  return `${visible(approval.approverName ?? approval.approver)}, the approver`;
}

/** A pending request that waits on you: you can approve or reject it. */
export function waitsOnYou(approval: Approval): boolean {
  return approval.state === "pending" && approval.waitsOnOwner === true;
}

export interface ApprovalGroups {
  /** Waiting on you: Approve and Reject. */
  yours: Approval[];
  /** Waiting on an approver: shown with their name, never decided here. */
  others: Approval[];
}

/** The pending requests split by who decides them, each group in the server's order (newest first). */
export function approvalGroups(approvals: readonly Approval[]): ApprovalGroups {
  const pending = approvals.filter((a) => a.state === "pending");
  return { yours: pending.filter(waitsOnYou), others: pending.filter((a) => !waitsOnYou(a)) };
}

/** Needs you's Approvals row: how many wait on you, how many of those are the fixed core's, and how many wait on others. */
export function approvalsSummary(approvals: readonly Approval[]): { yours: number; title: string; subtitle: string | null } {
  const { yours, others } = approvalGroups(approvals);
  const core = yours.filter((a) => isFixedCore(a.kind)).length;
  const title = yours.length > 0 ? `${yours.length} wait${yours.length === 1 ? "s" : ""} on you` : "None wait on you";
  const parts = [core > 0 ? `${core} only you can approve` : null, others.length > 0 ? `${others.length} on others` : null];
  const subtitle = parts.filter(Boolean).join(" · ");
  return { yours: yours.length, title, subtitle: subtitle === "" ? null : subtitle };
}

/** How many requests wait on you, from the state's pending ones (Needs you's badge counts them). */
export function approvalsWaitingOnYou(state: EmberhutState | undefined): number {
  return (state?.approvals ?? []).filter(waitsOnYou).length;
}

/** How many of agent `agent`'s requests wait on you, from the state's pending ones (its room's header counts them). */
export function agentWaitingOnYou(state: EmberhutState | undefined, agent: string): number {
  return (state?.approvals ?? []).filter((a) => a.agent === agent && waitsOnYou(a)).length;
}

/** A count of an agent's requests waiting on you, as its button says it: "2 wait on you", "No approvals wait on you". */
export function waitingWords(count: number): string {
  if (count === 0) return "No approvals wait on you";
  return `${count} wait${count === 1 ? "s" : ""} on you`;
}

/** The agents with a room of their own, by room: every agent but the default one, whose room is Main. */
export function agentsByRoom(agents: readonly AgentRow[] | null | undefined): Map<number, AgentRow> {
  return new Map((agents ?? []).filter((a) => a.id !== mainAgent).map((a) => [a.room, a]));
}

/** The line under an agent's name: who it works for, and Off while disabled. */
export function agentLine(agent: AgentRow): string {
  return `Works for ${visible(agent.principal)}${agent.enabled ? "" : " · Off"}`;
}

/** The caption under an agent room's name in its header: what it's doing (Off while disabled) and who it works for. */
export function agentCaption(agent: AgentRow, status: string): string {
  return `${agent.enabled ? status : "Off"} · works for ${visible(agent.principal)}`;
}

/**
 * What a screen reader says of a request's card: everything the card shows that tells it apart (the kind, the fixed
 * core's mark, the agent, the line, the target, a payment's amount, and who decides when that isn't you).
 */
export function approvalCardLabel(approval: Approval, agents: readonly AgentRow[] = [], currency?: string | null): string {
  return [
    kindName(approval.kind),
    isFixedCore(approval.kind) ? "only you can approve this" : null,
    `by ${agentName(approval, agents)}`,
    visible(approval.summary),
    `to ${visible(approval.target)}`,
    approval.amountCents == null ? null : amountText(approval.amountCents, currency),
    waitsOnYou(approval) ? null : `waits on ${approverRole(approval)}`,
  ]
    .filter(Boolean)
    .join(", ");
}

/** What you're asked to confirm before an Approve, naming the agent, what it does and where. */
export function approveQuestion(
  approval: Approval,
  agents: readonly AgentRow[] = [],
  currency?: string | null,
): { title: string; message: string } {
  const who = agentName(approval, agents);
  const amount = approval.amountCents == null ? "" : ` for ${amountText(approval.amountCents, currency)}`;
  return {
    title: `Approve this ${kindName(approval.kind).toLowerCase()}?`,
    message: `${who} may then do it once${amount}, to ${visible(approval.target)}, exactly as shown. Anything changed after this needs a new OK.`,
  };
}

/** Why a decision or a withdrawal didn't go through, said plainly; `refresh` when the request should be fetched again. */
export function decisionFailure(error: unknown): { message: string; refresh: boolean } {
  if (error instanceof ApiError) {
    if (error.status === 409)
      return { message: "This request changed or was already decided, so nothing was done. Here it is as it is now.", refresh: true };
    if (error.status === 404) return { message: "This request is gone.", refresh: true };
    // The request may have gone out and its answer been lost: say so, and read it again.
    if (error.unreachable)
      return { message: "Couldn't reach Emberhut: it may or may not have gone through. Here it is as the server has it.", refresh: true };
    return { message: `Not done: ${error.message}`, refresh: true };
  }
  return { message: `Not done: ${error instanceof Error ? error.message : String(error)}`, refresh: false };
}
