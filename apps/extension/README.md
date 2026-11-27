# apps/extension

The Emberhut browser extension: Emberhut's hands in the owner's own Chrome profiles, one copy per profile, each paired with the
owner's server. Manifest V3, in TypeScript (a browser extension runs only JavaScript), bundled with esbuild. Package
`@emberhut/extension`, extension name "Emberhut". ADR 0023 has the decisions; ADR 0021 the server's side.

## What it does

- Keeps one WebSocket to the server (`<server>/api/chrome/ws`) and relays the browser tools' DevTools commands to
  `chrome.debugger` in Emberhut's own tabs. Its first message names the protocol's version (`"protocol": 1`); the named
  messages are `emberhut.*` (`emberhut_api::chrome::extension`; `src/protocol.ts`).
- Touches only its own tabs: the "Emberhut" tab group (blue, the apps' accent), a tab it opened, or any popup or tab one of
  them opened (a sign-in window, say). Never a blocked site (`src/scope.ts`: password managers, account security pages,
  the Web Store, and the owner's own list; hosts compared without a final dot, paths without doubled slashes, escapes or
  case): the tab's address and the one it is going to are checked on every command, and Emberhut lets go of a tab before a
  navigation to a blocked site (in its page or a frame) commits.
- Sends only the DevTools commands the server's engine uses on a page (`src/cdp-rules.ts`, an allow-list): nothing that
  reaches past the page (the browser, storage, cookies, service workers, passkeys, other targets, network fetches),
  no direct DOM edits, no drag and drop, no paste, and only on the tab's own session (no frames' or workers' sessions).
  Of the tab's events only the page's own go back (no network events), with any cookie or authorization header taken
  out.
- **Passwords.** The guard covers the input and fill commands (`src/background/guard.ts`):
  - typing (`Input.insertText`, a key that carries text) is refused while the focus is a password or one-time-code
    field (`input[type=password]`, or autocomplete `current-password`, `new-password`, `one-time-code`), checked from
    an isolated world the page can't reach; a focus in another site's frame, which can't be checked from there, is
    refused too;
  - a function run on an element (how the server fills, reads or acts on a field) is refused on such a field;
  - a field that was a password field in the page's document still counts after its type is changed (the memory is
    the top document's, starts when Emberhut attaches, and follows the same element only; a same-site frame's fields are
    checked as they are now);
  - Enter and Tab aren't typing, so a form the owner filled in and handed back can be sent;
  - paste (Ctrl or Cmd+V, Shift+Insert, by key, code or virtual key code; editing commands on a key) is refused
    everywhere, as it would hand the page the owner's clipboard. Not covered: the primary-selection paste of a middle
    click on Linux.

  Script the server evaluates in the page is not read by the extension: the server's own rules cover it (it blanks
  password values in everything it reads, and its step gate holds final steps). Every refusal says to hand the browser
  to the owner.

- **Final steps** (submit, send, pay, delete…) wait for the owner's OK in the server's step gate; the extension only acts
  on the server's calls and has no way around it.
- **Pause** (the popup, the pill on the page, the app) lets go of every tab and keeps Emberhut out for up to 10 minutes, or
  until the owner hands back; then it comes back by itself. Hand-offs and take-backs show as notifications.
- The pill "Emberhut is driving · Pause" and Emberhut's own cursor are drawn from an isolated world, in a closed shadow root that
  is aria-hidden, so the page and the assistant's snapshots never see or press them (`src/page-scripts.ts`). A page
  that removes, restyles or covers the pill gets it back within a second. The page's
  console is quiet while Emberhut holds a tab, and a page that closes itself says so first.
- Recovers by itself: Chrome's debugging bar or another extension's frame cutting the link (the tab is offered again),
  a computer that slept, a server that went away (back-off: 20 s, 40 s, 80 s, then every 2 minutes; `src/backoff.ts`).

## Its id

The manifest's `key` fixes the extension's id, the same in every profile and from every copy of the folder:

    omnhmmnnbibfdohdhcbcomdhbgpehfnj

It is `emberhut_api::chrome::extension::ID`; the server and the Mac helper check zips against it. Never change or drop the
key. The private key that matches it is kept outside the repository, with the maintainers' signing material; it is only
needed to pack a signed CRX, which the extension doesn't use.

## Pairing

The server's address comes only from pairing. In the Emberhut app, Settings → Chrome profiles → Add, then open the link in the
profile. A link on the server this profile is already paired with pairs by itself. A link on any other server is
offered first, since any page could show a link: before the profile is paired, as a notification and in the popup;
once paired, in the popup only, so a page can't raise a prompt to switch servers. A server is offered at most once per
browser session. Pairing tells the server only the code (and this profile's old token only when it is that same
server's); what the profile is (its version, Google account, system and Chrome) goes only in the hello, to the server
it paired with. Or paste the link, or its code and the server's address, in the popup.

## Installing and updating

Not from the Web Store. Every profile on a computer loads it unpacked (`chrome://extensions`, Developer mode, Load
unpacked) from one folder: on a Mac `~/Library/Application Support/Emberhut/Extension`, which the Mac helper keeps in step
with the zip the server serves ([docs/mac-helper.md](../../docs/mac-helper.md)). When the server says the folder holds a
newer version (`emberhut.update`), the extension reloads itself from it, never while it holds a tab and only once per
version: a reload that comes back still older (the profile loads the extension from another folder) isn't tried again
for that version. Elsewhere, unzip the server's `/chrome/emberhut-extension.zip` into a folder that stays.

Keep "Allow access to file URLs" off for the extension at `chrome://extensions`: with it off, Chrome lets no extension
debug a `file://` page, and the extension refuses them as well.

## Building it for a server that spells some words differently

A build can carry a wire dialect, the same file the desktop client (`emberhut-client`) and the mobile app are built
with: `EMBERHUT_WIRE_DIALECT=/path/to/dialect.json pnpm --filter @emberhut/extension run build`. `scripts/build.ts`
checks the file (a malformed one fails the build) and esbuild's `define` puts its table into the bundle; unset or empty,
the bundle carries an empty string and the extension speaks its own words, rewriting nothing.

```json
{
  "words": [["helper", "assistant"]],
  "prefixes": [
    ["other.", "emberhut."],
    ["K-", "T-"]
  ]
}
```

Each pair is the server's spelling, then the extension's, and the rules are the desktop client's and the mobile app's
(`src/dialect.ts`; written out in [apps/mobile/README.md](../mobile/README.md), with the cases all three answer alike in
`crates/emberhut-client/tests/fixtures/dialect.json`). On the socket (`src/messages.ts`), with the example above:

- every message's name goes through it both ways, so the server's `other.start` is read as `emberhut.start` and
  `emberhut.stopped` is sent as `other.stopped`;
- the params of the protocol's own named messages (`src/protocol.ts`) go through it too, keys and string values (a
  handback `by` of `helper` is read as `assistant`), except a `token`, which is opaque;
- a relayed call's arguments, its answer, the browser's events and the first message's params carry the pages' own text
  and addresses, and go as they are. `headers` is accepted and unused.

The manifest, its key and the extension's id stay the same with any dialect.

## Working on it

- `pnpm install --frozen-lockfile` at the repository root.
- `pnpm --filter @emberhut/extension run check`: types, lint, format, tests, the build into `dist/` and
  `dist/emberhut-extension-<version>.zip` (the version from `manifest.json`; the same sources give the same zip from any
  working directory, and the build fails if a local path lands in a bundle). Point the
  server's `EMBERHUT_CHROME_EXTENSION_ZIP` at that zip.
- The logic that needs no browser (the rules, the pause, the back-off, pairing links, versions, the page scripts, the
  popup's words, the zip writer, the wire dialect) has vitest tests; the modules under `src/background/` call Chrome's
  APIs and are checked by type and lint. The server's tests drive a scripted extension over a real socket.
- A new version: bump `version` in `manifest.json`, build, and serve the zip.
