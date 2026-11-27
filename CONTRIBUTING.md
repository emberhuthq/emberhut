# Contributing to Emberhut

Thanks for helping. This page is the whole process.

## Layout

See [docs/architecture.md](docs/architecture.md): every folder and why it exists.

## Toolchain

`rust-toolchain.toml` pins the compiler; `rustup` picks it up. The workspace's
`rust-version` is the MSRV and moves only together with that file.

The desktop app on Linux needs the GPUI system libraries, and ALSA's (`libasound2-dev`) for
the microphone (dictation); the app needs `libasound2` at run time:

```sh
sudo apt-get install libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev \
  libx11-xcb-dev libxcb1-dev libfontconfig-dev libfreetype-dev libssl-dev libzstd-dev pkg-config \
  libasound2-dev
```

For the TypeScript side (the mobile and web apps and `packages/`), use the Node in `.nvmrc`
(CI runs exactly that one; `nvm use`, `fnm use` or `volta` read it; `engines` takes 22.18 or a
later 22, which runs `app.config.ts`'s TypeScript imports, and `@types/node` matches that floor)
and the pnpm version pinned in `package.json` (`npm install -g pnpm@<that version>`, or
`corepack enable` on Node versions that still ship corepack), then
`pnpm install --frozen-lockfile`.

On macOS the Xcode command line tools are enough: gpui-kit builds GPUI with its runtime
shaders, so the Metal shaders are compiled by the system when the app starts, not by
`xcrun metal` at build time (see apps/desktop/README.md, "Metal shaders").

## Before you open a pull request

CI (`.github/workflows/ci.yml`) runs these, with these flags, and sets
`CARGO_BUILD_WARNINGS=deny` so any warning (rustc, clippy, rustdoc or Cargo's own) fails.
Run the same locally:

```sh
export CARGO_BUILD_WARNINGS=deny
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked
cargo doc --workspace --all-features --no-deps --locked
cargo nextest run --workspace --all-features --locked
cargo test --workspace --all-features --doc --locked
cargo build --workspace --release --locked
cargo deny --locked check advisories bans licenses sources
cargo machete
gitleaks git --config .gitleaks.toml --redact .
gitleaks dir --config .gitleaks-public.toml --redact .
```

CI also measures library coverage (`cargo llvm-cov nextest`) and runs the tests and the
release build on macOS too. A maintainer merges a pull request only when the `ci-ok` check,
which needs every other job, is green.

## Code standards

- Follow the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/).
- Clippy's `pedantic` group is on for the whole workspace. Silence a lint only with
  `#[expect(clippy::lint_name, reason = "why")]`, never a bare `#[allow]`.
- No `unwrap`, `expect`, `panic!` or `unreachable!` outside tests, unless the case is truly
  unreachable and an `#[expect]` says why. No `let _ =` on a value that must be used.
- Libraries return typed errors (`thiserror`); `anyhow` is for binaries' edges only.
- Every public item has a doc comment: `missing_docs` is a workspace lint, and every crate root
  starts with a `//!` line saying what the crate is.
- Visibility is as narrow as it can be (`unreachable_pub` is on): `pub(crate)` inside binaries.
- Both lints reach every crate root, tests included: each file in `tests/`, `examples/` and
  `benches/` starts with a `//!` line, and helpers shared between tests are `pub(crate)`.
- Keep modules small and focused: one module hides one decision.
- New behaviour comes with tests. Changes to the HTTP contract come with a contract test
  in `crates/emberhut-api/tests` and in the server.
- A decision that is hard to reverse gets an ADR in `docs/adr`.

## Commits

Small commits with a clear subject line in the imperative ("Add the pairing route").

## License

By contributing you agree that your work is dual licensed under MIT and Apache-2.0, as in
the README, without any additional terms or conditions.

## Secrets and personal data

Never commit secrets, tokens, keys or `.env` files, and keep real personal data out of code,
tests and docs: no real names, IP addresses, hostnames or email addresses (use `example.com`).
The CI scan enforces this with gitleaks and `.gitleaks.toml`.

## Writing for a public repository

Code, comments, test names, docs, ADRs, commit messages and pull request texts describe the
feature and its technical reasons, and nothing else:

- no people, by name or otherwise, and no account of who asked for or decided something;
- no private task or ticket ids, no private paths (home directories, a tool's local data
  directory) and no private sources (internal reports, private repositories, chats);
- a reason is technical: a constraint, a measurement, an alternative compared on its merits. A
  public project may be named as prior art.

The contract's wire words (a header name, a JSON value, a link scheme, a file name) are values,
not prose: each crate or app keeps them in one place (`emberhut-api`'s `wire` module,
`emberhut-core`'s `memory::wire` and `task::wire`, the desktop app's `src/wire.rs`, the mobile
app's `src/api/wire.ts`), each with a doc comment saying what it names.

CI enforces part of this on the files of each commit: `.gitleaks-public.toml` has rules for
home-directory paths and for phrases that say who asked for or decided something. Test data is
allowed by the rules' allow lists. The rest is for the author and the reviewer.
