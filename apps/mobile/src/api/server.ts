import type { SecureStorage } from "./auth";
import { isLocalHost } from "./localNetwork";

const key = "emberhut.server";

/** Where this phone's server is: typed in at pairing, kept in secure storage, the base of every request. */
export interface ServerAddress {
  /** The address as "https://host:port" (no trailing slash), or null before one was given. */
  readonly url: string | null;
  load(): Promise<string | null>;
  save(url: string): Promise<void>;
}

export function createServerAddress(storage: SecureStorage): ServerAddress {
  let url: string | null = null;
  return {
    get url() {
      return url;
    },
    async load() {
      url = serverUrlFrom((await storage.getItem(key)) ?? "");
      return url;
    },
    async save(next) {
      // Held only once kept: an address that couldn't be saved is never the one requests go to.
      await storage.setItem(key, next);
      url = next;
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
