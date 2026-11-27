# 0006. Reports in a web view: gpui-webview, on macOS only

- Status: Proposed (accepted once the Mac checks in apps/desktop/README.md pass)
- Date: 2026-11-25

## Context

The report panel beside the chat shows an HTML report (a page the assistant or a worker made)
with gpui-kit's `TextView::html`. That converts the page to rich text, which drops its CSS,
flattens tables and never runs its scripts, so charts are missing. A report needs a real web
view: a sandboxed WKWebView in which the page runs its own CSS and inline scripts, a
Content-Security-Policy comes first, the store is private, and the page can't navigate. A link
opens in the browser.

GPUI has no web view of its own. gpui-kit, our UI kit, publishes one: `gpui-webview`
(longbridge/gpui-kit, `crates/webview`). It is built on Wry, Tauri's web view library, which
uses WKWebView on macOS.

## Decision

**Use gpui-webview 0.11 for the report panel on macOS, and only there.** Both crates are
declared under `[target.'cfg(target_os = "macos")'.dependencies]` in apps/desktop/Cargo.toml.
`gpui-webview` hosts the view. `wry` (package `lb-wry`) is there because gpui-webview takes
Wry's `WebViewBuilder`, and the builder carries the settings below. Elsewhere the panel keeps
`TextView::html`. It is also the fallback on macOS when the web view can't be made, and the
reason is logged. `EMBERHUT_REPORT_WEBVIEW=off` turns the web view off, to check the fallback and
to measure what the web view costs. Chat messages keep `TextView::html` everywhere.

What was checked:

| Question | Finding |
| --- | --- |
| Licence | gpui-webview: Apache-2.0. lb-wry: Apache-2.0 OR MIT. The other crates it brings to macOS: `dpi` (Apache-2.0 AND MIT), `objc2-web-kit` and `objc2-exception-helper` (Zlib OR Apache-2.0 OR MIT), `cookie` and `time-macros` (MIT OR Apache-2.0). All are on deny.toml's allow list, so deny.toml is unchanged. |
| cargo deny | `cargo deny --locked check advisories bans licenses sources`: ok, with our configuration's default scopes. Widened to `unmaintained = "all"` and `unsound = "all"`, four entries are new: `proc-macro-error` (RUSTSEC-2024-0370, unmaintained), `fxhash` (RUSTSEC-2025-0057, unmaintained), `glib` 0.18.5 (RUSTSEC-2024-0429, unsound) and `rand` 0.7.3 (RUSTSEC-2026-0097, unsound). All four come from Wry's GTK and Android dependencies, which no target of ours builds (`cargo tree -i` finds none of them for Linux, macOS, Android or Windows). The duplicate-version warnings (warn only) grow by 14, all from those other platforms' dependencies. |
| Version fit | gpui-webview 0.11.0 depends on gpui-kit 0.11.0 (`default-features = false, features = ["x11"]`), the gpui-kit we already use. Feature unification gives one gpui-kit, and there is one gpui-pre (0.3.14) in Cargo.lock: the only gpui* package added is gpui-webview. The two crates are released together, weekly, by the same maintainer (0.8.0 on 19 Oct, then every week up to 0.11.0 on 23 Nov). We run gpui-kit's default backend (gpui-pre), so gpui-webview's `gpui-fast` feature stays off. Its compile-time check enforces that match. |
| Maintenance | longbridge/gpui-kit is active: pushed daily, about 21k stars. The webview's README still calls it experimental. Its open webview issues are on GPUI Fast or Linux, not our setup: #4057 (scroll flicker under gpui-fast), #3860 (X11 keyboard focus). |
| Wry fork | `lb-wry` 0.53.3 is Wry 0.53.3 republished by the same maintainer (October 2025). Diffed against upstream 0.53.3, it adds `build_async` wrappers and a Windows-only `flume` dependency. The WKWebView code is upstream's. Upstream Wry is at 0.57.4. gpui-kit issue #3859 asks to move to upstream Wry, and PR #4104 (merged 25 Nov, unreleased) does it. When gpui-webview moves, our `wry` line becomes upstream `wry` at the same version, and these settings are checked against its changelog. |
| What it pulls in | macOS: 7 crates (gpui-webview, lb-wry, dpi, objc2-web-kit, objc2-exception-helper, cookie, time-macros). WebKit is the system's, so nothing is bundled. The desktop also names `objc2` and `objc2-web-kit` itself, for the WebKit configuration below; both were in the tree already. Linux: nothing (`cargo tree` is identical). Cargo.lock grows by 101 packages (about 1,200 lines): the gtk-rs 0.18 stack, webkit2gtk, soup3, javascriptcore, kuchikiki and html5ever, webview2-com, jni and ndk, windows-sys 0.45, rand 0.7, syn 1 and two phf generations among them. A lock file covers every platform, so they are listed, but nothing of ours builds them. cargo deny and Dependabot scan them all, and we accept the alerts they bring (the four above are the first). |

**Not on Linux, for now.** On Linux, Wry needs WebKitGTK 4.1 and GTK 3 (system libraries and
their -dev packages in CI and the dev image). gpui-webview also needs an X11 window (it refuses
Wayland), and it drives GTK's main loop from GPUI. That is a new system dependency, a second
event loop and a windowing restriction, for a platform where the app is a development target.
The macOS-only dependency keeps Linux CI and the dev image as they are. A later ADR can add
Linux.

**The security posture.** Each setting is in `ui/report/webview.rs`, and the decisions behind
them are in gpui-free modules with tests (`policy.rs`, `sandbox.rs`, `guard.rs`):

- **Loaded from the string.** Wry's `with_html` calls `loadHTMLString(html, baseURL: nil)`, so
  the page is `about:blank` with a null origin. No file is written, so there is no file URL
  and nothing to delete.
- **A Content-Security-Policy first.** The policy goes right after the doctype. It allows the page's
  own inline styles, scripts, fonts and media, and pictures inline or over https. It blocks requests
  (`connect-src 'none'`), forms, frames and `<base>`. Plain http is never allowed, so a page can't
  reach a server on the local network.
- **A guard script before the page's own** (a Wry initialization script, in every frame; a page
  script, not a bridge). It removes the camera, microphone, screen capture, location,
  notifications, credentials, `RTCPeerConnection` and `MediaRecorder`. lb-wry's UI delegate
  grants every media capture request without a prompt, and 0.53 has no way to refuse one, so
  the APIs must not be there. Upstream Wry 0.57's `with_permission_handler`, answering Deny to
  every kind, is the later fix, to add with the move to upstream Wry. The script also cancels
  file pickers (`<input type="file">` clicks, `showPicker`), because lb-wry's UI delegate opens
  a real file panel and can't be told not to. And it turns the user's clicks on http, https and
  mailto links, HTML or SVG, into `window.open` (below). A script's own click on a link is
  cancelled.
- **Windows only from a click.** A WebKit configuration of our own (Wry takes one) turns
  `javaScriptCanOpenWindowsAutomatically` off. WebKit then lets
  `window.open` through only while the page handles the user's click or key. This needs six
  `unsafe` objc2 calls, all public WebKit API, under one `#[expect(unsafe_code)]`: two
  constructors (the configuration and the non-persistent data store), one getter (the
  configuration's preferences) and three property setters (the data store, the media that
  waits for the user, and `javaScriptCanOpenWindowsAutomatically`). The same configuration
  holds the private data store and makes all media wait for the user.
- **No navigation away.** The navigation handler allows the page's first load (once) and jumps
  within it (`about:blank#…`). A new window that is http, https or mailto (the user's link
  click, a `target="_blank"` link, a `window.open` during a click) goes to the default browser
  or mail app through `cx.open_url`, at most one a second, and is never a window of ours. A
  navigation of the page itself (a script setting `location`, a meta refresh, a link the guard
  didn't see) goes nowhere until the page has finished loading. After that, at most three of them
  per shown page reach the browser, one a second, and the rest are refused, with a warning
  logged once. Everything else is refused.
- **No bridge.** There is no IPC handler and no custom protocol. Wry injects a `window.ipc` stub
  into every page, but with no handler registered, its `postMessage` reaches nothing.
- **Private storage.** A non-persistent data store. The page sees none of the app's cookies or
  storage and keeps nothing.
- **No autoplay, no link previews, no downloads, no drops.** Media plays only after the user
  starts it. Force-click previews are off, because they load the linked page past the policy.
  Wry 0.53 allows every download by default, so the handler refuses them. Dropped files are
  refused, so a file dragged onto the page is never loaded as a file URL.
- **Devtools off in release.** `with_devtools(cfg!(debug_assertions))`. Wry builds the inspector
  into a release build only with its `devtools` feature, which nothing here enables.

What stays open:

- Wry's navigation handler gets the URL only. It doesn't get the navigation type or the target
  frame, which a native WKWebView delegate would check ("a link the user activated in the main
  frame"). So the user's clicks are told apart by arriving as gesture-gated new windows.
  - A page that navigates itself can still send three pages of its choosing to the browser
    after it has loaded.
  - A page script can open one window per user click, anywhere in the page.
  - A link whose click the page's own handler stops before the guard sees it counts against
    the three and waits for the load.
- The policy's `img-src https:` lets a script send data out in
  picture URLs. The guard removes `RTCPeerConnection`, which CSP doesn't govern.
- The guard is a page script. A frame the page makes might not get it, if WebKit doesn't inject
  user scripts into a new `about:blank` frame. That frame would then have `getUserMedia`,
  `RTCPeerConnection` and file pickers. The Mac checklist tests all three. The permission
  handler of upstream Wry closes the camera and microphone part for good.

**A rule while that stays open: Emberhut's app never declares camera or microphone use while
reports render in a web view without a permission handler.** Without
`NSCameraUsageDescription` and `NSMicrophoneUsageDescription` in its Info.plist, macOS doesn't
hand Emberhut the camera or the microphone, whatever Wry grants a page. A test in xtask's plist
generation (`xtask/src/bundle_macos/plist.rs`) fails if either key appears, and points here.
Adding dictation or calls to the app means adding the permission handler first.

Rejected: turning WebKit's media and peer connections off engine-wide through its private
preferences (`mediaDevicesEnabled`, `peerConnectionEnabled` and the like, set with
`setValue:forKey:` as lb-wry does for `developerExtrasEnabled`). That would cover every frame,
but it is private API: undocumented, free to change or vanish in any macOS release, and
silently ignored when it does. The guard, the plist rule and the upstream permission handler
are the defence instead.

**One web view at a time.** The panel holds at most one. It is made when a fetched report shows
and dropped, which removes its native view, when the panel closes, shows another report, or shows
a report that is loading or failed. Nothing is kept hidden. Resizing the panel or letting it fill
the window keeps the same view: gpui-webview sets the native view's frame from the element's
layout on every frame it draws, so the view follows the panel's bounds. The page gets Emberhut's own
light or dark appearance (gpui-webview's `set_theme`), so `prefers-color-scheme` follows Emberhut's
choice rather than the system's. After the policy goes a style with no weight (`:where(:root)`)
giving the root Emberhut's surface and text in each appearance, so a page without colours of its
own sits on the panel in its text rather than in WebKit's white and black (Wry's background colour
does nothing on macOS). A preset change reaches an open page when it is next opened.

## Consequences

- Reports show as designed on the Mac: CSS, layout, tables, inline-script charts, RTL and Arabic
  through WebKit. A report that loads its scripts from a CDN runs without them, because the
  policy blocks remote scripts. "Open in browser" (unchanged) shows it with them.
- On gpui-pre the native view sits above GPUI content in its bounds and isn't clipped. A GPUI
  tooltip, menu or popover over the panel's body would be hidden under the page. Nothing of ours
  overlaps it today: the answer toast sits in the page column, and while the report fills the
  window in the page's place, in a strip of its own under the report, outside the web view's
  bounds. For the same reason, the header shows the file name as a second line rather than as a
  tooltip.
- While the page has keyboard focus, keys go to WebKit, so Esc doesn't close the panel there.
  The close button does.
- The fallback covers a web view that can't be made, or that `EMBERHUT_REPORT_WEBVIEW=off` turns
  off. It does not cover a page that fails after the view was made. Wry 0.53 doesn't report a
  WebKit content process that died, so the panel stays blank. A warning is logged when the page
  hasn't finished loading after 15 s (a slow picture does that too). Open in browser and
  reopening the report still work.
- lb-wry has two side effects on the window. Making a web view activates the app ("make sure the
  window is always on top"), so a report whose fetch finishes after the user switched away
  brings Emberhut to the front, and `--background` runs come forward when a report opens. It also
  sets the window's titlebar separator style to none, from the first report on. If the
  activation is a problem in use, it goes upstream (gpui-kit / Wry).
- The web view's own memory and CPU are measured on a Mac. apps/desktop/README.md says how, and
  what we expect.
- CI's macOS jobs (clippy, tests, release build) compile gpui-webview and Wry. The Linux jobs
  don't change.
- When gpui-kit 0.12 ships with upstream Wry, the `wry` dependency moves with it in the same
  change.

## Addendum: the browser's live view

The Browser window shows the assistant's browser live, as the stream's own web page (a WebRTC
client) in a second kind of web view, also macOS only (`apps/desktop/src/ui/browser/stream.rs`).
It reuses this decision's crates and differs from a report on purpose:

- **Loaded from the stream's address**, which the server gives per device with a password for
  what the device may do (view only, or control). The sign-in rides in the address's user
  part, which WebKit answers the stream's Basic challenge with; Wry 0.53 has no
  authentication handler. It is put in an `https` address only, never sent in the clear. The
  view is made afresh for each session.
- **No guard script and no CSP.** The page needs what the report guard removes: WebRTC for the
  picture, and the keyboard, mouse and clipboard while the person drives. Its own scripts (the
  clipboard bridge, ⌘ as Ctrl, the screen's size, the stream's buttons hidden) run first.
- **A bridge for the clipboard only, while driving.** Wry's IPC handler takes two messages, a
  clipboard read (answered by a script call with the text) and a write; anything else is
  dropped. The bridge is there only in a control session (watching, the page has no use for
  the clipboard), and a message counts only when the frame that posted it is on the stream's
  origin and it carries the bridge's random token, which only the bridge's own script, in the
  main frame, holds.
- **Kept on the stream.** The navigation handler allows only the stream's own origin (and
  `about:` frames); new windows and downloads are refused, drops are kept from WebKit, the data
  store is private, and devtools are on in debug builds only.
- **No camera or microphone.** The rule above still holds: Emberhut declares neither, so macOS
  gives the page neither, whatever lb-wry's UI delegate grants.
