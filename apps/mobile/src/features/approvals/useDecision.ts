import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import type { Approval } from "@/api";
import { inBackground } from "@/log";
import { api } from "@/services";
import { queryKeys } from "@/state/query";
import { afterAnswer, OneAtATime, type AfterAnswer, type ApprovalAction } from "./decision";

export type { ApprovalAction } from "./decision";

/**
 * Sends a decision or a withdrawal for request `id`, one at a time: `send` refuses a second while one is out, and `turn`
 * lets the screen hold the turn while it asks (a second tap opens no second question). What the caches do after is
 * `afterAnswer`'s: the answer replaces the page's copy, a refusal that means the request changed reads it again, a 404
 * lets it go, and the lists and the states are read again.
 */
export function useDecision(id: number) {
  const client = useQueryClient();
  // One guard for the screen's life: made once, never replaced.
  const [turn] = useState(() => new OneAtATime());
  const apply = (after: AfterAnswer) => {
    if (after.set) client.setQueryData(queryKeys.approval(id), after.set);
    if (after.remove) client.removeQueries({ queryKey: queryKeys.approval(id), exact: true });
    for (const queryKey of after.invalidate) inBackground(client.invalidateQueries({ queryKey }), "useDecision: invalidateQueries");
  };
  const mutation = useMutation({
    mutationFn: (action: ApprovalAction): Promise<Approval> =>
      action.kind === "decide" ? api.approvals.decide(id, action.decision, action.payloadHash) : api.approvals.withdraw(id),
    onSuccess: (approval) => apply(afterAnswer(id, { ok: approval })),
    onError: (error) => apply(afterAnswer(id, { error })),
    onSettled: () => turn.release(),
  });
  /** Sends `action` if no answer is out (the caller may hold the turn already, for its question); whether it went. */
  const send = (action: ApprovalAction, held = false): boolean => {
    if (!held && !turn.take()) return false;
    mutation.mutate(action);
    return true;
  };
  return { ...mutation, send, turn };
}
