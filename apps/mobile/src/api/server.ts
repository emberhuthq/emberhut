import type { SecureStorage } from "./auth";
import { isLocalHost } from "./localNetwork";

/**
 * The prefix of the secure-storage keys a server's pairing is kept under: the server's own, and the fallback server's
 * (the one the app goes back to when its server stops answering).
 */
export const slotPrefix = { server: "emberhut.", fallback: "emberhut.fallback." } as const;

/** Which of the two servers: this phone's server, or its fallback server. */
export type Slot = keyof typeof slotPrefix;

/** Where a server is: typed in at pairing, kept in secure storage, the base of every request to it. */
export interface ServerAddress {
  /** The address as "https://host:port" (no trailing slash), or null before one was given. */
  readonly url: string | null;
  /**
   * The server speaks this build's wire dialect (src/api/dialect.ts): its requests and answers are rewritten through it.
   * True unless turned off, so a server paired before servers had this flag keeps the build's behaviour. A build without
   * a dialect has nothing to rewrite either way.
   */
  readonly speaksDialect: boolean;
  load(): Promise<string | null>;
  save(url: string): Promise<void>;
  setSpeaksDialect(on: boolean): Promise<void>;
  /** Forgets the address (and the flag): a fallback server that was removed. */
  clear(): Promise<void>;
}

/** The address kept under `prefix` (slotPrefix): this phone's server by default. */
export function createServerAddress(storage: SecureStorage, prefix: string = slotPrefix.server): ServerAddress {
  const key = `${prefix}server`;
  // Only "off" is ever stored: no entry means the flag's default.
  const dialectKey = `${prefix}dialect`;
  let url: string | null = null;
  let speaksDialect = true;
  return {
    get url() {
      return url;
    },
    get speaksDialect() {
      return speaksDialect;
    },
    async load() {
      const [kept, dialect] = await Promise.all([storage.getItem(key), storage.getItem(dialectKey)]);
      url = serverUrlFrom(kept ?? "");
      speaksDialect = dialect !== "off";
      return url;
    },
    async save(next) {
      // Held only once kept: an address that couldn't be saved is never the one requests go to.
      await storage.setItem(key, next);
      url = next;
    },
    async setSpeaksDialect(on) {
      if (on) await storage.deleteItem(dialectKey);
      else await storage.setItem(dialectKey, "off");
      speaksDialect = on;
    },
    async clear() {
      await Promise.all([storage.deleteItem(key), storage.deleteItem(dialectKey)]);
      url = null;
      speaksDialect = true;
    },
  };
}

/** A typed address made whole, with whether it is plain http (shown as not encrypted); or why it can't be used. */
export type AddressCheck = { ok: true; url: string; plain: boolean } | { ok: false; error: string };

const notAnAddress = "That isn't a server address. It looks like https://my-server.example.com or my-server:18990.";

/**
 * A server address as typed ("emberhut.example.com", "https://Emberhut.Example.com/", an IP and port), made whole: no trailing
 * slash, and https:// when no scheme is typed. Plain http only reaches localhost or a private or CGNAT IP literal
 * (localNetwork.ts), where it is also the default; to a host name it is refused, since the pairing code and the tokens
 * would cross whatever network lies between in clear text.
 */
export function checkServerAddress(input: string): AddressCheck {
  const typed = input.trim();
  if (typed === "") return { ok: false, error: notAnAddress };
  const given = /^([a-z][a-z0-9+.-]*):\/\//i.exec(typed)?.[1]?.toLowerCase();
  const match = /^(?:[a-z][a-z0-9+.-]*:\/\/)?([a-z0-9.-]+|\[[0-9a-f:]+\])(:\d{1,5})?(\/[^?#\s]*)?$/i.exec(typed);
  if (!match || (given !== undefined && given !== "http" && given !== "https")) return { ok: false, error: notAnAddress };
  const [, rawHost, port = "", path = ""] = match;
  const host = rawHost!.toLowerCase();
  const local = isLocalHost(host);
  const scheme = given ?? (local ? "http" : "https");
  if (scheme === "http" && !local)
    return {
      ok: false,
      error: "Plain http only reaches this phone or a private network address. Use https:// for a server by name.",
    };
  return { ok: true, url: `${scheme}://${host}${port}${path.replace(/\/+$/, "")}`, plain: scheme === "http" };
}

/** The address made whole (checkServerAddress), or null when it can't be used. */
export function serverUrlFrom(input: string): string | null {
  const check = checkServerAddress(input);
  return check.ok ? check.url : null;
}
