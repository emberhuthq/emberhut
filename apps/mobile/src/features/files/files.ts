// What a server file is, by its name: how the viewer shows it, and an HTML report's title and sandbox.

export type ViewerKind = "image" | "video" | "pdf" | "html" | "text" | "doc";

const extensions: Record<string, ViewerKind> = {
  png: "image",
  jpg: "image",
  jpeg: "image",
  gif: "image",
  webp: "image",
  heic: "image",
  heif: "image",
  bmp: "image",
  mp4: "video",
  mov: "video",
  m4v: "video",
  webm: "video",
  mkv: "video",
  "3gp": "video",
  pdf: "pdf",
  html: "html",
  htm: "html",
  txt: "text",
  md: "text",
  markdown: "text",
  log: "text",
  csv: "text",
  json: "text",
};

export function extensionOf(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot < 0 ? "" : name.slice(dot + 1).toLowerCase();
}

export function viewerKind(name: string): ViewerKind {
  return extensions[extensionOf(name)] ?? "doc";
}

/** An HTML page: a report card in the chat, the sandboxed viewer when opened. */
export const isReport = (name: string): boolean => viewerKind(name) === "html";

/** The file's name without its extension: a report's title until its page is read. */
export function stem(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(0, dot) : name;
}

const entities: [string, string][] = [
  ["&lt;", "<"],
  ["&gt;", ">"],
  ["&quot;", '"'],
  ["&#39;", "'"],
  ["&nbsp;", " "],
  ["&mdash;", "—"],
  ["&ndash;", "–"],
  ["&amp;", "&"],
];

/** A page's title: its <title>, else its first <h1>, as plain text; null when it has neither. */
export function reportTitle(html: string): string | null {
  const found = /<title\b[^>]*>([\s\S]*?)<\/title>/i.exec(html)?.[1] ?? /<h1\b[^>]*>([\s\S]*?)<\/h1>/i.exec(html)?.[1];
  if (found === undefined) return null;
  let text = found.replace(/<[^>]+>/g, "");
  for (const [entity, ch] of entities) text = text.split(entity).join(ch);
  text = text.replace(/\s+/g, " ").trim();
  return text === "" ? null : text;
}

/**
 * What a report may load: its own inline styles, scripts, fonts and media, and pictures inline or over https (never plain
 * http: a page can't reach a server on the private network); no requests, forms, frames or base address.
 */
const reportPolicy =
  "default-src 'none'; img-src https: data: blob:; media-src data: blob:; style-src 'unsafe-inline'; " +
  "script-src 'unsafe-inline'; font-src data:; connect-src 'none'; form-action 'none'; base-uri 'none'; frame-src 'none'";

/**
 * The page with the policy as its very first element and the viewer's tap script right after it, ahead of anything of its
 * own: the script runs first by document order, so it takes its references while they're still the browser's.
 */
export function sandboxed(html: string, token: string): string {
  const head = `<meta http-equiv="Content-Security-Policy" content="${reportPolicy}"><script>${reportTapScript(token)}</script>`;
  const doctype = /^\s*<!doctype[^>]*>/i.exec(html);
  return doctype ? doctype[0] + head + html.slice(doctype[0].length) : head + html;
}

/** The report's only page: the HTML it was given, at about:blank. Every other load (a script's, a redirect's, data:, a
 * custom scheme) is stopped, so nothing reaches the system's link handler unless it was tapped. */
export const isReportPage = (url: string): boolean => url === "about:blank" || url.startsWith("about:blank#");

/**
 * A real tap on a link, posted to the app instead of followed (the WebView itself never navigates). It removes itself,
 * keeps its own references to everything it uses before the page's scripts run, runs in strict mode (so a page can't reach
 * its listener through a getter's caller), and builds its message from plain strings (JSON.stringify of an object would
 * consult a toJSON the page can plant): a page can neither read the viewer's token nor forge a message with it.
 */
export function reportTapScript(token: string): string {
  if (!/^[\w-]+$/.test(token)) throw new Error("A viewer's token is letters, digits and dashes.");
  return `
(function () {
  "use strict";
  const script = document.currentScript;
  if (script) script.remove();
  const bridge = window.ReactNativeWebView;
  const send = bridge.postMessage;
  const apply = Reflect.apply;
  const stringify = JSON.stringify;
  const own = (proto, name) => Object.getOwnPropertyDescriptor(proto, name).get;
  const target = own(Event.prototype, "target");
  const prevent = Event.prototype.preventDefault;
  const closest = Element.prototype.closest;
  const attribute = Element.prototype.getAttribute;
  const startsWith = String.prototype.startsWith;
  const href = own(HTMLAnchorElement.prototype, "href");
  const token = "${token}";
  document.addEventListener("click", (e) => {
    let a = null;
    try {
      a = apply(closest, apply(target, e, []), ["a[href]"]);
    } catch (_) {
      return;
    }
    if (!a) return;
    const link = apply(attribute, a, ["href"]);
    if (link !== null && apply(startsWith, link, ["#"])) return;
    apply(prevent, e, []);
    if (e.isTrusted) apply(send, bridge, ['{"token":' + stringify(token) + ',"open":' + stringify(apply(href, a, [])) + "}"]);
  }, true);
})();
`;
}

/** The address a tap asked for, when it came from the viewer's own script (its token) and is one to open: http(s) only. */
export function tappedLink(message: string, token: string): string | null {
  try {
    const { open, token: sent } = JSON.parse(message) as { open?: unknown; token?: unknown };
    if (token === "" || sent !== token) return null;
    return typeof open === "string" && /^https?:\/\//i.test(open) ? open : null;
  } catch {
    return null;
  }
}
