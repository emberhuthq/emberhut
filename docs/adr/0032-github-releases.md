# 0032. GitHub releases: built in CI, signed for macOS on the Mac, versions from the tag

- Status: Accepted
- Date: 2026-11-28
- Supersedes parts of: [ADR 0005](0005-releases-and-updates.md) (hosting, the build counter, the
  release commands and the tag job; its signing-on-the-Mac trust model stays)
- Builds on: [ADR 0030](0030-the-hut-inside-the-app.md) (the server inside the app bundle)

## Context

ADR 0005 designed releases for a private repository: a feed on the app's own server behind its
login, a build counter committed in `Cargo.toml`, a `CHANGELOG.md` section as the notes, and a
Mac that builds, signs, notarizes and uploads everything. Since then three things changed:

- **The repository is public.** Its downloads can come from GitHub, where anyone can fetch them
  without an account, and ADR 0005's section 5 already named GitHub Releases as the channel once
  the source is public.
- **There are more platforms than one Mac can build.** The desktop app now has a Windows
  installer, and the hut's server ships on its own for Linux (x64, arm64), macOS and Windows.
  One Mac can't build Windows and Linux binaries; GitHub's hosted runners can.
- **Builds need trying before they are final.** A release candidate has to reach the apps
  that opt in without reaching every app.

## Decision

### 1. Releases are GitHub Releases

Every release is a GitHub release of `emberhuthq/emberhut`, made by
`.github/workflows/release.yml`, with these assets (V is the full version, e.g. `0.1.0-rc.1`):

- `Emberhut-V-macos-universal.dmg`: one disk image for Apple silicon and Intel (the binaries are
  joined with `lipo`), the app and the bundled server signed with the Developer ID identity and
  the hardened runtime, the app and the image notarized and stapled. ADR 0005's open question 3
  recommended one image per processor; a universal image is one download page entry and one
  updater asset for both, at the cost of a build twice as long, which CI's dry run and the Mac
  absorb.
- `Emberhut-V-windows-x64.msi`: a per-user installer (no administrator prompt).
- `emberhut-server-V-linux-x64.tar.gz`, `…-linux-arm64.tar.gz`, `…-macos-universal.tar.gz`
  (signed and notarized) and `…-windows-x64.zip`, each with the licenses and the README.
- The updater's feeds, `emberhut-desktop-macos.json` and `emberhut-desktop-windows.json`, in
  ADR 0005's format 1, with absolute `https://github.com/…/releases/download/vV/<file>` URLs. The
  universal image is listed for `arm64` and for `x86_64`, each with its own signature, since the
  signature's trusted comment names the processor.
- `SHA256SUMS` of every other asset, and `SHA256SUMS.minisig` by the update key.

The feed's notes are the merged pull requests GitHub lists for the release (`- title (#N)` lines,
at most 4,000 bytes), generated from the pull request titles rather than a hand-kept changelog:
the titles already follow the `area: plain sentence` rule, and a second copy would drift. The
release page shows GitHub's generated list with an install section above it.

The app's server feed (ADR 0005 section 5) and its four routes are not built: an installed app
reads GitHub's public API, with no credential, and verifies every payload as before.

### 2. Who builds and signs what

- **CI builds everything that isn't Apple-signed.** A pushed `v*` tag runs the build stage: it
  checks the tag (below), builds the Linux and Windows servers and the Windows installer on GitHub's
  hosted runners, and opens a **draft** release with them.
- **The Mac builds and signs the macOS assets.** `cargo xtask release mac --tag <tag>` runs on
  the Mac whose Keychain holds the Developer ID identity. It checks that the checkout is clean
  and exactly at the tag, that the tag on `origin` is the same commit, that the tag's workflow run
  succeeded and that the draft exists; then it builds the universal app and server, signs and
  notarizes them (with a notarytool keychain profile or an App Store Connect API key read in
  place), runs Gatekeeper's checks and the app's `--self-check` from the disk image when a desktop
  session is there, uploads the two assets to the draft, and starts the publish stage.
- **CI publishes.** The publish stage (`workflow_dispatch`, `stage=publish`, on the tag's ref)
  refuses unless the draft holds every asset a release must have. It downloads all the draft's
  assets, whichever job added them, unpacks
  every archive, the disk image and the installer, and scans their contents and the binaries'
  strings with the repository's gitleaks rules and a private set of rules; any finding stops it.
  Then it writes and signs the feeds and sums with the update key, checks them as the app would,
  uploads them, writes the notes and publishes the draft.

ADR 0005's trust section stands for Apple's credentials: the Developer ID identity never leaves
the Mac's Keychain, and no Apple credential (certificate, API key, password) is a GitHub secret.
A compromised workflow or action therefore still can't make an app that macOS opens without a
warning.

**The update key is a CI secret.** It is the one exception, and it is limited:

- it lives only in the GitHub environment `release`, whose deployment rule admits `v*` tags only;
  the publish stage is the only job bound to it, and only a run on a tag's ref reaches it;
- pull requests never get it: their runs are dry runs with no environment, no secrets and a
  read-only token, and the workflow has no `pull_request_target` trigger;
- every action is pinned by its full commit SHA, and no job restores a cache, so a release is
  built from the tagged sources and the lock file alone;
- it is not Apple's: a stolen update key alone can't make a build that macOS opens, and a stolen
  Apple identity alone can't make a payload the updater installs;
- it is rotated as ADR 0005 describes (an app that trusts both keys ships first), and
  `cargo xtask release keygen` makes a new one with no other tool.

Signing the feed in CI lets a release finish without the Mac staying involved after its upload,
and puts the feed's inputs (CI's own assets and the Mac's) in one place. The alternative keeps
the update key on the Mac too: `release mac` would download CI's assets, sign the feed and sums
and publish, and the workflow would need no secret but the scan rules. That remains an option if
the environment's rule or GitHub's secret handling ever proves weaker than assumed; the feed
code is the same either way.

### 3. Versions and builds come from the tag

- A tag is `vX.Y.Z-rc.N` (N from 1 to 98) or `vX.Y.Z`, and `X.Y.Z` must be the workspace's
  `version`, which stays plain. Nothing in `Cargo.toml` changes per tag.
- `EMBERHUT_VERSION` is the tag without the `v`; `EMBERHUT_BUILD` is
  `X*1,000,000 + Y*10,000 + Z*100 + N` (99 for the final release). It rises with semver, is the
  same on every rebuild of a tag, and needs no committed counter; Y and Z stay below 100 so the
  order holds. This replaces ADR 0005's `[workspace.metadata.release] build`: the first release's
  build (10001) is far above any build either phone store has seen.
- The Windows installer's `ProductVersion` is `X.Y.(Z*100 + N or 99)`, because Windows Installer
  compares only the first three fields.
- On macOS, `CFBundleShortVersionString` is `X.Y.Z` (Apple accepts only numbers there),
  `CFBundleVersion` is the build, and `EmberhutVersion` holds the full version.
- `cargo xtask release version <tag>` does the check and prints these values; the workflow, the
  Mac's leg and `bundle-macos` (when `EMBERHUT_VERSION` and `EMBERHUT_BUILD` are set) all use it.
- Before building, the workflow requires the tagged commit to be on `main` and CI's `ci-ok`
  check to have passed on it, waiting up to an hour for a run in progress.

### 4. Channels

A release candidate is a GitHub pre-release; a final release is a full release. The app's
"Stable" channel installs full releases only, and its "Release candidates" channel installs both,
so a final release newer than the candidates still wins. The build number decides which release
is newest.

### 5. Windows is unsigned until there is a certificate

The installer and its executables carry no Authenticode signature yet, so SmartScreen warns on
first run; the release notes say so and point at `SHA256SUMS`. The updater's own minisign check
protects updates either way. Once a certificate exists (Azure Trusted Signing, or an OV/EV
certificate), `signtool` signs the two executables and the installer in the Windows job, and the
updater also checks the installer's Authenticode signature (docs/releasing.md).

## Consequences

- A release is a tag, one command on the Mac, and nothing else; each step refuses to go on when
  something doesn't match, and each can be resumed (docs/releasing.md).
- A release still needs the Mac for its macOS assets. Until the Mac has run its step, the release
  stays a draft that only maintainers see.
- The update key is in GitHub; its exposure is limited to the tag-only publish stage, and
  macOS's own checks don't depend on it.
- There is no `CHANGELOG.md`: the release notes are only as good as the pull request titles.
- The phones keep their store builds (ADR 0005 section 6); their build numbers come from the same
  formula when their release step is built.
