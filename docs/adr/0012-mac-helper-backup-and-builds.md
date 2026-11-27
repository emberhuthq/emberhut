# 0012. The Mac helper, the backup and the newest builds; no Sparkle feed

- Status: Proposed
- Date: 2026-11-26

## Context

The apps need three small features around the server's own health and the apps' updates, and
a fourth is a common way to update a Mac app:

- **The Mac row.** The app's Settings show the owner's Mac: whether the helper on it answers,
  whether the screen is locked, and how long it has been idle, with "Lock now" (`GET /api/mac`,
  `POST /api/mac/lock`). The server asks a small HTTP helper on the Mac (`POST /health`, `POST
  /lock`, a bearer token), over a private network. Nothing ever unlocks the Mac.
- **The backup.** `GET /api/backup` answers a `.tar.gz` of the server's data: a consistent
  snapshot of the database and the files it keeps. A job on another
  machine pulls it nightly.
- **The newest builds.** Once a store has processed a build, the release step sends
  `POST /api/settings` with `{ "build": { "platform": "ios" | "mac", "number": N } }`. The
  number only moves up, the state carries `builds: { ios, mac }`, and an older app shows
  "Build N is ready".
- **A Sparkle update feed.** `POST /api/mac-app/<name>` takes a notarized DMG or a Sparkle
  appcast (RSS) from the release script, with the server's key, and keeps the newest three DMGs;
  `GET /api/mac-app/<name>` serves the appcast, a DMG, or the newest DMG under a fixed name.
  A Sparkle updater reads it.

ADR 0002 keeps SQL to the migrations, and ADR 0005 specifies how emberhut's own apps are released
and updated.

## Decision

### The Mac helper

- The helper is behind a `MacHelper` trait (`services/mac.rs`) with two calls, `health` and
  `lock`. `HttpMacHelper` is the real one. `NoMacHelper` is the default, and tests use a fake.
- It is configured with `--mac-helper-url` / `EMBERHUT_MAC_HELPER_URL` (an `http://` URL, as the
  helper is reached on a private network) and `--mac-helper-token-file` /
  `EMBERHUT_MAC_HELPER_TOKEN_FILE` (a file holding the bearer token, read at start-up), always
  together. The token is a file, never a flag, so it doesn't show in the process list.
- The wire is small. Each call is `POST <url>/<call>` with `{}` and the
  bearer. The row is `{ "online": false }` alone when the helper is not set up or doesn't answer
  in 5 s, and `{ "online": true, "locked", "idle" }` when it does (`idle` is `null` when the
  helper doesn't say). "Lock now" waits up to 10 s. A refusal, a missing helper or one that
  doesn't answer is 400 with the helper's words, or with what went wrong.
- The rest of the helper (an app's window, the whole screen) arrives with the assistant's Mac
  tools, on the same trait.

### The backup

- The archive holds `emberhut.db`, a consistent snapshot, and every entry of the server's data
  folder (the database's folder) named in `services::backup::DATA_ENTRIES`. A part that keeps
  files next to the database adds its folder's name there. Nothing outside the data folder goes
  in (other databases, configs, service units are specific to one machine): an operator backs
  up the rest of their machine with their own tools.
- **The secrets directory is not in it** (`--secrets-dir`: the push services' keys and the
  secrets saved through one-time links). A backup is a download any signed-in device can make;
  with the secrets in it, a lost phone's session would carry away every key the server holds.
  The secrets are few, each can be made again where it came from (Apple's and Google's
  consoles, the service a token is for), and an operator keeps and restores them on their own.
- **The snapshot is made by SQLite with `VACUUM INTO`**, on one of the pool's own connections
  (`emberhut_db::BackupStore`). Copying the file's bytes while the server writes would be torn,
  and recent commits are only in the write-ahead log. `VACUUM INTO` reads in one transaction,
  so it sees one moment, log included, while writers go on. **This ADR allows that one
  statement outside SeaORM's builders**, in `emberhut-db/src/backup.rs` only. SeaORM has no
  backup API, and opening a second connection for SQLite's online-backup API could upset the
  pool's locks.
- **One backup at a time.** Each makes a full copy of the database, so a second request while
  one runs is 503 rather than another copy. The snapshot goes into a private folder in the data
  folder (`.emberhut-backup-*`, owner-only, deleted when the archive is sent), on the database's
  own disk rather than in a system temp folder that may be small, so a failure there is a 500
  before any byte is sent; a folder a crash left there is removed when the next backup starts. The archive is then streamed as it is written, file by file. Only regular
  files and folders go in; links, sockets, FIFOs and devices are skipped unopened and logged
  (a FIFO would block for ever). A file that changes while it is read goes in at its listed
  size, so the archive stays well formed. The answer is never gzipped again by the server's
  compression layer.
- Only a signed-in device gets it, even in report mode. Report mode lets requests without a
  session read, and the backup holds every secret's hash.

### The newest builds

- The builds live in their own table, `latest_builds` (one row per platform; migration
  `m20261126_000005_builds`), with the "only moves up" rule in `emberhut-core::builds`. A table of
  their own, rather than one settings value, keeps this part apart from the settings store, and the
  store reads and raises a number in one transaction.
- The platforms are `ios` and `mac`, the builds the release step reports. ADR 0005 section 6
  replaces this banner with the `ios` and `android` release feeds once they exist.
- `build` is one action of the settings dispatcher (ADR 0009; `settings/build.rs`), which
  calls `BuildService::apply` with the body's `build` and answers `{ ok: true, ...state }`
  like every settings change, or 400 `{ ok: false, error }` for a report that is not one.

### The update feed: ADR 0005's only

The server serves no Sparkle feed. emberhut's desktop app doesn't use Sparkle: its updater is
emberhut's own code, and its feed is a typed JSON document in the contract (`GET`/`PUT
/api/releases/v{format}/{platform}` and `/api/releases/files/{name}`, ADR 0005 sections 4 and 5). It
is published with a publish-only key, never the server's full key. A `/api/mac-app/*` feed (an RSS
appcast and DMGs, which only Sparkle reads) would add a second, untyped channel that no emberhut app
reads, and an upload route on the full key that ADR 0005 rules out. emberhut-server implements ADR
0005's routes instead (its part 7), once it has program keys.

## Consequences

- A server with no helper configured shows the Mac offline, and "Lock now" says no helper is
  set up. Nothing else changes.
- Restoring a backup is: stop the server, unpack the archive into an empty data folder, put the
  secrets directory back from where it is kept, and start the server on its `emberhut.db`.
- `emberhut-db` has one statement outside SeaORM, and this ADR names it. Any other one needs its
  own ADR.

## Left out

- **A Sparkle update feed**, as a whole: an appcast and DMGs
  (`GET /api/mac-app/{name}`) and the release script's upload with the server's full key
  (`POST /api/mac-app/{name}`). ADR 0005's update channel takes its place: its
  `GET /api/releases/v{format}/{platform}` and `GET /api/releases/files/{name}` routes, and
  `PUT` routes with a publish-only key.
