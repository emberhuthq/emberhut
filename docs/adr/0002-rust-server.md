# 0002. The server: axum, SeaORM on SQLite, opaque device tokens

- Status: Proposed
- Date: 2026-11-25

## Context

emberhut needs a server for its apps: one HTTP/SSE contract that the desktop, mobile and web
apps all speak. The contract is `docs/api.md` and the types in `crates/emberhut-api`; this ADR is
about everything behind it. The server is in Rust, like the desktop app and the shared crates, so
the contract's types are one set of Rust types on both sides: a field the server renames or a
shape it changes breaks the client's build, not a user's session, and the domain rules are
written once and tested without I/O.

The first slice is: pairing a device (`POST /api/auth/pair/new` from a signed-in device,
`POST /api/auth/pair` from the new one), renewing a session (`POST /api/auth/refresh`), the
app's state for a room (`GET /api/state?room=`, with `ETag` and `304`), the live stream
(`GET /api/live?room=`, SSE), sending a message (`POST /api/talk`) and marking a room read
(`POST /api/seen`).

It is one person's assistant, self-hosted: one process, a handful of devices, a few requests
a second at most. What matters is that it is simple to run (one binary and one file), hard to
get wrong (typed all the way down), and easy to test.

## Decision

### Crates, and which way they depend

```
emberhut-server ──► emberhut-db ──► emberhut-core
     │  └──────────────────────► │
     └──► emberhut-api
```

- **`emberhut-core`**: the domain rules with no I/O and no workspace dependencies: tokens and their
  hashes, session lifetimes and rotation rules, pairing codes (making, showing, normalising,
  checking) and the wrong-code brake, rooms and messages, `ETag`s. Time and randomness come in
  through two traits, `Clock` and `RandomSource`, so every rule is unit-tested at any instant
  with any bytes.
- **`emberhut-db`**: the SeaORM entities, the migrations, and stores (`SessionStore`,
  `PairingStore`, `RoomStore`, `MessageStore`) that take and return `emberhut-core` types. It is the
  only crate that knows the schema, and its entities are private to it. Anything that changes
  more than one row is one store method and one transaction.
- **`emberhut-server`**: a library with a thin `main.rs`. Services (`AuthService`,
  `PairingService`, later the state, live and talk services) take their stores, `Clock` and
  `RandomSource` in `new()` and apply `emberhut-core`'s rules. They are wired once, in
  `AppState::new`, which `main` (and every test) calls. No DI container: constructor injection,
  with a trait only where a test swaps the dependency (the clock, randomness, the assistant).
  Modules: `config`, `error`, `app` (state, middleware), `routes`, `services`, `telemetry`,
  `shutdown`; the slice adds `auth`, `pairing`, `state`, `live` and `talk` handlers.
- The server uses `emberhut-api`'s request and response types and defines no copies of them.
  Until the contract lands on `main`, `ApiError`'s private body types and `config::AuthMode`
  stand in for emberhut-api's `ErrorBody`, `AuthReply` and `AuthMode`; the first slice renders
  through emberhut-api's types and maps the mode to its type. Core's ids convert to and from `i64`
  (`From` both ways, as emberhut-api's do), so the boundary is `api::RoomId::from(i64::from(id))`
  (the orphan rule rules out a direct `From` between two foreign types), with emberhut-api imported
  as `use emberhut_api as api;` where both crates have a type of the same name.

### HTTP: axum on tokio, middleware from tower-http

axum 0.8 on a multi-threaded tokio runtime. Every request goes through one `ServiceBuilder`
stack, outermost first:

1. `SetSensitiveRequestHeaders` (`Authorization`, `Cookie`): never in a log line.
2. `SetRequestId` / `PropagateRequestId`: the client's `x-request-id`, or a new UUID, carried
   by the request's span and echoed in the response.
3. `TraceLayer`: one span per request (method, path, id) and a line per response.
4. `CatchPanic`: a panicking handler answers 500 with the usual error body.
5. A request timeout (`tower::timeout` under `HandleErrorLayer`): 503 with the error body.
   It bounds the time to the response head, so a live stream is not cut off.
6. In the slice, gzip `Compression` for bodies over 16 KiB: the state is hundreds of KiB of
   JSON polled every few seconds, and compresses to about a quarter. tower-http's default
   predicate never compresses `text/event-stream`, so the live stream is not buffered.

Request bodies are capped by axum's default limit (2 MB); none of the slice's bodies is close.

**Errors.** Each layer has its own `thiserror` enum (`PairingRefusal` in core, `DbError` in db,
`AuthError`/`PairingError` in the services), and `ApiError` maps them to a status and a JSON body
whose `error` is a sentence the app shows as it is: `{"error": "not found"}`, and `{"error": "Sign
in first.", "signIn": true}` for a missing session. A sign-in route's refusal (a pairing code or a
refresh token not taken) is answered in the contract's `{ok: false, status, error}` shape instead,
because clients read those routes' answers as one type whether they succeed or not. A 5xx is logged
with its whole cause chain and answered with a fixed sentence: nothing about the server's insides
reaches a client. `429` carries `Retry-After`.

### OpenAPI: the server describes itself

The router is built with utoipa-axum (`OpenApiRouter` and `routes!`), every handler carries
`#[utoipa::path]`, and `emberhut-api` derives `ToSchema`/`IntoParams` behind its `openapi`
feature, which the server turns on. The document is therefore assembled from the routes the
server really serves, and cannot list one it doesn't. The server:

- exposes a function returning the document, and serves it at `GET /api/openapi.json`;
- commits it as `crates/emberhut-server/openapi.json`, checked by a snapshot test
  (`openapi_document_is_up_to_date`, rewritten when `EMBERHUT_BLESS=1`), with utoipa's
  `preserve_order` and a fixed pretty-print so the file is deterministic;
- bans `axum::Router::route` and `axum::Router::route_service`, and utoipa-axum's own
  `OpenApiRouter::route` and `route_service` (which take a route without its document), with
  clippy's `disallowed-methods`, so a route cannot bypass the document. The entries are in the root
  `clippy.toml` (a crate-local one would replace it, not add to it), without `allow-invalid`:
  crates that don't depend on axum don't warn about the path anyway, and without the flag clippy
  says so when a rename in axum makes the ban match nothing. `merge` and `nest` of a plain
  `Router` could still bring routes in, so the ban is a guard and the next test is the proof;
- tests that the router's set of paths equals the document's, and calls every documented route
  (success and error cases) checking the real status and body against the document's schema for
  that response (the `jsonschema` crate).

### Validation

`emberhut-api` derives `garde::Validate` on request types behind its `validate` feature. The server
reads JSON bodies with a `Valid<T>` extractor that runs the validation before the handler and
answers a failure with the error body (400) and its reasons. As the contract has it, an empty body
reads as `{}` and the content type is not checked. Otherwise the server is on the side of saying no
clearly: a body that isn't JSON is a 400, not a 500 with the parser's message; a room in a body that
isn't positive is a 400 (emberhut-api's rule); a field over emberhut-api's length limits is refused
rather than cut. The sign-in routes answer a failed validation in their own `AuthReply` shape (a
missing or blank code: 400 "Type the pairing code."; a refresh body that names no token: 401 "Sign
in again.", as for any unknown token). Pairing reads its body as `docs/api.md` does: the code first,
by its type (missing, not text or blank is "Type the pairing code.", whatever else the body holds);
`install`, `name` and `model` may be left out or null (emberhut-api defaults them), and an empty
`install` means none, so the device gets a new row (on a refresh too). While the brake is on, a
request without a usable code gets the brake's 429; a typed code longer than any code is a wrong
code (401, counted on the brake). A body over axum's limit (2 MB) is 413 with the error body (in the
`AuthReply` shape on the sign-in routes). The extractor's errors stay typed (`Invalid`, with the
parser's or the rules' error as their source) until the answer is written. A room in a query
(`?room=`) is read leniently, as the contract says: missing, not a positive whole number, or not an
existing room means Main. Lenient normalising of what the server takes (a blank device name replaced
by "A device", a typed code's dashes and case ignored) is a domain rule and lives in
`emberhut-core`.

### Data: SeaORM 2 on SQLite, migrations at start-up

**No SQL in application code.** The SQL lives in migrations, which make the schema; everything
else uses SeaORM's typed query builder through `emberhut-db`'s stores.

| | SeaORM 2 (chosen) | Diesel 2 |
| --- | --- | --- |
| Async fit with axum/tokio | Async throughout, on sqlx's pool | Synchronous; `diesel-async` has no native SQLite connection, so it would need `spawn_blocking` or a sync wrapper around every query |
| SQLite | First-class (sqlx's driver): WAL, foreign keys, busy timeout, `BEGIN IMMEDIATE` | First-class |
| Migrations | `sea-orm-migration`: Rust migrations built with SeaQuery's schema builder, embedded in the binary, applied by `Migrator::up` | `diesel_migrations`: SQL files embedded in the binary |
| Type safety | Typed entities, columns and queries; checked against the schema by tests, not the compiler | The query DSL is checked against `schema.rs` at compile time |
| Testability | Any connection: a real SQLite file per test | The same |
| Maturity, community | Widely used, active; 2.0 is recent | The oldest and most proven Rust ORM |
| Compile times | Heavier: proc macros, and `sea-orm-migration` turns on SeaORM's default features | Heavier still on large schemas (the DSL's generics) |

SeaORM wins on the axis that matters most here, async on the same runtime as axum, and it is
the closest to EF Core (entities, migrations, a typed builder). Diesel's compile-time checking
is real, but every table and store method here is covered by integration tests against a real
database, which catches the same mistakes and more (constraints, transactions). Raw sqlx with
`query!` was ruled out by the rule above. **SQLite rather than Postgres**: a self-hosted,
single-user server is one binary and one file to back up, with no second service to run;
SeaORM keeps a later move to Postgres open.

How the database is used:

- `emberhut_db::Database::connect` opens the file (making it if needed) with foreign keys on, the
  write-ahead log (readers don't wait for the writer), `synchronous = NORMAL` and a 5 s busy
  timeout, then applies the migrations the file lacks. A database URL is checked first: only a
  writable file is taken (an in-memory database would lose every session once its connections
  idle out, a read-only one could sign nobody in). The driver's own parser reads the URL, and
  of its query options only `mode=rw` or `mode=rwc` is taken: every other one (`immutable`,
  `vfs`, `cache`, and any added later) changes how the file is opened, in any spelling the
  driver decodes. The file's name is checked as the driver decodes it too: `:memory:` and any
  `file:` URI are refused.
- **Migrations run in one write transaction** (`BEGIN IMMEDIATE`, then `Migrator::up`, then
  commit). `sea-orm-migration` wraps a migration in a transaction only on Postgres; on SQLite
  every statement would commit on its own, and a migration that failed or was killed half-way
  would leave tables behind that the next start trips over. In one transaction, a failed
  migration leaves the file as it was, and of two processes starting on the same file the second
  waits for the first and finds nothing to do. (Opening a brand-new file can meet a lock SQLite
  does not wait on: one connection switching the file to the write-ahead log while another sets
  it up. A locked open is retried for up to the busy timeout.) Both are tested.
- Migrations are forward-only once on `main`: a schema change is a new migration, and each one
  names its tables and columns with its own identifiers rather than the entities', so a later
  entity change cannot rewrite history. Every migration also has a `down`, tested going down and
  up again.
- Transactions that write start with `BEGIN IMMEDIATE`, so two writers queue on the busy
  timeout instead of one failing when its read turns into a write.
- Instants are stored as integer milliseconds since the epoch: they compare and sort exactly in
  SQL. Ids are newtypes in `emberhut-core` (`DeviceId`, `RoomId`, `MessageId`, `PairingCodeId`)
  and plain integers only inside `emberhut-db`.
- The first schema: `devices` (one row per app install), `sessions` (at most one row per
  device: the hashes of its current tokens and of the pair they replaced), `pairing_codes`,
  `rooms` (Main, id 1, is made by the migration) and `messages`.

### Configuration

clap with derive and `env`: every setting is a flag with an `EMBERHUT_*` variable
(`EMBERHUT_DATABASE_URL`, `EMBERHUT_LISTEN`, `EMBERHUT_AUTH_MODE`, `EMBERHUT_REQUEST_TIMEOUT`,
`EMBERHUT_SHUTDOWN_GRACE`, `EMBERHUT_LOG`), parsed into typed values (`SocketAddr`, a checked
`DatabaseUrl`, `Duration`s within bounds, a checked log filter), so a bad value stops the
server before it opens anything. clap rather than figment or config: one source of truth for
flags, variables, defaults and `--help`, and the command line is needed anyway.
`emberhut-server` serves; `emberhut-server pair` prints a pairing code. There are no secrets to
configure: the server makes its own and keeps only their hashes. Settings are read at start-up;
changing one (the auth mode included) takes a restart, so the mode a running server enforces is
the one its start-up log shows.

### Logs

`tracing`, written by `tracing-subscriber` as one line per event on standard error (coloured
only on a terminal), filtered by `EMBERHUT_LOG` (`EnvFilter` syntax, default `info`). Each request's
lines carry its span (method, path and request id) and end with a line giving its status and
latency. A client's own `x-request-id` is used only when it is at most 64 visible ASCII
characters; otherwise the server makes one, since the id goes into the logs. sqlx logs every
statement through `log` at `debug` and any statement slower than a second at `warn`; the `log`
records are bridged into `tracing` (SeaORM's own spans are at `trace` and carry no statement
text).

### Graceful shutdown

Ctrl-C or `SIGTERM` (what service managers and `docker stop` send) cancels a
`CancellationToken` held by the app state. axum stops accepting connections and waits for open
ones; live streams watch the same token and end, which otherwise would hold the shutdown up
forever. The talk service watches it too: it takes no new message (503 "The server is
restarting."), drops the turns still queued (each with a log line; their messages stay in the
chat, unanswered), and keeps its running turns (and the assistant's own task) in a
`TaskTracker`, which the shutdown waits on. Each `Turn` carries a child of the shutdown
token, so a long turn can save what it has and end within the grace.
Connections and running turns together get the grace period (`EMBERHUT_SHUTDOWN_GRACE`, 10 s);
then the server stops regardless, and the database pool is closed. The `OpenAPI` document is
made before the server listens: one that cannot be written stops the start.

### Auth: devices with opaque tokens

- A device that has proven itself gets an **access token** (works for an hour) and a **refresh
  token**. Both are 256 random bits from the OS, in URL-safe base64, and opaque to clients. The
  database keeps only their SHA-256: no slow hash is needed for 256-bit secrets, and since a
  token is looked up by its hash, no secret is compared in memory.
- **Opaque, not signed** (rather than `{device, expiry}` signed with an HMAC key, or JWTs, the
  same idea): there is no signing key to make, keep and rotate, signing a device out stops its
  tokens at once, and checking a token is one indexed lookup, which a signed token would need
  anyway to see whether the device was signed out.
- **Refresh rotation:** using a refresh token replaces both tokens, in an update conditional on
  the old hash, so of two concurrent refreshes with one token exactly one wins and an old token
  never works twice (401 `Sign in again.`).
- **The previous access token works until its own expiry.** The contract hands out
  `{access, expires}`, and a client renews shortly before `expires` while other requests are
  still on their way with the token it had. So a refresh moves the current pair's hashes to a
  "previous" pair instead of dropping them: the old access token passes until its own expiry,
  and signing out deletes the row, which stops both pairs at once.
- **Reuse of a refresh token is detected** (RFC 9700, section 4.14), with a grace for retries.
  The replaced refresh token's hash is kept, with the time it was replaced. Coming back within
  two minutes of that turnover, it is most likely a client that never got the answer to its
  refresh (a dropped connection) and holds only the token it sent: it gets a fresh pair, and the
  pair issued moments before, which nobody received, stops working. The grace runs from the
  turnover, so re-issuing does not extend it. Coming back later, it is a reuse: the device is
  signed out (every token of it stops) and a warning names the device. The trade-off, accepted:
  someone holding a copy of the replaced token who uses it within those two minutes gets the
  session, and the device's own newer tokens stop (it has to pair again). Without the grace, an
  ordinary lost answer would end the same way for the device, and much more often.
- **One row per install:** the app's install id finds the device's row when it signs in again,
  which replaces its tokens and undoes a sign-out. A device whose app sent no install id takes
  one on its next refresh, if no other device has it.
- A device that has not used its session for **14 days** is signed out at its next refresh,
  and is left out of the device list meanwhile. Its last-seen time is written at most once a
  minute.
- **Pairing codes:** 8 characters from 31 capitals and digits without look-alikes (no 0/O, 1/I/L),
  drawn without bias and shown as `ABCD-EFGH`; one of about 850 billion. A code works once, for 10
  minutes; only its hash is kept; the typed code's case, spaces and dash don't matter. Using it up
  and opening the device's session are one transaction. Wrong codes are braked for every caller at
  once: after 5 within 10 minutes, no code is taken (429 with `Retry-After`) until the oldest is 10
  minutes old, at most 720 guesses a day. A try counts against the brake from the moment it starts,
  before its code is looked up, and is forgiven if the code exists; so guesses sent all at once are
  braked like guesses sent one by one (tested with twenty at once). The brake is in memory; a
  restart clears it, which a guesser cannot cause, and which is also how the owner lifts it while
  pairing a device during an attack (the brake's first refusal is logged as a warning that says so).
  The cost of a shared brake: anyone who can reach `/api/auth/pair` can keep pairing stopped with
  five requests every ten minutes. Signed-in devices are unaffected, and pairing can wait.
- **Who may make a code:** a signed-in device (`POST /api/auth/pair/new`), or
  `emberhut-server pair` on the server's own machine, which is how the first device gets in (with
  Docker, `docker exec <container> emberhut-server pair`): shell access to the server's machine is the
  root of trust. `pair` opens the server's database only if it is there, is emberhut's, and its
  schema is up to date, so it can neither make a code in a new, empty database the server does
  not know nor migrate the schema under a running server; it prints which database it used. It
  decides before it writes anything: the file is first opened read-only, with no setting that
  persists in it (the journal mode), and its migrations are read without creating their table.
  A SQLite file that is not emberhut's is refused and left byte for byte as it was. A request without
  a session never makes a code, in either mode: a session made from it would outlive a later
  switch to enforcing.
- **No shared server key in this slice.** `emberhut-server pair` is how a code is made without
  a device, and the programs' routes are not in emberhut yet. When they come (a later ADR), each
  program gets its own key, made by the server's command line, stored as its SHA-256 and looked
  up by it like a token, so it can be revoked on its own.
- **Modes:** `enforce` (the default) answers a request that needs a session and has none with 401
  `{error, signIn: true}`. `report` lets it through and logs it, once an hour per address, to see
  what enforcing would refuse; it is an aid for moving existing clients onto sessions and belongs
  only behind a network that already limits who can connect. The server logs a warning at start-up
  whenever `report` is on.

### `GET /api/state`: ETag and 304

The state's `ETag` is a digest of the exact body the server would send: the first 128 bits of
its SHA-256, base64url, as a **weak** tag (`W/"…"`, because the body may be compressed on the
way out). The same bytes give the same tag on any server and after any restart, and there is
nothing to invalidate when something changes. It stays stable because serialisation is
deterministic: struct fields in declaration order, ordered collections only (`Vec`,
`BTreeMap`, never a `HashMap`). It is cheap: SHA-256 runs at hundreds of MB/s, so a few hundred KiB
cost about a millisecond. `If-None-Match` is compared weakly (RFC 9110), lists and `*`
included; a match answers `304` with the tag and no body. Building the state still costs its
queries on every poll; a per-room revision counter that skips the build is the next step if
that ever shows up.

### `GET /api/live`: one watch channel per room

Each room has a `tokio::sync::watch` channel holding its current live value, made on first use.
A watch rather than a broadcast: a subscriber always reads the latest value, so one that falls
behind skips to it instead of replaying stale steps (or losing them to a lagged buffer). The
end of the last turn, the value going back to none, is never lost; two turns back to back may
read as one to a subscriber that was behind, which a client cannot tell apart anyway. A
`LiveSink` sets what the assistant is doing (`doing`) and, when a runtime streams one, the
answer written so far (`writing`). A stream
subscribes to its room, sends
the room's current value first, then every change, each as one SSE event `data: <JSON>` of
`emberhut-api`'s `LiveEvent`, and `data: null` when a turn ends (nothing is being written). A
comment line `: ping` every 15 s (axum's `KeepAlive`) keeps proxies from closing a quiet
stream. The stream ends when the client goes or the server starts shutting down. It is sent
with `Cache-Control: no-cache` and `X-Accel-Buffering: no`, and never compressed. A test checks
that every event the server emits deserialises as `LiveEvent`, and the endpoint's OpenAPI
description names it.

### `POST /api/talk`: sending a message, in the first slice

The message is stored at once in its room (`MessageStore::append`) and the request answers
straight away; the message shows in the state and history from then on. The turn is handed to
an `Assistant` trait (the responder seam) on a task of its own, one turn at a time per room.
A room holds at most 8 turns at once, the running one included: a ninth message is refused
with 429 and the error body ("Too many messages are waiting...") rather than queued without
end. The
assistant gets the room and the conversation so far and a sink for live updates; what it
says goes to the room's live value, its reply is stored as the assistant's message, and the
stream gets `data: null` at the end. A failing assistant leaves an event line in the chat
("Something went wrong while answering"), its cause in the log only. A panicking one does the
same: `respond` runs on a task of its own, so its panic comes back as that task's `JoinError`,
which is logged; and the turn's end (the live value back to none) is a drop guard, so no
failure leaves a room shown busy. The real agent runtime
is a later ADR. Until then the server ships with `Silent`, which answers nothing, and tests use
a scripted double that says its steps, can be held mid-turn, fails or panics on demand, and
replies. The clock, the randomness and
the assistant are the three `Dependencies` a test swaps. A reply to a message that doesn't
exist is stored as a plain message. Attachments are not taken yet:
the contract has `POST /api/uploads` and `GET /uploads/…` (docs/api.md), but this server
doesn't serve them yet, so nothing it holds can be attached, and a message that comes with files
is refused with 400
("Attachments aren't supported yet."), whatever its text, rather than stored without them; a
blank message is refused too, and no empty message is ever stored. `POST /api/seen` goes
through the talk service, which notes the newest message the person has seen; the state's
unread counts the assistant's messages after it, by id (exact, where comparing times is not).
The migration that brought this in sets it, for a room already seen, from the newest message
at or before its `seen_at`.

### What the state holds, in the first slice

The state is built per request from one snapshot (`SnapshotStore`), read in a single read
transaction so its parts agree: every room's line (unread replies since it was
seen, the last thing said, busy and doing from the live values), the room's newest 60
messages with their quotes, `earlier`, `unread`, and `auth` (the mode and the signed-in
devices; `bound` means a device has signed in, since emberhut binds no outside identity). What
the server has nothing for yet is empty or null in the contract's shape: tasks, work,
sessions, projects and goals are empty lists, `ui` hides goals and projects (the default),
`account` has every field null, `theme` is null. `x-emberhut-looking` marks
the room on screen for 10 s (`Presence`); there are no notifications yet to hold back, so it
changes nothing else. Each room's line still costs two queries inside that transaction; at a
handful of rooms that is nothing, and a summary table (with the revision counter above) is the
next step if it ever shows.

### Tests

- `emberhut-core`: unit tests of every rule, with `ManualClock` and chosen instants, and a scripted
  `RandomSource` where the exact bytes matter (the unbiased mapping onto the code alphabet).
- `emberhut-db`: integration tests (one test binary, a module per store) against a real SQLite file
  per test, migrations applied; unit tests that take every migration down and up again, that a
  migration failing half-way leaves nothing behind, that two processes can start on a new file
  at once, and that every connection has the settings it was given. Concurrency is tested on a
  multi-threaded runtime: two pools writing one file, one code redeemed by many at once.
- `emberhut-server`: unit tests of the error mapping and the middleware (timeout, panic), and an
  integration test binary that builds the whole app in-process over a fresh database with a
  `ManualClock`, and drives it through `tower::ServiceExt::oneshot` or a real socket on
  `127.0.0.1:0`. In the slice: an integration test for every route; contract tests that the JSON
  the server sends deserialises into `emberhut-api`'s types and matches the contract's fixtures; and
  the OpenAPI tests above. A manual run listens on 18990, inside the development container only.

### Alternatives

- **actix-web**: fast and mature, but its own runtime and service model sit outside the tower
  ecosystem the middleware above comes from, and utoipa-axum has no equal there.
- **poem** (with poem-openapi): tidy, but a much smaller community, and its OpenAPI story would
  replace utoipa, which `emberhut-api`'s optional derives are built on.
- **Diesel, sqlx, Postgres**: see the data section.

## Consequences

- One self-contained binary over one SQLite file: backups are a copy of the file (or
  `VACUUM INTO`), and there is no second service to run.
- Every rule is in `emberhut-core` and tested without I/O; every query is in `emberhut-db` and tested
  against a real database; the server is wiring and HTTP.
- SeaORM and `sea-orm-migration` are the heaviest dependencies. `sea-orm-migration` 2.0.4 takes
  SeaORM with its default features (chrono, time, uuid, rust_decimal, JSON, streams and
  sea-schema), which emberhut-db does not ask for; features unify across the build, so a separate
  migration crate would not help. Accepted for now: builds are slower for it, and the gain is no
  SQL in application code. The fix belongs upstream (`sea-orm-migration` depending on SeaORM with
  `default-features = false`), and the extras can go when it lands.
- Tokens are opaque and stored hashed, so a stolen copy of the database cannot be used to sign
  in with a token, and signing a device out is immediate. Each authenticated request costs one
  indexed lookup. Pairing codes are only about 40 bits, so a copy of the database lets anyone
  with a GPU recover an unused code within its ten minutes; whoever can read the live database
  can usually write it too, so this is documented rather than defended with a pepper (which
  would be the one secret the server keeps outside its database).
- The wrong-code brake and the live channels live in one process's memory. That fits a
  single-user server; running several instances would need both moved into the database.
- The OpenAPI document is generated from the routes and committed; a change to the API shows
  up as a diff to `openapi.json` in review.
