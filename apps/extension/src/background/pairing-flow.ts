// Pairing from the link: opened in this profile, a link on the server this profile is paired with pairs by itself
// (pairing again, say after a repair in the app). A link on any other server is only offered (offers.ts): the owner
// confirms that server's address, since any page could show a pairing link, and the server a profile pairs with is
// the one that drives it. Never from a tab Emberhut drives, opened or adopted.

import { inBackground, messageOf } from "../log";
import { codeOfOffer, linkAction, offerId, originOf } from "../offers";
import { linkOf, type PairWith } from "../pairing";
import type { Offer } from "../status";
import { badge } from "./badge";
import { adopted, made, reported } from "./held";
import { pairWith } from "./link";
import { session, setSession, settings, step } from "./state";

const tried = new Set<string>();

/** The origins offered in this browser session. */
async function offered(): Promise<string[]> {
  const got = await chrome.storage.session.get("offered");
  return Array.isArray(got.offered) ? got.offered.filter((s): s is string => typeof s === "string") : [];
}

/** Pairs, and says how it went in the popup's steps. */
async function pairNow(pair: PairWith): Promise<void> {
  try {
    await pairWith(pair);
    await step(`Paired with ${pair.server}`);
  } catch (e) {
    await step(`Pairing with ${pair.server} didn't work: ${messageOf(e)}`);
    throw e;
  } finally {
    await badge();
  }
}

/** A tab showing a pairing link: paired by itself on the known server, else offered. */
export async function pairFromTab(tab: chrome.tabs.Tab): Promise<void> {
  if (tab.id === undefined || reported.has(tab.id) || adopted.has(tab.id) || made.has(tab.id)) return;
  const pair = linkOf(tab.url || tab.pendingUrl || "");
  if (!pair || tried.has(pair.code)) return;
  tried.add(pair.code);
  const saved = await settings();
  const action = linkAction(pair, saved, await offered());
  if (action === "ignore") return;
  if (action === "pair") {
    await pairNow(pair).catch(() => undefined);
    return;
  }
  await chrome.storage.session.set({ offered: [...(await offered()), originOf(pair.server)] });
  const { offer } = await session();
  if (offer) await chrome.notifications.clear(offerId(offer));
  await setSession({ offer: pair });
  if (action !== "offer-notification") return;
  await chrome.notifications.create(offerId(pair), {
    type: "basic",
    iconUrl: "icons/emberhut-128.png",
    title: "Pair this Chrome profile with Emberhut?",
    message: `With the Emberhut server at ${pair.server}. Pair only if that is your own server.`,
    buttons: [{ title: "Pair" }, { title: "Not now" }],
    priority: 2,
    requireInteraction: true,
  });
}

/** Every open tab checked for a pairing link (the extension was just added from the pairing page, say). */
export async function pairTabs(): Promise<void> {
  for (const tab of await chrome.tabs.query({})) await pairFromTab(tab);
}

/** The owner confirmed the offered pairing (the one with `code`, when a notification names it). */
export async function acceptOffer(code?: string): Promise<void> {
  const { offer } = await session();
  if (!offer || (code !== undefined && offer.code !== code)) throw new Error("That pairing offer is gone");
  await dismissOffer();
  await pairNow(offer);
}

/** The offer withdrawn. */
export async function dismissOffer(): Promise<void> {
  const { offer } = await session();
  await setSession({ offer: null });
  if (offer) await chrome.notifications.clear(offerId(offer));
}

/** An offer's notification's buttons: Pair, or Not now; for that offer only. */
export function onOfferButton(id: string, button: number): void {
  const code = codeOfOffer(id);
  if (code === null) return;
  inBackground(button === 0 ? acceptOffer(code) : dismissOffer(), "pairing offer");
}

/** The offer, for the popup. */
export async function currentOffer(): Promise<Offer | null> {
  return (await session()).offer;
}
