# 0001. Workspace layout and quality bar

- Status: Accepted; layout amended by [ADR 0003](0003-monorepo-layout.md)
- Date: 2026-11-25

## Context

emberhut (the project; emberhut.com is its website) ships a desktop app, Emberhut, and a server, and
later more clients. They speak one HTTP/SSE contract. The project is open source and
Rust-first, so the code has to read like idiomatic Rust to people who know the language
well.

## Decision

**One Cargo workspace** with:

- `apps/desktop`: the GPUI desktop app, package and binary `emberhut-desktop`, shown as "Emberhut"
  (`Emberhut.app` on macOS, bundle id `com.emberhut.desktop`).
- `crates/emberhut-api`: the contract types only (serde, no I/O). Both sides depend on it.
- `crates/emberhut-client`: a blocking client for the contract. Blocking because GPUI runs
  its own executor and the app moves network calls to background threads; it keeps the
  app free of a second async runtime.
- `crates/emberhut-server`: the server.
- `xtask`: repository automation in Rust, run with `cargo xtask`.

**Naming.** The project, repository and Cargo workspace are `emberhut`; every crate is
`emberhut-<part>` (`emberhut-api`, `emberhut-client`, `emberhut-server`, `emberhut-desktop`). The domain
`emberhut.com` names the website, docs links and the reverse-DNS ids (`com.emberhut.*`).

**Pinned toolchain.** `rust-toolchain.toml` pins the compiler and the workspace sets
`edition = "2024"` and `rust-version` to the same release. Both move together, in one
pull request.

**Lints live in the workspace** (`[workspace.lints]`), and every crate opts in with
`[lints] workspace = true`:

- clippy `pedantic` as warnings; CI sets `CARGO_BUILD_WARNINGS=deny`, so every warning
  (rustc, clippy, rustdoc, Cargo) fails the build;
- `unwrap_used`, `expect_used`, `panic`, `unreachable`, `let_underscore_must_use`, `todo`,
  `dbg_macro`, `print_*` warned; `clippy.toml` lets tests use the panicking ones;
- `missing_docs` and `unreachable_pub` for every crate;
- `unused_workspace_dependencies` denied;
- `allow_attributes` and `allow_attributes_without_reason`: exceptions are
  `#[expect(lint, reason = "...")]`, so a stale exception fails the build;
- `unsafe_code` denied.

**Errors.** Libraries return their own `thiserror` enums. `anyhow` appears only at a
binary's edge (`main`, xtask).

**CI gates every merge** through one gating check, `ci-ok` (required once branch protection
is available): `cargo fmt --check`, clippy on all targets and features, rustdoc,
`cargo nextest run` and doc tests on Linux and macOS, release builds on both, library coverage, `cargo deny check` (licenses, advisories, bans, sources),
`cargo machete`, and a gitleaks scan for secrets and personal data. Actions are pinned to
commit SHAs and tools to versions; Dependabot proposes the bumps.
CI runs on GitHub's hosted runners: every check on Ubuntu, and clippy and rustdoc, the tests
and the release build on macOS too.

**License:** MIT OR Apache-2.0, the Rust ecosystem's norm.

## Consequences

- A contract change breaks the build of every side until both agree.
- Pedantic clippy costs some ceremony (`#[must_use]`, `# Errors` docs); that is the
  point.
- The desktop app needs GPUI's system libraries to build on Linux; CI installs them.
