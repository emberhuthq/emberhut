# 0021. The browser the assistant drives, the owner's Chrome profiles and the Mac tools

- Status: Proposed; amended by [ADR 0023](0023-browser-extension.md)
- Date: 2026-11-26
- Amended by: [ADR 0023](0023-browser-extension.md): the extension is in the repository, in
  TypeScript, with a fixed id; it speaks the protocol in `emberhut_api::chrome::extension` and is
  updated from the owner's server through a folder the Mac helper keeps.

## Context

The assistant needs hands on the web and on the owner's Mac:

- **Its own browser.** A real Chrome on the server's machine (a virtual display, a persistent
  profile, `DevTools` on localhost), driven with Playwright over CDP. The `browser_*` tools open
  pages and read them as an accessibility tree with refs, click, type, press, scroll, wait,
  manage tabs, and move files in and out. The owner watches its whole screen live in the app
  (a stream served by the browser's machine) and can take over at any time; agents wait while
  they drive.
- **The owner's own Chrome profiles.** A browser extension in each profile keeps a WebSocket to
  the server. A relay on the server speaks CDP to Playwright and `chrome.debugger` to the
  extension, so the same tools, with the same guards, work in the owner's real logins. Profiles
  are paired with a one-time link; the owner can pause the assistant there for up to 10 minutes.
- **The rules.** Agents never type a password or a code: they hand the browser to the owner
  (`browser_handoff`), who hands it back, and the assistant may take it back early
  (`browser_takeback`). A final step (submit, send, install, pay, delete…) waits for the owner's
  OK; `browser_plan` names a job's steps once. API keys on a page are masked everywhere the
  assistant, a log or a picture sees them, and `browser_save_secret` moves one from the page into
  the server's secrets unread. Each task's browsing is logged on the task.
- **The Mac.** The Mac helper (ADR 0012) also drives one app at a time in the background
  (`mac_apps`, `mac_app_*`: a window as elements with refs, pressed, typed into, its menus), and,
  with the owner's OK or on an idle Mac, the whole screen (`mac_screen`).

Playwright has no Rust port, and the server must not start a browser or reach a network in its
tests. The tasks, the assistant's events and the owner's standing OKs ("grants") are other
areas' and are built in parallel.

## Decision

### One engine seam, a CDP client of our own

- `services::browser::engine::BrowserEngine` is one browser, tab by tab: tabs, navigation, the
  page as a tree with refs, questions to the page or an element (each a named query, not free
  JavaScript), clicks, fills, keys, scrolling, waits, pictures with keys blacked out, files into
  fields, the page's own file picker held for the app, dialogs. The rules (masking, gating,
  hand-offs, file places) live in the browser service over it.
- `services::browser::cdp::CdpEngine` is the real one, our own CDP client over a WebSocket
  (`tokio-tungstenite`), on a `CdpLink`: `DirectLink` reaches a Chrome by its `DevTools` address
  (`EMBERHUT_BROWSER_CDP`, `http://host:port` or the browser's `ws://`), one flat session per tab;
  the extension's link sends each command through `chrome.debugger` in that tab. The tree comes from
  `Accessibility.getFullAXTree`, written as lines (`- button "Save" [ref=e12]`, children indented)
  with refs kept per tab; steps find an element again by its ref (`DOM.resolveNode`). Clicks and
  keys are real input events. A file goes into a field as a `File` the page builds from the bytes,
  so nothing is copied to the browser's machine; a download is fetched in the page with its own
  login and read back in 4 MB pieces; each piece is written to a new owner-only `.part` file as it
  arrives (with `tokio::fs`), so a download of up to 500 MB is never held whole. The app's upload is
  streamed to disk the same way and read back once, as the page takes a file in one message.
- The tests script the engine (`tests/server/fake_browser.rs`) and, for the extension, the
  extension itself over a real local socket: the real CDP engine then drives a scripted tab.
  No browser is started.

### The rules

- Masking (keys by shape, field values that are keys, password values blanked), blocked sites,
  folding a big tree, the final-step and secret-field words, file types and places are pure
  rules in `emberhut_core::browser`.
- **Blocked sites on every surface.** `browser_open` and a new tab refuse a blocked site's
  address, and after each tool's step (a click, a key, a redirect) a tab that reached one, or
  a browser or extension page, is closed and let go of, as the extension does in the owner's
  Chrome.
- **Enter sends.** Enter, with any modifier, is checked by what has the focus before it is
  pressed: in a form whose submit reads as a final step, on a button that reads as one, or in a
  text box, text area or editor outside any form (a chat composer, a mail's body) it is a step
  for the gate, the last with the text written there. Only a search field, a form's plain field
  whose submit isn't final, or no focus at all goes without.
- **Final steps go through a `StepGate`.** Its default, `CardGate`, asks one Go ahead / Stop card
  per step on its task and lets the step through once the owner says Go ahead; for an ordinary
  step that Go ahead (or a planned job's) covers the rest of the job; money and account security
  ask every time; a message or post asks with its text. The grants area's richer gate (standing
  OKs, an OK'd text in any channel, the job's request) replaces it in `Dependencies::gate`
  without the tools changing.
- **Tasks and events go through a `TaskDesk`**: a note on a task, its "Needs you" line, a card and
  the owner's choice on it, a file attached, an event for the assistant. Until the tasks area
  wires its own, `NoDesk` keeps nothing; the browser's own log (`browser/logs/task-<id>.md` in
  the data folder, keys masked, owner-only) is kept either way, and so is the Mac's
  (`mac/logs`). Both folders are in the backup.
- The owner's hold on the browser outlives a restart (the `browserHold` setting).
- The live view itself is served by the browser's machine. `GET /api/browser/session` gives its
  address (`EMBERHUT_BROWSER_VIEW`) and the password for what the device may do, from the secrets
  `browser-view` and `browser-control`; the user is `owner`.

### The owner's Chrome profiles

- `chrome_devices` (each profile's token hashed, its label, account, machine, versions) and
  `chrome_tasks` (a task's own browser: a profile, or 0 for the server's) are tables;
  `ChromeService::set_task_browser` is what the tasks area's `task_create` / `task_update`
  `browser` argument calls.
- The extension's protocol is `emberhut_api::chrome::extension` (`services::chrome::wire` on the
  server). Amended by [ADR 0023](0023-browser-extension.md): the extension is in the repository
  (`apps/extension`). `GET /chrome/emberhut-extension.zip` serves a build artifact from
  `EMBERHUT_CHROME_EXTENSION_ZIP`, and the version every profile should run is read from that zip's
  `manifest.json`. Pairing links point at `EMBERHUT_CHROME_URL`, else the public URL. The pairing
  page and the socket need no app session: a link's code, or the profile's own token, is the key.
- Since the socket needs no session, it reads little before it knows the profile: its first
  message (the pairing or the hello) must come within 10 seconds and be at most 8 KiB (a bigger
  one closes the socket with 1009, anything but text with 1003), and nothing more is read until
  the hello succeeded (a pairing closes the socket once answered). After the hello the socket's
  own cap is 64 MiB a message. The WebSocket layer still takes in one
  frame up to that cap before the check sees it, so a stranger can hold that much for at most
  the 10 seconds; the cap can't be lowered for the first message alone in axum's WebSocket.
- A socket the server closes (the profile unpaired, paired again under another name, or
  connected again) is let go of as one the extension closes: its pending calls fail, it is no
  longer listed online, and the activity log says its link closed.
- Left out, as they would serve only a particular network setup: reopening an offline profile
  through the Mac helper, and finding a profile's machine by its host name on the local network
  (`machine` stays what the extension says, or unknown). An
  unused profile is still let go after 5 minutes, so the owner's screen can sleep.
- The routes that hand out a credential or act in the browser (`/api/browser/session`, `input`,
  `upload`, `POST /api/chrome`) need a signed-in device in either auth mode, as the backup and
  pairing codes do: report mode's pass without a session is for reading.

### The Mac tools

- `MacHelper` gains `hands(path, body, wait)`, with a default that says no helper is set up, so
  other helpers compile. The tools call the helper's paths (`/which`, `/open`, `/snapshot`,
  `/describe`, `/press`, `/set`, `/keys`, `/menu`, `/screen/*`, `/health`).
- An app is asked for once and remembered (`macApps`); plain dev and system tools never ask;
  Terminal is only looked at; a final step goes by the step gate; the whole screen starts on the
  job's OK or a Mac idle 5 minutes, never right after the owner stopped it.
- The app is the one `/which` resolves (its bundle id, else its name), not the words the agent
  used: Chrome and the browsers built on Chromium are refused however they are named (a pid, a
  bundle id, another name), so the owner's Chrome is reached only through the extension.
- Return (with any modifier), or text with a new line, in an app that sends as the owner is a
  send for the gate. On the whole screen, where the app in front isn't known, Return and a
  typed new line are sends every time, and a click that doesn't say what it presses
  (`element`) goes by the gate as a step of its own.

### WhatsApp Web

- `BrowserService::whatsapp_draft` (open the chat by number or exact name, type the draft line
  by line, read it back) and `whatsapp_send` (press Send, wait for the tick) are the messaging
  area's `WhatsAppWeb` seam; its error and tick types map one for one.

## Consequences

- With `EMBERHUT_BROWSER_CDP` unset the browser tools say no browser is set up; nothing else
  changes. The browser's machine runs its own Chrome and live view; the server only connects.
- Our CDP client covers what the tools use, not Playwright's whole surface: no auto-waiting for
  an element to hold still, no frames inside frames. A page that needs those is handed to the
  owner, as a page that closes itself is today.
- Until the tasks and grants areas are wired, final steps can be asked but the cards go nowhere,
  so they never go through: the safe side.
