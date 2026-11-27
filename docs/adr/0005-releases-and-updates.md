# 0005. Releases and updates

- Status: Proposed
- Date: 2026-11-25

## Context

Emberhut.app is built and signed by `cargo xtask bundle-macos`, but there are no releases: no
version beyond the workspace's `0.1.0`, no release notes, and no way for an installed app to
update itself. This ADR starts from the usual Sparkle release flow of a macOS app that
updates from its own server, which is proven in practice, and keeps releases private until the
repository is public.

**The Sparkle flow it starts from.**

- **Numbers.** A marketing version that rarely changes, and a build number: a whole number
  given by hand to the release script, which becomes `CFBundleVersion`. The update bar talks
  in builds ("Build N is ready").
- **Cutting a release.** A release script, run on the release Mac, makes a Developer ID build
  with the identity in the Mac's Keychain. It checks the signature
  (`codesign --verify --deep --strict`) and that every linked framework is in the bundle. It
  notarizes and staples the app, puts it in a DMG, then signs, notarizes and staples the DMG
  and checks it with `spctl`. It signs the DMG with Sparkle's EdDSA (Ed25519) key, read from a
  key file (the login Keychain holds a copy), and writes an appcast (Sparkle's RSS feed) for
  that one build: build, version, minimum macOS, notes and the DMG's signature. The notes are
  the last commit's subject. Nothing runs in CI.
- **Hosting.** The script uploads the DMG, then the feed, so the feed never points at a
  missing file. It uses `POST /api/mac-app/<name>` on the app's own server, with the server's
  key. The server writes each file to a temporary name and renames it into place, and keeps
  the newest three DMGs. `GET /api/mac-app/<name>` serves the feed, a DMG, or the newest DMG
  under a fixed name. The GET sits behind the same checks as every other API route (the
  private network, or a device session once sign-in is enforced), so nothing is public.
- **Updating.** Sparkle is in the app, and the feed's URL is the app's own server. Every
  request carries the device's access token. The public key is in `Info.plist`, and Sparkle
  verifies the signature before it unpacks anything. It checks on launch and every 4 hours,
  downloads quietly, and then a one-line bar at the top of the chat says "Build N is ready",
  with "Restart to Update" and a close button. The close button hides the bar until a newer
  build. The app installs and relaunches by itself once the Mac has been idle for 10
  minutes. The app menu's "Check for Updates…" asks now: Sparkle answers "You're up to date"
  or offers the build, and a build already downloaded installs at once. Settings shows the
  version, the build, when it last checked, and "Check for updates". A debug build never
  updates itself.
- **The phone** stays on TestFlight. Once App Store Connect has processed a build, a small
  program tells the server the newest build number. The phone's state then carries that
  number, and the same bar opens TestFlight.

Emberhut can't use Sparkle as it is. Sparkle is an Objective-C framework, and the app is Rust and
GPUI. Its appcast is RSS, where the rest of emberhut's contract is JSON. And its feed would sit
outside the typed contract the server implements. What Sparkle does is a few hundred
lines of Rust, and this ADR specifies them.

## Decision

### 1. Versions and the changelog

- **Product version: semver**, plain `MAJOR.MINOR.PATCH` (`0.1.0`, …), with no pre-release
  or build suffix. It is the workspace's `version` in `Cargo.toml`, which every crate
  inherits. While it is `0.x`, a minor release may break things.
- **Build number: a whole number that only goes up**, one counter for the whole product,
  desktop and phones alike. It is in `Cargo.toml` as `[workspace.metadata.release] build = N`,
  and every release raises it by one. It is committed, so a tag rebuilds with the same
  number, and a reviewer sees it change.
- **Where the counter starts.** The phone app already has two counters in
  `apps/mobile/app.config.ts`: `BUILD_NUMBER = 4` (Android `versionCode`) and
  `IOS_BUILD_NUMBER = 3`. Play refuses a `versionCode` that is not higher than every one it
  has seen, and App Store Connect takes a build number once per version. So the first value is
  one more than the highest build either store has accepted for `com.emberhut.mobile`, and at
  least 5. The maintainer confirms the highest accepted numbers in Play Console and App Store
  Connect before the first release, and the release that sets the value says what was seen.
- **Which builds carry it.** Only `cargo xtask release publish` (section 2) passes the build to
  the compiler, as `EMBERHUT_BUILD`. The app reads it with `option_env!`. Every other build is a
  development build that never updates itself: plain `cargo build`,
  `cargo xtask bundle-macos` with or without `--install`, and CI. bundle-macos has no flag that
  sets it.
- **Where they show.** In `Info.plist`, `CFBundleShortVersionString` is the version and
  `CFBundleVersion` is the build (today both hold the version, and bundle-macos changes that;
  a development build writes `0`). The app shows "Emberhut 0.2.0 (build 12)" in About and in
  Settings, and "0.2.0 (development build)" otherwise.
- **The phones read the same pair.** `app.config.ts` runs in Node when Expo builds. It reads
  the repository's root `Cargo.toml` with a TOML parser (`smol-toml`, a dev dependency of
  `apps/mobile`), takes `workspace.package.version` and `workspace.metadata.release.build`, and
  sets `version`, `ios.buildNumber` and `android.versionCode` from them. `BUILD_NUMBER`,
  `IOS_BUILD_NUMBER` and `VERSION` go away, so there is one source.
- **One `CHANGELOG.md`**, in the [Keep a Changelog](https://keepachangelog.com/) format,
  written in plain words for the person who uses the app rather than as commit subjects. A
  pull request that changes something users see adds a line under `## Unreleased`, and the
  reviewer checks it (CONTRIBUTING.md says so). Releasing turns that heading into
  `## 0.2.0 (build 12) - 2026-12-01` and opens a new, empty `Unreleased`. That section is the
  release notes everywhere: in the feed, on TestFlight and Play, and later on GitHub Releases.
  `cargo xtask changelog notes 0.2.0` prints the section's body, and
  `cargo xtask changelog check` fails when the version's section is missing, empty, or names
  another build. A build-only bump (Open question 5) rewrites the heading to name every build of
  the version, `## 0.2.0 (builds 12, 13) - 2026-12-01`, and the check accepts that form when its
  last number is the workspace's build.

Alternatives: the build number as the number of commits on `main` (it changes meaning if
history is rewritten, and a reviewer can't see it); a date stamp (unreadable in "build N");
notes generated from commit subjects (written for developers, not users);
separate counters per store (what the phone has now, which drifts from the desktop's).

### 2. Cutting a release

1. **Prepare.** `cargo xtask release prepare 0.2.0` makes a branch. It sets the workspace
   version, raises the build, and dates the changelog section. The change goes through a
   normal pull request and CI.
2. **Tag.** After the merge, `cargo xtask release tag` makes the annotated tag `v0.2.0` on the
   merged commit and pushes it. It first checks that the local `main` is `origin/main` and
   that this commit holds the release's version. The real gate is step 4's checks on the Mac;
   a GitHub ruleset that limits `v*` tags to maintainers is a setup step on top, if the plan
   allows rulesets on a private repository (see Open questions).
3. **CI on the tag** (`.github/workflows/release.yml`, `on: push: tags: ['v*']`,
   `contents: read`, no secrets, its own concurrency group `release-<tag>`). It checks out with
   `fetch-depth: 0`, runs every gate in `ci.yml` (made reusable through `workflow_call`), and
   then the release checks. A called workflow's `concurrency` is evaluated in the caller's
   context, so `ci.yml`'s group would be `ci-<sha>`, the same as main's run of that commit:
   part 6 adds `github.workflow_ref` to that group and checks it on the first tag. If it
   doesn't hold, the tag run only waits for main's run, since neither cancels the other.
   The release checks:
   - the tag is annotated, is `v` plus the workspace version, and its commit is on `main`;
   - the build is greater than the previous release tag's;
   - `cargo xtask changelog check` passes.

   The job summary shows the notes. CI builds and tests, but it signs nothing and publishes
   nothing.
4. **Publish, on the Mac.** `cargo xtask release publish` refuses to run unless:
   - the tree is clean, and HEAD is exactly a release tag that matches `origin`;
   - the newest `release.yml` run for that commit has `event` `push`, `headBranch` equal to
     the tag and `conclusion` `success`. It asks GitHub with the Mac's own `gh` login
     (`gh run list --commit <sha> --workflow release.yml --json …`), and offers no flag to
     skip;
   - the server's feed, read with the publish key, has no build as high as this one.

   It then runs bundle-macos (the `dist` profile) with
   `--dist --sign <identity> --notarize --keychain-profile <name>` and `EMBERHUT_BUILD` set. The
   identity is the Developer ID Application certificate in the Mac's login Keychain. That
   signs with the hardened runtime, notarizes and staples the app. Publish then makes
   `Emberhut_0.2.0_12_macos_<arch>.dmg` with the app and an `/Applications` link, signs, notarizes
   and staples the DMG, and checks it with
   `spctl -a -t open --context context:primary-signature`. Then it **smoke-runs the exact
   app** in that DMG: it attaches the image as the updater does (section 3) and runs
   `Emberhut.app/Contents/MacOS/emberhut-desktop --self-check --background`. In that mode the app uses
   a fresh temporary data directory and no server, opens its window without taking focus,
   and exits 0 once the window has drawn its first frame (the updater's "healthy"), or fails
   after 60 s. A build that won't load, start or draw stops the release here, before any Mac
   sees it. The run proves loading, the signature, start-up and drawing on the publishing Mac
   only (its macOS, its arch, no server); older systems and the sign-in and network paths are
   left to the watchdog (section 3). Drawing needs a logged-in desktop session on the Mac with
   the display awake: publish refuses to start outside it (`launchctl managername` is not
   `Aqua`, as over SSH or from a launchd job), keeps the display awake during the run
   (`caffeinate -d -u`), and on a timeout says "No first frame in 60 s: is the display awake
   and the screen unlocked?". Last, it signs the DMG with the update key (section 3), writes
   the feed (section 4), and uploads the DMG first and the feed second.

**Trust.** No certificate, signing key or notary credential is a GitHub secret. Signing
happens on the Mac, and the keys stay in its Keychain: the Developer ID certificate, the
notarytool profile, the update key and the publish key. CI can't make a build that an
installed app accepts, so the repository's CI and its third-party actions are not part of the
signing chain. The hand-off from the tag to the Mac is the tag itself, and publishing checks
the tag, the clean tree and CI's verdict.

Alternative: signing in CI with a `.p12` and the update key as secrets. Releases could then
happen without the Mac, but every workflow change and every pinned action would become a way
to the signing keys. It is not worth it for a single maintainer who releases from one Mac.

### 3. The update

**Where it shows.**

- **Check for Updates…** is in the app menu, after About. It checks now. With nothing newer,
  it answers in a small dialog "You're on the latest (0.2.0, build 12)", with how old the
  server's feed is ("published 3 days ago"). With something newer, it downloads and verifies
  it first ("Downloading Emberhut 0.3.0…"), then offers "Emberhut 0.3.0 (build 15) is ready", with
  its notes and the buttons "Restart to Update" and "Later". When the check fails, it says why
  ("Couldn't reach your server", "The update didn't verify; Emberhut stays at 0.2.0"). If an
  update is already downloaded, it offers it at once.
- **The update bar.** Once an update is downloaded and verified, one quiet line at the top of
  the chat says "Emberhut 0.3.0 is ready", with "Restart to Update" (one click: swap and
  relaunch) and a close button that hides it until a newer build. Settings has these rows:
  version and build, last checked, the feed's age, and "Check for
  updates".
- **When it checks.** On launch, then every 4 hours, and when the Mac wakes if the last check
  is older than that. It never checks in a development build, and never in an app that is not
  signed with a Developer ID team (an ad-hoc signed app fails closed). Whether a downloaded
  update also installs by itself after 10 idle minutes, as in the Sparkle flow above, is an
  open question.

**Before the swap, the payload must pass every check, in this order.** The running app is not
touched until they all pass.

1. **The feed entry.** Its build is greater than the running build: an equal or lower build is
   never installed, so the app never downgrades. It is not a build the app rolled back (the
   refusal record, below). Its minimum macOS fits this Mac, and its architecture is this Mac's.
2. **Space, size and SHA-256.** The app needs free space of at least three times the
   payload's size where it writes (the DMG, the copied bundle, and a margin). The download
   streams into an owner-only (`0700`) directory under the app's cache directory, hashing as
   it goes. It is cut off past `min(size, 1 GiB)`: the feed's `size`, and a ceiling compiled
   into the app that no feed can raise. It must end with exactly that size and the feed's `sha256`.
   On any error the partial file is deleted.
3. **The update signature.** It is in the [minisign](https://jedisct1.github.io/minisign/)
   format, Ed25519. The app verifies it with `minisign-verify`, and xtask signs with the
   `minisign` crate. The signature's trusted comment is signed too, and must read exactly
   `emberhut-desktop macos <arch> <version> build <build> sha256:<hex> notes:<hex>`, where the
   last field is the SHA-256 of the UTF-8 bytes of the JSON-decoded `notes` string, as it is
   (no trimming, no newline or Unicode normalisation), so xtask, the server and the app hash
   the same bytes however the JSON is re-serialised. That binds the payload and its notes to
   the feed entry: a server can't relabel an old signed DMG as a new release, or change the
   notes the app shows.
4. **The new bundle.** The DMG is attached with
   `hdiutil attach -nobrowse -readonly -mountrandom -noautoopen`. `Emberhut.app` is copied out with
   `ditto` to `.Emberhut.app.new` next to the installed app (same volume), and the DMG is detached.
   The copy must then pass all of these:
   - `codesign --verify --deep --strict`;
   - `codesign --verify -R <requirement>`. The requirement is built at run time from the
     running app's own signature: `anchor apple generic`, `identifier "com.emberhut.desktop"`, a
     Developer ID chain (`certificate 1[field.1.2.840.113635.100.6.2.6]` and
     `certificate leaf[field.1.2.840.113635.100.6.1.13]`), the leaf's team equal to the
     running app's, and `notarized`. No team id is written in the repository;
   - its `Info.plist` has bundle id `com.emberhut.desktop`, the feed's version, and a
     `CFBundleVersion` equal to the feed's build;
   - `spctl --assess --type execute`, as an extra check only: it answers yes when the user has
     turned Gatekeeper off, so notarization is pinned by the requirement above.

   The DMG is read more than once by path (to hash it, to verify its signature, to attach it).
   That is not a boundary that matters, because the codesign checks run on `.Emberhut.app.new`
   itself, which is exactly the bundle that is swapped in.

**Keys.** The update key's private half stays on the Mac: in the login Keychain, plus an
offline backup. `cargo xtask release keygen` makes it and prints the public key.
`apps/desktop/src/updates/keys.rs` holds the trusted public keys, each with its minisign key
id. A signature is checked against the key with its id, and an unknown id fails.

- **Rotation:** a release signed with the old key ships an app that trusts both keys. Once
  installed apps have moved to it, a later release drops the old key.
- **Leaked:** the maintainer still holds the key, so a normal release signed with it ships an
  app that trusts only a new key. Until then, a leak alone can't ship an update, because the
  Developer ID team check and notarization still apply. That is the reason for two signers.
- **Lost:** no release can be verified by installed apps, so each Mac needs a manual install
  of a DMG trusting the new key. Developer ID and Gatekeeper still check that one. This is why
  the offline backup matters.

**The swap.** The app updates only an `Emberhut.app` that holds bundle id `com.emberhut.desktop`, in a
folder it can write to, and not from a read-only or translocated location. Otherwise it offers
the download instead, and asks the user to move Emberhut to Applications first.

1. `Emberhut.app` and `.Emberhut.app.new` are exchanged in one atomic step (`renamex_np` with
   `RENAME_SWAP`), so there is never a moment without an `Emberhut.app`. The workspace denies
   `unsafe_code`, so this goes through a crate's safe wrapper: rustix's `renameat_with` with
   `RenameFlags::EXCHANGE` if it maps to the macOS call, which part 2 checks first. If it
   doesn't, a one-function module with an `#[expect(unsafe_code, reason = …)]`, like the
   desktop's existing memory readout, does it.
2. The old bundle, now at `.Emberhut.app.new`, is renamed `.Emberhut.app.old`. The app writes the
   marker `update.json` in its data directory (the app's files in the desktop README, never
   next to the bundle): `from` and `to` (the old and new builds), `state` (`pending`), the
   time, and the pids of the watchdog and of the new copy. Every write of the marker, by
   either process, goes to a temporary file that is renamed over it.
3. The old app becomes the update's **watchdog**. It waits for any renewal already in flight
   and saves the session store, then stops every request (polls, the live stream, renewals),
   so only one process ever renews. It closes its window and switches to the accessory
   activation policy: no Dock icon, no menu bar, no place in ⌘-Tab, so the user sees one Emberhut.
   It spawns the new copy's executable directly (`Emberhut.app/Contents/MacOS/emberhut-desktop`, in a
   process group of its own so it outlives the watchdog), records its pid, and watches it for
   up to 2 minutes.
4. The new copy is **healthy** once its window has drawn its first frame, whatever it shows
   (the chat, sign-in, offline or an error). No network is needed, so a Mac that relaunches
   offline or while the server is down keeps a good build. Until it is healthy, the new copy
   changes the format of nothing in the data directory (`session.json`, `config.json`,
   `theme.json`, `layout.json`), so a rolled-back app can read everything it finds. Once
   healthy, it sets `state` to `healthy`. The watchdog then deletes `.Emberhut.app.old` and the
   marker, and exits.
5. If the new process exits, or the 2 minutes pass, without `healthy`, the watchdog ends the
   new process (`SIGTERM`, then `SIGKILL` after 5 s) and only then reads the marker again. If
   `healthy` arrived meanwhile, it keeps the new build as in step 4, and, if it was the one
   that ended the new process, starts the kept build again (`open -n`) before it exits; a new
   copy that exited on its own after `healthy` (the user quit it) is not restarted. Otherwise
   it exchanges
   `.Emberhut.app.old` back into place, sets `state` to `rolled-back` with the reason, and logs
   it. It then starts a fresh process of the restored `Emberhut.app` (`open -n`), which reads
   `session.json` from disk (the new copy may have renewed the session), and exits. The
   restored app sees the marker and says "The update to Emberhut 0.3.0 didn't start; Emberhut stays
   at 0.2.0".

While it watches, the watchdog:

- passes a reopen or open-document event (a Dock click, a Finder open of Emberhut.app) on to the
  new copy, by activating the pid in the marker, rather than dropping it;
- on a quit request, stops watching and exits, leaving the new build in place;
- on the system's power-off or logout notification, stops watching without rolling back, so a
  new copy that the logout quits before its first frame is not taken for a broken one.

When it stops watching that way, the marker stays `pending`. A new copy that starts and finds
a `pending` marker whose watchdog pid is no longer a live Emberhut process cleans up by itself once
healthy.

A `rolled-back` marker is the refusal record. The app doesn't offer that build again for
7 days, or until the feed offers a newer one, whichever is first, and then deletes the marker.

The rescue runs in the old app because it is the one copy known to work. A rescue inside the
new build can't help a build that dies before its own code runs: a library that won't load, a
signature the kernel refuses, a crash at start. **What it covers:** a new build that fails,
hangs or crashes before its first frame while the watchdog runs, including what publish's
smoke run can't see (an older macOS, a symbol missing there). **What it doesn't:** a power cut,
the watchdog being force-quit, or a quit or logout of the watchdog (it then stops watching on
purpose), in those 2 minutes, with a broken new build; the broken build stays, and needs a
manual install of the DMG. A build that fails after its first frame is an ordinary bug, fixed
by the next release.

**When something fails**, nothing of the running app has changed, or (after the exchange) the
watchdog puts it back. Staged files are deleted, the bar does not appear, and the reason goes
to the log. A manual check also says it. A payload that failed verification is not fetched
again until the feed offers a different `sha256`. The app downloads at most one payload per
check, and after a failed verification it waits 24 hours before the next download, doubling
each time it fails again.

**What a server, or anyone on the path to it, can and can't do.** The feed itself is not
signed, and the server is often reached over plain HTTP on a private network. So a hostile
server or network can:

- withhold updates for ever (a freeze), or serve an old feed again (a replay), or claim a
  higher `minimumOs`. The app shows the feed's age (`published`) in Settings and in the
  dialog, so a frozen feed shows, but nothing more is done about it: this is accepted;
- offer a build newer than the running one but older than the latest;
- see when the device checks;
- show a false version number in "Downloading Emberhut 0.3.0…".

It can't:

- install anything that isn't signed by both the update key and the running app's Developer
  ID team, and notarized;
- downgrade the app, or install the same build again;
- change the notes the app shows for a release it installs, since they are signed. Notes of
  older releases the app skipped are shown as plain text only, under "Earlier releases
  (unverified)";
- fill the disk, or make the app download more than one capped payload per check, and, after
  a failed verification, no more than one a day.

### 4. The feed

It is a small JSON document, one per platform and format. Readers ignore fields they don't
know, as for the rest of the contract. The types are in `emberhut-api` (`emberhut_api::releases`), so
the server, `emberhut-client`, the app and xtask, which writes the feed, share one definition.

```json
{
  "format": 1,
  "product": "emberhut-desktop",
  "platform": "macos",
  "published": "2026-12-08T10:00:00.000Z",
  "releases": [
    {
      "version": "0.3.0",
      "build": 15,
      "date": "2026-12-08",
      "notes": "- Rooms open faster.\n- …",
      "minimumOs": "13.0",
      "assets": [
        {
          "arch": "arm64",
          "url": "/api/releases/files/Emberhut_0.3.0_15_macos_arm64.dmg",
          "size": 41234567,
          "sha256": "<64 hex digits>",
          "signature": "<the minisign signature file, as text>"
        }
      ]
    }
  ]
}
```

- `platform` is one of `macos`, `ios` and `android` (an enum in emberhut-api).
- `releases` is newest first. It lists the last N releases (default 3) with the assets of every
  arch they ship. An older release drops from the feed and from the server's storage together
  (section 5), so every asset listed is stored. The dialog shows the notes of every listed
  release newer than the running one.
- `assets` defaults to empty (`#[serde(default)]`): the phones' feeds have no assets and only
  say which build is newest (section 6).
- **Formats.** The app reads any `format` up to its own. It asks for its own first, and on a
  `404` tries the next lower format it reads. A change that breaks readers is a new format at
  a new path (section 5). Servers keep every format they have ever served, filled by every
  release (a few KB each), unless a later ADR retires one, so a Mac that was off for months
  still finds its way. A body whose `format` is higher than the app reads (possible only on a
  misconfigured server) makes the app say "Update Emberhut by hand" and do nothing else.
- On a server's feed, `url` is a server path. The app fetches it with `emberhut-client` and the
  device's session, so the token never goes to another host: `emberhut-client` already refuses
  anything but a server path. Two client additions are needed:
  - `Client::release_feed(platform, format)`;
  - a streamed `Client::download(path, dest, max_len)`, because DMGs exceed `file()`'s 64 MiB
    body limit.

### 5. Hosting: private, behind the app's login

**The routes.** These go in `docs/api.md` in its own terms, with a section "Releases", and
the server implements them:

| Route | Who | What |
| --- | --- | --- |
| `GET /api/releases/v{format}/{platform}` | a signed-in device, or the publish key | the feed; `404` when nothing is published |
| `PUT /api/releases/v{format}/{platform}` | the publish key | replaces the feed |
| `GET /api/releases/files/{name}` | a signed-in device | a payload (`Content-Length`, `Range`), never compressed |
| `PUT /api/releases/files/{name}` | the publish key | stores a payload |

- A payload is always sent as it is, with no `Content-Encoding`, whatever the request's
  `Accept-Encoding`: api.md's gzip rule doesn't apply to it (a DMG is compressed already, and
  the size cut-off relies on `Content-Length`). `Client::download` sends
  `Accept-Encoding: identity`.
- Payload names match `Emberhut_<version>_<build>_<platform>_<arch>.dmg`. A `PUT` writes to a
  temporary file and renames it into place, and refuses to overwrite a name whose bytes
  differ.
- A feed `PUT` is refused unless every asset it lists is stored with that size and `sha256`.
  After the feed `PUT` succeeds, the server deletes the payloads that no feed it serves lists
  any more. Pruning happens then, never on a payload `PUT`, so a live feed never points at a
  missing file.
- The server is not trusted for any of this: the app verifies everything itself. These rules
  only keep a bad publish from breaking the feed.
- The publish key is a publish-only key: it can do these two `PUT`s and the feed `GET`, and
  nothing else. It is not the server's full key, and no device session can publish.

**Recommendation: (a), the app's own server.**

- **(a) emberhut-server, chosen.** The feed sits behind exactly the login the app already has.
  The server implements the four routes. The publish key is a program key of the kind ADR 0002
  plans: made on the server's command line, stored as its SHA-256, scoped to publishing, and
  revocable on its own. The payloads go in a `releases/` folder next to the database.
- **(b) A private GitHub release fetched with a token, rejected.** Every installed app would
  hold a GitHub credential, and one that can read a private release can read the private
  source. Revoking it means shipping a new app. GitHub's rate limits apply, and access would
  depend on a GitHub account rather than on the app's own sign-in. An object store with signed
  URLs is rejected too: it is one more service, and one more credential to keep.

**When the repository goes public**, GitHub Releases becomes the channel. Each tag's release
gets the DMG and `feed-macos-v1.json`, of the same format, whose `url`s are absolute `https`
links to that release's assets. The stable feed URL is
`https://github.com/emberhuthq/emberhut/releases/latest/download/feed-macos-v1.json`. "latest" skips
drafts and pre-releases. The updater supports two feed sources from its first version:

- `server`: a server path, fetched with the session (the default while private);
- `url`: an absolute `https` URL, fetched with no credentials at all. Up to 5 redirects are
  followed, `https` only, which GitHub's asset links need. No host is trusted: the asset is
  accepted only by the checks in section 3, the same as from a server.

The source is a compiled-in default, which `config.json` (`"updates": {"feed": …}`) can
override. Going public is then one release whose default is `url`. The server keeps
publishing its feed through a transition, so apps that are not yet updated still find the
release that moves them. Nothing breaks, and the checks are the same.

### 6. Mobile (short; tracked with the mobile app)

The phones use the same version, build and `CHANGELOG.md` section (section 1 says how
`app.config.ts` reads them):

- iOS goes to TestFlight, with the section as "What to Test";
- Android goes to the Play internal track, with the section as the release notes, cut to
  Play's 500 characters and ending in a link to the full notes.

After the stores have processed a build, the release step publishes the `ios` or `android`
feed. The phone shows its version and build in Settings. Its "Check for updates" reads that
feed, and the same quiet bar opens TestFlight or Play. The stores install and verify the
update; the app never does. A store upload that has to be redone (failed processing) needs a
new build number; how it gets one is an open question.

### 7. The work that follows

Each part has its own files, and each file has one writer, with one exception: dependencies.
The part that first uses a dependency adds it to its crate's manifest and to the root
`[workspace.dependencies]` together, as the workspace's `unused_workspace_dependencies` and
`cargo machete` require. Those lines, `Cargo.lock`, and `deny.toml` (if a new license must be
allowed) are shared and append-only, and are combined in merge order. The new ones:
`minisign-verify`, `sha2` and `rustix` (part 2), `minisign` (part 5, which reuses `sha2`).
Part 1 goes first. Parts 2 to 6 then run in parallel against its types. Part 3 builds on the
store API that part 2 opens with.

| Part | What | Files it alone writes |
| --- | --- | --- |
| 1. Contract | the feed types and contract tests; `release_feed` and streamed `download`; the "Releases" section; the `[workspace.metadata.release]` entry with the first build | `crates/emberhut-api/src/releases.rs`, `crates/emberhut-client/src/`, `docs/api.md`, root `Cargo.toml`'s `[workspace.metadata.release]` |
| 2. Updater engine | the check, cadence and downgrade rule; both feed sources, with the plain `https` fetch for `url` feeds; download, staging and the verify chain; the exchange, the health report, the watchdog (its marker protocol, accessory policy and events) and the refusal record; `--self-check`; the keys; the startup hooks (the marker, the check loop); the `updates.feed` setting; the store entity and its API | `apps/desktop/src/updates/`, `apps/desktop/src/store/updates.rs`, `apps/desktop/src/store/mod.rs`, `apps/desktop/src/{main,app,cli,config}.rs`, `apps/desktop/Cargo.toml` |
| 3. Updater UI | "Check for Updates…", the version in About, the bar and mounting it at the top of the chat, the Settings rows | `apps/desktop/src/menus.rs`, `apps/desktop/src/ui/update_bar.rs`, `apps/desktop/src/ui/mod.rs`, `apps/desktop/src/ui/chat/mod.rs`, `apps/desktop/src/ui/settings.rs` |
| 4. Versioning and changelog | `EMBERHUT_BUILD` and `CFBundleVersion`; `changelog notes` and `check`; `release prepare` and `tag`; the `release` command with stubs that part 5 fills; the first `CHANGELOG.md`; the Unreleased rule; xtask's new `emberhut-api` dependency in architecture.md | `xtask/src/cli.rs`, `xtask/Cargo.toml`, `xtask/src/changelog.rs`, `xtask/src/release/{mod,prepare,tag}.rs`, `xtask/src/bundle_macos/`, `CHANGELOG.md`, `CONTRIBUTING.md`, `docs/architecture.md` |
| 5. Publish | `release publish` and `release keygen`: the pre-flight checks, the DMG and its notarization, the smoke run, the update signature, the feed, the uploads | `xtask/src/release/{publish,dmg,feed,keygen,upload}.rs` |
| 6. CI tag job | `release.yml` on `v*`; `ci.yml` callable through `workflow_call` | `.github/workflows/` |
| 7. Server routes | emberhut-server's four routes, after its program keys | `crates/emberhut-server/src/{routes,services}/releases.rs` and the server's config and `app.rs` wiring, in that part's own branch |
| 8. Mobile wiring | `app.config.ts` reads the workspace version and build; its constants go; `smol-toml` | `apps/mobile/app.config.ts`, `apps/mobile/package.json`, the lockfile (mobile track) |
| 9. Maintainer setup | confirm the stores' highest build numbers; make the update key and its offline backup; make the publish key on the server; the `v*` tag ruleset if the plan allows it | GitHub, Play Console, App Store Connect, the Mac's Keychain |

## Consequences

- A release is three commands: prepare, tag, publish. Each refuses to go on when something
  doesn't match, and only the last needs the Mac.
- No signing material is in GitHub. A compromised workflow can't ship an update that installed
  apps accept, because it would need both the Developer ID and the update key.
- Updates need the Mac that holds the keys. Losing the update key strands installed apps until
  someone installs a new DMG by hand, so the key's offline backup is part of the release
  setup.
- The updater is emberhut's own code, a few hundred lines of security-sensitive Rust, where the
  flow it starts from uses Sparkle. It is held to the same bar: every check and every swap step
  has a test, and a failure leaves the running app as it was.
- The feed is part of the contract, so its routes appear in `openapi.json` once emberhut-server
  has them, and every server that has them serves the same bytes to the same app.
- One build counter means the phones' numbers jump to the shared one at the first release,
  and a desktop-only release uses up a number the phones never ship. Both stores accept gaps.

## Open questions

Each has the recommendation this ADR assumes.

1. **The counter's first value.** Recommended: one more than the highest build either store has
   accepted, at least 5, confirmed in Play Console and App Store Connect before the first
   release.
2. **Idle auto-install.** The Sparkle flow installs a downloaded update by itself after 10
   idle minutes. Recommended: the same, but only when no message is being written, and off by
   a Settings switch. The alternative is one click only.
3. **x86_64.** Recommended: arm64 only until an Intel Mac needs Emberhut. Then one DMG per arch
   (the feed already allows it) rather than a universal binary, which doubles the build time.
4. **Tag protection on a private repository.** GitHub rulesets on private repositories need a
   paid plan. Recommended: add the `v*` ruleset if the plan has it. Without it, nothing is lost
   that matters, because publish's checks on the Mac are the gate.
5. **Mobile re-uploads.** A store upload that has to be redone needs a new build number.
   Recommended: `cargo xtask release bump-build`, a small pull request that raises only the
   build and adds it to the changelog heading (section 1), with no tag and no desktop publish.
   The desktop feed skips that number. The
   alternative is a full patch release each time.
6. **The update key's backup.** Recommended: an offline copy (for example an encrypted file on
   a drive kept apart from the Mac), not a cloud copy that syncs. The maintainer chooses where.
7. **The minimum macOS.** Recommended: keep `13.0`, today's `LSMinimumSystemVersion`.
