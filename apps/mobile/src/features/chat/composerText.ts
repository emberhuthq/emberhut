import type { SetStateAction } from "react";
import { minTap } from "@/theme/tokens";

/** Each room's draft after a change to one room's: the others stay as they were, and an emptied one is dropped. */
export function setDraft(drafts: Readonly<Record<number, string>>, room: number, change: SetStateAction<string>): Record<number, string> {
  const next = typeof change === "function" ? change(drafts[room] ?? "") : change;
  const { [room]: _was, ...others } = drafts;
  return next === "" ? others : { ...others, [room]: next };
}

/** Dictated words after what's typed, a space between. */
export function appendSaid(typed: string, said: string): string {
  return typed === "" || /\s$/.test(typed) ? typed + said : `${typed} ${said}`;
}

/** What the send button does: send, stop Emberhut's answer (nothing typed while it works), or nothing yet. */
export function sendAction(input: { text: string; files: number; sending: boolean; working: boolean }): "send" | "stop" | "wait" | "none" {
  if (input.sending) return "wait";
  if (input.text.trim() !== "" || input.files > 0) return "send";
  return input.working ? "stop" : "none";
}

/** The hit slop on each side that gives a button of `size` points the app's minimum touch area (minTap). */
export const slopFor = (size: number): number => Math.max(0, (minTap - size) / 2);
