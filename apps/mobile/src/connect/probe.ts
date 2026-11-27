import { decodeBase64Url } from "./encoding";
import { isNormalHost, isNormalSocketAddress, routeAddress } from "./hosts";
import { machineIdOf, statement } from "./machineId";
import { isPublicKey, verifyStrict } from "./signature";

// The probe (docs/api.md, "GET /api/connect/probe"): a route answers only when the machine signs this probe's nonce and
// kind with its key, from where the route says it is. The phone uses it before sending a pairing link's code anywhere.

/** `ProbeAnswer`, as read. */
export interface ProbeAnswer {
  id: string;
  publicKey: string;
  via: "direct" | "relayed";
  local: string;
  host: string;
  signature: string;
}

/** What refused an answer before its signature is checked: the vectors' `refusedBy`. */
export type ProbeDecodeError = "publicKey" | "wrongKey" | "via" | "reached" | "host" | "local" | "signature" | "shape";

export type ReadProbe = { ok: true; answer: ProbeAnswer } | { ok: false; refusedBy: ProbeDecodeError };

/**
 * Reads a probe's answer: its key is a valid point with its id, `via` is known, `local` is empty exactly when relayed,
 * and `host` and `local` are in normal form.
 */
export function readProbeAnswer(body: unknown): ReadProbe {
  if (typeof body !== "object" || body === null) return { ok: false, refusedBy: "shape" };
  const { id, publicKey, via, local, host, signature } = body as Record<string, unknown>;
  if ([id, publicKey, via, local, host, signature].some((field) => typeof field !== "string")) return { ok: false, refusedBy: "shape" };
  const key = decodeBase64Url(publicKey as string, 32);
  if (!key.ok || !isPublicKey(key.bytes)) return { ok: false, refusedBy: "publicKey" };
  if (machineIdOf(key.bytes) !== id) return { ok: false, refusedBy: "wrongKey" };
  if (via !== "direct" && via !== "relayed") return { ok: false, refusedBy: "via" };
  if ((via === "relayed") !== (local === "")) return { ok: false, refusedBy: "reached" };
  if (!isNormalHost(host as string)) return { ok: false, refusedBy: "host" };
  if (local !== "" && !isNormalSocketAddress(local as string)) return { ok: false, refusedBy: "local" };
  if (!decodeBase64Url(signature as string, 64).ok) return { ok: false, refusedBy: "signature" };
  return {
    ok: true,
    answer: {
      id: id as string,
      publicKey: publicKey as string,
      via,
      local: local as string,
      host: host as string,
      signature: signature as string,
    },
  };
}

/** Why a read answer isn't the expected machine's answer to this probe. */
export type ProbeCheck = { ok: true } | { ok: false; error: "otherMachine" | "signature" };

/** Checks a read answer against the probe sent: the machine expected, and its signature over this nonce and kind. */
export function checkProbeAnswer(answer: ProbeAnswer, probe: { nonce: Uint8Array; kind: string; expected: string }): ProbeCheck {
  if (answer.id !== probe.expected) return { ok: false, error: "otherMachine" };
  const key = decodeBase64Url(answer.publicKey, 32);
  const signature = decodeBase64Url(answer.signature, 64);
  if (!key.ok || !signature.ok) return { ok: false, error: "signature" };
  const text = new TextEncoder();
  const signed = statement("emberhut probe v1", [
    probe.nonce,
    text.encode(answer.id),
    text.encode(probe.kind),
    text.encode(answer.via),
    text.encode(answer.local),
    text.encode(answer.host),
  ]);
  return verifyStrict(key.bytes, signed, signature.bytes) ? { ok: true } : { ok: false, error: "signature" };
}

/**
 * Whether an answer to a probe of `kind` binds to the route it was sent on: an address route (`loopback`, `lan`, a `vpn`
 * address) was reached directly at its address and port; a name route (`public`, a VPN's name) is https and was reached
 * directly for its name; a relay route is https and relayed for its name. Kinds this version doesn't know never match,
 * and `ssh` is checked against its forward, which the phone doesn't make.
 */
export function probeBinds(answer: ProbeAnswer, kind: string, route: { kind: string; url: string }): boolean {
  if (kind !== route.kind) return false;
  const address = routeAddress(route.url);
  if (address === null) return false;
  const byAddress = () => address.isAddress && answer.via === "direct" && answer.local === address.socket;
  const byName = (via: ProbeAnswer["via"]) => address.scheme === "https" && answer.via === via && answer.host === address.host;
  switch (route.kind) {
    case "loopback":
    case "lan":
      return byAddress();
    case "vpn":
      return address.isAddress ? byAddress() : byName("direct");
    case "public":
      return byName("direct");
    case "relay":
      return byName("relayed");
    default:
      return false;
  }
}

/**
 * The kind a pairing link's route is probed as: links carry URLs without kinds. An address is an address route
 * (`loopback` for this phone's own, `lan` otherwise, which binds as a VPN address does); a name is a name route
 * (`public`, which binds as a VPN's name does).
 */
export function linkRouteKind(url: string): "loopback" | "lan" | "public" | null {
  const address = routeAddress(url);
  if (address === null) return null;
  if (!address.isAddress) return "public";
  return address.socket?.startsWith("127.") || address.socket?.startsWith("[::1]") ? "loopback" : "lan";
}
