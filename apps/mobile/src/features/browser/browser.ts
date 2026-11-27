import { OWNER, taskKey, type BrowserInfo, type BrowserTab } from "@/api";
import { isLocalHost } from "@/api/localNetwork";

// Emberhut's browser as the server reports it, read for this device: who drives, what's on screen, what Emberhut asks of you.

/** You drive it from this device (or Emberhut gave it to you and no device took it yet). */
export const isMine = (info: BrowserInfo, device: string | null): boolean =>
  info.control === OWNER && (info.device === null || info.device === undefined || info.device === device);

/** Emberhut handed it over and none of your devices has taken it yet: opening it here takes it. */
export const isUnclaimed = (info: BrowserInfo): boolean => info.control === OWNER && info.device === "";

/** Who has it, in a few words over the page. */
export function statusText(info: BrowserInfo, device: string | null): string {
  if (isUnclaimed(info)) return "Emberhut asked you to take over";
  if (isMine(info, device)) return "You're driving";
  if (info.control === OWNER) return "Driving on your other device";
  if (!info.driver) return "Watching";
  return `${info.driver === "Emberhut" ? "Emberhut is driving" : `${info.driver} is driving`}${info.task ? ` · ${taskKey(info.task)}` : ""}`;
}

/** The chat's line while the browser waits on you; null when it doesn't. */
export function handoffLine(info: BrowserInfo | null | undefined, device: string | null): string | null {
  if (!info || info.control !== OWNER) return null;
  if (!isUnclaimed(info) && !(isMine(info, device) && info.handoff)) return null;
  return info.handoff ? `Emberhut handed you the browser: ${info.handoff}` : "Emberhut handed you the browser";
}

export const activeTab = (info: BrowserInfo): BrowserTab | undefined => info.tabs?.find((t) => t.active);

/** The tab on screen as the address bar shows it: its site, "New tab", or the title of one of Chrome's own pages. */
export function site(info: BrowserInfo): string {
  const tab = activeTab(info);
  const url = tab?.url ?? info.url;
  if (url === "" || url === "about:blank") return "New tab";
  const m = /^https?:\/\/([^/?#:]+)/.exec(url);
  if (m) return m[1]!.replace(/^www\./, "");
  return tab?.title || url;
}

export const isSecure = (info: BrowserInfo): boolean => (activeTab(info)?.url ?? info.url).startsWith("https://");

/**
 * Why the live view's page didn't open. On iOS, plain http reaches only localhost and private or CGNAT IPs (App
 * Transport Security lets nothing else through), so an http address by name is told as that. Android allows cleartext,
 * so there, as for every other failure, it's the page's own reason, in the system's words or by its HTTP status.
 */
export function liveViewFailure(url: string, why: string | number | undefined, os: string): string {
  const m = /^(https?):\/\/(\[[^\]]+\]|[^/?#:]+)/i.exec(url);
  const host = m?.[2] ?? url;
  if (os === "ios" && m?.[1]?.toLowerCase() === "http" && !isLocalHost(host))
    return `The live view's address (${host}) can't be opened: it needs https, or a private IP over http.`;
  const detail = typeof why === "number" ? `HTTP ${why}` : why?.trim();
  return `The live view (${host}) didn't load${detail ? `: ${detail}` : "."}`;
}

/** The whole address, for editing it. */
export function address(info: BrowserInfo): string {
  const url = activeTab(info)?.url ?? info.url;
  return url === "about:blank" ? "" : url;
}
