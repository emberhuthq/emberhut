# 0031. The browser extension on the Chrome Web Store: one listing every hut pairs with

- Status: Proposed
- Date: 2026-11-28
- Builds on: [ADR 0023](0023-browser-extension.md) (the extension, its protocol and its fixed id),
  and replaces its "Not the Chrome Web Store" point; [ADR 0030](0030-the-hut-inside-the-app.md)
  (the desktop app's setup)

## Context

ADR 0023 kept the extension off the Chrome Web Store: every profile loaded it unpacked from a folder
the Mac helper kept in step with the owner's server. That works for the people who build Emberhut,
and badly for anyone else: Load unpacked needs Developer mode, Chrome warns about developer-mode
extensions, the folder needs the Mac helper to stay current, and a profile on Windows or Linux has
no helper at all. Emberhut is self-hosted, so there is no Emberhut server to point an update URL
at, and Chrome honours off-store installs only on managed machines.

## Decision

### One listing, the same id

- The extension is published once, publicly, on the Chrome Web Store. Every self-hosted hut pairs
  with that one extension; the hut's address still comes only from pairing (ADR 0023), so the
  listing holds no server and no account.
- The store item keeps the id the manifest's `key` gives (`omnhmmnnbibfdohdhcbcomdhbgpehfnj`,
  `emberhut_api::chrome::extension::ID`): its first upload carried the matching private key, which
  is how the store adopts an existing id. The store refuses a `key` in an uploaded manifest, so the
  store's zip leaves it out; the zip for loading unpacked keeps it, so a developer's build has the
  same id and pairs the same way.
- `emberhut_api::chrome::extension::STORE_PAGE` is the listing's address. The hut's pairing page
  (`/chrome/pair`) sends a profile without the extension there; loading a build unpacked is a
  developers' note under it.

### Installed by the desktop app, after the person agrees

- The desktop app's Chrome step offers "Add to Chrome". Only after that click it writes Chrome's
  external-extension entry, which points at the Web Store's update service
  (`https://clients2.google.com/service/update2/crx`), so Chrome itself fetches the store's copy:
  - macOS: `~/Library/Application Support/Google/Chrome/External Extensions/<id>.json`, per
    user. Chrome shows "New extension added" when it next starts, and the person clicks Enable.
  - Windows: the per-user registry key `HKCU\Software\Google\Chrome\Extensions\<id>`, value
    `update_url`. Chrome asks the same way.
  - Linux: Google Chrome reads only root-owned folders, so the app writes
    `/opt/google/chrome/extensions/<id>.json` through `pkexec` (the system asks for the person's
    password), and Chrome installs it without a further prompt.
- Then the app asks the hut for a pairing link and waits for the profile to pair, as for any
  profile. The store page is offered beside it for adding the extension at once, without waiting
  for Chrome's next start.

### What review asks of it

- **The fewest permissions.** `debugger` (the DevTools commands the hut's browser engine sends its
  own tabs), `tabs` (the tabs' addresses, to keep to its own tabs, check blocked sites and see a
  pairing link), `tabGroups` (its "Emberhut" group), `webNavigation` (letting go of a tab before a
  navigation to a blocked site commits), `storage` and `alarms`. No host permissions and no content
  scripts. `notifications` is optional: the popup asks for it when the person turns it on. The
  profile's Google account is no longer read (`identity` is gone); the person names the profile
  in the app.
- **Consent before pairing.** Every pairing the person starts (a link on a hut this profile isn't
  paired with, or one typed in the popup) goes through the extension's own consent page, which
  says what it sends to the hut (the addresses, text and screenshots of the pages in its tab
  group), that the hut clicks, types and runs script there, that the hut passes what a task needs
  to the AI model the person chose, and what never leaves; pairing waits for its Pair. Cancel on
  Chrome's debugging bar pauses Emberhut, as its own Pause does.
- **An encrypted link.** The pages Emberhut works in travel to the hut, so the extension pairs
  only over https, or plain http to this computer (loopback, which never leaves it) or a Tailscale
  address (100.64.0.0/10, fd7a:115c:a1e0::/48), which WireGuard encrypts.
- **No remote code in the extension.** Its own pages and worker run only the packaged files, under
  an explicit content security policy (`script-src 'self'; object-src 'none'`, no `eval`). What
  the hut runs in a page goes through `chrome.debugger`, the API the store's rules name for it, and
  is declared on the listing's privacy form. The bundles are not minified and carry no source maps.

### Released with the apps

- The extension's version is the workspace's (`Cargo.toml`); its build fails when `manifest.json`
  or `package.json` says another.
- CI builds the two zips (unpacked, and the store's) from a clean checkout twice, checks the
  bytes are the same, checks the store's zip has no `key`, no source map and no dotfile, and keeps
  them as the run's artifact; a release tag attaches them to its GitHub release.
- Store installs are updated by Chrome. The extension ignores the hut's `emberhut.update` when
  Chrome installed it from the store (its manifest then has an `update_url`); the Mac helper's
  folder stays for unpacked builds.

## Consequences

- A fix reaches people after the store's review (usually days), not with the hut. The protocol
  stays versioned (`"protocol": 1`), and a hut keeps talking to the previous extension release
  until the store has the next one.
- The listing is public: anyone can install the extension, and it does nothing until it pairs with
  a hut its owner confirms.
- The private key stays outside the repository; it is needed again only if the item had to be
  created anew.
