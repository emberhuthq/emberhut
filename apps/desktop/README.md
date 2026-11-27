# Emberhut, the desktop app

`emberhut-desktop` is emberhut's desktop client, built with [GPUI](https://www.gpui.rs) and
[gpui-kit](https://crates.io/crates/gpui-kit). It talks to an emberhut server over its HTTP/SSE API:
the chat in each room, the asks waiting on you (Needs you), agents and their approval requests, your tasks, what the assistant has
scheduled, the machines it works on, your calendar, your quick notes and diary, and Settings (appearance, the assistant's MCP servers and skills, the
Chrome profiles it works in, what it knows about you, sign-in, the server).

## Run it

```sh
cargo run -p emberhut-desktop --release
EMBERHUT_URL=https://emberhut.example.com cargo run -p emberhut-desktop --release
```

On Linux the build needs GPUI's system libraries; see [CONTRIBUTING.md](../../CONTRIBUTING.md).

### Which server

The first of these that is set:

1. the `EMBERHUT_URL` environment variable;
2. `server` in the app's `config.json`, e.g. `{ "server": "https://emberhut.example.com" }`;
3. `http://localhost:18990`, a server on this machine (the server's default port).

An address must start with `http://` or `https://`; one that doesn't is skipped with a warning.
When neither names one and this device has no session, the app opens on the first-run setup
(see "Setting up your ember") instead of waiting on a server that may not be there.

### The hut on this computer

The app can run the server itself, on this computer, so a new person needs no terminal: the
hut (`src/hut/`, [ADR 0030](../../docs/adr/0030-the-hut-inside-the-app.md)). `LocalHut` finds
the `emberhut-server` binary, starts it on `127.0.0.1:18990` (the next free port when another
program has that one), makes a pairing code for this app with the server's own `pair --json`,
and keeps it lit after the app closes or from login when asked. The onboarding calls it; nothing
starts until then.

- **The binary**: in the macOS bundle at `Emberhut.app/Contents/MacOS/emberhut-server` (put there
  by `cargo xtask bundle-macos`), else beside the app's executable (`target/<profile>/` for a
  build from source: `cargo build -p emberhut-server` next to the app), else the file
  `EMBERHUT_SERVER_BIN` names. On Linux there is no bundle: use one of the last two.
- **Its files**: `~/Library/Application Support/Emberhut/Hut/` on macOS,
  `$XDG_DATA_HOME/emberhut/hut/` (`~/.local/share/emberhut/hut/`) elsewhere: `emberhut.db`,
  `machine.key`, `secrets/` and `hut.json` (its port, the server build it last started, how it
  stays lit), all owner-only. Its log: `~/Library/Logs/Emberhut/hut.log`, or
  `$XDG_STATE_HOME/emberhut/hut.log`.
- **Keeping it lit**: the launch agent `com.emberhut.hut` on macOS, the systemd user unit
  `emberhut-hut.service` on Linux, else the app's child. A newer server in the app (an update)
  restarts the hut once.
- **The real run**: `hut::real_tests` starts a built server through `LocalHut`, pairs with its
  code over `POST /api/auth/pair` and stops it. It is ignored by default and refuses a `HOME`
  outside the temporary folder; its module says how to run it with `env -i`.

### A fallback server

`config.json` can name a fallback server, which the app turns to by itself when the server
stops answering, and say which of the two speaks the wire dialect the app was built with
(`EMBERHUT_WIRE_DIALECT`, see `emberhut-client`'s `dialect` module; `dialect` is `true` when
left out, and a build without a dialect ignores it):

```json
{
  "server": "https://new.example.com",
  "dialect": false,
  "fallback": { "server": "https://old.example.com", "dialect": true }
}
```

- While on the server, the app asks its `GET /health` every 15 s. After 3 failed checks in a
  row, or at start-up a first check and 2 quick retries that fail, every request goes to the
  fallback server, and a bar over the chat says the server isn't answering and that you're on
  the fallback server.
- The app never turns back by itself: what you do on the fallback server stays there, and
  turning back unasked would split your work between two servers. The bar's **Try again** asks
  the server and turns back only if it answers. A relaunch stays on the fallback server too
  (`fallback.json` remembers it) until you turn back.
- Each server has its own session. On macOS that is a Keychain item per server address, so
  moving a server's address from `server` to `fallback` keeps it signed in as the fallback;
  the new server is paired as usual. Elsewhere the fallback's session is in `fallback/`.

### Pairing without the window

`emberhut-desktop --pair ABCD-EFGH` (on macOS, `Emberhut.app/Contents/MacOS/emberhut-desktop --pair …`)
pairs this device with the server, never the fallback, using a one-time pairing code, without
opening a window. It keeps the session exactly where the app does (the Keychain on macOS), asks
the server whether it takes the session (`GET /api/auth/status`), and exits, 0 when it does. Use
it for a machine whose screen isn't at hand. `--pair -` reads the code from the standard input,
so it stays out of the process list. `--forget-session` removes this device's session with that
server from where the app keeps it: the session, its tokens and its install id, so the next
pairing is a new install. On macOS that is the server's Keychain item (and a `session.json`
waiting to move into it), removed without any dialog. A file-based keychain removes an item only
with its owner's right, which another build of the app may not hold; then the item is
overwritten with a mark that every later read takes for no item at all.

### The app's files

| Platform | Directory |
| --- | --- |
| macOS | `~/Library/Application Support/com.emberhut.desktop` (the bundle id, as macOS expects) |
| Linux | `$XDG_CONFIG_HOME/emberhut` (usually `~/.config/emberhut`) |
| Windows | `%APPDATA%\emberhut` |

- `config.json`: your settings: the server, and the fallback server (see above).
- `session.json`: this device's session, kept by `emberhut-client` (readable by you only). On
  macOS the session lives in the Keychain instead (a generic password of `com.emberhut.desktop`
  per server, never synced); a `session.json` from an earlier launch moves into it once. As
  with the file, a newer app's session is neither read nor overwritten, and an item that isn't
  a session is removed once, after which this device signs in again.
  A test run never touches the Keychain: the session is kept in `test-session.json` (readable
  by you only) and the account is signed in for that launch only. A test run is one with
  `EMBERHUT_TEST` set to anything but `0`; unset, a debug or automation build is one (set
  `EMBERHUT_TEST=0` to use the Keychain from one) and a release build isn't. Test harnesses and
  screenshot runs set `EMBERHUT_TEST=1`. A user with no default keychain (a `HOME` of its own,
  say) keeps the session in `session.json` instead, as macOS would otherwise ask, in a dialog,
  to make one.
- `setup.json`: how the first-run setup ended (`finished`, or `skipped` when "Light it" was put
  off), so it doesn't show again.
- `theme.json`: the last theme the server sent, so the app starts in your colours offline.
- `fallback.json`: that the app is on the fallback server, and for which server (see above).
- `layout.json`: the report panel's width, as a share of the window, so it opens as you left it.
- `view.json`: how the window was left: the open room (the next launch opens it), whether the
  sidebar's Rooms section is folded, and the sidebar's width. It is apart from `layout.json`
  because each file has one writer (the app's state here, the window's report panel there), so
  neither write can lose the other's keys.
- `chat-put-away.json`: the ask cards and Main's line you put away over the composer, so they
  stay away until something new brings them back.
- `window.json`: how you left the window (its size and place), which of the sidebar's
  sections are folded (More, Recent sessions, Projects, Goals), the width you dragged the
  column to, the size you last gave a file's own window, the calendar (a day, a week or a
  month), and each window of its own (Calendar, Sessions, Party, Dark, Browser, Diary, Quick
  Note): where you left it, and which were open when the app quit. Those open again at launch,
  behind the main window once you are signed in, except a quick note.
- `diary.json`: your quick notes as last read, with the ones written or changed here that the
  server hasn't taken yet (they are sent when it can be reached, so a note works offline), the
  ones the server refused (kept with its reason, so their words are never lost), and the quick
  note left unsaved.
- `last-launch-error.txt`: why the last launch failed, if it did (an app started from Finder
  or the Dock has no terminal to say it on).
- `update-check.json`: when the app last read the release feed, how old the feed was, the
  build whose update bar you closed, and the payloads that failed verification (macOS).
- `update.json`: an update on its way in, as the old and the new copy tell each other how it
  goes; after a rollback, the build not offered again for a week (macOS).

What the app keeps only to show a room at once lives in the platform's cache directory
(`~/Library/Caches/com.emberhut.desktop`, `$XDG_CACHE_HOME/emberhut`, `%LOCALAPPDATA%\emberhut`), one
folder per server: each room's newest 200 messages, so the first visit to a room shows them
before the server answers. At launch they wait for the server's first state, as the server may
no longer let this device in. Files fetched from the server (pictures, reports, anything
viewed, saved, shared or copied) are kept there too, in the server's `files/`, each under its
real name in a folder of its own address, so a file opened again comes at once; one nothing
used for a day is cleared out at the next launch. Like every file here they are yours only
(0700 folders, 0600 files, on Unix), and the server's folder is removed when this device is
signed out or the server refuses it. The system may clear it; the server has it all.

### Setting up your ember

The first time, the window shows "Set up your ember" instead of the app: the ember on the left,
asleep and grey, and five steps on the right, each with a short name and one plain line. The
first two get it going; the other three are optional and say so.

1. **Give your ember a home.** Here, on this computer (recommended), or on another computer
   already set up. Another computer opens the pairing form ("Find your ember"), which pairs with
   the address the app talks to. Here goes through a port, `store::setup::LocalHome`: whether
   Emberhut can live on this computer, starting it, and a pairing code from it. The app wires
   its implementation in `app.rs`; until it can start Emberhut itself, `DevHome` uses one already
   answering at the app's address (on this computer) and asks for its code, and otherwise says
   "This build can't start Emberhut here yet." beside the other computer's form.
2. **Light it.** The provider picker of Settings › Your ember, opened as the step shows (so a
   local Ollama is looked for). A save that goes through finishes the step.
3. **Let it reach you.** A pairing code made as the step opens, as a QR code (Settings ›
   Devices' Pair another device). A device that signs in while it shows finishes it.
4. **Show it your world.** Calendar, Gmail, Slack and Chrome, each as `GET /api/setup` says
   (`mac`, `gmail`, `slack`, `chrome`). Chrome's Connect opens the Chrome profiles' sheet for a
   new profile; the others can't be connected from the setup yet and say they wait in Settings ›
   Connections.
5. **Share your ember.** Where sharing is (Settings › Sharing, an early preview), and Done.

Then "Your ember is lit." sums up what it is now, with Say hello (Main) and Share your ember
(Settings › Sharing). The ember glows a little more with each step done: a spark once it has a
home, lit once it thinks, warmer once it can reach you or see your world, brightest at the end.
Its glow breathes slowly, and holds still under Reduce Motion.

The setup shows on a fresh install (no server named, no session), and for a device signed in to
an ember that isn't lit: `GET /api/setup`'s `provider` is `missing` or `off`, read once a launch
(a server without the route is taken as lit). Then it opens on "Light it", the home step done.
Once it shows, it stays until it ends: Say hello, or Later on "Light it". How it ended is kept in
`setup.json` and it never shows again; an ember that isn't lit after that gets a quiet line at
the top of the chat with Light it, which opens the picker in Settings.

Where the setup begins is one constant, `store::setup::DIRECTION`, read by `first_screen`:
`HomeFirst` (built); `AccountFirst` and `ShowFirst` begin as it does until they are built.

### Signing in

When the server wants this device signed in, the app asks for a one-time pairing code
(`ABCD-EFGH`, good once, for 10 minutes) made on a device that is already signed in, and trades
it for this device's own session. `emberhut-client` keeps the session and renews it, one renewal
at a time. Settings › Devices lists the signed-in devices (each can be signed out there, once
confirmed; signing out this device ends its session here at once) and makes a pairing code for
another device. While the server only reports sessions, a device
without one is asked to sign in once a launch and can say Not now; once it enforces them,
signing in is the only way in. A device the server doesn't let in at all (a 401 without a
sign-in: not on your network) sees a screen saying so, and nothing it held, until it is.

A pairing link (`https://emberhut.com/pair#id=…&code=…&route=…`, also without its scheme or as its
fragment alone) can be pasted in the same field. The app pairs only when the server it talks to
is one of the link's routes, so the code never goes to a server the link doesn't name; a link
without routes is refused, since it reaches its machine only through the relay, which the app
has no transport for yet. It keeps the session only when the answer carries the machine's key
with the link's id; otherwise it says "This server didn't prove it is the machine you meant."
Pair another device shows the code's link as a QR code with the code under it, when the server
gives one for that code.

### The account

The optional emberhut.com account (docs/adr/0007-connectivity.md, section 5) lists your machines
on every device you sign in on and lets them use the emberhut.com relay. It never lets anyone into
a machine: devices still pair with a code. It exists only in a build that sets all four of
these when it is compiled (read with `option_env!` in `src/account/switch.rs`):

| Variable | What it is |
| --- | --- |
| `EMBERHUT_ACCOUNT_ISSUER` | the account provider's issuer (`https://…`); OIDC discovery is at its `/.well-known/openid-configuration` |
| `EMBERHUT_ACCOUNT_CLIENT_ID` | the native OAuth application's public client id (no secret) |
| `EMBERHUT_DIRECTORY_URL` | the directory's origin, before `/v1` (docs/directory.md) |
| `EMBERHUT_RELAY_DOMAIN` | the relay's domain, which "Add this computer"'s link names |
| `EMBERHUT_ASSOCIATED_DOMAIN` | optional: `emberhut.com`, set only by a build signed with that associated domain |

**Self-hosters and builds from source leave them all unset.** Such a build has no account at
all: no Account group in Settings, no request to a provider or a directory, nothing in the
Keychain. Some set and others not is a mistake in the build: the log says so and the account
stays off. `http://` is taken only for this computer (a provider or directory on a developer's
machine). A self-hoster who runs their own account service sets their own values here, as
ADR 0007 section 7 describes.

With the account on, Settings has an Account group:

- **Sign in.** Every build signs in with the OAuth device grant (RFC 8628): a sheet shows the
  code, Open the sign-in page opens the provider's page in the browser, and the app polls at
  the interval given, slower after `slow_down`, until the code is confirmed, declined or runs
  out. A build that claims the associated domain (`EMBERHUT_ASSOCIATED_DOMAIN=emberhut.com`, the
  release bundle with its provisioning profile) uses the system's sign-in window on macOS 14.4
  and later instead: an `ASWebAuthenticationSession` with PKCE (S256), a random `state` and the
  callback `https://emberhut.com/oauth/callback`. Scopes are `openid offline_access`.
- **The tokens** live in one login Keychain item (service `com.emberhut.desktop.account`, never
  synced), never in `session.json`, and are renewed with the refresh token a minute before they
  run out. Signing out of the account touches no device session, and signing a device out
  touches no account. Linux and Windows have no keychain support yet: the account works there
  until the app quits, and Settings says so.
- **Your machines** from the directory, connected or when last seen, each with Unlink. They
  are fetched when the account signs in and while Settings is in front, at most every 30 s.
- **Add this computer to your account** asks the server on this computer (over loopback, with
  this device's session) to sign a link statement for the account, then posts it to the
  directory with the account's token; the server never sees that token. It works only when the
  app talks to a server on this computer. The directory's refusals are said in plain words
  (`linked-elsewhere`, `account-full`, a clock more than 5 minutes off).
- A 401 that one renewal can't fix signs the account out; a busy or unreachable directory (429,
  503) asks to try again later and never signs out.

### Logging

One logging stack: `log` with `env_logger` writing to stderr. `RUST_LOG` if set, else warnings
and the app's own news. emberhut-client logs through `tracing`; the app depends on `tracing` with its
`log` feature and installs no tracing subscriber, so the client's events become `log` records and
reach the same logger on every platform. `emberhut_client::LOG_DIRECTIVES` always go last, so the
HTTP library's unredacted wire trace never reaches the log.

### The app lock

On macOS every window opens covered by "Emberhut is locked" with Unlock, which asks for Touch ID or
the Mac's password (LocalAuthentication's device-owner check); the system's sheet shows at once.
Back after more than 5 minutes with no window on screen, the app locks again, and while no
window is on screen (hidden or minimised) the cover is up, so the Dock's and Mission Control's
snapshots show nothing. A Mac with no password set opens without asking. Linux and Windows have
no such check, so the app never locks there. A window counts as on screen from the moment it
opens until the system says otherwise, or a second later when its visibility is read again from
the system, so a window opening behind other apps (`--background`) isn't covered while the system
is still working out whether it shows, and one that really opens out of sight (hidden, on another
Space, on a sleeping display) is covered then. Touch ID or the password is asked for only once a
window is known to show or the app is in front. An unseen run (see Command line) has no lock and
no cover at all.

### Accessibility

VoiceOver reads the app through GPUI's AccessKit tree. GPUI reports an element only with an id
and a role, so the app's own controls are named in `ui/a11y.rs`'s terms: icon buttons by their
tips (Send, Attach, Stop Emberhut, Dictate, Reply, Back, Close), a room, a task or a Settings row read
as one line ("Main, 2 need you, 3 new, working"), folded sections as open or folded, the chosen
colour preset as selected and Paper as the default, a Needs you card as a group named by its
title. The toast, the working line ("Emberhut is working: …") and a file's download status are
announced as they change. Decoration (status dots, the avatar, icons beside a label) has no role
and stays out of the tree. The main controls carry stable identifiers (`send`, `attach`, `stop`,
`dictate`, `toast`, `undo`, `unlock`, `appearance row`, `preset blue`), the macOS
`AXIdentifier`, for UI automation. Markdown (a message, a task's text) reads as a document.

Every page and window is named the same way: a project's, a goal's, a person's and the weekly
report's rows, the spotlight's results (the picked one selected), the sessions and a session's
page (its reply box, its folds, the New session sheet as a dialog), the jobs and a job's page,
an environment's page and the usage card, the file viewer and the report panel's header, the
scheduled list, the diary and the quick note, the calendar (its days, chips and events in full,
the chosen mode selected, its title announced as it moves), Party, Dark and the browser window.
Status lines and problems there (a session's state, a party's, who drives the browser, a file
coming down) are announced as they change. `ui/a11y/views.rs` reads some of it back from a test
window.

Arabic reads from the right on those pages too: titles and short lines by most of their letters,
with a direction mark first (`ui/reads.rs`), and longer texts (a note, a session's answer, a
party's or the Dark room's lines) from the right edge when their first letter is Arabic. With
Reduce motion on, everything that moves on its own holds still: the app's own marks, and every
GPUI and gpui-kit animation (a job's pulse, a room's working dot, the kit's spinners), since
`motion.rs` sets GPUI's own reduce-motion flag too.

## Command line

| Flag | What it does |
| --- | --- |
| `--page needs\|tasks\|machines\|settings\|scheduled` | open that page beside the chat |
| `--room ID` | open that room's chat (the room open at the last launch when not given; Main the first time) |
| `LINK` | open an `emberhut://` link (see Links) |
| `--task ID` | open a task |
| `--window NAME` | open a window of its own (`browser`, `calendar`, `sessions`, `dark`, `diary`, `party`, `note`) |
| `--party ID` | open one party, in the Party window |
| `--size WIDTHxHEIGHT` | the window's size in logical pixels (default: as you left it, `1100x760` the first time) |
| `--no-prompt` | don't ask to show notifications at launch, and open without the app lock (macOS) |
| `--report NAME=PATH` | open a report from the server beside the page |
| `--sign-in` | show the sign-in screen whatever the server says (screenshots) |
| `--setup [home\|light\|reach\|world\|share\|lit]` | show the first-run setup on that step (home when none is named), or its "lit" screen, whatever the server says (screenshots) |
| `--theme PRESET`, `--appearance light\|dark\|system` | show a theme without saving it (screenshots) |
| `--background` | open without taking the keyboard focus from the app in front |
| `--display N` | open on that display (0 is the first) |
| `--unseen` | run unseen (see below), as `--drive`, `--bench` and `--self-check` always do |
| `--on-screen` | in an unseen run, open the windows on a display, still without the focus: for a private display captured whole (Xvfb) |
| `--opaque` | draw every surface opaque, without the sidebar's material (the "before" look) |
| `--flaky` | debug builds only: the state poll fails as if the connection dropped, 6 s of every 20, to see what reconnects do |
| `--task ID` | open that task's page, as a link to it in a message does |
| `--settings main\|ember\|appearance\|devices\|connections\|sharing\|privacy\|advanced\|account\|chrome-profiles\|about-you\|credentials\|archived-rooms\|machines` | open Settings' front door (`main`), with that page over it (archived rooms: Privacy, unfolded) (screenshots) |
| `--chrome-profile ID` | open Settings on that Chrome profile's sheet (screenshots) |
| `--environment ID` | open Settings with that environment's page over it (screenshots) |
| `--session ID` | open a session's page beside the chat |
| `--spotlight` | open with the spotlight up (screenshots) |
| `--project ID` | open a project's page beside the chat |
| `--self-check` | start with fresh, empty files and no server; exit 0 once the window has drawn its first frame, 1 after a minute (a release's smoke run) |

`emberhut-desktop --help` lists them all.

### Unseen runs

A run no one watches is unseen: one the automation drives (`--drive`, `--bench`), the smoke run
(`--self-check`), or one asked for with `--unseen`. The person at the computer may be working
while it runs, so in an unseen run every window the app opens (the main ones, the windows of
their own, a file's window) opens without the focus and out of sight. The app never comes to the
front and counts as in front all the same (rooms are read and polled as they would be), no
window goes full screen, notifications aren't asked for, there is no app lock, and the windows'
frames and which were open aren't kept for the next launch. If macOS can't be set up for this,
the run stops with an error rather than show a window. Every window opens through
`shell::unseen::open_window`, and `clippy.toml` refuses GPUI's own calls to open a window, bring
the app or a window forward, or go full screen anywhere else.

Out of sight differs by platform:

- macOS: a window no display shows gets no frames (GPUI's display link follows the window's
  screen), so each window stays on a display, below the desktop picture
  (`kCGDesktopWindowLevel - 1`), with clicks going through it, hidden by Mission Control, out of
  ⌘\` and the Window menu, without a shadow. It opens hidden and is ordered in only once it is
  all of that, so it never shows at a normal level. GPUI's window classes say they are visible
  (it stops drawing a window it thinks is covered), and GPUI's application class ignores every
  call to activate it (the report's web view makes one too); an activity keeps App Nap off.
  `screencapture -l WINDOW_ID` captures a window by its id, so screenshots still work.
- X11 and Windows: each window is beside every display (left of the leftmost one), whatever
  `--display` says (`--size` still sets its size). It draws as usual, but a grab of the whole
  display (`ffmpeg -f x11grab`) doesn't show it: add `--on-screen` there, which keeps the
  windows on a display, still without the focus.
- Wayland, and X11 under a desktop's window manager: a client can't place its windows, nor keep
  them from the focus, for sure. Run unseen runs on a private display (Xvfb) instead.

## Links

The app opens `emberhut://` links: `emberhut://chat` brings it forward, `emberhut://chat?room=N` opens a
room, `emberhut://chat?text=…` starts a message (percent-encoded), `emberhut://task/N` a task,
`emberhut://needs`, `tasks`, `machines`, `settings`, `scheduled` and `approvals` a page, `emberhut://approval/N`
an approval request, and `emberhut://browser`, `calendar`,
`sessions`, `dark`, `diary`, `party` (`party/N`) and `note` a window of its own. The macOS
bundle registers the scheme; on Linux a link is the command line's `LINK`.

## Windows

Pages open in a column beside the chat, which keeps its place; Back returns to the page under
the one shown and Esc closes the column. The column's left edge drags it wider or narrower
(double-click: the standard width). File › New Window opens another main window on the
same chat (the room is every window's); each window has a column of its own. On macOS the app keeps running when its last window closes, and the Dock icon opens
one again; elsewhere closing the last window quits. With every window out of sight nothing is
polled.

Some places are windows of their own, one each: the Calendar (Day, Week and Month, your
events with deadlines, decide-by days and today's plan), the Diary (your quick notes by day)
and the Quick Note. They open from File, the sidebar's More, the chat's note button and their
links (`emberhut://calendar`, `diary`, `note`). A note holds at most 20,000 characters, as the server
takes: past that, Save is off and the editor says by how much. A note the server refuses stays
in the diary as written, marked with its reason, until it is changed and sent again. In the
diary a swipe to the left over a note shows its Delete button (a swipe back, or a click, puts it
away); nothing is deleted until that button, or Delete in the note's menu, is pressed.

## Shortcuts

⌘ on macOS, Ctrl on Linux and Windows: ⌘0 your ember (the main room), ⌘1…⌘9 the rooms in the
sidebar's order, ⌘, Settings, ⌘[ Back in the column, ⌘⇧↩ lets a report fill the window beside
the sidebar and brings it back, Esc brings an expanded report back, otherwise closes the
report, then the column; ⌘N a new window, ⌘W closes it, ⇧⌘N a quick note, ⇧⌘D the diary, ⇧⌘Y
the calendar (Ctrl-Shift elsewhere). The sidebar shows and hides with ⌃⌘S on macOS
(Ctrl+Shift+S elsewhere). After a click in the sidebar, ↑ and ↓ move through it and open each
place they land on. macOS also has the menu bar, ⌘Q, ⌘H, ⌥⌘H, ⌘M and ⌃⌘F full screen (F11
elsewhere). Tab moves through the sidebar's rows and the column's buttons; Enter or Space opens
the one focused.

⇧⌘P opens the Party window from any window (Ctrl+Shift+P elsewhere); in it ⌥⌘N starts a new
party, and Return or ⌘Return sends the line that jumps in or steers.

⌘K (or Search in the chat's bar, or View › Search…) brings the spotlight up over the window: it
finds tasks (by key, title or text, done and archived ones too), projects and people, the
latest tasks with nothing typed; ↑ and ↓ move the pick, Return opens it, Esc or a click outside
closes it. A task opens over what the column shows; a project or a person on a page of its own.

In a quick note: ⌘↩ saves it, Esc closes it (what was written waits for the next one). In the
diary: ⌘N a new note. In the calendar: ⌘T today, ⌘← and ⌘→ the previous and next day, week
or month (a sideways drag or two-finger swipe too), Esc closes an event's details.

On a task's page in the column: ⌘D its main action (the first answer of a task that stands
for another's ask, I did it, Mark done, Unarchive; an ask's own answers are on the page), ⌘L
Not now, ⌃⌘A Archive (Ctrl+Alt+A off the Mac), ⌘[ Back, Esc closes it.

⇧⌘T opens the Sessions window (Window › Your Sessions). On the Sessions screen ⇧⌘N opens New
session; on the page of a session of your own ⌘. stops the turn it runs (what is queued runs
next).

In the file viewer: ⌘S Save to Downloads, ⌘+ (⌘= too) zoom in, ⌘− zoom out, ⌘0 fit (it wins
over Main while the viewer has the keyboard), Return Done, Esc closes the sheet, ⌃⌘F (F11 off
the Mac) full screen in a window of its own; in that window ⌃⌘F goes in and out of full screen
and Esc leaves it.

## The column

Each main window has one column beside the chat (`ui/column/`), and every page shows in it:
Needs you, Tasks, a task's page, Machines, Settings and the pages its rows open (Appearance,
the Chrome profiles, About you, an environment's, pushed over it), a project's, a goal's and a
person's pages, the weekly report, the pending approvals and a request's page. It holds a path of pages, the shown one
last; Back takes it off (from the first, or a task opened alone, the chat is alone again),
Close and Esc shut it. The room the chat shows is the store's, shared by every window.

A page is added in three places:

1. its variant at the end of `store::Page` (it may carry an id, as `Page::Task(TaskId)` does);
2. Close's name for it in `ui/column/frame.rs` (`name`, and `title` if it has no head of its
   own to name it; the column's width is one rule for every page, `ui/task_page/width.rs`);
3. its view in `MainWindow::page_view` (`ui/window.rs`): an entity made in `MainWindow::new`,
   or a function of the window.

It is opened from anywhere in a main window, with that window at hand (every click handler
has it):

- `column::open(page, window, cx)` afresh, alone in the column (the sidebar's rows, a menu, a
  link to a page; on the window itself `MainWindow::open`);
- `column::push(page, window, cx)` over what the column shows, so Back returns there (a task
  from the chat, a list or another task's page; alone when the column is shut);
- `column::toggle(page, window, cx)` open it, or shut the column if it shows it (a counter);
- `column::shows(page, window, cx)` whether it shows it (a lit row or counter).

From outside a window (a link, the command line, another window) `shell::go` takes the main
window used last. The store hears every window's path (`Store::column_changed`) and fetches for
what is shown: a task's page, the machines, Settings' switches (`Store::shows_page`).

In the Browser window, while you drive: ⌘Esc gives the browser back (Ctrl+Esc elsewhere), and
on macOS the page's ⌘ shortcuts reach the browser as Ctrl (⌘L, ⌘T, ⌘W, ⌘R, ⌘F, ⌘C, ⌘V…);
⌘Q, ⌘H, ⌘M, ⌘, and ⌘Tab stay the Mac's.

## How it is built

```
src/
  main.rs, app.rs   the command line, the log, and starting the app and its window
  cli.rs            the flags (clap)
  config.rs         which server, where the app's files are, which links may be opened
  wire.rs           the wire words the contract leaves to the server, spelled once
  backend/          the one seam to the server: the Backend trait, the app's own replies and
                    errors, and the adapter for emberhut-client (the data is emberhut-api's)
  store/            the app's data as one GPUI entity, a module per decision
  ui/               the views; every colour comes from ui/palette.rs
  theme.rs          the built-in theme and how a choice is applied
  material/         the window's material: which surfaces have it, Reduce Transparency
  shell/            the windows over the one store, where the app is for the user, links in
  links/            what an emberhut:// link names
  prefs.rs          the windows' frames and the sidebar's folds, kept across launches
  menus.rs          the menu bar, one function a menu
  motion.rs         Reduce motion; notifications.rs, asking to show notifications (macOS)
  format.rs         how sizes, counts and times read
  automation/       the automation hooks (the `automation` feature)
  prose/            how a message's text reads: blocks, links, tables, event runs, what is copied
  account/          the optional account: the build-time switch, sign-in (device grant, the
                    web authentication session), the Keychain item, the directory
```

The store's logic that needs no window is in plain types with their own tests, and the glue
that does (the poll, the live stream, sending messages and answers) is tested on GPUI's headless
test platform against a scripted backend (`backend/fake.rs`). Where each decision lives:

| Module | Decides |
| --- | --- |
| `store/chats.rs` | each room's chat as held: pages joined, earlier pages added |
| `store/rooms.rs` | which rooms the switcher lists (`is_listed`), and which room ⌘0…⌘9 open |
| `store/needs.rs` | what waits on you: Needs you by project or by when, most urgent first, its count, the open tasks |
| `store/needs_view.rs` | how Needs you looks: list or board, by project or time, its filter (`needs.json`), the board's folds |
| `store/need_card.rs` | what a Needs you card says: its kind, tier chip, asked or outdated, and its quick answers |
| `store/decision.rs` | a decision on its task's page: why and by when, asked and checked, the draft, option cards and rows |
| `store/spotlight.rs` | the spotlight: what it finds (projects, people, tasks), in order, and the pick |
| `store/places.rs` | what a project's and a person's pages fetch while they show |
| `store/projects.rs` | a project's page: progress, its asks, its work by area as lanes, the filter, its changes |
| `store/goals.rs` | a goal's ring and label, and the weekly report's bars |
| `store/x_posts.rs` | an X post's drafted replies: X's reply box, the post's numbers, Posted |
| `store/tasks/` | the Tasks list (sections, filter, search, archived) and the pages of the tasks the columns hold (their reply boxes, their actions) |
| `store/answers.rs` | an answer held for 4 s with Undo, then sent (`flush` sends it at once) |
| `store/poll.rs` | the state poll and its cadence (`next_poll`); whether it says "looking" |
| `store/live.rs` | the live stream of the assistant's steps, a thread per room |
| `store/images.rs`, `store/fetched.rs` | pictures and reports fetched by path, and what is kept |
| `store/files/fetch.rs` | a server file fetched into the kept files once however many ask, at once when kept; its chip's note |
| `store/files/uses.rs` | opening a file in its app, saving it into Downloads or where picked, copying it |
| `store/files/downloads.rs` | a name free in Downloads (`name (1).pdf`), copied without overwriting |
| `store/files/status.rs` | the status line about files: `Downloading name… 42%` after 250 ms, then how a use went for 2 s |
| `store/files/kind.rs` | how the viewer shows a file (picture, video, PDF, report, note, document), which uses it offers, whether it could run as a program |
| `store/files/titles.rs` | a report card's title, read once from its page |
| `ui/viewer/zoom.rs` | the viewer's zoom: steps, fit, a double-click's close-up, a pinch |
| `ui/viewer/pages.rs` | a PDF's pages stacked at a width, which are in sight, `3 / 12`, how sharp each is drawn |
| `ui/viewer/note.rs` | what the viewer shows of a text file: markdown, or plain text fenced; the first megabyte |
| `store/stills/` | a video's four still frames for the assistant: AVFoundation on macOS, FFmpeg elsewhere |
| `ui/inline_video/` | a video playing in its chat row or result tile (macOS): AVFoundation's frames as GPUI surfaces, its bar |
| `ui/files/actions.rs` | a file's uses (Share…, Save to Downloads, Save As…, Open in its app or Show in Finder, Copy) as a menu and buttons |
| `ui/files/drag.rs` | dragging a file out of the app as the real file |
| `pdf/` | a PDF's pages as pictures: PDFKit on macOS, Poppler's tools elsewhere |
| `heic.rs` | HEIC pictures turned into ones GPUI reads, where the platform can |
| `ui/edge.rs` | the column's left edge: dragged wider or narrower, double-click for the standard width |
| `store/reports.rs` | a report's page and its own title (`title_of`, as the reference client reads it) |
| `store/browser/` | the assistant's browser: this device's id, the live view's session per mode, take over and give back, the toolbar, a file for the page, its pictures (`frames.rs`), and what the window and rows say (`shown.rs`) |
| `ui/browser/scripts.rs` | the scripts ahead of the live view's page: the clipboard and ⌘ as Ctrl while driving, the picture's shape while watching, the stream's own buttons hidden |
| `ui/browser/policy.rs` | where the live view's web view may go: the stream's own address only |
| `ui/report/slot.rs` | the report panel's one web view: made, kept while its report shows, dropped |
| `ui/report/policy.rs` | where a report's navigations go: within the page, to the browser, nowhere (and how many) |
| `ui/report/sandbox.rs` | the Content-Security-Policy put ahead of a report in the web view |
| `ui/report/guard.rs` | the script that runs before a report's own: no hardware, no file pickers, clicks as `window.open` |
| `ui/report/width.rs` | the report panel's width: a share of the window, at least 380 px, leaving the chat 360 px while there is room |
| `ui/report/layout.rs` | the panel's edge dragged and reset, and the panel expanded and restored |
| `ui/report/kept.rs` | the panel's width kept in `layout.json`, and the default when the file is missing or broken |
| `ui/window/expansion.rs` | what brings an expanded report back (a page change, the sign-in screen, closing it), and where the focus goes |
| `json_file.rs` | the files kept as JSON (`theme.json`, `layout.json`): read, written whole through a rename, newest write first |
| `store/theme.rs` | the theme as held: cached, picked, previewed, fixed, one save at a time |
| `store/status.rs` | the assistant's status line and how errors are worded |
| `store/settings/switches.rs` | the MCP servers' and skills' switches: fetched every 10 s while Settings' front door or Connections is open in front, one switch at a time |
| `store/settings/about.rs` | what the assistant knows about you, by category, searched; a fix shows at once and is fetched again if refused |
| `store/settings/chrome.rs` | the Chrome profiles' changes and the sheet open over them; a change's answer replaces the state's profiles |
| `store/settings/on_duty.rs` | pausing and resuming the assistant, shown at once until the state agrees |
| `store/settings/appearance.rs` | the Appearance row's summary, a custom colour's contrast line, which typed colour is previewed |
| `store/settings/entries.rs` | every setting described once: id, title, fixed or live line, group and place, synonyms, everyday or technical, where it opens; ids checked against `apps/fixtures/settings-entries.json` with the phone's |
| `store/settings/now.rs` | how the things Settings speaks of stand, gathered from the store for the live lines, suggestions and status lines |
| `store/settings/search.rs` | Settings' search over the entries: words in any order, synonyms, title matches first, the closest few when nothing matches |
| `store/settings/suggest.rs` | the front door's Suggested rows, worked out from how things stand, never what is done |
| `format.rs` | how sizes read (`gigabytes`), times |
| `material/mod.rs` | which surfaces have material for the platform, Reduce Transparency and `--opaque` (`Material::choose`) |
| `material/alpha.rs` | how translucent the sidebar may be for its text to keep its contrast over any backdrop |
| `ui/look.rs` | the one place the palette and the material are put on the window |
| `ui/sign_in.rs` | the sign-in screen and its hint about pairing codes (`PAIRING_HINT`) |
| `store/setup.rs` | the first-run setup: when it shows (the gate's `Setup`), its steps and the ember's glow, how it ended (`setup.json`), the direction it begins in |
| `store/setup/home.rs` | `LocalHome`, the port for Emberhut on this computer, and this build's implementations |
| `ui/setup/` | the setup's view: the ember and its glow, the ladder of steps, the "lit" screen, the quiet line over the chat |
| `store/sign_in.rs` | pairing with a code or a pasted link: which servers a link's code may go to |
| `ui/qr.rs` | a pairing link drawn as a QR code, and the one drawn for the code shown (`PairingQr`) |
| `account/switch.rs` | whether this build has the account, from its build-time settings |
| `account/device_grant.rs` | the device grant's polls: the interval, `slow_down`, `expires_in` |
| `account/pkce.rs` | the sign-in window's PKCE verifier, `state`, authorization URL and callback |
| `account/session.rs` | the account's tokens and renewals, and what the directory's refusals do |
| `account/directory.rs` | the directory's requests, and its refusals read by their `code` |
| `account/model.rs`, `ui/settings/account.rs` | the account as Settings shows it, and its sign-in sheet |
| `ui/settings/front_door.rs` | Settings' front door: the search (focused on open; ↑↓, Return, Esc), the group chips, Suggested or the results with their matches marked |
| `ui/settings/group.rs`, `ui/settings/pages.rs` | a group page's frame (status line, anchored blocks scrolled to and washed, folded technical parts) and what each group's page holds |
| `prose/blocks.rs` | a message cut into markdown, quotes, tables and pictures; walls of text split every two sentences |
| `prose/links.rs` | task keys and bare addresses as links, the task a link opens, and where a link clicked in a message may lead |
| `prose/tables.rs` | a table as a grid whose columns share the width, or a card per row when too narrow |
| `prose/copy.rs` | what Copy, Copy Link and Explain simply take from a message |
| `prose/parsed.rs` | a message read once for the chat, within bounds on its length, blocks and table cells, and kept until its text changes |
| `store/asks.rs` | which asks the card over the composer shows, and what its buttons say |
| `store/put_away.rs` | the ask cards and Main's line put away, kept across launches |
| `store/column.rs` | the chat's room (every window's) and what each window's column holds, for what the store fetches |
| `ui/column/path.rs` | a window's column: the page opened afresh, the ones over it, Back and Close |
| `ui/column/mod.rs` | each main window's column, found from its window (`open`, `push`, `toggle`, `shows`) |
| `ui/column/frame.rs` | the column beside the chat, its bar (Back, Close) and ⌘[ (its width is `ui/task_page/width.rs`'s) |
| `shell/presence.rs` | where the app is for the user, summed over its windows |
| `shell/windows.rs` | main windows, windows of their own (one each), the Dock's reopen |
| `shell/restore.rs` | each window of its own where it was left, and those open at quit open again |
| `links/mod.rs` | the places a link names (`parse`) |
| `store/sessions/lists.rs` | the sessions every room's, once each: the sidebar's few, what runs now, the Sessions screen's sections |
| `store/sessions/filter.rs` | the Sessions screen's filter (room, who runs it, where) and each pick's count |
| `store/sessions/view.rs`, `store/sessions/watch.rs` | a session followed live: its first load, its stream from the last step, a message, Stop and settings |
| `store/sessions/turns.rs` | a transcript laid out: turns, said, tool calls with their results and helpers' steps, the fold of a finished turn |
| `store/sessions/sheet.rs` | a new session's choices, its folders and branches, and the draft its first message opens |
| `ui/sessions/` | the Sessions screen, a session's page, the new-session sheet, an external thread's page and the Sessions window |
| `store/working/environments.rs` | where the assistant works and what it reads through: fetched every 10 s while Settings' front door, Connections, Advanced or a row's page shows in front, one action at a time, what an action made kept for its page |
| `store/working/usage.rs` | the Claude accounts' use as the usage card words it, the account the assistant runs on, the checks at launch and on coming to the front |
| `store/working/jobs.rs` | the background jobs: fetched every 10 s while their pages show, a job's page kept once it ends, Stop and Clear asked first |
| `store/working/keys.rs` | the small keys a page carries for a row known by its text id |
| `store/approvals/mod.rs` | agents' approval requests: the pending ones fetched every 10 s while their pages show in front and on the live stream's `approvals` event, each open request read on its own, Approve asked first, one answer at a time on the payload hash shown, a 409 read again and said |
| `store/approvals/lists.rs` | who decides what: waiting on you or on an approver, Needs you's counts, the agents with rooms of their own |
| `store/approvals/words/` | how a request reads: its kind and state, a file's line, an exact amount, who decides, the question before Approve, why an answer failed |
| `store/party/` | Party: the list, a new party's form (its cast, caps and estimate), the party open followed live |
| `store/dark.rs` | the Dark room: its chat, the answer streaming in, an answer from before waited for |
| `ui/party/` | the Party window: the list, the New party sheet and a persona's editor, a party live |
| `ui/dark/` | the Dark room's window, always dark: its own field colours, buttons, tooltips, menu, question and code blocks (`parts.rs`), as the widgets follow the app's one theme |
| `updates/choose.rs` | which release to install: newer only, this Mac's processor and macOS, not one rolled back from |
| `updates/cadence.rs` | when to read the feed: at launch, then once the last read is 4 hours old |
| `updates/trusted.rs`, `updates/keys.rs` | the update signature's trusted comment, and the keys it may be signed with |
| `updates/record.rs` | the wait after a payload fails verification (a day, doubling) |
| `updates/marker.rs` | the update's marker: pending, healthy, rolled back; what the new copy does at its first frame |
| `updates/installer.rs`, `updates/install/` | staging, the swap and the watch over the new copy (the Mac's tools) |
| `store/updates.rs` | what the bar, Settings and Check for Updates… show; installing by itself after 10 idle minutes |

Unit tests run without a display: `cargo nextest run -p emberhut-desktop --all-features`. The
adapter's tests run against a small HTTP server on `127.0.0.1:0`, never a real one.


## The automation feature

`--features automation` adds two hooks that let a harness measure the app and drive its window
without the keyboard or mouse (for performance runs and end-to-end tests). They print their
results as lines on stdout. A normal build has neither.

```sh
cargo run -p emberhut-desktop --release --features automation -- --bench start
```

`--bench PLAN` measures and quits:

- `start`: launch to the window, the first state and the chat's first paint; memory then and
  10 s later, and the processor time those 10 s at rest cost (`rest_cpu_ms`).
- `scroll:SECONDS`: swipes up and down the chat a step a frame; frame times and processor time.
- `rooms:ID,ID,…`: opens the rooms in turn, three rounds, timing each switch to the chat's paint.
- `demo`: a tour of every screen, for a screen recording (stays open).

`--drive FILE` replays each line appended to `FILE` into the window as GPUI events (keys through
the window's bindings and the focused field, clicks through its hit-testing). Both runs are
unseen (see Unseen runs): the windows open beside the displays and the app never comes to the
front.

```text
page needs|tasks|machines|settings   room ID    open a page or a room
click X Y                            a left click at that point (logical pixels)
type TEXT                            the text, a keystroke a character
key KEYSTROKE                        one keystroke: enter, escape, cmd-a, backspace…
scroll X Y DY                        a scroll wheel at that point (DY < 0: down the page)
guard TASK                           undo the answer just given unless it is for that task
wait MS                              let the window paint
mem                                  the memory now
state                                what the app holds: page, room, needs, the toast…
a11y                                 the window's accessibility tree, while a screen reader listens
quit
```

## The macOS app

On a Mac with the Xcode command line tools, `cargo xtask bundle-macos` builds `emberhut-desktop` and
the hut's `emberhut-server` in one cargo run (release profile, 4 jobs, low priority) and writes
`target/bundle/Emberhut.app` (bundle id `com.emberhut.desktop`, the server in `Contents/MacOS`
beside the app and signed first) and `target/bundle/Emberhut-<version>-macos-<arch>.zip`.
`--no-hut` leaves the server out, for a quicker build of an app that only talks to a server
elsewhere. Without a signing identity the app is signed
ad hoc and runs only on the Mac that built it. To hand it out, sign it with your Developer ID and
notarize it:

```sh
cargo xtask bundle-macos --dist --sign "Developer ID Application: …" \
  --notarize --keychain-profile <name>
```

Make the notarytool profile once with `xcrun notarytool store-credentials <name>`;
`EMBERHUT_SIGN_IDENTITY` and `EMBERHUT_NOTARY_PROFILE` can stand in for the two options. `--install`
copies the app to `/Applications/Emberhut.app`, and only ever replaces an earlier copy of this app
(bundle id `com.emberhut.desktop`). `--automation` builds it with the `automation` feature, for a
test harness that drives the installed app. See `cargo xtask bundle-macos --help`.

### Metal shaders

gpui-kit always builds GPUI with its runtime shaders on macOS (its `runtime_shaders` feature is
on for every desktop target, and Cargo features only add up, so the app can't turn it off). The
shaders are compiled from source by the system's Metal driver when the app starts, not by
`xcrun metal` at build time, which is why full Xcode isn't needed. That is a decision: it costs
a shader compile at every launch, to be measured on a Mac with the automation build's
`--bench start` (`window_opened_ms`, `chat_paint_ms`). If precompiled shaders matter later,
that is a feature request to gpui-kit. No CI job opens a window, so a broken shader compile
would only show at launch on a Mac.

### The window's material

On macOS the window blurs what is behind it (GPUI's `WindowBackgroundAppearance::Blurred`) and
the sidebar is drawn a little translucent over it, so a hint of the desktop shows through the
sidebar. gpui-kit's root, which fills the whole window with the theme's background, is left
clear then (`ui/look.rs`). The page, the report, the composer and the title bar (AppKit's own)
stay opaque, so text on them reads as the theme has it. GPUI's blur is of what is behind the
window, so a translucent strip inside the opaque page would show the desktop through a hole
rather than frost the chat.

How translucent the sidebar may be is worked out from the theme's colours
(`material/alpha.rs`): the lowest alpha from a floor (0.90 light, 0.84 dark, `ui/palette.rs`) at
which every colour the sidebar draws text in (text, secondary, faint, the accent of "waiting",
the info colour of "running") still reads over any backdrop, on plain rows and on rows under
the pointer. The fill over a black and over a white backdrop bound the fill over any other, so
a colour whose luminance lies outside that range reads worst at one of the two, and one inside
it doesn't read (some backdrop between makes the fill that very colour). "Reads" is WCAG AA
(4.5:1) for a colour that had it on the opaque sidebar, and four fifths of its contrast for the
faint one, which never had it. Selected rows are filled with an opaque token.

An alpha above 0.96 counts as no material: the glass would be too faint to see, so the sidebar
is opaque and the window isn't blurred for it. With the server's presets light mode works out
at 1 (each has an ink a hair above AA, its faint text or paper's ember and green, so a dark
window behind a light-mode app leaves no room) and so stays opaque; dark mode works out at 0.94
and has the material. A light theme with stronger colours gets real glass. Hiding the sidebar (⌃⌘S) turns the blur off
too, since nothing translucent is left.

Every surface is opaque when macOS's Reduce Transparency is on (System Settings › Accessibility
› Display; the app follows the setting while it runs, through its change notification), with
`--opaque`, and on Linux and Windows, where blur depends on the compositor. The material follows
the theme, light or dark, and the sidebar being shown. At rest it costs the app nothing: no animation, no timer, and a
repaint only when the setting changes; the blur itself is composited by the window server.

#### Checking it on a Mac

Build the automation build once (`cargo build -p emberhut-desktop --release --features
automation`) and run `target/release/emberhut-desktop`. Put a colourful window or wallpaper behind
the sidebar. `screencapture -o -w NAME.png` takes the window (click it; `-o` leaves out the
shadow). Each check passes when:

1. **Before and after.** `--appearance dark --opaque`, then `--appearance dark`, then the same
   with `light`. Pass: in the dark "after" shot the backdrop's colour visibly tints the
   sidebar; in the "before" shots and in both light shots (opaque with the server's presets)
   the sidebar is a flat colour. The page, the report and the composer look the same in all.
   With the sidebar hidden (⌃⌘S) the dark window looks as with `--opaque`.
2. **The title bar.** Pass: it looks the same with and without `--opaque` (AppKit's grey, no
   desktop showing through it, the traffic lights where they were).
3. **A light window behind a dark-mode app.** A white page behind the sidebar,
   `--appearance dark`. Pass: "waiting" (the accent) and "running" read as clearly as with
   `--opaque`; the faint times are still readable. (Light mode is opaque with the server's
   presets, so a dark window behind it changes nothing.)
4. **Rows.** Hover a room and select one. Pass: the hover tint and the selected row look as
   they do with `--opaque`, the selected row fully opaque.
5. **Shadow and border.** Pass: the window has its shadow and its rounded corners, and the line
   between the sidebar and the page is there, as with `--opaque`.
6. **Live switches.** With the app running: switch the theme preset in Settings, switch light
   and dark (in Settings and in the system's appearance), and turn Reduce Transparency on and
   off. Pass: the sidebar follows each without a restart; it is translucent only in dark with
   Reduce Transparency off, and flat like `--opaque` otherwise.
7. **The cost at rest**, in dark (where the material is on), each with and without `--opaque`:

   ```sh
   target/release/emberhut-desktop --appearance dark --bench start   # rest_cpu_ms, settled_10s
   top -l 6 -s 10 -stats pid,command,cpu,power | grep -E 'emberhut-desktop|WindowServer'
   ```

   `top` samples the app and the window server (which draws the blur) every 10 s for a
   minute, with the app left alone and nothing moving behind it. Pass: `rest_cpu_ms` is within
   20 % or 20 ms of the `--opaque` run, whichever is more; the app's `%CPU` at rest stays under
   1 % in both; WindowServer's `%CPU` is within 1 point of the `--opaque` run. For the GPU,
   Activity Monitor's % GPU column (View › Columns) and Window › GPU History: pass when the app
   shows no GPU use at rest in either run. (`powermetrics` would give the GPU's power, but it
   needs sudo.)

### Updates

A release build (one built with `EMBERHUT_BUILD`, by the release command) updates itself from its
server's release feed (`GET /api/releases/v1/macos`, docs/api.md "Releases"; ADR 0005 has the
whole design). It reads the feed at launch and once the last read is 4 hours old, downloads the
newest newer build for this Mac and checks it before anything changes: its size and SHA-256, its
minisign signature by a trusted key with the trusted comment naming that very release, then the
bundle in the image (`codesign --verify --deep --strict`, the running app's own Developer ID team
and notarization, its `Info.plist`, `spctl`). Then the bar at the top of the chat offers Restart
to Update; with no input for 10 minutes and no message being written it installs by itself. The
swap is one atomic exchange of the bundles; the old copy leaves the Dock, starts the new one and
watches it for 2 minutes, and puts itself back if the new one doesn't draw its first frame.

A development build, an app that isn't signed with a Developer ID, and the app off macOS never
check: Settings shows the version and says why. No update key is trusted yet (`updates/keys.rs`),
so for now every payload fails its signature check and nothing is installed.

To check it on a Mac, with a Developer ID build of build N installed in `/Applications`, a
server serving a feed with build N+1 signed by a key listed in `keys.rs`: Emberhut › Check for
Updates… Pass: "Emberhut <version> (build N+1) is ready" with its notes; Restart to Update quits,
and the new build opens within seconds, its About saying build N+1, and
`~/Library/Application Support/com.emberhut.desktop/update.json` is gone. A payload whose bytes or
signature don't match: "The update didn't verify; Emberhut stays at …", and nothing changes in
`/Applications`.

### Share

A Chrome profile's pairing link has Share beside Copy on macOS, and every file has Share… in
its menu, in the viewer and in the report panel's header: AppKit's share menu
(`NSSharingServicePicker`, `src/ui/share.rs`) opens at the pointer with the link, or with the
file itself under its real name. Elsewhere the app has no share menu, so Share isn't offered.
To check it on a Mac: Settings › Chrome profiles › Add, Get link, then Share; and a file's
Share… in the chat (right-click it). Pass: the system's menu (Mail, Messages, AirDrop...)
opens under the pointer, and the service picked gets the link or the file, named as in the
chat.

## Files

A file in the chat or among a task's results opens in the viewer on a click: a sheet over the
window at 85% of its size, its name and Open in a window on top, its uses below (Share…, Save
to Downloads, Save As…, Open in Preview or whichever app opens that kind, Copy for a picture or
a text file), the zoom for a picture, a PDF or a note, and Done. The whole file comes down
first, with this device's sign-in, once however many ask; the status line at the top of the
window says `Downloading name… 42%` while it takes a moment, then `Saved to Downloads as
name (1).pdf`, `Copied` or `Couldn't download name`. A right-click on a chip, a picture or a
result has the same uses, and each drags out to Finder or another app as the real file (the
drag starts its download; one not here yet when the pointer leaves the window isn't dropped).

- A picture fits the sheet on black and zooms (⌘+, ⌘−, ⌘0, a pinch, a double-click for a
  close-up and back), scrolling both ways once it is bigger. A HEIC photo shows on macOS
  (ImageIO reads it); elsewhere it is a file chip, as GPUI has no HEIC decoder.
- A PDF shows its pages one after another, fitted to the width, with the page you are on
  (`3 / 12`). Only the pages in sight are drawn, by PDFKit on macOS and by Poppler's
  `pdftoppm` elsewhere (with `pdfinfo` for the pages' sizes); without Poppler's tools the
  viewer says so and opens the PDF in its app.
- A note or a text file reads as selectable text (markdown formatted), zoomed with its text
  size; a file over a megabyte shows its first megabyte.
- A video plays on macOS in AVKit's player (the system's controls and full screen), and a
  deck, document or sheet previews in Quick Look's view (slides scroll, sheets have tabs).
  Elsewhere the viewer opens them in their default app. In the chat (and as a task's result) a
  video shows its first still frame (sent with it), shaped like the video, with a play mark.
  On macOS a click plays it right there: AVFoundation decodes it and GPUI draws its frames as
  a surface, scrolled and clipped with the chat, with a bar (play or pause, the time, a track
  to click to a place, full screen in a window of its own). One plays at a time, and it pauses
  once it is out of sight. Elsewhere the click opens it in the viewer.
- A report opens in the report panel instead, and its card in the chat shows the page's own
  title, a Report label and the file's name.
- A file that could run as a program (a script, an app, an installer, a `.terminal`, a
  `.webloc` or `.fileloc`, an `.exe`, a `.desktop`…, by its extension and on macOS by the type
  the system gives it) is never handed to the system to open: the viewer says so, nothing of it
  is previewed (a script's text still reads as a note), and Show in Finder (Show in Folder
  elsewhere) takes Open in its app's place everywhere. On macOS every file the app writes is
  quarantined (`LSFileQuarantineEnabled`), so Gatekeeper checks whatever would run from one.

In the composer a video shows as one chip, its first still frame as its picture with a play
mark; the still frames that go with it for the assistant (at 5, 35, 65 and 95 % of its length)
have no chips of their own, and go when it is taken out. On macOS AVFoundation takes them;
elsewhere, and for a video AVFoundation can't read, FFmpeg's `ffprobe` and `ffmpeg` where they
are installed (without them the video goes alone).

### Checking the viewer on a Mac

The viewer's Mac parts (PDFKit, AVKit, Quick Look, HEIC, Share) are checked by hand on a Mac:

1. A PNG and a HEIC photo from an iPhone in the chat. Pass: both show in the chat, open in the
   viewer, zoom with a pinch, ⌘+ ⌘− ⌘0 and a double-click.
2. A 12-page PDF. Pass: pages show sharp at fit and at 3x, `3 / 12` follows the scroll.
3. A video sent from the iPhone. Pass: the chat shows its first frame with a play mark (no
   extra stills); the viewer plays it with the system's controls, and its full screen works.
4. A pptx, a docx and an xlsx. Pass: Quick Look previews each in the viewer.
5. Open in a window, then ⌃⌘F. Pass: the file's own window goes full screen; Esc leaves it;
   the next file's window opens at the size you left the last one.
6. With a report open in the panel, open a picture. Pass: the sheet isn't covered by the
   report; Done brings the report back.
7. Attach a `.mov` in the composer (without FFmpeg installed). Pass: the chip shows its first
   frame with a play mark, upright for a portrait video; the sent message carries four
   `· frame n.jpg` stills.
8. Click a video in the chat, then in a task's results. Pass: it plays in its row with sound;
   the bar's play/pause, time and track work; scrolling it out of sight pauses it; scrolling
   past it keeps it inside the chat (nothing drawn over the composer or a menu); full screen
   opens it in a window of its own.

## The assistant's browser

The Browser window (the sidebar's Browser, `emberhut://browser`, `--window browser`) shows the
assistant's own browser on the server's machine, with who drives over the page. Take over gives
you the mouse, keyboard and clipboard and the agents wait; Give back wakes the assistant.
Opening the window takes over a browser the assistant handed you that none of your devices has
yet. Each device names itself (`browser-device` in the app's directory): one drives, the others
watch.

On macOS the page is the live stream in a web view (`ui/browser/stream.rs`), signed in with the
session the server gives this device, which changes whenever control does. The sign-in is sent
only to an `https` address; an `http` one shows the pictures instead, and says why while you
drive. Elsewhere there is no web view: while someone else drives the window shows the page's
latest picture every 3 s (while you drive on another device no picture is sent, and it says
so); while you drive the toolbar works, and Open in browser shows the live view in the default
browser (Copy password puts the session's password on the clipboard for its sign-in, and takes
it off a minute later unless something else was copied since; not offered for an `http`
address).

### Checking the live view on a Mac

CI compiles it on macOS without opening it, so these are checked by hand against a server with
the live view:

- Opening Browser shows "Connecting…", then the browser's screen; against a server without the
  live view, "isn't running on the server yet".
- Watching while nobody drives, the picture keeps the screen's shape (not squeezed to the
  window). The stream's floating keyboard button and its drawer handle aren't shown.
- Take over: the pill says "You're driving", the toolbar's buttons turn on, the page takes
  clicks, scrolling and typing. ⌘L, ⌘T and ⌘W act in the browser (as Ctrl), and no key stays
  held there afterwards; ⌘Q, ⌘H, ⌘M and ⌘Tab stay the Mac's.
- Copy in the page and paste in another app, and the other way round (text).
- Give back with ⌘Esc: the pill says the assistant is driving or Watching, the page stops
  taking input.
- A page with a file field: clicking it shows "The page wants a file" under the page; Choose…
  sends a file into the field.

## Reports

A report (an HTML page from the chat) opens in the panel beside the page. Its header shows the
page's own title, with the file name under it, and its uses as in the viewer (Share… on macOS,
Save to Downloads, Save As…, Open in its app, Copy), Open in browser, Expand and Close. The
panel's left edge drags it wider or narrower, as the column's does (double-click: the standard
width); both widths are kept across launches. On macOS the page shows as its real HTML in a
WKWebView, with its CSS, layout, tables and inline-script charts. WebKit handles right-to-left
text and Arabic, and the page gets Emberhut's own light or dark appearance. Elsewhere, and when the
web view can't be made, it shows as gpui-kit's rich text, and the reason is logged. Setting
`EMBERHUT_REPORT_WEBVIEW=off` gives the same rich text on macOS. A page that fails after its view
was made (WebKit's process died) leaves the panel blank: Wry doesn't report it, and the log
warns when a page hasn't loaded after 15 s. [ADR 0006](../../docs/adr/0006-report-webview.md)
gives the choice and the web view's lockdown:

- The page runs under the reference client's Content-Security-Policy. Inline scripts and styles run.
  Pictures load inline or over https. Requests, frames, forms and plain http are blocked.
- A guard script runs first. It removes the camera, microphone, screen capture, location,
  notifications, credentials, peer connections and recording, and it cancels file pickers.
- A link the user clicks (http, https or mailto, in HTML or SVG) opens in the default browser
  or mail app, as does `target="_blank"`, at most one a second. The page itself never navigates
  away. A page that sends itself somewhere reaches the browser at most three times, and only
  once it has finished loading.
- There is no bridge to the app, no custom protocol, and no file access: no file URL, no file
  picker, no downloads, no drops. Storage is private and keeps nothing. Media doesn't autoplay.
  Force-click link previews are off.
- Devtools are on in debug builds only.

The panel opens at 46% of the window. Dragging its inner edge resizes it: it keeps at least
380 px, and the chat keeps at least 360 px while the window has room for both (in a narrower
window the chat gives way). Double-clicking the edge returns it to 46%. The width is kept, as a
share of the window, for the next launch; a drag that leaves the panel as wide on screen as it
was keeps nothing. The expand button in the header (or ⌘⇧↩) lets the report fill the window
beside the sidebar; the same button, the same keys or Esc bring it back to its width, and give
the keyboard focus back to the composer when the page is a chat. Choosing a page or a room (even
the one showing), the sign-in screen coming up, and closing the report bring it back too, so the
page never changes out of sight. While it is expanded, the answer toast (with Undo) shows under
the report. The web view follows the panel as it is dragged or expanded; it is the same view
throughout, so the page keeps its place and its state.

While the page has the keyboard focus, Esc goes to the page; the close button closes the panel,
and the header's button brings an expanded report back.
Opening a report brings Emberhut to the front (Wry activates the app when it makes a web view;
not in an unseen run), and from the first report on, the window's titlebar has no separator line.

### Checking the web view on a Mac

CI compiles the web view on macOS but never opens a window, so these checks are done by hand on
a Mac, each with its pass condition. Use the installed automation build
(`cargo xtask bundle-macos --automation --install`). Put these reports on the server:

- `LIGHT`: a page of text and a table;
- `HEAVY`: charts drawn by inline scripts and a long table;
- `check.html` and `slow.html`, below.

WebKit runs the page in its own processes (`com.apple.WebKit.WebContent`, `.Networking` and
`.GPU`). So the web view's cost is the app's own process plus the WebKit processes that appear
with it. The `--drive` step `mem` reads the app's process only, so it can't measure that cost
alone.

```sh
APP=/Applications/Emberhut.app/Contents/MacOS/emberhut-desktop
webkit() { pgrep -f 'com\.apple\.WebKit\.(WebContent|Networking|GPU)' | sort; }
run() { : > /tmp/emberhut-drive; "$APP" --background --drive /tmp/emberhut-drive "$@" & }
measure() {   # the app and the WebKit processes it added, after a minute at rest
  sleep 60; pid=$(pgrep -nx emberhut-desktop); new=$(comm -13 /tmp/webkit-before <(webkit))
  for p in $pid $new; do footprint -p "$p" | grep -i 'phys_footprint:'; done
  pids=$(echo $pid $new | tr ' ' ,)
  ps -o pid=,time=,command= -p "$pids"; sleep 60; ps -o pid=,time= -p "$pids"
}
webkit > /tmp/webkit-before   # what other apps' web views already run
```

The two `ps` lines are a minute apart: the difference in `time` is the CPU used in a minute at
rest. Quit the app between cases. Each case compares with case 0, which uses the same build and
the same report as rich text:

| Case | Start | Pass |
| --- | --- | --- |
| 0. Baseline: rich text | `export EMBERHUT_REPORT_WEBVIEW=off RUST_LOG=info; run --report Light=/files/LIGHT.html; measure; unset EMBERHUT_REPORT_WEBVIEW RUST_LOG` | The report shows as rich text, and the log says `the report shows as rich text: web views are off`. No WebKit process of ours. Note the app's footprint (F0) and CPU per minute (C0). |
| 1. Light report | `run --report Light=/files/LIGHT.html; measure` | The app's footprint is at most F0 + 40 MB. The new WebKit processes together use at most 120 MB. The app's CPU per minute is at most C0 + 0.2 s (the web view adds no GPUI frames). The WebKit processes' CPU is 0 at rest. |
| 2. Heavy report | `run --report Heavy=/files/HEAVY.html; measure` | The app as in 1. The WebKit processes together use at most 250 MB. Their CPU is 0 at rest unless the page animates, and then the app's CPU still passes 1. |
| 3. Closed again | in 2, without clicking the page, `echo "key escape" >> /tmp/emberhut-drive; measure` | The panel closes. Within 30 s the report's WebContent process exits (if WebKit keeps one spare, it is at most 30 MB: note it). The app's footprint is back within 10 MB of F0. |
| 4. Switching | `run --report Light=/files/LIGHT.html`, then open the light and heavy reports from the chat in turn, ten times each, ending on the heavy one, then `measure` | One WebContent process for the report. Its footprint and the app's are each within 20 MB of case 2's (no growth from the switches). |

Then check the page itself with `run --report Check=/files/check.html`:

```html
<!doctype html>
<html lang="ar" dir="rtl"><head><meta charset="utf-8"><title>تقرير تجريبي</title></head>
<body style="font: 16px system-ui; margin: 24px">
<h1>تقرير تجريبي</h1>
<p>هذا نص عربي مع English وأرقام 2026 في السطر نفسه.</p>
<p><a href="https://emberhut.com">رابط</a> · <a href="https://emberhut.com/docs" target="_blank">نافذة
جديدة</a> · <a href="#end">إلى النهاية</a> · <a href="mailto:test@example.com">بريد</a></p>
<canvas id="chart" width="320" height="120"></canvas>
<p>CSP: <span id="csp">no violation yet</span></p>
<p>Camera: <span id="camera">…</span> · Peer connection: <span id="peer">…</span></p>
<p>In a frame: camera <span id="frame">…</span> · peer connection <span id="frame-peer">…</span>
· file input (click it): <iframe id="files" style="height: 40px; width: 260px"></iframe></p>
<p><svg width="160" height="24"><a href="https://emberhut.com/svg"><text x="0" y="18">SVG link</text></a></svg>
· <button id="mail">window.open a mailto</button></p>
<p><input type="file" id="file"> <button id="pick">showPicker</button> <span id="picker">…</span></p>
<p><audio id="sound" controls src="data:audio/wav;base64,UklGRiQAAABXQVZFZm10IBAAAAABAAEAQB8AAIA+AAACABAAZGF0YQAAAAA="></audio>
<span id="autoplay">…</span></p>
<p><button id="later">Open emberhut.com from a timer in 2 s</button> ·
<button id="spam">Navigate the page five times</button></p>
<div style="height: 1500px"></div><p id="end">النهاية</p>
<script>
const say = (id, text) => document.getElementById(id).textContent = text;
const g = document.getElementById('chart').getContext('2d');
[40, 90, 60, 110].forEach((h, i) => g.fillRect(260 - i * 70, 120 - h, 50, h));
document.addEventListener('securitypolicyviolation', (e) => say('csp', 'blocked by ' + e.violatedDirective));
fetch('https://emberhut.com').then(() => say('csp', 'fetch ALLOWED'), () => {});
say('camera', navigator.mediaDevices ? 'mediaDevices PRESENT' : 'no mediaDevices');
say('peer', window.RTCPeerConnection ? 'RTCPeerConnection PRESENT' : 'no RTCPeerConnection');
const frame = document.body.appendChild(document.createElement('iframe'));
say('frame', frame.contentWindow && frame.contentWindow.navigator.mediaDevices ? 'PRESENT in a frame' : 'none in a frame');
say('frame-peer', frame.contentWindow && frame.contentWindow.RTCPeerConnection ? 'PRESENT in a frame' : 'none in a frame');
const files = document.getElementById('files').contentDocument;
if (files) files.body.innerHTML = '<input type="file">';
document.getElementById('mail').onclick = () => window.open('mailto:test@example.com?subject=window.open');
document.getElementById('pick').onclick = () => { try { file.showPicker(); } catch (e) { say('picker', 'no picker'); } };
document.getElementById('sound').play().then(() => say('autoplay', 'autoplay PLAYED'), () => say('autoplay', 'no autoplay'));
document.getElementById('later').onclick = () => setTimeout(() => window.open('https://emberhut.com'), 2000);
document.getElementById('spam').onclick = () => [1, 2, 3, 4, 5].forEach((n) => setTimeout(() => location = 'https://emberhut.com/?n=' + n, n * 1500));
location = 'https://emberhut.com/redirect-during-load';
</script>
</body></html>
```

Each line passes when:

- The header reads «تقرير تجريبي» with `check.html` under it.
- The text is right-aligned and reads right to left, the Arabic letters are joined, and the
  English and the number sit in place within the line.
- Four bars are drawn (the inline script ran).
- With the Mac online, the CSP line reads `blocked by connect-src` (the fetch was refused by the
  policy, not by the network), never `fetch ALLOWED`.
- The camera line reads `no mediaDevices` and the peer connection line `no RTCPeerConnection`.
  The two frame lines read `none in a frame`. If one reads `PRESENT in a frame`, the guard
  doesn't reach new frames: note which, it is ADR 0006's open point. There is no camera or
  microphone prompt, the camera light stays off, and Emberhut doesn't quit.
- Clicking the file input opens no panel, nor does the one inside the frame (if that one opens a
  panel, cancel it and note it: the frame is unguarded for pickers too). The showPicker button
  opens none either, and its line reads `no picker`.
- The sound line reads `no autoplay`, and pressing the player's play button raises no error.
- No browser tab opens on its own when the page opens (the redirect during the load goes
  nowhere).
- «رابط», «نافذة جديدة» and the SVG link each open in the default browser, once per click, and
  the panel keeps the report. «بريد» opens a Mail compose window, every time it is clicked
  (more than three times too: a click doesn't spend the page's budget).
- "window.open a mailto" opens a Mail compose window with the subject `window.open`: WebKit
  hands a mailto `window.open` to the new-window path. If nothing opens, note it: mailto
  clicks then need the navigation path.
- «إلى النهاية» scrolls the page to «النهاية».
- "Open emberhut.com from a timer in 2 s" opens nothing (a window needs a click at the time).
- "Navigate the page five times" opens at most three tabs, a second or more apart. The log
  (`RUST_LOG=info`) warns once that the report has used its three. A link clicked after that
  still opens.
- Selecting text in the page and pressing ⌘C copies it (paste it in another app).
- A file dragged onto the page does nothing.
- In the release build, right-clicking the page offers no Inspect Element. A force click on a
  link shows no preview.
- "Open in browser" in the header still opens the report from the server.
- Resizing: with `check.html` scrolled down a little, drag the panel's inner edge left and
  right, slowly and fast, also with the pointer passing over the page. The page follows the edge
  while it moves, with no gap or overlap at the edge, and keeps its scroll position (it isn't
  loaded again). The pointer shows the resize cursor over the edge. Double-clicking the edge
  returns the panel to 46%. Quit and open the app again: the panel opens at the width you left.
- Expanding: the header's expand button (then ⌘⇧↩) lets the page fill the window beside the
  sidebar, and the same button, ⌘⇧↩ or Esc (with the page not focused) bring it back to its
  width, the page still at the same place. Esc with the panel at its width closes it. Click in
  the page and press ⌘⇧↩: note whether it reaches the app (WebKit keeps Esc while the page has
  the focus, and may keep this too); the button works either way. Answer an ask, then expand
  within 4 s: the toast with Undo shows under the page, not hidden behind it.
- Appearance: with macOS in Light, set Emberhut to Dark in Settings. A report that styles
  `@media (prefers-color-scheme: dark)` shows dark, and switching Emberhut back to Light turns it
  light without reopening it. The page doesn't flash white when it opens in Dark. A report with
  no colours of its own sits on the panel's surface in the theme's text in both, with no white
  behind it and no black text in Dark.
- A slow picture: `slow.html` is a page with an https picture from a server that answers after
  30 s, and a link. Clicking the link while the picture loads opens it in the browser at once.
  After 15 s the log warns that the report hasn't finished loading.
- `--background`: opening a report brings Emberhut to the front (Wry's activation, ADR 0006). This
  is expected for now. Note it if it gets in the way. An unseen run (`--drive`) stays behind.

## Fonts and pictures

Linux and Windows use Inter, built in (SIL Open Font License 1.1, see
`assets/fonts/Inter-LICENSE.txt`); macOS uses its system font. `assets/face.png` is the
assistant's face in the sidebar and on the sign-in screen: the ember, on a transparent ground,
with a cream rim for dark themes. `assets/AppIcon-1024.png` is the source of the macOS icon,
drawn on Apple's 1024 grid (the 824 px tile and its shadow included). Both come from the brand
masters in `assets/brand/`.

The views' icons are Lucide's, through gpui-kit: its default bundle (its components' own icons)
plus the ones the views name beyond it, listed in `src/assets.rs` (`AppIcons`), not the whole
catalog. A test reads the source for every icon a view names and fails when one isn't served
(it would draw blank) or when a listed one is no longer used: a view that names a new icon adds
it there.

## Agents and approvals

Every agent but the default one (the state's `agents`) has a room of its own, listed in the
sidebar under Agents with its name, who it works for (Off while disabled) and how many of its
requests wait on you; it takes no ⌘ shortcut. In its room the bar marks it as an agent, with who it
works for beside the status, and its approvals button (what waits on you) opens that agent's whole
list.

The state's `approvals` are the pending requests; the ones waiting on you count in Needs you's
badge, and Needs you's Approvals row (how many wait on you, how many of those only you can
approve, how many on others) opens the list (`GET /api/approvals?state=pending`): what waits on
you, each with Approve… (which opens the request, its question asked under the payload: a decision
is made only on a request's page), then what waits on an approver, read-only with who that is. A
request's page reads it by id and shows its exact payload: the agent, the kind, the target, a
payment's amount with its currency, the text word for word in a fixed-width selectable block,
each file's name, size and the first 12 hex digits of its SHA-256. Approve asks first, under the
payload; Approve and Reject send the payload hash of what is on screen, so a request that changed
meanwhile decides nothing (409), is read again and shown as it is, with a line that says so; an
answer lost on its way says it may or may not have gone through and reads the request again. A
read that started before an answer never brings the request back as pending. A request that
can't be read says why, with Retry, or that it isn't there. A waiting request, or an approved one
not used yet, can be withdrawn.
Payments, passwords and codes, secrets, account security and anything in your name are marked as
only yours to approve. Characters that would change how a request's words read without showing
themselves (Unicode's format characters, such as bidi overrides and zero-width characters, and
control characters but a line break and a tab) are written out as `[U+202E]`, as on the server's
link page and in Slack (`store/approvals/shown.rs`). The room's live stream says when a request changes (its `approvals`
event): the state, the list and each open request are read again.
