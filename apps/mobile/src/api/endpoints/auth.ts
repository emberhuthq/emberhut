import type { EmberhutHttp } from "../http";
import type { AuthStatus } from "../types";

export function authEndpoints(http: EmberhutHttp) {
  return {
    /** Whether the server enforces sign-in, and whether this request came with a session. */
    status(): Promise<AuthStatus> {
      return http.get<AuthStatus>("/api/auth/status");
    },
  };
}
