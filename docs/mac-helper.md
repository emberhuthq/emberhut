# The Mac helper: the browser extension's folder

The Mac helper (ADR 0012) is a small HTTP service on the owner's Mac that the server calls with
the helper's bearer token (`--mac-helper-url`, `--mac-helper-token-file`). Besides the Mac row,
"Lock now", the call recorder and the assistant's Mac tools, it keeps the browser extension's
folder in step with the zip the server serves (ADR 0023). This page is the contract between the
two; the types are `emberhut_api::mac::{helper, ExtensionFolder}`, the server's side is
`MacHelper::extension` and `MacHelper::install_extension` in
`crates/emberhut-server/src/services/mac.rs`.

## The folder

Every Chrome profile on the Mac loads the extension unpacked from one folder:
`~/Library/Application Support/Emberhut/Extension`. The extension's id is fixed by the `key` in its
`manifest.json` (`emberhut_api::chrome::extension::ID`), so it is the same in every profile and from
every copy of the folder.

## GET /extension

What the folder holds now: 200 `{ "version": "1.2.3", "path": "…" }`, the manifest's `version`
and the folder's path; `{ "version": null }` when the folder is missing, empty, or holds no
readable manifest.

## POST /extension/install

The extension's zip as the body: `Content-Type: application/zip`, at most 20 MB
(`helper::EXTENSION_ZIP_MAX`); the server never sends a bigger one. The helper:

1. refuses a body over 20 MB, or one that isn't a zip;
2. refuses a zip whose `manifest.json` (at its top) has no `key`, or a `key` that doesn't give the
   extension's fixed id (Chrome's rule: the first 128 bits of the SHA-256 of the key's DER bytes,
   in hex, each digit written `a` to `p`);
3. refuses a zip with an entry outside the folder (an absolute path, a `..` part, a link);
4. unpacks it into a new temporary folder beside the folder, then renames it into place
   (the old one moved aside and removed), so a profile never loads a half-written folder.

Answers: 200 `{ "version": "…", "path": "…" }`, the folder as it is now; 400 `{ "error": "…" }`
with why it refused or failed (the folder is then as it was).

## When the server calls them

On its own, once both the helper and the zip (`EMBERHUT_CHROME_EXTENSION_ZIP`) are set up: right after
it starts (beside start-up, which never waits for it or fails with it) and then every 5 minutes
(`FOLDER_EVERY`), the server asks `GET /extension`. It compares the answer with the zip's version as
last read (the zip is read again only when its file changes). If the folder is missing, empty
(`version: null`) or older, and the version has a try left, it reads the zip once (unless it was
just read for its change): the bytes it sends are the ones whose key must give the fixed id and
whose version is the one aimed at. It sends `POST /extension/install` with those bytes and checks
the answer's version. Then every profile on the Mac that speaks the extension's own protocol and
still runs an older version is told `emberhut.update { version }`, once per version; a round that finds
the folder already current tells them too. So on a fresh setup the folder fills before any profile
runs the extension.

Also when a profile that speaks the extension's own protocol says hello from macOS
(`os: "mac"`) with a version older than the zip served, and on the socket's 30-second tick while
that is so, the server makes the same check and install. Once the folder holds the zip's version
it tells the profile `emberhut.update { version }`, once, and after an install it tells the Mac's
other older profiles too (one may have asked while the install was under way). The profile
reloads itself from the folder (`chrome.runtime.reload()`) when it holds no tab, and never twice
for one version. The reference extension's spelling is never told.

Both share one limit: the server installs one version at most 3 times, 10 minutes apart, whoever
asks, so they never install it twice at once (a second asker meanwhile waits for the next try);
and it tries one profile and version at most 3 times, 10 minutes apart. A new zip version gets
tries of its own. A helper that doesn't answer `GET /extension` is asked again later (only the
debug log notes it). Everything else goes to the activity log once per version and kind of
failure: a refusal of either call, an install that went unanswered (it used a try), a helper
that says it unpacked an older version, a zip that can't be read or isn't the extension, and a
version that used all its tries (the folder then stays as it is until another build is served).
A profile that was told and still reports the older version loads the extension from another
folder: it isn't told again, the activity log says so, and the app shows it as out of date. A
server with no helper or no zip set up does nothing.
