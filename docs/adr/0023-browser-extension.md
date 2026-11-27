# 0023. The browser extension: in the repository, in TypeScript, its own protocol, updated from the owner's server

- Status: Proposed
- Date: 2026-11-26
- Amends: [ADR 0021](0021-browser-chrome-and-mac-tools.md), where the extension was not vendored.
- Specified in: `apps/extension`, `emberhut_api::chrome::extension` (the protocol's names and the
  extension's id), `emberhut_api::mac::{helper, ExtensionFolder}` and
  [docs/mac-helper.md](../mac-helper.md) (the Mac helper's extension folder).

## Context

ADR 0021 built the server's side of the owner's Chrome profiles; the extension itself stayed
outside the repository. That leaves the product's hands in the owner's browser in code this
repository doesn't build, test or review, and an extension that isn't updated with the server it
must match.

The extension runs in Chrome, which runs only JavaScript (or WebAssembly loaded by JavaScript)
in an extension's service worker and pages. It is loaded in each of the owner's profiles, may
drive the owner's logged-in sites, and must keep working while the server changes.

## Decision

### TypeScript, in `apps/extension`

- The extension is a Manifest V3 extension named "Emberhut", in TypeScript because the browser
  forces JavaScript; strict `tsconfig`, eslint as `apps/mobile` has it, vitest for the logic that
  needs no browser. esbuild bundles the service worker and the popup into `dist/`, and
  `scripts/zip.ts` writes `emberhut-extension-<version>.zip` (the version from the manifest; the same
  files always give the same bytes). It is a pnpm workspace package, so CI's js job runs its check
  (types, lint, format, tests, build, zip).
- The extension is small modules, each hiding one decision: the
  scope and blocked sites, the refused DevTools commands, the pause, the socket and its back-off,
  the page's pill and cursor, pairing, the recovery when Chrome cuts a link, the popup.
- Its behaviour: its own "Emberhut" tab group (blue, the apps' accent;
  the design tokens package is still a placeholder), only its own tabs and any popup or tab they
  open, never a blocked site, never a DevTools command that reaches past the page, Pause for 10 minutes
  and back by itself, hand-off and hand-back with notifications, the pill in an isolated world
  and a closed, aria-hidden shadow root, its own cursor, the quiet console and the
  page-closed-itself binding, re-offers after a cut or another extension's frame, sleep recovery,
  pings and back-off.
- No server address is built in. The server comes from pairing. A pairing link on the server the
  profile is already paired with pairs by itself; a link on any other server is offered first,
  because any page can show something that looks like a pairing link, and the server a profile
  pairs with is the one that drives it: as a notification and in the popup before the profile is
  paired, in the popup only once it is, each server at most once per browser session. The
  pairing message carries only the code (and the profile's old token only for the same server);
  the profile's version, account and system go only in the hello to the server it paired with.

### What the extension lets through

- Blocked sites are compared without a host's final dot, with paths' escapes decoded, doubled
  slashes made one and case ignored. The tab's address and the one it is going to are checked
  on every command, and the extension lets go of a tab before a navigation to a blocked site (in
  the page or a frame) commits.
  - A residual window remains: the let-go runs when Chrome reports the navigation
    (`webNavigation.onBeforeNavigate`), and a navigation that commits fast (from cache, or a
    same-document one) may commit first. Until the debugger is detached, scripts the server
    added to new documents (`Page.addScriptToEvaluateOnNewDocument`) may run in the blocked
    page, though no command reaches it: every command checks the tab's address first.
  - Any page Emberhut drives can make Emberhut let go of its tab by framing (or linking to) a blocked
    site. That is a denial of service only: the tab stays open and Emberhut can be handed it again.
- DevTools commands are an allow-list: those the server's engine sends a page through the
  extension (`crates/emberhut-server/src/services/browser/cdp.rs`), on the tab's own session only.
  Everything else is refused: the browser, storage of any kind, cookies, service workers,
  passkeys (`WebAuthn`), other targets and their sessions, network fetches outside the page,
  direct DOM edits, drag and drop, IME input. So is a command that names a context or a world
  (the engine names none), which keeps the server out of the extension's own isolated world
  (the pill, its Pause, the password guard's memory); events about that world never go back. A command the engine starts to use is added to
  the list with it. Only the page's own events go back (no network events), with any cookie or
  authorization header and cookie list taken out.

### A fixed id, by key

- The manifest's `key` (an RSA-2048 public key) fixes the id, `omnhmmnnbibfdohdhcbcomdhbgpehfnj`,
  in every profile and from every copy of the folder (Chrome's rule: the first 128 bits of the
  key's SHA-256, in hex written `a` to `p`). It is `emberhut_api::chrome::extension::ID`. The server
  installs through the Mac helper only a zip whose key gives it, and the helper checks the same.
  The private key is kept outside the repository; an unpacked extension doesn't need it.

### The protocol

- The extension's message names are its own (`emberhut.start`, `emberhut.hello`, `emberhut.paired`,
  `emberhut.pause`, `emberhut.resume`, `emberhut.name`, `emberhut.step`, `emberhut.handoff`, `emberhut.handback`,
  `emberhut.stopped`, `emberhut.unpair`, `emberhut.cut`, `emberhut.blocked`, `emberhut.framed`, and the new
  `emberhut.update`), in `emberhut_api::chrome::extension`, and the server speaks only these.
- The first message (`pair` or `hello`) carries `"protocol": 1`. The extension's pings are
  answered (`pong`), so a socket that died with the computer's sleep is told from a quiet one.
- The zip is served at `/chrome/emberhut-extension.zip`. The pairing page says Emberhut.

### Updated from the owner's server, through a folder the Mac helper keeps

- The extension is loaded unpacked, from one folder per computer
  (`~/Library/Application Support/Emberhut/Extension` on a Mac), shared by every profile there. The
  Mac helper keeps that folder in step with the zip the server serves: `GET /extension` says
  which version it holds, `POST /extension/install` takes the zip, checks its key and its paths,
  unpacks it beside the folder and renames it into place ([docs/mac-helper.md](../mac-helper.md)).
- The server brings the folder up to the zip (installing only when the folder is empty or
  behind) on its own, at start-up and every 5 minutes, so a fresh setup needs no manual unzip and
  no profile running the extension first; and when a profile on the Mac runs an older version than the zip. Either way it tells such
  profiles `emberhut.update`, once. The extension reloads itself from the folder
  (`chrome.runtime.reload()`), never while it holds a tab, and never twice for one version, so a
  profile that loads the extension from another folder doesn't loop. The server installs one
  version at most 3 times, 10 minutes apart, whichever path asks, tries one profile and version
  at most 3 times, 10 minutes apart, and doesn't tell a profile again that came back still older
  (the activity log says it loads the extension from elsewhere). With no helper set up, nothing
  happens; either way the app shows the profile as out of date.
- Not the Chrome Web Store: the extension's job is to drive the owner's own logins for one
  owner's server; a store listing would publish it, put a review between a fix and the owner's
  browsers, and update it on the store's schedule rather than with the server it must match.
- Not a CRX installed by policy (`ExtensionInstallForcelist` with an update URL on the owner's
  server): on macOS and Windows, Chrome honours off-store force-installs only on managed
  machines (enrolled in a device management service or joined to a domain), which the owner's
  own computers are not. Loading unpacked from a folder the helper keeps works on any computer,
  and needs no signing key at run time.

### Passwords, and final steps

- The extension never types into or fills in a password. Its guard covers the input and fill
  commands. Before `Input.insertText`, or `Input.dispatchKeyEvent` carrying text, it checks what
  has the focus from the pill's isolated world (through open shadow roots and same-site frames):
  a password field (`input[type=password]`, or autocomplete `current-password`, `new-password`,
  `one-time-code`) refuses the command, saying to hand the browser to the owner. A focus in
  another site's frame can't be read from there, so it is refused too. Before a function is run
  on an element (how the server fills, reads or acts on a field), that element is checked the
  same way, and a function on no element is refused. A field that was a password field in the
  page's top document still counts after its type is changed (the isolated world remembers it).
  The memory starts when Emberhut attaches to the tab (a field changed before that is checked as it
  is now) and follows the same element only: a page that replaces the field with a new plain
  one gets a new element, checked as it is.
  Enter and Tab aren't typing, so a form the owner filled in and handed back can be sent. Paste
  (its shortcuts by key, code or virtual key code, and editing commands on a key) is refused
  everywhere, as it would hand the page the owner's clipboard; the primary-selection paste of a
  middle click on Linux is a stated limit no key check sees; drag and drop and direct DOM edits are outside the allow-list.
  These checks run on the owner's machine, beside the server's own rules.
- Script the server evaluates in the page (`Runtime.evaluate`) is not read by the extension: it
  can't tell what a script does. That is covered by the server: its tools never put a password
  in a script, it blanks password values in everything it reads, and its step gate holds final
  steps.
- Reading stays the server's side: it blanks password values in everything it reads (ADR 0021).
- Final steps stay the server's step gate's (ADR 0021): the extension acts only on the server's
  calls, presses nothing on its own, and has no path around the gate.

## Consequences

- The extension is built, tested and reviewed here; a server and its extension change together.
- On a computer without the Mac helper (Linux, Windows), the owner reloads a new version
  themselves; the app marks the profile as out of date.
- A page whose focus is inside another site's frame can't be typed into by the assistant, and a
  function can't be run on a password field even to read it; such pages are handed to the owner,
  as a page that needs frames within frames already is.
- "Allow access to file URLs" stays off for the extension: Chrome then lets it debug no `file://`
  page, and it refuses them itself as well.
- The extension's checks of Chrome's own APIs are type and lint checks; its behaviour in a real
  browser is checked by hand, as the server's tests drive a scripted extension, not Chrome.
