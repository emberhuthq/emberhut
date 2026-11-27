# 0017. Runners on other machines: their sessions, machines, accounts and jobs

- Status: Proposed
- Date: 2026-11-26
- Specified in: [api.md, "Runners"](../api.md#runners), ["Machines"](../api.md#machines), and
  the state's `sessions`, `mySessions`, `machines`, `accounts`, `usage`, `assistantAccount` and
  `account`.

## Context

A runner is a program on one of the person's machines that runs coding-agent sessions. The
server drives its runners over each runner's own HTTP API (`POST /sessions`, `POST
/sessions/{id}/send|stop|interrupt|settings`, `GET /sessions/{id}/transcript|live`, `GET
/folders`, `GET /branches`, `GET /accounts`, `GET /jobs`, `GET /files`, `PUT
/sessions/{id}/files`), and the runners tell it what happens (`POST /api/runner/events` for
every change of a session, `POST /api/runner/machine` every 20 s with a sample of the machine,
`GET /api/runner/mcp` for the MCP servers their sessions get too). Around that the apps show
the sessions (the list,
a transcript, a live stream, the person's replies and their own new sessions with folder and
branch pickers), each machine's capacity, the assistant's accounts and their use, the
background jobs, the MCP servers and skills switches, and the Environments page.

ADR 0007 says how anything reaches a server: by one of its routes, direct or through the relay,
proving the machine first. ADR 0002 left out a shared server key and said that
programs on the person's machines would get keys of their own when their routes came. ADR 0013
made every coding thread, the runners' sessions included, a row of `threads`.

## Decision

### Transport

- **Runners use a configured, trusted address with their own token; they don't follow ADR
  0007's route rules yet.** ADR 0007 has a client prove the machine on a route (the probe,
  answered with the machine's key) before it sends a token there. Runners don't do that: they
  neither know nor pin the machine's key. So each side reaches the other at an address it is
  configured with (the runner, the server's; the server, the runner's `--runner NAME=URL`),
  and each such address must be one the operator trusts: `https://` with a certificate the
  caller checks, or a network that already limits who can connect and listen (loopback, a VPN,
  a private network). A runner's token is a bearer secret: whoever reads it on the way can
  report as that runner and read the MCP servers it is given, and the server sends it to
  whatever answers at the runner's address.
- **A runner token never goes through the relay.** The relay ends TLS and sees every header,
  `Authorization` included (ADR 0007, "TLS, and what the relay sees today"), and a runner
  can't tell a relayed route from a direct one without the machine's key. A runner is not
  configured with the server's relay address, and `--runner` never names one; runners use
  direct routes only. When runners pin the machine's key (and the relay carries only sealed
  traffic, as ADR 0007 plans), they can pick routes as ADR 0007 says, the relay included; that
  change is the runner's and needs no new route on the server.
- **Each runner has its own token, not a shared key.** `--runner-tokens` (`EMBERHUT_RUNNER_TOKENS`)
  names a file of `NAME=TOKEN` lines, read again whenever its modification time or size
  changes (one look at its metadata per request), so a new token works without a restart.
  A presented token is compared with each runner's in constant time (`subtle`).
  `POST /api/runner/events`, `POST /api/runner/machine` and `GET /api/runner/mcp` take
  `Authorization: Bearer <token>` and nothing else (a device's session is refused with 401
  `runners use their token`); the token says which runner it is, and a runner may only report
  for itself (another runner's name is 400). The tokens are kept as text rather than as hashes
  (ADR 0002's suggestion for program keys) because the server presents the same token to the
  runner when it calls it; the file's permissions are the operator's to set.
- **The server calls a runner at its configured address.** `--runner NAME=URL`
  (`EMBERHUT_RUNNER`) for each, `--runner-name NAME=WORDS` (`EMBERHUT_RUNNER_NAMES`) for its machine in
  words, as the Mac helper (ADR 0012) and the coding-thread app (ADR 0013) are configured, not
  discovered. A runner the server can't reach still reports; what needs the server to call it
  (a transcript, the pickers, sending a turn) answers 502 with why until it can. The runners'
  HTTP API is behind a trait (`RunnerApi`, `HttpRunners` over reqwest, a fake in the tests).
- No route row needed superseding for ADR 0007: the runner routes are plain routes with a key
  per runner behind them, and the trusted path above is the condition for using them.

### Sessions

- A runner's event carries the session and its turn as they are now, so taking one twice
  changes nothing. It becomes the session's thread through `ThreadService::record` (a
  followed session's end is news for the assistant): what it was asked (without the server's
  status-line ask), what it is doing, its latest answer and status line, its question, its
  account, folder and branch, and for the person's own sessions their model and effort. A note
  or an account move goes to the activity log.
- The sessions list, the state's `sessions` (running and queued first, 30 at least) and
  `mySessions` (the person's own, 40 at most) read those threads. The transcript, the live
  stream, the reply, stop, interrupt and settings pass through to the session's runner, with
  what the server knows added (the machine in words, the account's name). The person opens
  their own sessions on the machine and in the folder they pick.
- The assistant's tools `thread_start`, `session_read`, `session_send`, `session_send_file` and
  `session_stop` do the same for the assistant. `thread_start` skips a machine that is offline
  or too full (the `machines` rules) unless forced, starts the session followed, and names the
  task in the session's meta.
- A session's id goes into a runner's path only as a `SessionId`: `r-` and lowercase hex
  digits, checked once, whether it came from a route or from a tool's arguments. A path is
  joined to the runner's address and sent with its token, so an id with `/`, `.`, `?` or `#`
  in it would otherwise reach another of the runner's endpoints; the tools' ids come from the
  assistant, which a prompt injection may steer.
- Reading (the lists, a transcript, the live stream, the pickers, the Machines view, the
  accounts, the jobs, the switches and Environments pages, and "Check now", which only asks
  for a fresh look) passes report mode's gate like any route. Everything that acts needs a signed-in device in either auth mode, since a session
  run on a runner can run code on its machine and read files there: opening a session,
  replying, stopping, interrupting and changing one, a worker's reply and stop, stopping a
  job, pinning the assistant's account, turning a server or skill off, and the `stopTask`
  settings action. Without a session they answer 401 `signIn`, as enforce mode does.

### Machines and accounts

- Each machine's latest sample (`machine_latest`) and a day of 5-minute points, each the worst
  of its samples (`machine_points`), are kept. The states, warnings, words and limits are in
  `emberhut-core::machine`. The state's `machines` is each machine's error
  at the last look: the coding-thread app's machines, then the runners'.
- Every runner keeps its own list of accounts and looks at their use itself. The server reads
  each runner's list every minute (`GET /accounts`, its logins never kept) into `usage`, one row
  per machine and account, and "Check now" asks every runner to look again (`POST
  /accounts/check`, at most once in 2 minutes). The person's pin of the assistant's account is a
  pref (`prefs`, under keys of this server's own); the state's account the assistant runs on
  (`assistantAccount` on the wire, listed in `emberhut_api::wire`)
  and `account` say which account the assistant runs on and its use. No account is named in the code: the ids,
  names and scopes are the runners'.

### Files, jobs, switches and Environments

- Files go into a session's inbox with a reply or `session_send_file` (only from the uploads,
  home and `/tmp` folders, 200 MB at most). Never sent, whatever is asked: the secrets folder,
  the database and its `-wal`, `-shm` and `-journal` files, the machine's key (`machine.key`
  beside the database, or `EMBERHUT_MACHINE_KEY`), the configured token and key files (the
  runners', the coding-thread app's, the Mac helper's and the runners' MCP servers file), the
  agent accounts' configuration directories (`ID=DIR`, where the CLI keeps its login), and
  the home's `.ssh`, `.gnupg`, `.aws` and `.config`. Each is resolved to its real path when the
  server starts, and a file is compared by its real path, so a link in `/tmp` or the home
  folder doesn't take one out. The set is one (`services::private::PrivatePaths`), and the
  chat's `send_file` refuses the same files. Files brought from a runner land in `uploads/<machine>-<8 hex>/<name>` (the same file,
  unchanged, once: `pulls`). `GET /uploads/{path}` serves such a path, one level deep and only
  in a real folder of the uploads folder.
- The background jobs are the runners' sessions with a turn on and what each runner lists of
  its machine (`GET /jobs`); stopping one asks its runner.
- The switches keep the MCP servers and skills turned off, give them to every session the
  assistant starts and push them to the runners (`POST /off`). The Environments page lists the
  runners, the coding-thread app's machines and the assistant's accounts, with "Check now".
- What belongs to the assistant's runtime is behind small traits it implements (`services::
  workers_view`): a worker watched as the session `w-<task>` (its steps, a reply, stop), the
  `stopTask` settings action, and the session's live MCP status and reconnect. The worker
  runtime (ADR 0018) and the agent runtime (ADR 0014) implement them; without an agent CLI the
  server runs with the `NoWorkers` stand-in, so every `/api/runner/sessions/w-{id}/...` route
  answers 404 `no such task` and `stopTask` stops nothing. The routes and the action are tested
  against a fake runtime too.

### Left out, on purpose

- Scanning the server's own host for jobs (its service manager's units, agent processes and the
  ports they serve) and clearing failed units from there. This server doesn't manage the service
  manager of the machine it runs on (it may run in a container, or on another system): a runner on
  that machine lists its jobs like any other.
- Account rotation details of one installation (named accounts, a provider's pay-per-token
  account, work kept to certain accounts by client) are the runners' business, not the
  server's. Its machine-trouble events, a turn's run on its task, the task links and the
  projects' runner folders wait for the assistant's events, tasks and projects.

## Consequences

- The apps' Sessions, Machines, Usage, MCP servers and skills, and Environments screens work
  for anyone who configures runners.
- A runner speaks the runner API above with its own token.
- Each new runner needs a line in the tokens file on the server and the same token on the
  runner; revoking one is deleting its line.
- A runner and the server need a trusted path between them. A runner outside the person's
  own networks reaches the server over TLS at a public address, never through the relay,
  until runners pin the machine's key.

## Left out of the contract

- `POST /api/jobs/clear`: clearing failed service-manager units on the server's own host,
  which this server doesn't scan or manage (see "Left out, on purpose").
