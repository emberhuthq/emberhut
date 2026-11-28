// What the service worker tells the popup, and what the popup asks of it.

import type { Pause } from "./pause";

/** A step in the popup's "Recent" list. */
export interface Step {
  at: number;
  text: string;
}

/** A pairing link seen in a tab, on a server this profile isn't paired with: the owner confirms it in the popup. */
export interface Offer {
  server: string;
  code: string;
}

/** One of Emberhut's tabs, as the popup lists it. */
export interface TabLine {
  title: string;
  url: string;
}

/** The popup's view of the extension. */
export interface Status {
  paired: boolean;
  name: string | null;
  label: string | null;
  server: string | null;
  version: string;
  connected: boolean;
  error: string;
  paused: Pause | null;
  driving: boolean;
  handoff: string | null;
  notice: string | null;
  steps: Step[];
  blocked: string[];
  defaults: readonly string[];
  tabs: TabLine[];
  offer: Offer | null;
  /** Chrome notifications are on (an optional permission the owner grants in the popup). */
  notifications: boolean;
}

/** What the popup asks the service worker. */
export type PopupAsk =
  | { kind: "status" }
  | { kind: "pair"; code: string; server: string }
  | { kind: "accept" }
  | { kind: "review" }
  | { kind: "dismiss" }
  | { kind: "pause" }
  | { kind: "share" }
  | { kind: "handback" }
  | { kind: "settings"; blocked: string[] }
  | { kind: "unpair" };

/** The service worker's answer when it couldn't do what was asked; a `Status` otherwise. */
export interface Refused {
  refused: string;
}

/** What the worker answers a popup or page: the status, or why it couldn't. */
export type Answer = Status | Refused;

/** Why the worker refused, or null when `answer` is the status (whose own `error` is the link's, not a refusal). */
export function refusal(answer: Answer): string | null {
  return "refused" in answer ? answer.refused : null;
}
