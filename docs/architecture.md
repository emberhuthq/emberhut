# Architecture

emberhut is one repository for every part of the product: the Rust server and desktop app, the
React Native mobile app, the web app, and the TypeScript packages they share. The decision and
its reasons are in [ADR 0003](adr/0003-monorepo-layout.md).

## The rule

**The Rust server is the single source of truth for the API.** The schemas come from the
`emberhut-api` types, and the paths from the server's handlers. Tests hold the generated OpenAPI
document to what the server really does:

- every served route is documented, and every documented one is served;
- real responses are validated against the document;
- every live-stream event is a `LiveEvent`.

A snapshot test keeps `crates/emberhut-server/openapi.json` current. The TypeScript client in
`packages/api-client` is generated from that file, and a CI job fails if the committed client
differs. [ADR 0003](adr/0003-monorepo-layout.md) has the details. These checks arrive with the
server's first slice and the first TypeScript package; the Status table below says what exists
today.

Design tokens have one committed source, `packages/design-tokens/tokens.json`. The desktop's
`build.rs` and the TypeScript package's build step generate their code from it, so there is no
generated token file to keep in step.

## Layout

```
emberhut/
├── apps/
│   ├── desktop/          GPUI desktop app, Rust (crate emberhut-desktop)
│   ├── mobile/           React Native app, iOS and Android
│   └── web/              web app, TypeScript and React
├── crates/
│   ├── emberhut-server/      the server: an axum binary, and the OpenAPI document it serves
│   ├── emberhut-core/        domain logic, no I/O
│   ├── emberhut-db/          SeaORM entities and migrations
│   ├── emberhut-api/         request and response types, the OpenAPI schemas
│   ├── emberhut-client/      the Rust HTTP client the desktop app uses
│   ├── emberhut-identity/    machine keys, machine ids and signed statements
│   └── emberhut-relay/       the relay and the account directory: one binary (the directory so far)
├── packages/             TypeScript packages, one pnpm workspace
│   ├── api-client/       TS client generated from the server's OpenAPI document
│   ├── connect/          (planned) TS route rules and statements, on the shared vectors
│   └── design-tokens/    one JSON source of colours, type and spacing
├── docs/
│   ├── adr/              architecture decision records
│   └── architecture.md   this page
├── deploy/               the self-host package: Docker image and compose file
│   └── relay/            (planned) the relay's container image and compose file
├── xtask/                Rust task runner: macOS bundle, release
├── patches/              fixes to npm packages, applied by pnpm on install
├── Cargo.toml            the Rust workspace
├── package.json          the pnpm workspace root
├── pnpm-workspace.yaml
└── .github/workflows/    CI
```

| Folder | Why it exists |
| --- | --- |
| `apps/` | Everything a user installs or opens; each app is built and shipped on its own. |
| `apps/desktop` | The Mac (later Linux and Windows) app, in GPUI so it shares Rust types with the server. It can run the server on its own computer, the hut ([ADR 0030](adr/0030-the-hut-inside-the-app.md)): the binary ships in the bundle and is driven as a process, never linked. |
| `apps/mobile` | The iOS and Android app, in React Native, one codebase for both phones. |
| `apps/web` | The browser app, in TypeScript and React, sharing the generated client with mobile. |
| `crates/` | Every Rust library and the server, so `crates/*` is the Rust workspace's member glob. |
| `crates/emberhut-server` | The HTTP/SSE server: routing, middleware, wiring. It is the only place that serves the API, so it owns `openapi.json`. |
| `crates/emberhut-core` | Domain rules (pairing, sessions, rooms, ETags, agents and approvals) with no I/O, so they're unit-tested without a database or network; the desktop app shares the ones it must apply as the server does. |
| `crates/emberhut-db` | SeaORM entities, migrations and repositories in one place, the only crate that knows the server's schema. |
| `crates/emberhut-api` | The contract types every Rust side shares, and their OpenAPI schemas (optional feature). |
| `crates/emberhut-client` | A blocking Rust client for the contract, used by the desktop app on background threads. |
| `crates/emberhut-identity` | A machine's Ed25519 keys, the machine id derived from the public key, and the one encoding of every signed statement ([ADR 0007](adr/0007-connectivity.md)), with no I/O, so every side signs and checks the same bytes. |
| `crates/emberhut-relay` | The relay machines dial out to and the account directory, one binary ([ADR 0007](adr/0007-connectivity.md)). The directory is built (`src/directory/`, with its own SQLite file and migrations, since the server's schema in `emberhut-db` is not the relay's); the gateway is planned. |
| `packages/` | TypeScript shared by mobile and web, kept in one pnpm workspace. |
| `packages/api-client` | The TS client generated from the server's OpenAPI document, so mobile and web share one checked contract. |
| `packages/connect` | (planned) The route rules and the statement encoding in TypeScript for mobile and web, held to the Rust ones by the shared test vectors in `crates/emberhut-api/tests/fixtures/connect/`. |
| `packages/design-tokens` | One source of colours, type and spacing, read by the TS build and the desktop's `build.rs`. |
| `docs/` | Decisions (`adr/`) and this map, next to the code they describe. |
| `deploy/` | What self-hosters run: the server's Docker image and a compose file. |
| `deploy/relay/` | (planned) What a self-hosted relay runs: its container image and a compose file. |
| `xtask/` | Repository tasks in Rust (the macOS bundle, release), so there are no shell scripts to keep in sync. |
| `patches/` | Fixes to npm packages that their releases don't have yet, applied by pnpm on every install (`patchedDependencies` in `pnpm-workspace.yaml`), so a fix lives in the repository instead of a fork. |
| Root files | `Cargo.toml` is the Rust workspace, `package.json` and `pnpm-workspace.yaml` the JS one, `.github/workflows` CI for both. |

## Dependencies between the parts

```
emberhut-desktop ──► emberhut-client ──► emberhut-api ──► emberhut-identity
      ├────────────────────────────► emberhut-api
      ├──► emberhut-core
      └──► packages/design-tokens/tokens.json (read by build.rs)
emberhut-server ──► emberhut-api, emberhut-core, emberhut-db
emberhut-relay ──► emberhut-api, emberhut-identity
emberhut-db ──► emberhut-core
xtask ──► (no workspace crates)
mobile, web ──► packages/api-client ◄── crates/emberhut-server/openapi.json
            └─► packages/design-tokens

planned (ADR 0007):
emberhut-client, emberhut-server ──► emberhut-identity
mobile, web ──► packages/connect ◄── crates/emberhut-api/tests/fixtures/connect/ (the vectors)
```

- `emberhut-core` depends on no other workspace crate and does no I/O. Because it is pure, the
  desktop app uses it too where it must apply the server's own rule (which characters an
  approval request's words may not hide, `emberhut_core::approvals::shown`), so both sides show
  the same thing.
- `emberhut-identity` depends on no other workspace crate and does no I/O: the server keeps the
  key file, and callers send what is signed.
- `emberhut-api` depends only on `emberhut-identity`, whose keys, ids and signatures the connect,
  relay and directory types carry. It is serde types, with optional `openapi` (utoipa) and
  `validate` (garde) features.
- `emberhut-db` maps between its entities and `emberhut-core`'s domain types. Application code
  contains no SQL: the schema is defined in migrations.
- `emberhut-server` wires everything once, in `main`, into axum `State`. It uses constructor
  injection with no container.
- `emberhut-desktop` doesn't depend on `emberhut-server`: the hut it runs is the server's binary,
  bundled beside the app and started as a process (its `serve`, `pair --json`, `id` and
  `--version`), and spoken to over HTTP like any server.

## Plugging an area into the server

Three places in `crates/emberhut-server` take every feature area: the MCP endpoint's tools, the
settings actions and the state's keys. Each is a registry, so an area adds a module of its own
and a line ([ADR 0009](adr/0009-mcp-settings-and-state-groundwork.md) has the reasons). The
area's logic lives in its service (`services/<area>.rs`), tested on its own; these are thin.

- **An MCP tool** (`POST` at `emberhut_api::paths::MCP_ROOM`, called by a room's agent session): a
  module `src/mcp/tools/<area>.rs` (declared with `mod <area>;` in `src/mcp/tools/mod.rs`) with a
  type implementing `Tool`: its `name`, its `description`, an `Args` type deriving `Deserialize`,
  `utoipa::ToSchema` and `garde::Validate` (or `NoArgs`), and `async fn call(&self, ctx:
  &ToolContext, args)`, which calls the area's service through `ctx.app` and acts in `ctx.room`.
  Then one line in `room_tools()`: `tool(<area>::TheTool),`. Its test calls the endpoint
  (`tests/server/common.rs` has the helpers: `rpc`, `call_tool`, `text`): `tools/list` lists it,
  `tools/call` does what it says.
- **A settings action** (`POST /api/settings`): a module `src/settings/<area>.rs` (declared in
  `src/settings/mod.rs`) with a type implementing `Action`: its `key` (the body field, already
  in `ORDER`, which fixes when it runs) and `async fn apply(&self, ctx: &ActionContext, body:
  &Body)`, which reads its field (and any that go with it), does nothing for a field of the wrong
  kind, and refuses with `ActionError::Refused` (400 `{ ok: false, error }`). Then one line in
  `actions()`: `action(<area>::TheAction),`; and the field in `emberhut-api`'s `SettingsRequest`.
- **A state key** (`GET /api/state` and every settings answer): the area's service as a field
  of `services::state::Sections`, its line where `AppState::new` makes the `Sections`, and the
  key read from it in `StateService::state`'s `State`, in place of its stand-in
  (`issues: self.sections.tasks.issues().await?,`; a service error that is not a
  `DbError` goes through `.map_err(StateError::section)?`).

## Status

| Part | Exists today |
| --- | --- |
| Workspace, CI (fmt, clippy, rustdoc, tests, builds, coverage, deny, machete, secret scan, workflow lint) | yes |
| `crates/emberhut-api`, `crates/emberhut-client` | in progress |
| `apps/desktop`, `xtask` | in progress |
| `crates/emberhut-server`, `emberhut-core`, `emberhut-db` (ADR 0002 first) | in progress |
| OpenAPI document, its snapshot and route tests | with the server's first slice |
| `packages/api-client` and the `js` CI job | after the first slice |
| `packages/design-tokens` | later |
| `apps/mobile` | yes: the React Native app (iOS and Android), checked by the `js` CI job (ADR 0003); its API client is hand-written until `packages/api-client` |
| `apps/web`, `deploy/` | placeholders |
| The MCP endpoint and its tools, the settings actions, the state's sections (ADR 0009) | yes: the registries, with `theme`, `ui` and the theme routes; the areas add theirs |
| Tasks, projects, areas, findings, artifacts, runs and the plan (ADR 0015) | yes: the tables, the board and the state's keys, `/api/tasks*`, `/api/artifacts/*`, `/api/scheduled`, their settings actions and tools; the workers, runner sessions and projects' rooms plug in when their areas land |
| Task plans (ADR 0035): a task's steps (`plan_steps`), the plan tools, each task's plan in the state and on its page | yes: behind `--task-plans` (`EMBERHUT_TASK_PLANS`), off by default |
| The agent runtime (a CLI session per room, failover, interrupt), the wake rule, check-ins, watches, routines and the assistant's own tools (ADR 0014) | yes: `Silent` unless `EMBERHUT_AGENT_COMMAND` is set |
| The assistant's workers (a CLI session per task on `POST /mcp/worker`), ultra runs (`POST /ultra/gate`) and the guard rails: grants, the chat's OK, a card's Send it, Go ahead cards, outward steps, the ask guard, keys masked (ADR 0018) | yes: workers run when `EMBERHUT_AGENT_COMMAND` is set; the ask guard's judge with it |
| The agent's model provider, saved from the apps (ADR 0028), and the OpenAI bridge the CLI reaches `openai` and `openai-compatible` through, on its own loopback listener (ADR 0029) | yes: the bridge listens when `EMBERHUT_AGENT_COMMAND` is set |
| Releases: CHANGELOG, `cargo xtask release`, the tag job, the desktop updater (ADR 0005) | planned |
| Connectivity (ADR 0007): `crates/emberhut-identity`, the connect, relay and directory types in `emberhut-api`, and the shared vectors | yes |
| Connectivity (ADR 0007): the account directory (`crates/emberhut-relay`, `emberhut-relay serve --role directory`, docs/directory.md) | yes: plain HTTP behind a TLS proxy until the gateway brings TLS |
| Connectivity (ADR 0007): the server's probe (`GET /api/connect/probe`) | yes: direct listeners; relayed probes come with sealed sessions |
| Connectivity (ADR 0007): the server's route list, routes and their race in `emberhut-client` and `packages/connect`, the relay's gateway (`crates/emberhut-relay`, `deploy/relay/`), the machine's uplink, account sign-in | planned |
