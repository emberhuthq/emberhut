import { ApiError, reasonOf } from "../errors";
import type { EmberhutHttp } from "../http";
import type { EmberhutState } from "../types";
import { LOOKING_HEADER } from "../wire";

export interface StateRequest {
  room: number;
  /** The app is in front: the server holds this room's pushes for a few seconds. */
  looking: boolean;
  signal?: AbortSignal;
}

/**
 * GET /api/state with its ETag: an unchanged state comes back as 304 with nothing to download or parse, and the same
 * object as before is returned, so nothing re-renders. Tags are kept per room: one room's says nothing about another's.
 */
export function stateEndpoints(http: EmberhutHttp) {
  const held = new Map<number, { etag: string; state: EmberhutState }>();
  return {
    async get({ room, looking, signal }: StateRequest): Promise<EmberhutState> {
      const last = held.get(room);
      const headers: Record<string, string> = {};
      if (last) headers["If-None-Match"] = last.etag;
      if (looking) headers[LOOKING_HEADER] = String(room);
      const response = await http.send("/api/state", { query: { room }, headers, timeoutMs: 30_000, signal });
      if (response.status === 304 && last) return last.state;
      const body = await http.readBody(response);
      if (!response.ok || body === undefined) throw new ApiError(response.status, reasonOf(body) ?? `HTTP ${response.status}`, body);
      const state = body as EmberhutState;
      const etag = response.headers.get("ETag");
      if (etag) held.set(room, { etag, state });
      else held.delete(room);
      return state;
    },
    /** Forgets the tags, so the next answer is whole (after signing in again). */
    reset(): void {
      held.clear();
    },
  };
}
