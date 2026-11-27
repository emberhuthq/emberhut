# 0003. Monorepo layout: Rust, mobile, web and shared packages

- Status: Accepted
- Date: 2026-11-25
- Amends: [ADR 0001](0001-workspace-and-quality-bar.md) (the crate list, and emberhut-api's scope:
  optional `openapi` and `validate` features)

## Context

The repository started as a Cargo workspace (ADR 0001). It will also hold the React Native
mobile app, a web app, the TypeScript packages both use, and what self-hosters deploy. Four
apps (desktop, iOS, Android, web) and one server have to agree on one API without anyone
keeping copies in step by hand.

## Decision

**Layout.**

- `apps/` holds what users install or open: `desktop`, `mobile` and `web`.
- `crates/` holds the Rust libraries and the server.
- `packages/` holds the TypeScript packages, in one pnpm workspace.
- The rest are `docs/`, `deploy/` and `xtask/`.

[architecture.md](../architecture.md) gives the tree, with the reason for each folder.

**Rust crates.**

- `emberhut-server` is the axum server. It ships as a binary, with a thin `main.rs` over a `lib.rs`
  so integration tests can build the router in-process (the usual Rust shape for a testable
  binary).
- `emberhut-core` holds the domain logic. It does no I/O and depends on no workspace crate.
- `emberhut-db` holds the SeaORM entities, migrations and repositories. Application code contains
  no SQL. The schema is defined in migrations, with SeaORM's schema builder, and raw SQL only
  where the builder can't express something.
- `emberhut-api` holds the contract types. Its optional `openapi` feature derives utoipa schemas for
  them, and its optional `validate` feature derives garde rules for the request types. It also
  holds the live stream's event type, `LiveEvent`.
- `emberhut-client` is the blocking Rust client the desktop app uses.
- `emberhut-relay` is the relay and the account directory, one binary with a thin `main.rs` over a
  `lib.rs`, like the server (ADR 0007). Its directory keeps its own SQLite file, with its
  entities and migrations in the crate: it is not the server's database.
- `emberhut-desktop` (in `apps/desktop`) and `xtask` complete the workspace:
  `members = ["apps/desktop", "crates/*", "xtask"]`.

All names follow the layout recorded in architecture.md. Crates are `emberhut-<part>`, the Cargo
convention of a project-prefixed kebab-case package name.

**The API's source of truth is the Rust code, and tests hold the document to it.** The
guarantee has two halves, and each has its own check.

What comes from the code:

- **Schemas** come from the `emberhut-api` types.
- **Paths** come from the server's handlers. Each handler carries `#[utoipa::path]`, and the
  router is built with utoipa-axum, so a documented route is always a served route.
- **The other direction** (every served route is documented) is enforced two ways. clippy's
  `disallowed-methods` bans plain `axum::Router::route` in emberhut-server, and a test checks that
  the router's path set equals the document's.
- **Statuses and bodies** are written in the annotations by hand. Server integration tests call
  every documented route, success and error cases alike, and validate the real status and body
  against the document.
- **The live stream** (`text/event-stream`) can't be described by the OpenAPI 3.1 that utoipa
  emits. So its payloads are one serde-tagged `LiveEvent` enum in emberhut-api, registered as a
  component schema and named in the stream endpoint's description. A server test checks that
  every event it emits deserializes as `LiveEvent`.

What keeps the generated files current:

- **The document.** `crates/emberhut-server/openapi.json` is a snapshot test in emberhut-server. It
  builds the document, compares it with the committed file, and rewrites it when run with
  `EMBERHUT_BLESS=1`. Maps keep utoipa's default sorted order, so the file is deterministic and
  doesn't change when routes are only reordered. It is pretty-printed with a fixed format and
  sits beside the server, which produces it.
- **The TypeScript client.** `packages/api-client` is generated from that file by
  `openapi-typescript` (the types) and uses `openapi-fetch` (the runtime). Both are pinned in
  the committed `pnpm-lock.yaml`. A `js` CI job runs `pnpm install --frozen-lockfile`, then the
  generator, then `git diff --exit-code packages/api-client`.

One nuance against that layout, which gave emberhut-api "the OpenAPI document". emberhut-api
contributes the document's components: every schema, `LiveEvent` included. The server adds the
paths and owns the assembled file. Paths belong to handlers, and only the server has handlers.
The Rust half needs no xtask, so `cargo xtask` doesn't have to compile the server.

**Design tokens.** One JSON source, `packages/design-tokens/tokens.json`, is the only committed
token file:

- The desktop app's `build.rs` generates its Rust module into `OUT_DIR`.
- The TypeScript package generates its module in its own build step.

Neither output is committed, so neither can go stale.

**Tasks are Rust.** Release and the macOS bundle are `cargo xtask` subcommands, so a contributor
needs cargo and pnpm and no shell scripts.

**Status.** This ADR records the target. The parts land in this order: the contract crates and
the desktop app; then the server's first slice, which adds the document, its snapshot and the
route tests; then the TypeScript client and the `js` CI job; then the design tokens.
[architecture.md](../architecture.md) keeps a table of what exists today.

## Consequences

- A contract change is one pull request. The Rust side doesn't compile until it agrees. The
  snapshot test fails until the document is blessed, and the `js` job fails until the
  regenerated client is committed.
- Reviewers see every contract change as a diff of `openapi.json`.
- Until a package has code, `apps/mobile`, `apps/web`, `packages/*` and `deploy/` are
  placeholders with a README, and CI has no `js` job.
