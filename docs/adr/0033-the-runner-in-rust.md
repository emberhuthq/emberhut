# 0033. The runner in Rust, driving Claude Code through its command line

- Status: Proposed
- Date: 2026-11-28
- Builds on: [ADR 0001](0001-workspace-and-quality-bar.md) (the workspace and its quality bar) and
  [ADR 0017](0017-runners-machines-and-accounts.md) (runners, machines and accounts: the protocol)

## Context

ADR 0017 describes a runner, the program on each machine that runs coding-agent sessions for the
server, but the repository held only the server's side of that protocol. Without the runner nobody
can run Emberhut end to end from these sources.

A runner hosts Claude Code sessions: it starts one in a folder, sends it each turn, streams what it
does, interrupts it, resumes it after a restart, reads its account's use and moves it to another
account at a usage limit. Two ways exist to do that. One is Anthropic's Agent SDK, a TypeScript
library that starts Claude Code as a child process and talks to it. The other is Claude Code's own
command line, which the SDK itself drives: `claude -p` with `--input-format stream-json` and
`--output-format stream-json` keeps one process per session, reads user messages and control
requests (`interrupt`, `initialize`, `get_usage`) on standard input, and writes every message,
control answer and permission question on standard output.

## Decision

- **The runner is a Rust crate in the workspace**, `crates/emberhut-runner`, with one binary,
  `emberhut-runner`, on the workspace's stack and bar: tokio, axum for its API, reqwest with rustls
  to reach the server, SeaORM over SQLite for its own data, constructor injection with no
  container, clippy pedantic with `-D warnings`, no `unsafe`, cargo-deny. A thin `main.rs` over a
  library lets the tests run a whole runner in-process.
- **It drives Claude Code through its command line**, not the SDK: one `claude -p` process per open
  session, stream-json both ways, the account chosen by its `CLAUDE_CONFIG_DIR`, `--session-id` or
  `--resume` for the conversation, `--permission-mode bypassPermissions` (a session has full
  access, as ADR 0017 says), and `--permission-prompt-tool=stdio` for the owner's own sessions so
  that a question (`AskUserQuestion`) comes to the runner and goes to the app. Use is read with the
  `get_usage` control request, the login with `initialize`, both without a turn.
- **The agent is behind a trait**, `Engine`: Claude Code is the one engine today; another agent's
  command line (Codex, Gemini) can be added behind it, translating its messages into the same
  shapes.
- **The protocol stays ADR 0017's**, unchanged: the runner's routes, its events, samples and their
  shapes. Its tests read what it answers and reports with `emberhut-api`'s types.
- **One static binary per platform**, built by `runner-release.yml` for each release (Linux musl
  x64 and arm64, macOS arm64 and x64, Windows x64), with one-line installers. `emberhut-runner
  install --server URL --pair CODE` pairs it and installs it as a service of the platform: a
  systemd user unit, a launchd agent, a Windows service.

## Consequences

- No Node, npm or SDK on a runner's machine, and nothing of Anthropic's is redistributed: the
  person installs Claude Code; the runner finds `claude` on the `PATH` (or `agentCommand`).
- The runner follows Claude Code's command line and its stream-json, which change with Claude
  Code's releases. The parts it reads are few (the message types, `result`, the rate-limit event
  and three control requests), its tests pin them with a stand-in agent, and a live test runs the
  driver against the real command line.
- The SDK's in-process hooks are not available. The owner's questions use the permission channel
  (`can_use_tool` over standard input) instead of a hook; everything else the runner needs is in
  the stream.
- A Windows service runs as `LocalSystem`, so its Claude Code logins are that account's; an `api`
  or `bedrock` account avoids the question.
