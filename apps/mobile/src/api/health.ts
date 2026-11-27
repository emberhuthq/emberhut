import { buildUrl, timeoutSignal, type Fetch } from "./fetch";

/** How long the server gets to answer a health check. */
export const healthTimeoutMs = 4_000;

/** Whether a server answers: `GET <server>/health`, no session, a 2xx within the limit. */
export type HealthCheck = (url: string) => Promise<boolean>;

export function createHealthCheck(fetch: Fetch, timeoutMs = healthTimeoutMs): HealthCheck {
  return async (url) => {
    const limit = timeoutSignal(timeoutMs);
    try {
      const response = await fetch(buildUrl(url, "/health"), { method: "GET", signal: limit.signal });
      // The body says nothing more; read so the connection is let go.
      await response.text().catch(() => undefined);
      return response.ok;
    } catch {
      return false;
    } finally {
      limit.done();
    }
  };
}
