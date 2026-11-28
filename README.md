<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/emberhut-lockup-on-dark.svg">
    <img src="assets/brand/emberhut-lockup-on-light.svg" alt="Emberhut" width="360">
  </picture>
</p>

# Emberhut

**Emberhut** is an open-source personal assistant you run yourself: a desktop app, **Emberhut**,
and the server it talks to, both written in Rust. Website: [emberhut.com](https://emberhut.com).

> Status: early. The apps and the server are being built in the open; expect breaking changes.

## What's here

```
apps/desktop        the desktop app Emberhut (GPUI), macOS first, Linux and Windows next
apps/hands          Emberhut Hands, the helper on the owner's Mac (Accessibility, screen, recording)
apps/mobile         the React Native app for iOS and Android (moving in)
apps/runner         the runner: runs coding-agent sessions on each machine (TypeScript, Agent SDK)
apps/web            the web app, TypeScript and React (coming)
crates/emberhut-server  the server (axum)
crates/emberhut-core    domain logic, no I/O
crates/emberhut-db      SeaORM entities and migrations
crates/emberhut-api     the API types and their OpenAPI schemas
crates/emberhut-client  the Rust client the desktop app uses
packages/           TypeScript packages: the generated API client, design tokens
deploy/             Docker image and compose file for self-hosting
site/               the website, emberhut.com: static HTML on Cloudflare Pages
xtask/              cargo xtask: macOS bundle, release
docs/               architecture.md and the ADRs
assets/brand/       the Emberhut mark: vector masters, lockups and the social preview
```

The Rust server is the single source of truth for the API. The OpenAPI document and the
TypeScript client will be generated from it, with tests and CI holding both to it. See
[docs/architecture.md](docs/architecture.md).

## Build

```sh
cargo build --workspace              # everything
cargo run -p emberhut-desktop            # the desktop app (Emberhut)
cargo run -p emberhut-server             # the server
```

The desktop app needs a few system libraries on Linux; see [CONTRIBUTING.md](CONTRIBUTING.md).

## Quality bar

Every change passes `rustfmt`, `clippy` (pedantic, warnings denied), the test suite,
`cargo deny` and `cargo machete` in CI. See [CONTRIBUTING.md](CONTRIBUTING.md) and
[ADR 0001](docs/adr/0001-workspace-and-quality-bar.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed
as above, without any additional terms or conditions.
