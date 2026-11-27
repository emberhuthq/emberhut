// The popup's words for a status: pure, so the tests can read them.

import { clock } from "../pause";
import type { Status } from "../status";

/** The line under the title. */
export function headline(s: Status): string {
  if (!s.paired) return "Not paired yet";
  const name = s.label || s.name || "this profile";
  return `Paired as “${name}” · ${s.connected ? "connected" : "not connected"} · v${s.version}`;
}

/** What Emberhut is doing here, while not paused. */
export function driving(s: Status): string {
  return s.driving ? "Emberhut is driving in this Chrome now." : "Emberhut may work in this Chrome whenever it needs to.";
}

/** When the pause ends, as a time of day. */
export function pauseEnds(s: Status): string {
  return s.paused ? clock(s.paused.until) : "";
}

/** The built-in blocked sites, said once. */
export function builtIn(s: Status): string {
  return `Always blocked: ${s.defaults.join(", ")}, and chrome:// pages.`;
}
