import type { EmberhutHttp } from "../http";
import type { Approval, ApprovalDecision, ApprovalReply, ApprovalsPage, ApprovalsQuery } from "../types";

export function approvalEndpoints(http: EmberhutHttp) {
  return {
    /** GET /api/approvals?state=&agent=: the requests, newest first; without a filter, every one. */
    async list(query: ApprovalsQuery = {}, signal?: AbortSignal): Promise<Approval[]> {
      const page = await http.get<ApprovalsPage>("/api/approvals", { state: query.state, agent: query.agent }, { signal });
      return page.approvals;
    },
    /** GET /api/approvals/{id}: one request as it is now. */
    async get(id: number, signal?: AbortSignal): Promise<Approval> {
      const reply = await http.get<ApprovalReply>(`/api/approvals/${id}`, undefined, { signal });
      return reply.approval;
    },
    /**
     * POST /api/approvals/{id}/decision: your yes or no, on the payload hash you were shown. A 409 means the request changed
     * or was already decided; the answer is the request as it is after the decision.
     */
    async decide(id: number, decision: ApprovalDecision, payloadHash: string, reason?: string): Promise<Approval> {
      const reply = await http.post<ApprovalReply>(`/api/approvals/${id}/decision`, {
        decision,
        payloadHash,
        ...(reason ? { reason } : {}),
      });
      return reply.approval;
    },
    /** POST /api/approvals/{id}/withdraw: takes a pending request back; nothing it covers goes out. */
    async withdraw(id: number, reason?: string): Promise<Approval> {
      const reply = await http.post<ApprovalReply>(`/api/approvals/${id}/withdraw`, reason ? { reason } : {});
      return reply.approval;
    },
  };
}
