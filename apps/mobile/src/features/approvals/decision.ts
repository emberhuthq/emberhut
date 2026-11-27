import type { QueryKey } from "@tanstack/react-query";
import { ApiError, type Approval, type ApprovalDecision } from "@/api";
import { queryKeys } from "@/state/query";
import { decisionFailure, waitsOnYou } from "./model";

// What you do with a request, as plain logic the screen and the hook only wire up: which payload hash an answer carries,
// what is read again after it, and one answer at a time.

/** What you do with a request: approve or reject it on the payload hash you were shown, or withdraw it. */
export type ApprovalAction = { kind: "decide"; decision: ApprovalDecision; payloadHash: string } | { kind: "withdraw" };

/** Approve or reject the request on screen: the answer carries its payload hash, so a request that changed decides nothing. */
export function decideOn(onScreen: Approval, decision: ApprovalDecision): ApprovalAction {
  return { kind: "decide", decision, payloadHash: onScreen.payloadHash };
}

/**
 * Whether a question asked on `asked` still holds for the request as it is now: the same payload, still waiting on you.
 * A confirmation that outlived a re-read decides nothing.
 */
export function stillAsked(asked: Approval, now: Approval | undefined): boolean {
  return now !== undefined && now.payloadHash === asked.payloadHash && waitsOnYou(now);
}

/** Whether you can take a request back: while it waits, and once approved until it is used (an imported one never). */
export function canWithdraw(approval: Approval): boolean {
  return approval.state === "pending" || (approval.state === "approved" && approval.imported !== true);
}

/** What the caches do after an answer: the request's copy set or let go, and the keys read again. */
export interface AfterAnswer {
  /** The request as the server answered it: the page's copy. */
  set?: Approval;
  /** The server doesn't have it (404): its copy goes, so the page says it's gone. */
  remove: boolean;
  /** Read again: the request, the lists, the states (their counts). */
  invalidate: QueryKey[];
}

/** After an answer on request `id`: what the caches do with its outcome. */
export function afterAnswer(id: number, outcome: { ok: Approval } | { error: unknown }): AfterAnswer {
  const lists = [queryKeys.allApprovals, queryKeys.allStates];
  if ("ok" in outcome) return { set: outcome.ok, remove: false, invalidate: lists };
  if (!decisionFailure(outcome.error).refresh) return { remove: false, invalidate: [] };
  const gone = outcome.error instanceof ApiError && outcome.error.status === 404;
  return { remove: gone, invalidate: gone ? lists : [queryKeys.approval(id), ...lists] };
}

/** One answer at a time: a second tap while one is out (or a question is up) does nothing. */
export class OneAtATime {
  private busy = false;

  /** Takes the turn; false while another holds it. */
  take(): boolean {
    if (this.busy) return false;
    this.busy = true;
    return true;
  }

  /** Gives the turn back. */
  release(): void {
    this.busy = false;
  }

  get held(): boolean {
    return this.busy;
  }
}
