# 0033. The runner: in the repository, in TypeScript on the Agent SDK, paired and installed per user

- Status: Proposed
- Date: 2026-11-28
- Amends: [ADR 0017](0017-runners-machines-and-accounts.md), which specified the server's side of
  the runner protocol and left the runner itself outside the repository.
- Specified in: `apps/runner` (its [README](../../apps/runner/README.md): install, accounts,
  settings, security), `crates/emberhut-api/tests/fixtures/runner/` (what it sends, recorded) and
  `.github/workflows/runner-build.yml`.

## Context

ADR 0017 built everything the server needs from a runner: the routes a runner reports to, the
calls the server makes to it, the contract types and their fixtures. The runner itself, the process
on each machine that actually runs the coding-agent sessions, wasn't in the repository. Without it
nobody can run Emberhut end to end from these sources: a server with no runner can't open a single
session, and the protocol had only one side under test.

A runner hosts Claude Code sessions. Anthropic ships the way to run and drive them, the Claude
Agent SDK (streaming messages, partial text and thinking, hooks, interrupts, resuming by session
id, a separate configuration folder per account), as a TypeScript library with the Claude Code
binary per platform as optional dependencies. Driving the CLI's stream-json protocol from Rust
instead would mean reimplementing and tracking a moving, partly undocumented interface.

## Decision

### TypeScript on Node, in `apps/runner`

- The runner is `@emberhut/runner`, a pnpm workspace package in `apps/` (it is something a user
  installs). It is TypeScript because the Agent SDK is; this is the same kind of exception as the
  browser extension's (ADR 0023): the platform's library, not a preference.
- It runs on Node's own type stripping, with no build step: `erasableSyntaxOnly` in its
  `tsconfig` keeps every file runnable as written. Strict types, the extension's eslint and
  prettier rules, vitest; CI's js job runs its check like the other packages'.
- It keeps the protocol's words in one module (`src/wire.ts`), as the Rust crates keep theirs.

### Its side of the contract is tested against the server's

- Its tests drive a real runner over HTTP with a stand-in agent and a stand-in server, and check
  what it sends against the server's fixtures (`runner_*.json`, `machines.json`, `jobs.json`): no
  key the contract doesn't know, no value of another type.
- They also keep a record of what it sends (a transcript with every kind of step, the live stream,
  its answers, the pickers, the events it reports), with times, ids and folders made stable, in
  `crates/emberhut-api/tests/fixtures/runner/`. The contract tests read that record with the
  server's own types, and a server test reports the recorded events again and checks the session
  they make. A change on either side that breaks the other fails CI; a deliberate change to what
  the runner sends is written again with `EMBERHUT_BLESS=1`.

### Configuration, not installation details, in the code

- Everything about one installation is in the runner's settings file and accounts file: its name,
  the server, the token file, the port and addresses, the accounts and their providers
  (a subscription in its own Claude Code folder, an API key, AWS Bedrock), work groups that keep
  some work to some accounts, the instructions added to sessions' prompts, the worktrees folder.
  The code names no account, machine, person or path of anyone's.
- Account picking (most room first, a session stays while its account has room, moves and carries
  on at a usage limit, opt-in accounts last) is the runner's, as ADR 0017 left it.
- Its database keeps the shape it had outside the repository, so an existing runner's sessions
  carry over when it is replaced by this one.

### Pairing and installing

- A runner pairs with a one-time code from the server (the app's Machines page, or the server's
  command line) and gets its name and token in exchange; the server keeps the address it calls the
  runner at. The flag-configured runners of ADR 0017 keep working beside paired ones.
- Each release attaches a download per platform (linux x64 and arm64, macOS arm64 and x64,
  Windows x64): a pinned Node with its checksum checked, the runner, and its production
  dependencies from the lock file, built natively on that platform so the Agent SDK's binary
  matches. `install-runner.sh` and `install-runner.ps1` download one, check it against the
  release's `SHA256SUMS`, pair, and start it as a service of the user (a launchd agent, a systemd
  user unit, a scheduled task at logon): the agent's logins, the macOS Keychain and the user's
  folders are that user's, so the runner never runs as root or as a system service.

## Consequences

- One product is now in two languages on the server side of the wire. The record and the replay
  test are what keep the two in step; without them a field renamed on one side would only show up
  on a real machine.
- The download is large (Node and the Claude Code binary are most of it), and Node is a second
  runtime to keep current: the Node version is `.nvmrc`'s, as for the rest of the TypeScript.
- Sessions run with the agent's permissions bypassed, as the user: a runner's token can run code
  on its machine. The README's "Security" section says what the runner can touch, and its
  defaults (loopback only, home-folder pickers, file rules) keep the reachable surface small.
