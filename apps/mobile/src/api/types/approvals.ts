import type { Instant } from "./common";

/**
 * What an approval is for. post, send, merge, deploy, create and other follow the agent's policy; payment, credential,
 * secret, account_security and as_owner are the fixed core, always the owner's. A newer server may send a kind this app
 * doesn't know, so any other word is let through as it is.
 */
export type ActionKind =
  | "post"
  | "send"
  | "merge"
  | "deploy"
  | "create"
  | "other"
  | "payment"
  | "credential"
  | "secret"
  | "account_security"
  | "as_owner"
  | (string & {});

/** Where a request is in its life: pending, then approved or rejected, then used; or expired, or withdrawn. */
export type ApprovalState = "pending" | "approved" | "rejected" | "used" | "expired" | "withdrawn" | (string & {});

/** The channel a decision came through: the apps, an approver's Slack DM, a one-time link, the approver's own app. */
export type DecidedVia = "app" | "slack" | "link" | "webhook" | (string & {});

/** An approver's answer. */
export type ApprovalDecision = "approve" | "reject";

/** One file of a request, as the server read it from disk (never as the agent described it). */
export interface ApprovalFile {
  name: string;
  /** In bytes; none for an imported file that lacks it. */
  size?: number | null;
  /** The SHA-256 of its bytes, 64 hex digits. */
  sha256: string;
}

/** An approval request, with the exact payload it covers. */
export interface Approval {
  id: number;
  /** The agent that acts. */
  agent: string;
  agentName?: string | null;
  kind: ActionKind;
  /** Where it goes: a channel, a repository and branch, a URL, an account. Part of what is approved. */
  target: string;
  /** One line of the agent's. */
  summary: string;
  /** The exact words, if any. */
  text?: string | null;
  files?: ApprovalFile[] | null;
  /** The payload's hash, 64 hex digits: a decision carries it back. */
  payloadHash: string;
  /** "owner", or the external approver's id. */
  approver: string;
  approverName?: string | null;
  /** It waits on you: Approve and Reject are offered (else it is shown read-only). */
  waitsOnOwner?: boolean | null;
  /** A payment's amount, in cents. */
  amountCents?: number | null;
  /** The amount's currency ("EUR"): its agent's spending currency, set with amountCents. */
  amountCurrency?: string | null;
  /** The task whose step asked. */
  task?: number | null;
  state: ApprovalState;
  /** Why it was expired, withdrawn or rejected. */
  stateReason?: string | null;
  createdAt: Instant;
  /** When it stops waiting (for its answer, or to be used); none once it can't. */
  expiresAt?: Instant | null;
  decidedAt?: Instant | null;
  /** Who decided, as the channel proved it. */
  decidedBy?: string | null;
  decidedVia?: DecidedVia | null;
  usedAt?: Instant | null;
  /** The task or tool that used it. */
  usedBy?: string | null;
  /** From an earlier installation's history: over, and never usable. */
  imported?: boolean | null;
}

/** GET /api/approvals's filters; none: every request. */
export interface ApprovalsQuery {
  state?: ApprovalState;
  /** Only this agent's. */
  agent?: string;
}

/** GET /api/approvals: the requests, newest first. */
export interface ApprovalsPage {
  approvals: Approval[];
}

/** GET /api/approvals/{id}, and the answer to a decision or a withdrawal: the request as it is now. */
export interface ApprovalReply {
  approval: Approval;
}

/** POST /api/approvals/{id}/decision: yes or no on the payload hash you were shown. A hash that isn't the request's decides nothing. */
export interface DecisionRequest {
  decision: ApprovalDecision;
  payloadHash: string;
  /** Why, if you say (500 characters at most). */
  reason?: string;
}

/** The live stream's `approvals` event: a request changed. The app fetches what it shows again. */
export interface ApprovalsEvent {
  /** The request that changed. */
  id: number;
  /** Its state now. */
  state: ApprovalState;
  /** How many requests wait on you now. */
  pending: number;
}
