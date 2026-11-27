// The scripts the extension runs in the pages of Emberhut's tabs, as text for DevTools' Runtime.evaluate and
// Page.addScriptToEvaluateOnNewDocument.

/** The isolated world the pill, the cursor and the password check live in: the page's own scripts can't reach it. */
export const PILL_WORLD = "emberhut-pill";
/** The binding the pill's Pause calls (in its isolated world only). */
export const PILL_BINDING = "__emberhutPause";
/** The binding a page calls just before it closes itself (in the page's own world), so the server hears who closed it. */
export const CLOSING_BINDING = "__emberhutClosing";

const PILL_HOST = "emberhut-pill-host";
const CURSOR_HOST = "emberhut-cursor-host";
const POINT = "__emberhutPoint";
const BLUE = "#2563eb";

const SECRET_AUTOCOMPLETE = "/current-password|new-password|one-time-code/i";
const SECRET_FIELD = `(e) => !!e && e.tagName === "INPUT" && (String(e.type).toLowerCase() === "password" || ${SECRET_AUTOCOMPLETE}.test(e.getAttribute("autocomplete") || ""))`;
/** The isolated world's memory of the fields that were password or code fields in this document. */
const WAS_SECRET = "__emberhutWasSecret";

/**
 * Remembers, in the isolated world, every field that is or was a password or code field in this document: one whose
 * type is changed from password to text (or whose autocomplete loses its secret word) stays one for the password
 * guard. Every field there when Emberhut comes, and every one added or changed after.
 */
export const SECRET_MEMORY = `(() => {
  if (globalThis.${WAS_SECRET}) return;
  const was = (globalThis.${WAS_SECRET} = new WeakSet());
  const secret = ${SECRET_FIELD};
  const mark = (e) => { if (secret(e)) was.add(e); };
  const scan = (root) => { mark(root); for (const e of root.querySelectorAll ? root.querySelectorAll("input") : []) mark(e); };
  new MutationObserver((records) => {
      for (const r of records) {
        if (r.type === "attributes") {
          const old = r.oldValue || "";
          if ((r.attributeName === "type" && old.toLowerCase() === "password") || (r.attributeName === "autocomplete" && ${SECRET_AUTOCOMPLETE}.test(old))) was.add(r.target);
          mark(r.target);
        } else for (const n of r.addedNodes) if (n.nodeType === 1) scan(n);
      }
  }).observe(document, { subtree: true, childList: true, attributes: true, attributeFilter: ["type", "autocomplete"], attributeOldValue: true });
  if (document.documentElement) scan(document.documentElement);
})();`;

/** The pill's host's style: every property important, so the page's own CSS can't hide or move it. */
const HOST_STYLE = [
  ["all", "initial"],
  ["position", "fixed"],
  ["z-index", "2147483647"],
  ["left", "50%"],
  ["bottom", "14px"],
  ["transform", "translateX(-50%)"],
  ["display", "block"],
  ["visibility", "visible"],
  ["opacity", "1"],
  ["pointer-events", "auto"],
  ["clip-path", "none"],
  ["filter", "none"],
]
  .map(([name, value]) => `${name}:${value} !important`)
  .join(";");

/**
 * The pill on every page Emberhut drives, "Emberhut is driving · Pause", and Emberhut's own cursor. Drawn from the isolated world in
 * a closed shadow root that is aria-hidden, so the assistant's snapshots never see it (or press its Pause), and the
 * page's scripts can't press it either. The page can't take it away: removed, it comes back; its style is all
 * important and put back when changed; and it is kept the page's last element, above what the page adds. Emberhut never
 * takes the owner's focus: its tabs open behind theirs and its clicks and keys go to its tab alone through
 * chrome.debugger; the cursor shows where its mouse events went, for when the owner looks. The same script keeps the
 * password guard's memory (SECRET_MEMORY), in every frame.
 */
export const PILL = `(() => {
  ${SECRET_MEMORY}
  if (self !== top) return; // Chrome runs it in every frame too: one pill per page, the top frame's
  if (globalThis.__emberhutPill) return globalThis.__emberhutPill();
  let host = null, cursor = null, at = null, timer = 0;
  const watchers = [];
  const style = "${HOST_STYLE}";
  const stop = () => {
    clearInterval(timer);
    for (const w of watchers) w.disconnect();
    host?.remove(); cursor?.remove();
    delete globalThis.__emberhutPill; delete globalThis.__emberhutStop; delete globalThis.${POINT};
  };
  const draw = () => {
    if (typeof ${PILL_BINDING} !== "function") return stop(); // Emberhut's link to the tab is gone
    const root = document.documentElement;
    if (!root) return;
    for (const old of document.querySelectorAll("#${PILL_HOST}")) if (old !== host) old.remove();
    if (!host) {
      host = document.createElement("div"); host.id = "${PILL_HOST}"; host.setAttribute("aria-hidden", "true");
      const shadow = host.attachShadow({ mode: "closed" });
      shadow.innerHTML = '<style>div{display:flex;align-items:center;gap:10px;padding:7px 8px 7px 14px;border-radius:999px;background:${BLUE};color:#fff;font:600 13px/1 -apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;box-shadow:0 4px 14px rgba(0,0,0,.25)}i{width:8px;height:8px;border-radius:50%;background:#fff;animation:p 1.4s infinite}@keyframes p{50%{opacity:.35}}button{all:unset;cursor:pointer;padding:5px 11px;border-radius:999px;background:#fff;color:${BLUE};font:700 12px/1 inherit}</style><div><i></i>Emberhut is driving<button title="Emberhut lets go now and comes back by itself in 10 minutes">Pause</button></div>';
      shadow.querySelector("button").addEventListener("click", (e) => { e.stopPropagation(); ${PILL_BINDING}("pause"); });
      const kept = new MutationObserver(() => { if (host.getAttribute("style") !== style) host.setAttribute("style", style); });
      kept.observe(host, { attributes: true });
      watchers.push(kept);
    }
    if (host.getAttribute("style") !== style) host.setAttribute("style", style);
    if (host.getAttribute("aria-hidden") !== "true") host.setAttribute("aria-hidden", "true");
    if (host.parentNode !== root || root.lastElementChild !== host) root.appendChild(host);
  };
  const point = () => {
    if (!document.documentElement) return;
    if (!cursor || !cursor.isConnected) {
      document.getElementById("${CURSOR_HOST}")?.remove();
      cursor = document.createElement("div"); cursor.id = "${CURSOR_HOST}"; cursor.setAttribute("aria-hidden", "true");
      cursor.style.cssText = "all:initial;position:fixed;z-index:2147483647;left:0;top:0;pointer-events:none;transition:transform .12s ease-out;display:none";
      const root = cursor.attachShadow({ mode: "closed" });
      root.innerHTML = '<style>svg{display:block;filter:drop-shadow(0 1px 2px rgba(0,0,0,.35))}b{position:absolute;left:-9px;top:-9px;width:18px;height:18px;border-radius:50%;background:rgba(37,99,235,.35);transform:scale(0);opacity:0}b.on{animation:c .45s ease-out}@keyframes c{0%{transform:scale(.4);opacity:1}100%{transform:scale(1.8);opacity:0}}</style><b></b><svg width="22" height="22" viewBox="0 0 22 22"><path d="M3 2l15 8.2-6.6 1.6L8.6 19z" fill="${BLUE}" stroke="#fff" stroke-width="1.6" stroke-linejoin="round"/></svg>';
      cursor.ring = root.querySelector("b");
      document.documentElement.appendChild(cursor);
    }
    if (at) { cursor.style.display = "block"; cursor.style.transform = "translate(" + at[0] + "px," + at[1] + "px)"; }
  };
  globalThis.${POINT} = (x, y, type) => { at = [x, y]; point();
    const r = type === "mousePressed" && cursor?.ring; if (r) { r.classList.remove("on"); void r.offsetWidth; r.classList.add("on"); } };
  globalThis.__emberhutPill = draw;
  globalThis.__emberhutStop = stop;
  draw(); document.addEventListener("DOMContentLoaded", draw);
  const page = new MutationObserver(() => { if (globalThis.__emberhutPill) { draw(); if (at) point(); } });
  page.observe(document, { childList: true, subtree: true });
  watchers.push(page);
  timer = setInterval(draw, 1000);
})()`;

/** The pill and the cursor gone, and their watchers stopped, when Emberhut lets go of the tab (in the pill's world). */
export const UNPILL = `globalThis.__emberhutStop?.(); document.getElementById("${PILL_HOST}")?.remove(); document.getElementById("${CURSOR_HOST}")?.remove()`;

/** Emberhut's cursor moved to (x, y), with a ring on a press. */
export function pointAt(x: number, y: number, type: string): string {
  return `globalThis.${POINT}?.(${x}, ${y}, ${JSON.stringify(type)})`;
}

/**
 * Pages can't tell Emberhut is attached: their console goes nowhere. Under chrome.debugger every page looks like it has
 * DevTools open, and some sites watch for that and close their own tab: their checks log trap objects (a RegExp's, a
 * Date's or a function's toString, read only when a debugger takes the line) and time console.table; with a quiet
 * console nothing is read and nothing is slow. Nobody reads a page's console here. In the page's own world, as its
 * scripts are; and a page that still closes itself says so first (CLOSING_BINDING), so the server hears who closed
 * the tab.
 */
export const GUARD = `(() => {
  const quiet = () => {};
  for (const k of ["log", "info", "warn", "error", "debug", "dir", "dirxml", "table", "trace", "clear", "group", "groupCollapsed",
    "groupEnd", "count", "countReset", "time", "timeLog", "timeEnd", "timeStamp", "assert", "profile", "profileEnd"])
    try { Object.defineProperty(console, k, { value: quiet, writable: true, configurable: true }); } catch {}
  const close = window.close;
  if (typeof close !== "function" || String(close).includes("${CLOSING_BINDING}")) return;
  try { Object.defineProperty(window, "close", { value: function () { try { globalThis.${CLOSING_BINDING}?.(""); } catch {} return Reflect.apply(close, this, arguments); }, writable: true, configurable: true }); } catch {}
})()`;

/**
 * What has the focus, checked from the isolated world: "secret" for a field that is or was a password or one-time-code
 * field in this document (SECRET_MEMORY), "unknown" when
 * a frame from another site has it (its inside can't be read from here), "plain" otherwise. Through open shadow roots
 * and same-site frames.
 */
export const FOCUS_CHECK = `(() => {
  const was = globalThis.${WAS_SECRET};
  const now = ${SECRET_FIELD};
  const secret = (e) => now(e) || !!(was && was.has(e));
  let el = document.activeElement;
  for (let depth = 0; el && depth < 32; depth++) {
    if (secret(el)) return "secret";
    if (el.shadowRoot && el.shadowRoot.activeElement) { el = el.shadowRoot.activeElement; continue; }
    if (el.tagName === "IFRAME" || el.tagName === "FRAME") {
      let inner = null;
      try { inner = el.contentDocument; } catch {}
      if (!inner) return "unknown";
      el = inner.activeElement;
      continue;
    }
    return "plain";
  }
  return "plain";
})()`;

/** Whether the element a function is called on (`this`), in the isolated world, is or was a password or code field. */
export const ELEMENT_CHECK = `function () { const was = globalThis.${WAS_SECRET}; return (${SECRET_FIELD})(this) || !!(was && was.has(this)); }`;
