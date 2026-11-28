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
  until the owner hands back; then it comes back by itself. Hand-offs and take-backs show on the badge and in the popup,
  and as Chrome notifications once the owner turns them on there.
- The pill "Emberhut is driving · Pause" and Emberhut's own cursor are drawn from an isolated world, in a closed shadow root that
  is aria-hidden, so the page and the assistant's snapshots never see or press them (`src/page-scripts.ts`). A page
  that removes, restyles or covers the pill gets it back within a second. The page's
  console is quiet while Emberhut holds a tab, and a page that closes itself says so first.
- Cancel on Chrome's "started debugging this browser" bar pauses Emberhut, as Pause does.
- Recovers by itself: another extension's frame cutting the link (the tab is offered again),
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
offered first, since any page could show a link: before the profile is paired, on the extension's own consent page
(`static/consent.html`), which says what pairing sends and to whom (`src/disclosure.ts`) and waits for Pair; once
paired, in the popup only, so a page can't raise a prompt to switch servers. A server is offered at most once per
browser session. Or paste the link, or its code and the server's address, in the popup: that opens the same consent
page. A link on a server this profile was unpaired from asks again.

Pairing tells the server only the code (and this profile's old token only when it is that same server's); what the
profile is (its version, system and Chrome) goes only in the hello, to the server it paired with. The extension pairs
only over https, or plain http to this computer or a Tailscale (100.x or fd7a:115c:a1e0::) address, whose WireGuard
tunnel encrypts it (`src/transport.ts`).

## Permissions

| Permission                 | Why                                                                                                                                                           |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `debugger`                 | The hut's browser engine works in a page through the DevTools protocol: `chrome.debugger` in Emberhut's own tabs only, with the allow-list above              |
| `tabs`                     | The tabs' addresses: to keep to Emberhut's tabs and their popups, check every command against the blocked sites, and see a pairing link opened in the profile |
| `tabGroups`                | The blue "Emberhut" tab group that marks Emberhut's tabs                                                                                                      |
| `webNavigation`            | Letting go of a tab before a navigation to a blocked site commits, in the page or a frame                                                                     |
| `storage`                  | The pairing, the blocked sites, a pause; the session's recent steps                                                                                           |
| `alarms`                   | Reconnecting after the service worker sleeps, and ending a pause after 10 minutes                                                                             |
| `notifications` (optional) | Asked for in the popup when the owner turns on Chrome notifications for hand-offs; without it the badge, the popup, the pill and the app say the same         |

No host permissions and no content scripts: the extension reads nothing from a page Emberhut doesn't hold.

## Installing

From the Chrome Web Store, in every profile: the store's page is `emberhut_api::chrome::extension::STORE_PAGE`, the
same id as below ([ADR 0031](../../docs/adr/0031-extension-on-the-chrome-web-store.md)). The desktop app's Chrome step
adds it for you after you agree (Chrome's external-extension entry; you click Enable in Chrome), and the hut's pairing
page links to it. Chrome keeps it up to date; the hut's `emberhut.update` is ignored by a store install.

### Loading it unpacked (developers)

For a build of your own: `pnpm --filter @emberhut/extension run build`, then in `chrome://extensions` turn on
Developer mode, click Load unpacked and pick `apps/extension/dist` (or unzip `dist/emberhut-extension-<version>.zip`, or
the hut's `/chrome/emberhut-extension.zip`, into a folder that stays). Remove the store copy first: both have the same
id. On a Mac the helper can keep a folder in step with the zip the hut serves,
`~/Library/Application Support/Emberhut/Extension` ([docs/mac-helper.md](../../docs/mac-helper.md)); when the hut says that
folder holds a newer version (`emberhut.update`), an unpacked extension reloads itself from it, never while it holds a
tab and only once per version.

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
- `pnpm --filter @emberhut/extension run check`: types, lint, format, tests, the build into `dist/` and two zips
  (`scripts/zip.ts`): `dist/emberhut-extension-<version>.zip` to load unpacked or serve from the hut
  (`EMBERHUT_CHROME_EXTENSION_ZIP`), and `dist/emberhut-extension-<version>-store.zip`, the Chrome Web Store's upload,
  whose manifest has no `key` (the store keeps the item's own). The same sources give the same bytes from any working
  directory; the build fails if a local path lands in a bundle, and the zips take only what the extension runs (no
  source maps, sources or dotfiles; `scripts/release.ts`). The bundles are not minified.
- The version is the apps' own: `manifest.json` and `package.json` must say the workspace's `version` in the root
  `Cargo.toml`, or the build fails. Change the three together.
- CI builds both zips twice and compares them, and keeps them as the run's artifact; a `v*` tag attaches them to its
  release. The store listing's words, the permissions' reasons and the privacy answers are in
  [store/listing.md](store/listing.md).
- The logic that needs no browser (the rules, the pause, the back-off, pairing links, the transport rule, the
  disclosure, versions, the page scripts, the popup's words, the zip writer and the release rules, the wire dialect)
  has vitest tests; the modules under `src/background/` call Chrome's APIs and are checked by type and lint. The
  server's tests drive a scripted extension over a real socket.
