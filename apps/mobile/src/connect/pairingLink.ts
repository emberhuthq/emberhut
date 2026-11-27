import { parseForm } from "./form";
import { machineIdError } from "./machineId";

// The pairing link (docs/api.md, "The pairing link"), read by the same rules as emberhut-identity's PairingLink:
// `https://emberhut.com/pair#id=<machine id>&code=<code>&route=<url>…`. Everything is in the fragment, which browsers never
// send to a server. The fragment is form-encoded; `id` and `code` appear once each, `route` 0 to 8 times in the order of
// the machine's route list and never empty, other keys are ignored, and the whole link is at most 2,000 characters so
// its QR code stays scannable.

/** A pairing link: the machine's id, a one-time code, and routes to try in order. */
export interface PairingLink {
  /** The machine's id: the key in the pairing answer must have it. */
  id: string;
  /** The one-time code, as shown (ABCD-EFGH); the server reads it like a typed code. */
  code: string;
  routes: string[];
}

/** Why text isn't a pairing link: the vectors' error names. */
export type PairingLinkError =
  | { kind: "notAPairingLink" }
  | { kind: "tooLong" }
  | { kind: "missing"; key: "id" | "code" }
  | { kind: "repeated"; key: "id" | "code" }
  | { kind: "tooManyRoutes" }
  | { kind: "emptyRoute" }
  | { kind: "id" };

export type ParsedLink = { ok: true; link: PairingLink } | { ok: false; error: PairingLinkError };

/** Where every pairing link points, before its fragment. */
export const pairingLinkBase = "https://emberhut.com/pair";
const maxRoutes = 8;
const maxLength = 2_000;

/** Reads a whole link, `https://emberhut.com/pair#…`. */
export function parsePairingLink(text: string): ParsedLink {
  // In characters (code points), as the Rust reader counts them.
  if ([...text].length > maxLength) return refuse({ kind: "tooLong" });
  const start = `${pairingLinkBase}#`;
  if (!text.startsWith(start)) return refuse({ kind: "notAPairingLink" });
  let id: string | undefined;
  let code: string | undefined;
  const routes: string[] = [];
  for (const [key, value] of parseForm(text.slice(start.length))) {
    if (key === "id") {
      if (id !== undefined) return refuse({ kind: "repeated", key: "id" });
      id = value;
    } else if (key === "code") {
      if (code !== undefined) return refuse({ kind: "repeated", key: "code" });
      code = value;
    } else if (key === "route") {
      routes.push(value);
    }
  }
  if (id === undefined || id === "") return refuse({ kind: "missing", key: "id" });
  if (machineIdError(id) !== null) return refuse({ kind: "id" });
  if (code === undefined || code === "") return refuse({ kind: "missing", key: "code" });
  if (routes.length > maxRoutes) return refuse({ kind: "tooManyRoutes" });
  if (routes.some((route) => route === "")) return refuse({ kind: "emptyRoute" });
  return { ok: true, link: { id, code, routes } };
}

const refuse = (error: PairingLinkError): ParsedLink => ({ ok: false, error });

/** A refusal in words, for the pairing screen. */
export function linkErrorText(error: PairingLinkError): string {
  switch (error.kind) {
    case "notAPairingLink":
      return "That isn't an Emberhut pairing link. One starts with https://emberhut.com/pair#.";
    case "tooLong":
      return "That pairing link is too long to be one Emberhut made.";
    case "missing":
      return error.key === "id" ? "That pairing link doesn't say which machine it is for." : "That pairing link has no code.";
    case "repeated":
      return `That pairing link has more than one ${error.key === "id" ? "machine" : "code"}.`;
    case "tooManyRoutes":
    case "emptyRoute":
      return "That pairing link's addresses can't be read.";
    case "id":
      return "That pairing link's machine id can't be read.";
  }
}
