# 0013. The coding-thread app: followed over MCP, polled into the threads table

- Status: Proposed
- Date: 2026-11-26
- Specified in: [api.md, "Coding threads"](../api.md#coding-threads) and the state's `threads`,
  `followed` and the running threads.

## Context

Besides its own runner, the person may run coding agents in another coding tool on their
machines (here, "the coding-thread app"). Following that app's threads lets the assistant know
what every thread is about, hand one work and hear when it ends, and lets the apps show its
running threads beside the runner's sessions, read-only. The app is reached over its MCP
endpoint (JSON-RPC over HTTP, the streamable transport), one per machine, each with its own
token.

## Decision

- **Behind a trait.** `CodingThreadApp` (emberhut-server `services::coding_app`) lists a machine's
  threads, reads a thread's items and sends a thread a message. `McpCodingApp` is the real one;
  tests use a fake, and the stand-in MCP endpoint in the server tests checks the real client's
  session, calls, token and refusals. With no machine configured the server uses `NoCodingApp`
  and polls nothing.
- **Configured, not discovered.** Each machine is `--coding-app NAME=URL`
  (`EMBERHUT_CODING_APP`, comma-separated); its bearer token is a line `NAME=TOKEN` of the file
  `--coding-app-tokens` (`EMBERHUT_CODING_APP_TOKENS`), read at every call so a new token works
  without a restart. The server reads no other tool's credentials.
- **One table.** Every thread, the runner's sessions and the app's, is a row of `threads` (keyed
  by machine and id, one column per field), and follows are rows of `followed_threads`. A poll
  every 30 seconds stores each listed thread, reads up to 8 changed ones (first ask, latest ask,
  latest answer), forgets the ones a machine no longer lists, and keeps each machine's error in
  memory; an unreachable machine keeps its last list. A followed thread's end is returned to
  the caller (logged until the assistant's runtime takes such events).
- **The client is the server's one HTTP client** (`emberhut-server`'s `http` module: reqwest on
  rustls with ring, as ADR 0011 sets out), async on the runtime like every other outside call.
  The MCP client is split by concern: `coding_app/transport.rs` (the session, the protocol
  header, JSON or event-stream answers), `tokens.rs` (the tokens file, read with tokio's file
  system calls at every call), and `answers.rs`, which reads the app's answers into serde types,
  so a field the app renames fails loudly instead of reading as empty.
- **Wire names in one place.** The activity route's path, the state's running-threads key and the
  app's own tool names are kept in one place each (`emberhut_api::wire`, with the serde key on
  `State::t3_running`, and `services::coding_app::tool`).
- **Left out, on purpose.** The server starts no app threads, so it renames none and strips no
  prefix from the running list, and it rewrites no thread's answer. Until rooms carry projects and
  threads are linked to tasks, every app thread belongs to Main and the running list's `tasks` is
  empty. The unknown-machine error reads "No coding-thread app on that machine."

## Consequences

- The apps' Sessions screen and the state's thread lists show the app's threads for anyone who
  configures the app's endpoint.
- The assistant's tools `threads`, `thread_send` and `follow` (`mcp/tools/threads.rs`, in
  ADR 0009's registry) call `ThreadService::list`, `send` and `follow`. What needs tasks,
  accounts or runner machines stays out until those land: `thread_send`'s and `follow`'s
  `task_id` (its task key in front, the run it opens, the link), and `threads`' account use,
  runner machines and finished threads not reported yet.
- A token in a file is a secret on disk beside the database; the file's permissions are the
  operator's to set.
