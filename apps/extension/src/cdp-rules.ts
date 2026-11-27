// Which DevTools commands the server may send through to one of Emberhut's tabs, and what of the tab's events goes back.
// An allow-list: the commands the server's DevTools engine uses on a page (crates/emberhut-server/src/services/browser/
// cdp.rs and the extension link in services/chrome.rs), nothing else. So nothing reaches past the page (the browser,
// its storage, other sites' cookies and storage, service workers, passkeys, other targets, network fetches outside
// the page), nothing edits the page's DOM directly, and no drag-and-drop or IME input bypasses the password guard.
// The server only ever talks to a tab's own session, so a command for a child session is refused too.

/** The commands the server may send, each on a tab's own session. */
const ALLOWED: ReadonlySet<string> = new Set([
  "Accessibility.getFullAXTree",
  "DOM.enable",
  "DOM.getContentQuads",
  "DOM.resolveNode",
  "DOM.scrollIntoViewIfNeeded",
  "Input.dispatchKeyEvent",
  "Input.dispatchMouseEvent",
  "Input.insertText",
  "Page.addScriptToEvaluateOnNewDocument",
  "Page.captureScreenshot",
  "Page.enable",
  "Page.getNavigationHistory",
  "Page.handleJavaScriptDialog",
  "Page.navigate",
  "Page.navigateToHistoryEntry",
  "Page.reload",
  "Page.setInterceptFileChooserDialog",
  "Runtime.callFunctionOn",
  "Runtime.evaluate",
  "Target.getTargetInfo",
]);

/** Why `method` is never sent (for a child session when `sessionId` is set), or null when it may be. */
export function refusal(method: string, sessionId?: string): string | null {
  if (sessionId) return `${method} on another target's session isn't allowed in this Chrome`;
  return ALLOWED.has(method) ? null : `${method} isn't allowed in this Chrome`;
}

const nonEmpty = (value: unknown): boolean => typeof value === "string" && value.length > 0;
const record = (value: unknown): Record<string, unknown> =>
  typeof value === "object" && value !== null ? (value as Record<string, unknown>) : {};

/** A key's text that types nothing into a field: Enter or Tab (which submit, or move on). Space is typing. */
const NOT_TYPING = /^[\r\n\t]+$/;
const typing = (value: unknown): boolean => nonEmpty(value) && !NOT_TYPING.test(value as string);

/**
 * Whether the command types text into whatever has the focus: inserted text, or a key that carries text. Enter and
 * Tab aren't typing, so a form the owner filled in and handed back can still be sent or moved through.
 */
export function typesText(method: string, params: unknown): boolean {
  const p = record(params);
  switch (method) {
    case "Input.insertText":
      return nonEmpty(p.text);
    case "Input.dispatchKeyEvent":
      return typing(p.text) || typing(p.unmodifiedText);
    default:
      return false;
  }
}

/** DevTools' modifier bits. */
const CTRL = 2;
const META = 4;
const SHIFT = 8;

/** The virtual key codes of V and Insert (Windows' codes, which DevTools and the server use for both fields). */
const VK_V = 86;
const VK_INSERT = 45;

/**
 * Why a key event is never sent, or null: an editing command riding on the key (`commands`, paste among them), or a
 * paste shortcut (Ctrl or Cmd+V, Shift+Insert, by its key, code or virtual key code), which would put the owner's
 * clipboard into the page. A stated limit: on Linux a middle click pastes the primary selection (the last text
 * selected anywhere), which no key check sees.
 */
export function keyRefusal(method: string, params: unknown): string | null {
  if (method !== "Input.dispatchKeyEvent") return null;
  const p = record(params);
  if (Array.isArray(p.commands) && p.commands.length > 0) return "A key with editing commands isn't allowed in this Chrome";
  const modifiers = typeof p.modifiers === "number" ? p.modifiers : 0;
  const key = typeof p.key === "string" ? p.key.toLowerCase() : "";
  const code = typeof p.code === "string" ? p.code : "";
  const vk = [p.windowsVirtualKeyCode, p.nativeVirtualKeyCode];
  const v = key === "v" || code === "KeyV" || vk.includes(VK_V);
  const insert = key === "insert" || code === "Insert" || vk.includes(VK_INSERT);
  if ((v && modifiers & (CTRL | META)) || (insert && modifiers & SHIFT)) {
    return "Pasting isn't allowed in this Chrome: it would hand the page the owner's clipboard";
  }
  return null;
}

/** Why a function call is never sent, or null: one on no element (a whole context) isn't how the server calls. */
export function callRefusal(method: string, params: unknown): string | null {
  if (method !== "Runtime.callFunctionOn") return null;
  return nonEmpty(record(params).objectId) ? null : "A function on no element isn't allowed in this Chrome";
}

/** Whether the command runs a function on one element: checked for a password field before it is sent. */
export function onElement(method: string, params: unknown): boolean {
  return method === "Runtime.callFunctionOn" && nonEmpty(record(params).objectId);
}

/**
 * Why a command is never sent because it could reach into Emberhut's own isolated world (the pill, its Pause and the
 * password guard's memory), or null. The server's engine runs everything in the page's own world, so a command that
 * names any context (`contextId`, `executionContextId`, `uniqueContextId`) or any world (`worldName`) is refused:
 * Emberhut's world's contexts can't be guessed into, and no script or node is put there by the server.
 */
export function worldRefusal(method: string, params: unknown): string | null {
  const p = record(params);
  const named = ["contextId", "executionContextId", "uniqueContextId", "worldName"].some((key) => p[key] !== undefined);
  return named ? `${method} into another world than the page's isn't allowed in this Chrome` : null;
}

/** Whether an event is about Emberhut's own isolated world (its contexts, its bindings): such an event never goes back. */
export function aboutOwnWorld(method: string, params: unknown, worldName: string, contexts: ReadonlySet<number>): boolean {
  const p = record(params);
  const context = record(p.context);
  if (method === "Runtime.executionContextCreated") return context.name === worldName || contexts.has(Number(context.id));
  if (method === "Runtime.executionContextDestroyed" || method === "Runtime.bindingCalled") {
    return contexts.has(Number(p.executionContextId));
  }
  return false;
}

/** The event domains the server reads from a tab (dialogs, file pickers, loads, detaches): nothing else goes back. */
const EVENT_DOMAINS: ReadonlySet<string> = new Set(["Page", "Inspector", "Target", "Runtime", "DOM"]);

/** Whether a tab's event goes back to the server: an allowed domain, from the tab's own session. */
export function forwards(method: string, sessionId?: string): boolean {
  return !sessionId && EVENT_DOMAINS.has(method.split(".")[0] ?? "");
}

const COOKIE_HEADER = /^(cookie|set-cookie|set-cookie2|authorization|proxy-authorization)$/i;

/**
 * An event's params with every cookie the browser holds taken out: `associatedCookies` and `cookie` lists, and the
 * Cookie, Set-Cookie and Authorization headers of any `headers` object, at any depth. Applied to whatever goes back.
 */
export function stripCookies(value: unknown, depth = 0): unknown {
  if (depth > 16 || typeof value !== "object" || value === null) return value;
  if (Array.isArray(value)) return value.map((item) => stripCookies(item, depth + 1));
  const out: Record<string, unknown> = {};
  for (const [key, item] of Object.entries(value as Record<string, unknown>)) {
    if (key === "associatedCookies" || key === "cookies" || key === "blockedCookies" || key === "exemptedCookies") continue;
    if ((key === "headers" || key === "requestHeaders" || key === "responseHeaders") && typeof item === "object" && item !== null) {
      out[key] = Object.fromEntries(Object.entries(item as Record<string, unknown>).filter(([name]) => !COOKIE_HEADER.test(name)));
      continue;
    }
    out[key] = stripCookies(item, depth + 1);
  }
  return out;
}

/** What the server is told when the focus, or the element, is a password or code field. */
export const SECRET_REFUSED =
  "That is a password or one-time-code field: Emberhut never types or fills in a password or a code. Hand it to the owner (browser_handoff).";

/** What the server is told when the focus can't be checked (a frame from another site has it). */
export const SECRET_UNKNOWN =
  "The focus is in a frame from another site, which Emberhut can't check for a password field: hand it to the owner (browser_handoff).";
