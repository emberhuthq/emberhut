import type { EmberhutHttp } from "../http";
import type { NewPartyRequest, PartiesPage, PartyPage, PartyReply, Reply } from "../types";

const base = (id: number) => `/api/parties/${id}`;

export function partyEndpoints(http: EmberhutHttp) {
  return {
    /** GET /api/parties: the list, the presets, your past personas, the models; a server on older code answers 404. */
    list(signal?: AbortSignal): Promise<PartiesPage> {
      return http.get<PartiesPage>("/api/parties", undefined, { signal });
    },
    get(id: number, signal?: AbortSignal): Promise<PartyPage> {
      return http.get<PartyPage>(base(id), undefined, { signal });
    },
    /** A new party; it starts talking at once. The server's reason comes back as the ApiError's message. */
    start(request: NewPartyRequest): Promise<PartyReply> {
      return http.post<PartyReply>("/api/parties", request, { timeoutMs: 30_000 });
    },
    /** Your words join the talk ("say"), or a direction they all follow ("steer"). */
    say(id: number, text: string, kind: "say" | "steer"): Promise<PartyReply> {
      return http.post<PartyReply>(`${base(id)}/say`, { text, kind });
    },
    /** Pauses after the turn being said. */
    pause(id: number): Promise<PartyReply> {
      return http.post<PartyReply>(`${base(id)}/pause`);
    },
    /** Carries on: a paused one where it was, an ended one for more turns. */
    resume(id: number): Promise<PartyReply> {
      return http.post<PartyReply>(`${base(id)}/resume`);
    },
    stop(id: number): Promise<PartyReply> {
      return http.post<PartyReply>(`${base(id)}/stop`);
    },
    caps(id: number, caps: { maxTurns?: number; maxTokens?: number }): Promise<PartyReply> {
      return http.post<PartyReply>(`${base(id)}/caps`, caps);
    },
    delete(id: number): Promise<Reply> {
      return http.post<Reply>(`${base(id)}/delete`, {});
    },
  };
}
