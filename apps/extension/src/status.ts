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
}

/** What the popup asks the service worker. */
export type PopupAsk =
  | { kind: "status" }
  | { kind: "pair"; code: string; server: string }
  | { kind: "accept" }
  | { kind: "dismiss" }
  | { kind: "pause" }
  | { kind: "share" }
  | { kind: "handback" }
  | { kind: "settings"; blocked: string[] }
  | { kind: "unpair" };
