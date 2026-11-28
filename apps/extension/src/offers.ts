// Pairing offers: a pairing link seen in a tab, for a server other than the one this profile is paired with, is
// offered for the owner to confirm, at most once per origin per browser session (a site can't raise one offer per
// path). Before the profile is paired the offer opens the consent page (what pairing sends, and Pair or Not now);
// once paired, it waits in the popup alone, so a page can't raise a prompt to switch servers.

import type { PairWith } from "./pairing";

/** A server's origin: the scheme, host and port the limit on offers counts by. */
export function originOf(server: string): string {
  try {
    return new URL(server).origin;
  } catch {
    return server;
  }
}

/** What to do with a pairing link seen in a tab. */
export type LinkAction = "pair" | "offer-popup" | "offer-page" | "ignore";

/**
 * A link on the server this profile is paired with pairs by itself; one on another server (or after an unpair) is offered: in the popup alone while this profile
 * is paired, on the consent page before it's paired. A server already offered in this session is ignored.
 */
export function linkAction(link: PairWith, saved: { server: string | null; token: string | null }, offered: readonly string[]): LinkAction {
  if (saved.token && saved.server === link.server) return "pair"; // after an unpair, it asks again
  if (offered.includes(originOf(link.server))) return "ignore";
  return saved.token ? "offer-popup" : "offer-page";
}
