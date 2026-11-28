// Download: picks the right installer for this visitor from the repository's GitHub releases. Without JavaScript, or if
// GitHub doesn't answer, the plain links in the page stay: the latest release, all releases and the waitlist. Asks
// GitHub's public API straight from the browser (no token, no cookies, no referrer) and keeps the answer in this tab's
// session storage for ten minutes, so moving between pages doesn't ask again.
const REPO = "emberhuthq/emberhut";
const API = `https://api.github.com/repos/${REPO}/releases?per_page=20`;
const RELEASES = `https://github.com/${REPO}/releases`;
const GUIDE = `https://github.com/${REPO}/blob/main/docs/install.md`;
const WAITLIST = "mailto:hello@emberhut.com?subject=Emberhut%20waitlist&body=Please%20let%20me%20know%20when%20Emberhut%20is%20ready.";
// The Windows installer has no Authenticode signature yet, so SmartScreen warns. Flip this when it does.
const WINDOWS_SIGNED = false;
const CACHE = "emberhut-releases";
const CACHE_MS = 10 * 60 * 1000;
const TIMEOUT_MS = 5000;

const DESKTOP = {
  mac: { label: "Mac", suffix: "-macos-universal.dmg", note: "Apple silicon and Intel · macOS 13+" },
  windows: { label: "Windows", suffix: "-windows-x64.msi", note: "Windows 10 and 11 · 64-bit" },
};
const SERVERS = [
  ["Linux x64", "-linux-x64.tar.gz"], ["Linux arm64", "-linux-arm64.tar.gz"],
  ["macOS", "-macos-universal.tar.gz"], ["Windows x64", "-windows-x64.zip"],
];

/** The visitor's system: mac, windows, ios, android, linux or other. */
export function detectOs(nav = navigator) {
  const platform = nav.userAgentData?.platform || "";
  const ua = nav.userAgent || "";
  if (platform) {
    if (/android/i.test(platform)) return "android";
    if (/ios/i.test(platform)) return "ios";
    if (/mac/i.test(platform)) return "mac";
    if (/win/i.test(platform)) return "windows";
    if (/linux|chrome ?os|cros/i.test(platform)) return "linux";
  }
  if (/android/i.test(ua)) return "android";
  if (/iphone|ipad|ipod/i.test(ua)) return "ios";
  if (/macintosh|mac os x/i.test(ua)) return nav.maxTouchPoints > 1 ? "ios" : "mac"; // iPadOS asks for the desktop site
  if (/windows/i.test(ua)) return "windows";
  if (/linux|x11|cros/i.test(ua)) return "linux";
  return "other";
}

/** A tag's build number, the same order the release pipeline uses: vX.Y.Z-rc.N and vX.Y.Z (a final beats its RCs). */
export function buildOf(tag) {
  const m = /^v?(\d+)\.(\d+)\.(\d+)(?:-rc\.(\d+))?$/.exec(tag || "");
  if (!m) return -1;
  return Number(m[1]) * 1e6 + Number(m[2]) * 1e4 + Number(m[3]) * 100 + (m[4] ? Number(m[4]) : 99);
}

/** The newest release, and the newest stable one when the newest is a pre-release. */
export function choose(list) {
  const ok = (Array.isArray(list) ? list : []).filter((r) => r && !r.draft && buildOf(r.tag_name) >= 0)
    .sort((a, b) => buildOf(b.tag_name) - buildOf(a.tag_name));
  const newest = ok[0] || null;
  const stable = ok.find((r) => !r.prerelease) || null;
  return { newest, stable: newest && newest.prerelease ? stable : null };
}

const asset = (r, suffix, prefix = "Emberhut-") => r?.assets?.find((a) => a.name?.startsWith(prefix) && a.name.endsWith(suffix)) || null;
const version = (r) => r.tag_name.replace(/^v/, "");

async function releases() {
  try {
    const hit = JSON.parse(sessionStorage.getItem(CACHE) || "null");
    if (hit && Date.now() - hit.at < CACHE_MS && Array.isArray(hit.list)) return hit.list;
  } catch { /* storage blocked or bad JSON: ask again */ }
  const ctl = new AbortController();
  const timer = setTimeout(() => ctl.abort(), TIMEOUT_MS);
  try {
    const res = await fetch(API, { signal: ctl.signal, headers: { Accept: "application/vnd.github+json" }, credentials: "omit", referrerPolicy: "no-referrer" });
    if (!res.ok) throw new Error(`GitHub answered ${res.status}`);
    const raw = await res.json();
    if (!Array.isArray(raw)) throw new Error("not a list");
    // keep only what the page uses
    const list = raw.map((r) => ({
      tag_name: String(r.tag_name || ""), name: r.name, draft: !!r.draft, prerelease: !!r.prerelease, html_url: r.html_url, published_at: r.published_at,
      assets: (r.assets || []).map((a) => ({ name: String(a.name || ""), browser_download_url: a.browser_download_url, size: a.size })),
    }));
    try { sessionStorage.setItem(CACHE, JSON.stringify({ at: Date.now(), list })); } catch { /* not kept */ }
    return list;
  } finally { clearTimeout(timer); }
}

// a small element builder: text children only, never HTML from the API
function h(tag, attrs = {}, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) if (v != null && v !== false) el.setAttribute(k, v === true ? "" : v);
  for (const k of kids.flat()) if (k != null && k !== false) el.append(k);
  return el;
}
const safe = (url) => (typeof url === "string" && url.startsWith("https://github.com/") ? url : null);
const arrow = () => h("span", { class: "arrow", "aria-hidden": "true" }, "→");
const mb = (bytes) => (bytes > 0 ? `${Math.max(1, Math.round(bytes / 1e6))} MB` : null);
const date = (iso) => { const d = new Date(iso); return isNaN(d) ? null : d.toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric" }); };

function label(r) {
  return r.prerelease
    ? h("p", { class: "dl-ver" }, h("span", { class: "badge soon" }, `Preview · ${version(r)}`), h("span", {}, "Release candidate: may have rough edges."))
    : h("p", { class: "dl-ver" }, h("span", { class: "badge ok" }, `Stable · ${version(r)}`), date(r.published_at) && h("span", {}, `Published ${date(r.published_at)}`));
}

function unsigned() {
  return WINDOWS_SIGNED ? null : h("p", { class: "dl-warn" }, "Not code-signed yet: Windows SmartScreen may warn. Choose More info → Run anyway. ",
    h("a", { href: `${GUIDE}#windows` }, "How to install on Windows"));
}

function primary(r, os) {
  const d = DESKTOP[os];
  const a = asset(r, d.suffix);
  if (!a || !safe(a.browser_download_url)) return null;
  return h("div", { class: "dl-main" },
    h("div", { class: "ctas" }, h("a", { class: "cta-main", href: a.browser_download_url, "data-dl-primary": os }, `Download for ${d.label} `, arrow())),
    h("p", { class: "dl-note" }, [d.note, mb(a.size)].filter(Boolean).join(" · ")),
    os === "windows" ? unsigned() : null);
}

function secondary(r, os, prefix) {
  const d = DESKTOP[os];
  const a = asset(r, d.suffix);
  if (!a || !safe(a.browser_download_url)) return null;
  return h("li", {}, prefix || null, h("a", { href: a.browser_download_url, "data-dl-secondary": os }, `Download for ${d.label}`), h("small", {}, ` · ${d.note}`),
    os === "windows" ? unsigned() : null);
}

function servers(r) {
  const items = SERVERS.map(([name, suffix]) => [name, asset(r, suffix, "emberhut-server-")]).filter(([, a]) => a && safe(a.browser_download_url));
  if (!items.length) return null;
  return h("li", {}, "The hut on its own, as a server: ", ...items.flatMap(([name, a], i) => [i ? ", " : "", h("a", { href: a.browser_download_url }, name)]), ".");
}

function foot(r) {
  return h("p", { class: "dl-links" }, safe(r?.html_url) ? h("a", { href: r.html_url }, "All downloads and checksums") : h("a", { href: RELEASES }, "All releases"),
    h("span", { "aria-hidden": "true" }, " · "), h("a", { href: GUIDE }, "Install guide"));
}

function waitlistButton() {
  return h("div", { class: "ctas" }, h("a", { class: "cta-main", href: WAITLIST }, "Join the waitlist ", arrow()));
}

/** What the section shows for this OS and these releases, or null to keep the page's own links. */
export function render(list, os) {
  const { newest, stable } = choose(list);
  const phone = os === "ios" || os === "android";
  const phoneLine = h("p", { class: "dl-lede" }, "Emberhut runs on your Mac or PC; the phone app pairs with it. The iPhone and Android apps are in testing, not in the stores yet.");
  if (!newest) {
    return h("div", { class: "dl-body", "data-dl-state": "none" },
      h("p", { class: "dl-ver" }, h("span", { class: "badge soon" }, "Coming soon")),
      h("p", { class: "dl-title" }, "First preview coming soon"),
      phone ? phoneLine : h("p", { class: "dl-lede" }, "The first preview of the Mac and Windows app goes up here when it's ready. Join the waitlist and we'll write once when it is."),
      waitlistButton());
  }
  if (phone) {
    return h("div", { class: "dl-body", "data-dl-state": "phone" }, label(newest), phoneLine, waitlistButton(), foot(newest));
  }
  if (os === "mac" || os === "windows") {
    const main = primary(newest, os);
    if (!main) return null;
    const other = os === "mac" ? "windows" : "mac";
    const more = h("ul", { class: "dl-more" },
      secondary(newest, other, ""),
      stable ? (secondary(stable, os, `Prefer the stable ${version(stable)}? `)
        || (safe(stable.html_url) && h("li", {}, `Prefer the stable version? `, h("a", { href: stable.html_url }, `Emberhut ${version(stable)}`)))) : null,
      servers(newest));
    return h("div", { class: "dl-body", "data-dl-state": newest.prerelease ? "preview" : "stable" }, label(newest), main,
      more.childElementCount ? more : null, foot(newest));
  }
  // Linux and everything else: the desktop app is Mac and Windows; the hut runs as a server
  return h("div", { class: "dl-body", "data-dl-state": "server" }, label(newest),
    h("p", { class: "dl-lede" }, "The desktop app is for Mac and Windows. On Linux, run the hut as a server and pair the apps with it."),
    h("ul", { class: "dl-more" }, servers(newest), secondary(newest, "mac", ""), secondary(newest, "windows", "")),
    foot(newest));
}

export async function mount(root) {
  const body = root?.querySelector("[data-dl-body]");
  if (!body) return;
  let list;
  try { list = await releases(); } catch { return; } // offline, rate-limited or slow: the plain links stay
  const view = render(list, detectOs());
  if (!view) return;
  body.replaceChildren(view);
  root.dataset.dlReady = view.dataset.dlState;
}

if (typeof document !== "undefined") for (const root of document.querySelectorAll("[data-download]")) mount(root);
