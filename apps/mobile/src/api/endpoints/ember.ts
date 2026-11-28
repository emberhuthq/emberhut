import type { EmberhutHttp } from "../http";
import type { EmberName, EmberNameReply } from "../types";

export function emberEndpoints(http: EmberhutHttp) {
  return {
    /** GET /api/ember/name: what you call your ember, or "Your ember" (`custom: false`). */
    name(signal?: AbortSignal): Promise<EmberName> {
      return http.get<EmberName>("/api/ember/name", undefined, { signal });
    },
    /** POST /api/ember/name: renames it (null or blank: back to "Your ember"); a name it doesn't take is a 400. */
    rename(name: string | null): Promise<EmberNameReply> {
      return http.post<EmberNameReply>("/api/ember/name", { name });
    },
  };
}
