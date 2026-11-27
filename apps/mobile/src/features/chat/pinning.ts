import type { Pending } from "@/state/talk";

/**
 * How far above the newest message the list may sit and still count as at it: there, what arrives keeps the newest in
 * view; further up you're reading, and nothing moves the page.
 */
export const NEAR_NEWEST = 120;

/**
 * The scroll offset that shows the newest message in the inverted list, given the keyboard's share of the list's end
 * (the inset KeyboardChatScrollView reports). On iOS that share is a content inset at the list's end, so the newest
 * sits before offset 0, at minus the inset. On Android the keyboard controller moves the content by it instead and
 * scrolls it back, so once the keyboard is still the newest is at offset 0 whether the keyboard is up or not.
 */
export function newestOffset(os: string, keyboardInset: number): number {
  return os === "ios" && keyboardInset !== 0 ? -keyboardInset : 0;
}

/** How far the list sits from its newest message, from the offset its scroll events report. */
export function fromNewest(os: string, offsetY: number, keyboardInset: number): number {
  return offsetY - newestOffset(os, keyboardInset);
}

/** That far from the newest, you scrolled up to read: what arrives shows as a pill instead of moving the page. */
export function isAway(fromNewestPx: number): boolean {
  return fromNewestPx > NEAR_NEWEST;
}

/**
 * A message you just sent (or sent again): a fresh pending one, not yet answered or failed. The talk store makes a new
 * one per send; what happens to it later (a note, a failure) comes on a copy that carries it.
 */
export function isNewSend(before: Pending | undefined, now: Pending | undefined): boolean {
  return now !== undefined && now !== before && now.note === null && !now.failed;
}
