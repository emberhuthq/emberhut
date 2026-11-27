import type { Instant } from "./common";
import type { OWNER } from "../wire";

export interface BrowserTab {
  id: number;
  url: string;
  title: string;
  active: boolean;
}

/** Emberhut's browser, on the server's machine. `control` is "agent" (Emberhut or a worker drives) or OWNER (the owner does). */
export interface BrowserInfo {
  connected: boolean;
  url: string;
  title: string;
  driver?: string | null;
  task?: number | null;
  control: "agent" | typeof OWNER;
  handoff?: string | null;
  at?: Instant | null;
  error?: string | null;
  /** The page asks for a file: the app's picker sends it (POST /api/browser/upload). */
  fileRequest?: { multiple: boolean; at: Instant } | null;
  tabs?: BrowserTab[] | null;
  /** Chrome's own controls on screen (normally only the page). */
  chromeUI?: boolean | null;
  /** The device that drives; "" means Emberhut handed it over and no device has taken it yet. */
  device?: string | null;
}

/** The live view: a web client to load with HTTP Basic credentials, view-only or in control. */
export interface BrowserSession {
  ok?: boolean;
  /** Null while the live view isn't up. */
  url: string | null;
  user: string;
  password: string;
  mode: "control" | "view";
  screen?: { width: number; height: number } | null;
}

/** POST /api/browser/input. Mouse, keys and paste go through the live view itself, not here. */
export type BrowserInput =
  | { kind: "takeover"; device: string }
  | { kind: "handback" }
  | { kind: "back" | "forward" | "reload" | "newtab" }
  | { kind: "go"; url: string }
  | { kind: "tab"; id: number }
  | { kind: "closetab"; id?: number }
  | { kind: "chrome"; show: boolean };

/** A Chrome profile with the Emberhut extension. */
export interface ChromeProfile {
  id: number;
  name: string;
  label: string;
  email: string | null;
  machine: string | null;
  os: string | null;
  version: string | null;
  browser: string | null;
  outdated: boolean;
  connected: boolean;
  seen: Instant | null;
  created: Instant;
  grant: { until: Instant | null; task: number | null; by: string } | null;
  /** `until` is epoch milliseconds. */
  paused: { why: string; until: number } | null;
  control: "agent" | typeof OWNER;
  handoff: string | null;
  task: number | null;
  tasks: number[];
}

export interface ChromePairing {
  code: string;
  link: string;
  label: string;
  name: string;
  device: number | null;
  until: Instant;
  paired: boolean;
}
