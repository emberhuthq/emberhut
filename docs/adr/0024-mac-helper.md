# 0024. Emberhut Hands: the Mac helper as its own app, in Rust over objc2

- Status: Proposed
- Date: 2026-11-26
- Builds on: [ADR 0012](0012-mac-helper-backup-and-builds.md) (the helper's wire, its token
  file) and [ADR 0021](0021-browser-chrome-and-mac-tools.md) (the Mac tools on the server).
  Adds `apps/hands` to the workspace laid out by [ADR 0003](0003-monorepo-layout.md).

## Context

The server already speaks to a helper on the owner's Mac (`services/mac.rs`, `services/hands.rs`,
the `mac_*` tools and `mac_record`): `POST <url>/<path>` with a JSON body and a bearer token, for
`/health`, `/lock`, `/apps`, `/which`, `/open`, `/snapshot`, `/describe`, `/press`, `/set`,
`/keys`, `/menu`, `/screen/start|shot|act|stop` and `/record/start|stop|status`. The reference
helper that answers them is a Swift program in its own signed app with a launch agent; it is the
only program on the Mac holding Accessibility and Screen Recording for the assistant.

The browser extension (built separately) is loaded unpacked from one folder on the Mac, and
something there has to keep that folder up to date.

Two questions: does the helper stay a separate app or fold into the desktop app
(`apps/desktop`), and is it written in Rust or Swift?

## Decision

### A separate app, "Emberhut Hands"

Emberhut Hands is its own app bundle, `Emberhut Hands.app`, bundle id `com.emberhut.hands`, run at login by
the launch agent `com.emberhut.hands` (kept alive, in the owner's graphical session only).

- **The grants belong to the bundle that makes the calls.** macOS gives Accessibility, Screen
  Recording and the microphone to one signed bundle. Folded into the desktop app, those powers
  would belong to a large program that renders web pages, plays media and updates itself; a
  bug anywhere in it would run with them. A small helper whose only job is these calls keeps
  them in a few thousand lines that are easy to read.
- **It runs when the app doesn't.** The assistant works on the Mac while the desktop app is
  closed or being updated (ADR 0005's updater swaps the app's bundle); a launch agent keeps the
  helper up across logins and crashes.
- **The server reaches it the same way either way**: over HTTP, as ADR 0012 has it. Nothing on
  the server changes.
- The cost is a second bundle to build, sign and install, and a second row in each privacy list
  (only Emberhut Hands asks for these grants; the desktop app asks for none of them).

### Rust over objc2, no Swift

The helper is the binary crate `emberhut-hands` in `apps/hands`, written in Rust over objc2's
bindings. Every system call it needs has an Objective-C or C surface those crates bind:

| Need | Framework (crate) |
| --- | --- |
| The apps, opening one hidden, the one in front | AppKit `NSWorkspace`, `NSRunningApplication` (`objc2-app-kit`) |
| An app's elements: read, press, set, focus | ApplicationServices `AXUIElement` (`objc2-application-services`) |
| Keys to one app, the real mouse and keyboard, idle time | Core Graphics `CGEvent`, `CGEventSource` (`objc2-core-graphics`) |
| Window ids for a picture, the lock state, Screen Recording's grant | Core Graphics `CGWindowList`, `CGSession`, `CGPreflightScreenCaptureAccess` |
| The bar with Stop | AppKit `NSPanel`, `NSButton` (a small `define_class!` target) |
| Recording the Mac's sound and the microphone | ScreenCaptureKit `SCStream`, AVFoundation `AVAssetWriter`, Core Media (`objc2-screen-capture-kit`, `objc2-av-foundation`, `objc2-core-media`) |
| Sending recordings to the server | Foundation `NSURLSession` (the Mac's own TLS and trust settings; no TLS stack in the helper) |

- **Locking the screen** is the one call with no Objective-C or Swift surface: the menu bar's
  Lock Screen is the C function `SACLockScreenImmediate` in the private `login.framework`. The
  reference helper finds it with `dlopen`/`dlsym` from Swift; the Rust helper does the same with
  `libc`. Writing that one call in Swift would gain nothing, so no Swift is used at all.
- **Pictures** are taken by the system's `screencapture` (a window by its id, covered or
  hidden, or the main screen) and sized and made JPEG by `sips`, both run by absolute path, as
  the reference helper does. ScreenCaptureKit's screenshot API would add an ImageIO encoding
  step through more unsafe code for the same pictures.
- Every unsafe call sits in a small module of `apps/hands/src/mac/` with
  `#[expect(unsafe_code, reason = …)]`, as the desktop app does. The macOS code is behind
  `#[cfg(target_os = "macos")]`; on other systems the binary only says it runs on macOS, so the
  workspace builds and its tests run on Linux.

### One seam, tested anywhere

`emberhut_hands::desktop::Desktop` is every call the helper makes to the Mac (the apps, an
element's attributes and actions, input events, pictures, idle time, the lock, the bar). The
requests, the rules and the HTTP layer sit above it, and the tests drive them with a fake
desktop held in memory; the recorder's capture (`AudioSource`) and its sending (`Uploader`) are
seams of their own. `mac::MacDesktop` is the real one.

### The wire: the reference helper's

- Every path the server calls is served with the reference helper's bodies and answers
  (`{"text": …}`, `image` as base64 JPEG, `/which`'s `{name, bundle, running}`, `/describe`'s
  `{kind, label, secret, actions, app}`, `/health`'s `{ok, accessibility, screen, driving,
  recording, idle, locked, stopped}`); a refusal is `{"error": …}` with 400, 403, 404 or 409.
  The server passes the words to the agent.
- `POST /release` is new: it lets go of the app held (below). `/chrome/open` and
  `/chrome/profiles` are not served: ADR 0021 doesn't port reopening a profile through the
  helper.
- It listens on `127.0.0.1:8794`, and on one more address when configured (`--listen`, for a
  server on another machine). That address must be private (10/8, 172.16/12, 192.168/16, the
  carrier-grade NAT range 100.64/10 that private meshes use, or IPv6 `fc00::/7`), and only
  callers on this Mac or in that address's range are answered, as the reference helper checked
  its callers' addresses. Every request also carries the bearer token, compared in constant
  time. The token is read from a file (never a flag, so it
  isn't in the process list): `~/Library/Application Support/Emberhut/Hands/token` unless
  `--token-file` says otherwise, written (owner-only) with a new random token on the first
  start; the server's `--mac-helper-token-file` holds the same token. A new token file is
  written whole under another name and linked into place, so a crash never leaves half a token;
  a token file others
  than the owner may read is refused.
- A picture (a window's or the screen's) is a JPEG of at most 6 MB, sent as base64; the server
  reads a Mac tool's answer up to 16 MiB to hold it (`services/mac.rs`).
- The recorder speaks the reference helper's wire both ways: `/record/start {id, title}`,
  `/record/stop`, `/record/status`, and, once a recording stops, each track to the server's `POST
  /api/calls/in` (the last with `last=1`), every ten seconds of each track to `POST
  /api/calls/live`, with a device token read from `--server-token-file`. A call not sent waits in
  `~/Library/Application Support/Emberhut/Hands/calls` (owner-only) and is tried again every five
  minutes; one the server refuses outright (a 4xx other than a timeout, too many requests or a
  refused sign-in, 401 and 403, which the owner can still fix), or that failed 48 times (about four
  hours), is set aside in `calls-unsent` (under a name of its own if one of the same id is there)
  and named in `/record/status`. Without a server configured, recording is refused. The server's URL
  is https, or plain http only to a loopback, private or `.local` address (App Transport Security
  allows local networking only). A capture that hasn't started in 15 seconds is stopped; one the Mac
  stops by itself ends that recording (never a newer one: each recording is numbered), its tracks
  are sent, and the bar says so. A recording, like the screen, is kept only once the bar with Stop
  is up.

### The rules, in the helper too

The server keeps these rules; the helper keeps them again, so a bug or a stolen token on the
server's side still can't break them.

- **Never a password.** A password field (role or subrole `AXSecureTextField`, or a text field
  whose label says it wants a password or a code, by `emberhut_core::browser::is_secret_label`) is
  never typed into, set or read: a snapshot lists it without its value (the reference helper
  read the value to show dots; this one never asks for it), and `/set` refuses it. Typing, in
  one app or on the whole screen, checks what has the keyboard focus before the text, and before
  any key that isn't only navigation (Tab, Escape, the arrows, Home, End, the pages). After a
  Tab, Return or new line in the text the focus must move (it is looked at for up to 300 ms)
  before the rest is typed, and the rest is then checked where it went; only a new line in a
  multi-line text area stays where it is. Typing stops at a password field, and is refused
  when the app doesn't say what has the focus: only the app's "no value" answer means nothing
  has it, any other answer (an unsupported attribute, a timeout) is unknown. The refusal tells
  the agent to hand it to the owner. One request types at most 4,000 characters.
- **Never Chrome.** Chrome and the browsers built on Chromium are refused by the app found (its
  bundle id or name, by `emberhut_core::browser::mac_app_is_chromium`, the server's own rule), however
  they were named: the owner's Chrome is reached only through the extension.
- **Never these apps**: other terminals, System Settings, Keychain Access, Passwords and password
  managers, Script Editor, Automator, Shortcuts, Activity Monitor, Disk Utility, Migration
  Assistant, Screen Sharing, the login window, the system's consent and permission prompts
  (CoreServicesUIAgent, the user notification center, the accessibility warning, the
  LocalAuthentication agent) and its controls (Control Center, the menu bar's system items), and the
  helper itself (the reference helper's lists, with more terminals, password managers and system
  prompts); Terminal is only looked at; the Apple menu is never used. Emberhut's own desktop app
  (`com.emberhut.desktop`) is never used either: that is where the owner answers the assistant's Go
  ahead cards, so an assistant driving it could answer its own.
- **The app rules hold on the whole screen too.** A step is refused while one of those apps (or
  Terminal) is in front, and a click is refused on a window such an app owns, or where the Mac
  doesn't say which app is under the point: the server can't see what is under a click, so the
  helper asks the Mac.
- **One app at a time.** The helper holds one app: each step in an app takes it or keeps it, and
  its snapshot's refs belong to it alone. A step in another app is refused while the held one
  was used in the last 30 seconds (the words say which app is held and when it frees), unless
  the held app quit or `/release` let it go. The reference helper kept refs for every app and
  had no such rule; the hold makes "one app at a time" true on the Mac itself while letting an
  agent move on once it is done.
- **A bar with Stop.** While the whole screen is used, and while a call is recorded, a panel at the
  top of the main screen (above other windows, on every Space and over full-screen apps, never
  taking the focus) says so ("Emberhut is using your screen: …", "Emberhut is recording: …") with Stop. A
  session starts only once the bar is up (the start waits up to 3 seconds and is refused otherwise),
  and the main thread always shows the latest state, worked out under one lock, so a late update
  never brings back an old bar. Stop ends both at once, in the middle of a step too: the input is
  posted event by event, and each event first checks the session still runs. The owner's own mouse
  or keyboard ends the screen session the same way. The assistant posts from an event source of its
  own and counts the moves, presses and scrolls it posts; the Mac counts every one in the login
  session (`CGEventSourceCounterForEventType`), so any it counts beyond the assistant's is the
  owner's, even a key pressed right after one of the assistant's. Between steps, recent input once
  the assistant's own has settled is the owner's too. Its time (at most 60 minutes) ends it as well.
  The next `/screen/*` request answers 409 saying why it ended (the owner pressed Stop, or used the
  mouse), and `/health`'s `stopped` says how long ago. The helper refuses a new start for a minute
  after the owner ended one (the server asks the owner again within ten minutes), and while the
  screen is locked.

### The extension's folder

- `GET /extension` answers `{"version": "1.2.3", "path": "…"}` from the folder's
  `manifest.json`, or `{"version": null}`.
- `POST /extension/install` takes the zip as the body (`Content-Type: application/zip`, at most
  20 MB) and answers `{"version", "path"}` or 400 `{"error"}`. The zip is refused unless its
  manifest's `key` gives the extension's fixed id (`extension::id::EXTENSION_ID`, the id
  emberhut-api names), every entry is a plain file or folder inside the folder (no absolute paths,
  no `..`, no links, nothing encrypted), and it unpacks to at most 100 MB in at most 5,000
  entries. A zip whose files sit in one top folder is taken from inside it.
- It is unpacked into a new folder beside `~/Library/Application Support/Emberhut/Extension` and
  swapped into place in one rename (an exchange with the old folder, which is then removed), so
  Chrome never loads half an extension; a failure leaves the old folder as it was.

### Building and installing

`cargo xtask bundle-hands` builds `emberhut-hands`, assembles `target/bundle/Emberhut Hands.app`
(`Info.plist`: no Dock icon, the microphone's purpose, macOS 15 or later), signs it with the
hardened runtime and its entitlements (the microphone) ad hoc or with `--sign` /
`EMBERHUT_SIGN_IDENTITY` as `bundle-macos` does, and zips it. `--install` puts it in
`~/Applications/Emberhut Hands.app` (an app already there is replaced only when it is this helper),
writes `~/Library/LaunchAgents/com.emberhut.hands.plist` (with `--listen`, `--server-url` and
`--server-token-file`, made absolute, as its environment): the running agent is stopped first,
the bundle swapped, and the agent started again with `launchctl` (tried a few times, as launchd
may refuse a bootstrap right after a bootout); if the swap fails, the old helper is put back
and started again. What the helper would refuse at start (a public `--listen` address, plain
http to a public server) is refused at install time, by the same rules
(`emberhut_core::private_network`). A Developer ID
signature keeps the Mac's grants across rebuilds; an ad hoc one loses them at every build.

### Beside the reference helper, and rolling back

Emberhut Hands installs beside the reference helper and touches nothing of it: a different bundle
id, launch agent label, port (8794) and token file. The server moves to it by pointing
`--mac-helper-url` and `--mac-helper-token-file` at it. Rolling back is unloading its launch
agent (`launchctl bootout gui/$(id -u)/com.emberhut.hands`), removing `~/Applications/Emberhut
Hands.app`, its plist, its folder `~/Library/Application Support/Emberhut/Hands` (token and unsent
calls) and its log `~/Library/Logs/Emberhut Hands.log`, and turning it off in Privacy & Security;
the server points back at the reference helper. The extension's folder is left for the
extension.

## Consequences

- The workspace's members gain `apps/hands`. Linux CI builds and tests it (the rules, the
  HTTP layer, the zip checks, the recorder's logic); only a Mac exercises the objc2 calls, the
  bar and recording.
- The owner grants Accessibility and Screen Recording (and the microphone, asked at the first
  recording) to Emberhut Hands once.
- The helper's routes are a private API between the server and the Mac, not the server's
  documented one: they are registered with axum directly, and no OpenAPI document lists them.
- An agent that switches apps within 30 seconds of its last step waits, or calls `/release`
  first once the server learns to.

## Not ported

- **The reference helper's menu bar recorder**: its status item with a face and a timer, its
  menu (record with a title, the live copilot switch, the latest transcripts), the ⌃⌥⌘R hot
  key and the live transcript panel. A recording starts from the app's Record button or the
  `mac_record` tool, and the bar with Stop shows while it runs.
- **Reopening the owner's Chrome profiles** (`/chrome/open`, `/chrome/profiles`), as ADR 0021
  decides.
