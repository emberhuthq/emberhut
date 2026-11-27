import { minTap } from "@/theme/tokens";

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
