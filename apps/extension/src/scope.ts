// Which sites and tabs are Emberhut's to touch: decided here, on the owner's machine, not by the server.

/** The tab group Emberhut works in, in every window. */
export const GROUP_TITLE = "Emberhut";
/** Its colour in Chrome's palette: blue, the apps' accent (apps/mobile/src/theme/tokens.ts). */
export const GROUP_COLOR = "blue";

/**
 * Never Emberhut's, whatever the server asks: password managers' own pages, account security pages, the Web Store. The
 * owner adds more in the popup.
 */
export const BUILT_IN_BLOCKED: readonly string[] = [
  "1password.com",
  "1password.ca",
  "1password.eu",
  "passwords.google.com",
  "myaccount.google.com/security",
  "myaccount.google.com/signinoptions",
  "account.apple.com",
  "appleid.apple.com",
  "chromewebstore.google.com",
  "chrome.google.com/webstore",
];

/** A host as compared: lowercase, without the final dots a fully qualified name may carry (`1password.com.`). */
function hostKey(host: string): string {
  return host.toLowerCase().replace(/\.+$/, "");
}

/**
 * A path as compared: percent-escapes decoded (where they decode), runs of slashes made one, lowercase, so
 * `//Security` and `/%73ecurity` are `/security`.
 */
function pathKey(path: string): string {
  let decoded = path;
  try {
    decoded = decodeURIComponent(path);
  } catch {
    // An escape that doesn't decode stays as it is.
  }
  return decoded.replace(/\/{2,}/g, "/").toLowerCase();
}

/**
 * Whether `url` is out of bounds: a blocked site (a host and its subdomains, from a path on when the entry has one),
 * or anything but a web page (chrome://, other extensions, files). An empty address and about:blank are not.
 */
export function isBlocked(url: string | undefined, mine: readonly string[]): boolean {
  if (!url || url === "about:blank") return false;
  let u: URL;
  try {
    u = new URL(url);
  } catch {
    return true;
  }
  if (!/^(https?|data|blob):$/.test(u.protocol)) return true;
  if (u.protocol === "blob:") return isBlocked(u.pathname, mine); // blob:https://site/… is that site's
  const host = hostKey(u.hostname);
  const path = pathKey(u.pathname);
  return [...BUILT_IN_BLOCKED, ...mine].some((entry) => {
    const [entryHost = "", ...rest] = entry.replace(/^https?:\/\//i, "").split("/");
    const blocked = hostKey(entryHost);
    const from = rest.length ? pathKey(`/${rest.join("/")}`) : "";
    return blocked !== "" && (host === blocked || host.endsWith(`.${blocked}`)) && path.startsWith(from);
  });
}

/** Whether a tab is out of bounds: where it is, or where it is going. */
export function isTabBlocked(tab: { url?: string; pendingUrl?: string }, mine: readonly string[]): boolean {
  return isBlocked(tab.url, mine) || isBlocked(tab.pendingUrl, mine);
}

/** A site's host for the owner's eyes (or the address's start when it has none). */
export function hostOf(url: string): string {
  try {
    return new URL(url).host || url.slice(0, 40);
  } catch {
    return url.slice(0, 40);
  }
}

/** Chrome's new-tab page: a tab the owner opened while one of Emberhut's was in front, which Chrome gives it as opener. */
export function isNewTabPage(url: string): boolean {
  return /^(chrome|chrome-search|edge):\/\/(newtab|new-tab-page)/.test(url);
}

/** The owner's own list from the popup's box: one site per line, blanks dropped. */
export function blockedList(text: string): string[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
}
