// Pairing offers: a pairing link seen in a tab, for a server other than the one this profile is paired with, is
// offered for the owner to confirm, at most once per origin per browser session (a site can't raise one offer per
// path); the offer's notification names its
// code, so a click answers that offer and no later one.

import type { PairWith } from "./pairing";

/** The offer's notification id prefix; the code follows it. */
export const OFFER_PREFIX = "pair-offer:";

/** A server's origin: the scheme, host and port the limit on offers counts by. */
export function originOf(server: string): string {
  try {
    return new URL(server).origin;
  } catch {
    return server;
  }
}

/** What to do with a pairing link seen in a tab. */
export type LinkAction = "pair" | "offer-popup" | "offer-notification" | "ignore";

/**
 * A link on the paired server pairs by itself; one on another server is offered: in the popup alone while this profile
 * is paired (a page can't raise a notification to switch servers), with a notification too before it's paired. A
 * server already offered in this session is ignored.
 */
export function linkAction(link: PairWith, saved: { server: string | null; token: string | null }, offered: readonly string[]): LinkAction {
  if (saved.server === link.server) return "pair";
  if (offered.includes(originOf(link.server))) return "ignore";
  return saved.token ? "offer-popup" : "offer-notification";
}

/** The notification id for an offer. */
export function offerId(link: PairWith): string {
  return `${OFFER_PREFIX}${link.code}`;
}

/** The code a notification id answers for, or null when it isn't an offer's. */
export function codeOfOffer(id: string): string | null {
  return id.startsWith(OFFER_PREFIX) ? id.slice(OFFER_PREFIX.length) : null;
}
