# Releasing Emberhut

Releases are GitHub releases of this repository, made by `.github/workflows/release.yml` and
`cargo xtask release` ([ADR 0032](adr/0032-github-releases.md)). A release goes through five
steps, each of which checks the one before it:

1. **Tag**: a maintainer pushes `vX.Y.Z-rc.N` or `vX.Y.Z` on a commit of `main`.
2. **CI builds and drafts**: the tag's workflow run builds the servers and the Windows installer
   and opens a draft release with them.
3. **The Mac signs**: on the Mac whose Keychain holds the Developer ID identity,
   `cargo xtask release mac --tag <tag>` builds, signs and notarizes the macOS disk image and
   server, uploads them to the draft and starts the publish stage.
4. **CI publishes**: the publish stage scans every asset, writes the updater's signed feeds and
   `SHA256SUMS`, writes the notes and publishes the draft.
5. **Live**: the release page lists everything, and the apps on its channel find it.

## Versions

- A **release candidate** is `vX.Y.Z-rc.N`, N from 1 to 98: a GitHub pre-release, installed by
  the app's "Release candidates" channel only.
- A **final release** is `vX.Y.Z`: installed by every channel.
- `X.Y.Z` must equal `version` in the root `Cargo.toml`. Start a new version with a pull request
  that changes it; the tags themselves change no file.
- The build number is derived from the tag: `X*1,000,000 + Y*10,000 + Z*100 + N` (99 for the
  final), so `v0.1.0-rc.1` is build 10001 and `v0.1.0` is 10099. The Windows installer's version
  is `X.Y.(Z*100 + N or 99)`: `0.1.1` for `v0.1.0-rc.1`, `0.1.99` for `v0.1.0`.
- `cargo xtask release version v0.1.0-rc.1` checks a tag and prints every value.

## Cutting a release candidate

```sh
git switch main && git pull --ff-only
cargo xtask release version v0.1.0-rc.1      # checks the tag against the workspace version
git tag -a v0.1.0-rc.1 -m "Emberhut 0.1.0-rc.1"
git push origin v0.1.0-rc.1
```

Then wait for the tag's run of the **Release** workflow (`gh run watch`), and do the Mac's step
below.

## Cutting a final release

The same, with the plain version, once the last release candidate has been tried:

```sh
git tag -a v0.1.0 -m "Emberhut 0.1.0"
git push origin v0.1.0
```

## What the workflow checks

On the tag (the **plan** job), before building anything:

- the tag is `vX.Y.Z-rc.N` (N 1 to 98) or `vX.Y.Z`, without leading zeros, and its `X.Y.Z` is the
  workspace's version;
- the tagged commit is on `main`;
- CI's `ci-ok` check passed on that commit (it waits up to an hour for a run in progress).

Before publishing (the **publish** job):

- the release is still a draft, marked pre-release exactly when the tag is a release candidate;
- it holds at least the six built assets below; whatever else is on the draft is downloaded,
  scanned and listed in `SHA256SUMS` too (feeds and sums from an earlier attempt are made anew);
- every asset, unpacked (archives, the disk image, the installer), and every binary's strings
  pass gitleaks with the repository's `.gitleaks.toml` (through
  `.github/actions/release-scan/binaries.toml`, which allows only the few addresses that
  dependencies compile into the binaries) and with the private rules in `PRIVATE_WORDS_TOML`;
  so do the feeds, the sums and the notes it writes;
- the feeds and sums it signed verify against the public update key the app trusts.

A pull request that changes the release machinery (the workflow, `xtask/`, the installer's
sources, `crates/emberhut-api/src/releases.rs`) runs a **dry run**: the same builds plus an
unsigned universal macOS build, the same scan, and a feed signed with a key made for the run. It
has no secrets and no environment, publishes nothing, and keeps its outputs as the run's
artifacts.

## The Mac's step

Once, on the Mac:

- the Developer ID Application identity in the login Keychain
  (`security find-identity -v -p codesigning` lists it);
- notary credentials: either a keychain profile made from the App Store Connect API key,
  `xcrun notarytool store-credentials emberhut-notary --key <AuthKey_ID.p8> --key-id <ID> --issuer <issuer UUID>`,
  or the key file in place, given to each run with `--notary-key`, `--notary-key-id` and
  `--notary-issuer` (it is read where it is, never copied);
- both Rust targets: `rustup target add aarch64-apple-darwin x86_64-apple-darwin`;
- `gh auth login` with access to the repository's releases and workflows.

Each release, from a clean checkout of the tag:

```sh
git fetch --tags origin
git switch --detach v0.1.0-rc.1
export EMBERHUT_SIGN_IDENTITY="Developer ID Application: <name> (<team id>)"
cargo xtask release mac --tag v0.1.0-rc.1 --keychain-profile emberhut-notary
```

It checks the checkout (clean, exactly at the tag, the same commit as the tag on `origin`), that
the tag's build-stage run ("Release v0.1.0-rc.1 (build)") succeeded and that the draft exists; then builds under `nice` (`-j 4`
unless given), signs, notarizes, runs Gatekeeper's checks and the app's `--self-check` from the
disk image (only in a logged-in desktop session; otherwise it says it skipped them), uploads
`Emberhut-V-macos-universal.dmg` and `emberhut-server-V-macos-universal.tar.gz` to the draft, and
starts the publish stage. Before starting it, it signs the runner: CI puts
`emberhut-runner-V-macos-arm64.tar.gz` and `…-macos-x64.tar.gz` on the draft unsigned (an
unsigned binary hangs behind the macOS firewall), and `release mac` downloads each into a
private folder, signs `emberhut-runner` (hardened runtime, timestamp, identifier
`com.emberhut.runner`), notarizes it as a zip, repacks it with the same folder, names and modes
(checking that `tar -tzf` lists the same entries) and uploads it again with `--clobber`. With no
runner archives on the draft it says so and skips this. `--no-publish` stops before starting the
publish stage.

## The assets

V is the full version (`0.1.0-rc.1`).

| Asset | What it is | Made by |
| --- | --- | --- |
| `Emberhut-V-macos-universal.dmg` | The desktop app for macOS 13+, Apple silicon and Intel, with the server inside (ADR 0030); signed, notarized and stapled | the Mac |
| `Emberhut-V-windows-x64.msi` | The desktop app for Windows 10 2004+ x64, per-user install, with the server; not code-signed yet | CI |
| `emberhut-server-V-linux-x64.tar.gz`, `…-linux-arm64.tar.gz` | The server alone, for Linux | CI |
| `emberhut-server-V-macos-universal.tar.gz` | The server alone, for macOS; signed and notarized | the Mac |
| `emberhut-server-V-windows-x64.zip` | The server alone, for Windows | CI |
| `emberhut-runner-V-macos-arm64.tar.gz`, `…-macos-x64.tar.gz` | The runner for macOS; signed and notarized | CI builds, the Mac signs |
| `emberhut-desktop-macos.json`, `emberhut-desktop-windows.json` | The updater's feeds: the release, its notes, and each installer's URL, size, SHA-256 and minisign signature | the publish stage |
| `SHA256SUMS`, `SHA256SUMS.minisig` | The SHA-256 of every other asset, and its signature by the update key | the publish stage |

Each server archive holds the binary, `LICENSE-MIT`, `LICENSE-APACHE` and `README.md`.

## When a step fails

Nothing is public until the publish stage's last command, so a failed release is a draft to fix
or throw away.

- **The plan job fails** (a malformed tag, a commit not on `main`, CI not green): delete the tag
  (`git push origin :refs/tags/vX.Y.Z-rc.N` and `git tag -d vX.Y.Z-rc.N`), fix, and tag again.
  For a CI run that was still going, rerun the plan once it's green: `gh run rerun <run id>`.
- **A build fails in CI**: if it was a flake, `gh run rerun <run id> --failed`, or redo the whole
  stage with `gh workflow run release.yml --ref <tag> -f tag=<tag> -f stage=build`. If the code needs
  a fix, delete the draft (`gh release delete vX.Y.Z-rc.N --yes`) and the tag, merge the fix, and
  tag again; for a release candidate, the next number (`-rc.N+1`) avoids reusing a tag anyone may
  have fetched.
- **`release mac` fails**: it changes nothing on GitHub before its upload, so fix the cause (a
  notarization rejection is in `target/bundle/notary-log.txt`) and run it again. After a failure
  between the upload and the publish stage, run it again (uploads replace the assets) or start
  the stage by hand: `gh workflow run release.yml --ref <tag> -f tag=<tag> -f stage=publish`.
- **The publish stage fails**: read why (a scan finding names the file; a missing asset is
  listed). Fix the asset (rerun the build or `release mac`) and start the stage again with the
  command above; it replaces the feeds and sums it wrote before. A scan finding in a binary
  means a source change: delete the draft and the tag, fix, and tag again.

## Secrets and keys

The GitHub environment **`release`** holds the workflow's only secrets, and its deployment rule
admits `v*` tags only. Only the publish job uses it. No Apple credential is in GitHub.

- **`UPDATE_SIGNING_KEY`**: the update key's secret half, an unencrypted minisign secret key
  file. Its public half is in `apps/desktop/src/updates/keys.rs` (`TRUSTED`) and in
  `release.yml` (`UPDATE_PUBLIC_KEY`). To rotate it:
  1. `cargo xtask release keygen --out-dir <a private folder>` writes `emberhut-update.key`
     (mode 0600) and `emberhut-update.pub`, and prints the key id and public key;
  2. ship a release whose app trusts both the old and the new key (add the new one to `TRUSTED`);
     it is still signed with the old key;
  3. once installed apps have that release, put the new key in the secret
     (`gh secret set UPDATE_SIGNING_KEY --env release < emberhut-update.key`), change
     `UPDATE_PUBLIC_KEY` in `release.yml`, and drop the old key from `TRUSTED` in a later
     release;
  4. keep an offline backup of the secret file, then delete the working copy.

  A key that leaked is rotated the same way, faster: the old key stops being trusted in the very
  next release.
- **`PRIVATE_WORDS_TOML`**: the gitleaks rules for words that must not appear in a release. To
  change them: `gh secret set PRIVATE_WORDS_TOML --env release < private-words.toml`.
- **On the Mac, not in GitHub**: the Developer ID Application certificate (renewed in Apple's
  developer account and imported into the Keychain; set the new name in `EMBERHUT_SIGN_IDENTITY`)
  and the App Store Connect API key used for notarization (revoked and made anew in App Store
  Connect, then stored again with `xcrun notarytool store-credentials emberhut-notary …`).

## Signing the Windows installer

The Windows installer and its executables aren't Authenticode signed yet, so SmartScreen warns
on first run and the release notes say how to get past it. Once there is a certificate (Azure
Trusted Signing, or an OV or EV code signing certificate):

1. in the `windows` job of `release.yml`, sign `emberhut-desktop.exe` and `emberhut-server.exe`
   before they go into the installer, then the `.msi` itself, with `signtool sign /fd sha256 /tr
   <timestamp URL> /td sha256` (or the Trusted Signing action), its credentials in the `release`
   environment and the job bound to it on tags;
2. in the updater, check the downloaded installer's Authenticode signature and publisher as well
   as its minisign signature before running it;
3. drop the SmartScreen note from the release notes in `release.yml`.
