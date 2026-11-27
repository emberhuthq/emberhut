import { readJson, timeoutSignal, type Fetch } from "@/api/fetch";

// The account directory's client (docs/directory.md, version 1): the account's machines, and unlinking one. Every call
// carries a Clerk session token made with the `emberhut-directory` template. The directory finds machines; it never lets
// the phone into one: that still takes a pairing code from the machine.

/** One of the account's machines, as `GET /v1/machines` lists it. */
export interface DirectoryMachine {
  id: string;
  publicKey: string;
  name: string;
  relayUrl: string;
  /** The machine's uplink to the relay is up now. */
  connected: boolean;
  /** When it last was, in milliseconds since the epoch; null if never. */
  lastSeen: number | null;
  linkedAt: number;
}

/**
 * Why a directory call failed, as the phone acts on it:
 * - signIn: the account's sign-in is over (401 `sign-in`, or no token to send): sign out of the account;
 * - later: the directory is busy or can't check tokens right now (429, 503): try again later, never sign out;
 * - refused: any other refusal, with the directory's sentence;
 * - unreachable: no answer at all.
 */
export type DirectoryErrorKind = "signIn" | "later" | "refused" | "unreachable";

export class DirectoryError extends Error {
  constructor(
    readonly kind: DirectoryErrorKind,
    message: string,
    /** The directory's `code`, when it gave one. */
    readonly code?: string,
    /** Seconds to wait, from `Retry-After`. */
    readonly retryAfter?: number,
  ) {
    super(message);
    this.name = "DirectoryError";
  }
}

export interface DirectoryOptions {
  /** The directory's origin, `https://<relay domain>` (src/config/account.ts). */
  baseUrl: string;
  fetch: Fetch;
  /** A fresh Clerk session token for the directory (`getToken({ template: "emberhut-directory" })`); null when signed out. */
  token: () => Promise<string | null>;
  timeoutMs?: number;
}

export function createDirectory({ baseUrl, fetch, token, timeoutMs = 30_000 }: DirectoryOptions) {
  async function call(method: "GET" | "DELETE", path: string): Promise<{ status: number; body: unknown }> {
    let bearer: string | null;
    try {
      bearer = await token();
    } catch {
      // Clerk couldn't make a token (offline, or its own API busy): not a reason to sign out.
      throw new DirectoryError("later", "Couldn't reach the account. Try again in a moment.");
    }
    if (bearer === null) throw new DirectoryError("signIn", "Sign in to your account again.");
    const limit = timeoutSignal(timeoutMs);
    let response: Response;
    try {
      response = await fetch(`${baseUrl}${path}`, {
        method,
        headers: { Authorization: `Bearer ${bearer}`, Accept: "application/json" },
        signal: limit.signal,
      });
    } catch {
      throw new DirectoryError("unreachable", "Can't reach your account's directory. Check this phone's connection.");
    } finally {
      limit.done();
    }
    const body = await readJson(response).catch(() => undefined);
    if (response.ok) return { status: response.status, body };
    throw refusal(response, body);
  }

  return {
    /** The account's machines. */
    async machines(): Promise<DirectoryMachine[]> {
      const { body } = await call("GET", "/v1/machines");
      const list = (body as { machines?: unknown } | undefined)?.machines;
      if (!Array.isArray(list)) throw new DirectoryError("refused", "The directory's answer couldn't be read.");
      return list.flatMap((entry) => {
        const machine = machineFrom(entry);
        return machine ? [machine] : [];
      });
    },
    /** Unlinks a machine from the account: it can't use the account's relay any more. Its devices stay paired with it. */
    async unlink(id: string): Promise<void> {
      await call("DELETE", `/v1/machines/${encodeURIComponent(id)}`);
    },
  };
}

export type Directory = ReturnType<typeof createDirectory>;

/**
 * A refusal (`{ error, code }`) as the phone acts on it, never by its sentence: only `code: "sign-in"` signs the account
 * out (a 401 without it, from something in front of the directory, is a plain refusal); 429 and 503 are "later", whatever
 * sent them, since waiting is always safe.
 */
function refusal(response: Response, body: unknown): DirectoryError {
  const { error, code } = (typeof body === "object" && body !== null ? body : {}) as { error?: unknown; code?: unknown };
  const sentence = typeof error === "string" && error !== "" ? error : undefined;
  const named = typeof code === "string" ? code : undefined;
  if (named === "sign-in") return new DirectoryError("signIn", sentence ?? "Sign in to your account again.", named);
  if (response.status === 429 || response.status === 503) {
    const wait = retryAfter(response.headers.get("Retry-After"));
    return new DirectoryError(
      "later",
      wait === undefined ? "The account is busy. Try again in a moment." : `The account is busy. Try again in ${waitText(wait)}.`,
      named,
      wait,
    );
  }
  return new DirectoryError("refused", sentence ?? `The directory said no (HTTP ${response.status}).`, named);
}

/** `Retry-After` in seconds: the directory sends a number of seconds; a date or anything else is ignored. */
function retryAfter(header: string | null): number | undefined {
  if (header === null || !/^\d+$/.test(header.trim())) return undefined;
  return Number(header.trim());
}

const waitText = (seconds: number): string => (seconds < 90 ? `${Math.max(seconds, 1)} s` : `${Math.round(seconds / 60)} min`);

/** One listed machine, or null when an entry lacks what the phone shows (readers ignore fields they don't know). */
function machineFrom(entry: unknown): DirectoryMachine | null {
  if (typeof entry !== "object" || entry === null) return null;
  const m = entry as Record<string, unknown>;
  if (typeof m.id !== "string" || typeof m.name !== "string" || typeof m.publicKey !== "string") return null;
  return {
    id: m.id,
    publicKey: m.publicKey,
    name: m.name,
    relayUrl: typeof m.relayUrl === "string" ? m.relayUrl : "",
    connected: m.connected === true,
    lastSeen: typeof m.lastSeen === "number" ? m.lastSeen : null,
    linkedAt: typeof m.linkedAt === "number" ? m.linkedAt : 0,
  };
}
