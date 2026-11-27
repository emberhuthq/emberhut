import { encodeBase64Url } from "@/connect/encoding";
import { checkProbeAnswer, linkRouteKind, probeBinds, readProbeAnswer } from "@/connect/probe";
import { buildUrl, readJson, timeoutSignal, type Fetch } from "./fetch";

/**
 * What a pairing link's route showed before its code is sent there:
 * - proved: the machine answered the probe, signed with the link's machine key, from where the route says it is;
 * - answered: an Emberhut server older than the probe answered its sign-in status in Emberhut's exact shape (the pairing answer's
 *   key is then the only proof, checked before anything is kept);
 * - none: no answer, or one from something else, or from another machine. The code is never sent there.
 */
export type RouteAnswer = "proved" | "answered" | "none";

/** Asks a route whether the machine `machineId` is there. */
export type Reach = (url: string, machineId: string) => Promise<RouteAnswer>;

/** How long a route gets to answer, as connect.md's race gives each one. */
export const reachTimeoutMs = 2_500;

/**
 * Probes a route (`GET /api/connect/probe`, docs/api.md) with a fresh nonce, as the kind its URL gives (linkRouteKind).
 * Only a server that answers the probe with 404 or 405 (one that predates it) is asked `GET /api/auth/status` instead,
 * and counted only for a 200 of exactly that answer's shape: a 401, a captive portal's page or any other answer is not
 * Emberhut. Once servers serve the probe, a route that doesn't pass it is never used.
 */
export function createReach(fetch: Fetch, random: (size: number) => Uint8Array, timeoutMs = reachTimeoutMs): Reach {
  return async (url, machineId) => {
    const kind = linkRouteKind(url);
    if (kind === null) return "none";
    const limit = timeoutSignal(timeoutMs);
    try {
      const nonce = random(32);
      const probe = await fetch(buildUrl(url, "/api/connect/probe", { nonce: encodeBase64Url(nonce), kind }), {
        method: "GET",
        signal: limit.signal,
      });
      if (probe.status === 404 || probe.status === 405) return (await answersStatus(fetch, url, limit.signal)) ? "answered" : "none";
      if (probe.status !== 200) return "none";
      const read = readProbeAnswer(await readJson(probe));
      if (!read.ok) return "none";
      const checked = checkProbeAnswer(read.answer, { nonce, kind, expected: machineId });
      return checked.ok && probeBinds(read.answer, kind, { kind, url }) ? "proved" : "none";
    } catch {
      return "none";
    } finally {
      limit.done();
    }
  };
}

/** `GET /api/auth/status` answered 200 with Emberhut's `AuthStatus`, field for field. */
async function answersStatus(fetch: Fetch, url: string, signal: AbortSignal): Promise<boolean> {
  const response = await fetch(buildUrl(url, "/api/auth/status"), { method: "GET", signal });
  if (response.status !== 200) return false;
  const body = (await readJson(response)) as Record<string, unknown> | undefined;
  return (
    typeof body === "object" &&
    body !== null &&
    (body.mode === "report" || body.mode === "enforce") &&
    typeof body.bound === "boolean" &&
    typeof body.signedIn === "boolean" &&
    Array.isArray(body.devices)
  );
}

/**
 * The route to pair over: every one is asked at once, and the first in the link's order (the machine's) that proved the
 * machine wins, then the first that answered as an older Emberhut. Null when none did.
 */
export async function firstReachable(routes: readonly string[], machineId: string, reach: Reach): Promise<string | null> {
  const answers = await Promise.all(routes.map((route) => reach(route, machineId)));
  return routes[answers.indexOf("proved")] ?? routes[answers.indexOf("answered")] ?? null;
}
