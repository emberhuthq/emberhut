# emberhut-runner

The runner is the program on each of your machines (each hut) that runs coding-agent sessions for your Emberhut
server. The server and the apps decide what to do; the runner does it on the machine where the code, the tools and the
logins are: it starts Claude Code sessions in a folder (through Anthropic's
[Claude Agent SDK](https://docs.claude.com/en/docs/agent-sdk/overview)), sends them each turn, picks an account
with room and moves a session to another one when an account runs out, and reports every change to the server as it
happens. The server's side of this protocol is [ADR 0017](../../docs/adr/0017-runners-machines-and-accounts.md) and
[api.md, "Runners"](../../docs/api.md#runners); this package is the other side.

**Why TypeScript.** The repository is Rust ([ADR 0001](../../docs/adr/0001-workspace-and-quality-bar.md)), with
TypeScript where the platform requires it (the mobile and web apps, the browser extension,
[ADR 0023](../../docs/adr/0023-browser-extension.md)). The runner is one of those exceptions: the Claude Agent SDK, which
runs and drives Claude Code sessions (their streaming, hooks, interrupts, resume and account folders), is a TypeScript
library, and the runner is a thin, long-lived host around it. It runs on Node's own type stripping, with no build step
([ADR 0033](../../docs/adr/0033-the-runner-in-typescript.md)).

## Install

One command per machine. Get a pairing code first: in the app, **Machines → Add a hut** (it fills in the command for
you), or on the server's machine `emberhut-server pair-runner`.

**macOS and Linux**

```sh
curl -fsSL https://github.com/emberhuthq/emberhut/releases/latest/download/install-runner.sh | sh -s -- --server https://your-server.example.com --code ABCD-1234
```

**Windows** (PowerShell)

```powershell
$env:EMBERHUT_SERVER="https://your-server.example.com"; $env:EMBERHUT_PAIR_CODE="ABCD-1234"
irm https://github.com/emberhuthq/emberhut/releases/latest/download/install-runner.ps1 | iex
```

The installer downloads the release's build for your platform (its own Node, the runner and its dependencies, about
the size of Claude Code itself), checks its SHA-256 against the release's `SHA256SUMS`, pairs the runner with your
server and starts it as a service of your user: a launchd agent on macOS, a systemd user unit on Linux, a scheduled
task at logon on Windows. Nothing needs administrator rights. Each release is versioned with the apps; running the
installer again upgrades in place. On a Linux machine you aren't logged in to, keep the user unit running with
`loginctl enable-linger`.

Options: `--name` (the runner's name on the server, default the machine's), `--advertise URL` (the address the
server calls the runner at, see [Network](#network)), `--version` (a given release).

After installing:

```sh
emberhut-runner status             # does it answer, and is it paired
emberhut-runner accounts           # its accounts, their use and logins
emberhut-runner accounts login default
emberhut-runner service status     # what the service manager says (its log is runner.log in the runner's home)
```

## Accounts and limits

A runner runs sessions on **accounts**, listed in `accounts.json` in its home. A new runner lists one: `default`,
whatever login the machine's Claude Code already has. Each account is one of:

| `provider`         | What it is                                                                            | Its login                                                                                                                     |
| ------------------ | ------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| `claude` (default) | A Claude subscription, in its own Claude Code folder (`configDir`; none: `~/.claude`) | `emberhut-runner accounts login <id>`: Claude Code's own sign-in, in that folder. The runner never reads or copies the login. |
| `api`              | An Anthropic API key, or any Anthropic-compatible endpoint (`baseUrl`)                | The key in a file in the runner's `secrets/` folder (`keyFile`, default `anthropic-api-key`), readable by you only.           |
| `bedrock`          | Claude on AWS Bedrock (`region`; `models` maps model ids to your inference profiles)  | A Bedrock API key in `secrets/` (`keyFile`, default `aws-bedrock-api-key`).                                                   |

```json
[
  { "id": "work", "name": "Work subscription", "configDir": "~/.claude-work", "order": 1 },
  { "id": "me", "name": "My subscription", "order": 2 },
  { "id": "api", "name": "API key", "provider": "api", "optIn": true }
]
```

How a session gets its account:

- **The most room first.** The runner reads each subscription's use (the 5-hour and weekly windows Claude Code's
  usage view shows) every 10 minutes, and from every rate-limit event a running session sees. A new session goes to
  the account that may take its work with the most room; `order` breaks ties (lower first), and an account with less
  than 10% left goes after the others.
- **A session stays put while its account has room**, so nothing moves mid-run. Between turns it may move to the
  default account (`defaultAccount` in the settings) or off one that is nearly full; it resumes there with its whole
  conversation.
- **At a usage limit it moves and carries on.** When a turn ends on a limit, that account is out until it resets, and
  the session moves to the next account and carries on with the same turn. When every account is out, the turn ends
  saying when the first one is free, and the session carries on then by itself. An `api` or `bedrock` account that
  refuses or throttles a turn is out for 15 minutes.
- **Opt-in accounts** (`optIn: true`, typically pay-per-token ones) take work only when it asks for them (by account
  id, or a model id only that provider runs) or when every other account is out.
- **Work groups** keep some work to some accounts: define a group in the settings by its project or folder pattern,
  and list it on the accounts that take it (`groups`); `groupsOnly: true` keeps an account to its groups' work. Work in
  a group runs only on its accounts, and a runner with none refuses it with a plain message.
- `enabled: false` lists an account without ever using or looking at it.

Sessions on every account share one transcript store and one memory per project: a new account folder links the
default folder's `projects`, settings, instructions, agents, commands and plugins, so a session moves between accounts
with its context.

## Settings

`config.json` in the runner's home (`EMBERHUT_RUNNER_HOME`; by default `~/Library/Application Support/Emberhut
Runner` on macOS, `~/.local/share/emberhut-runner` on Linux, `%LOCALAPPDATA%\Emberhut Runner` on Windows). Pairing
writes `name` and `server`; the rest is optional.

| Key              | Default                     | What it is                                                                                                                                                                                                 |
| ---------------- | --------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `name`           | the machine's name          | The runner's name on the server (letters, digits, `-`, `_`).                                                                                                                                               |
| `server`         |                             | The server's address.                                                                                                                                                                                      |
| `tokenFile`      | `token`                     | The runner's token (written by pairing; only you may read it).                                                                                                                                             |
| `port`           | `8806`                      | The port its API listens on.                                                                                                                                                                               |
| `listen`         | `["127.0.0.1"]`             | The addresses it listens on.                                                                                                                                                                               |
| `advertise`      | `http://<listen[0]>:<port>` | The address the server calls it at.                                                                                                                                                                        |
| `accountsFile`   | `accounts.json`             | The accounts.                                                                                                                                                                                              |
| `defaultAccount` |                             | An account that goes first for any work it may take.                                                                                                                                                       |
| `groups`         | `{}`                        | Work groups: `{ "client": { "projects": "regex", "folders": "regex" } }`.                                                                                                                                  |
| `worktreesDir`   | `~/Documents/worktrees`     | Where a session's git worktree is made: `<worktreesDir>/<repo>/<branch>`.                                                                                                                                  |
| `branchPrefix`   | `emberhut/`                 | The prefix of a worktree branch a session doesn't name.                                                                                                                                                    |
| `model`          | the agent's default         | The model a session runs on when it names none.                                                                                                                                                            |
| `instructions`   |                             | `{ "work": "...", "direct": "...", "machine": "..." }`: text added to sessions' system prompts (for the assistant's sessions, for your own, about this machine). `work` and `direct` replace the defaults. |
| `skillsDir`      |                             | A folder of skills linked into every account's skills.                                                                                                                                                     |
| `shareMcp`       | `true`                      | Whether sessions get the MCP servers the server hands out.                                                                                                                                                 |
| `jobLabels`      |                             | macOS: a regular expression over launchd labels the Background jobs page lists.                                                                                                                            |
| `maxFileBytes`   | 200 MB                      | The largest file taken in or sent out.                                                                                                                                                                     |
| `machineEvery`   | `20000`                     | How often a machine sample goes to the server, in ms.                                                                                                                                                      |

The environment overrides a few, for service managers and tests: `EMBERHUT_RUNNER_NAME`, `EMBERHUT_SERVER`,
`EMBERHUT_RUNNER_PORT`, `EMBERHUT_RUNNER_TOKEN_FILE`, `EMBERHUT_RUNNER_ACCOUNTS`.

## Network

The runner and the server each call the other at a configured address with the runner's token, so each address must
be one you trust ([ADR 0017](../../docs/adr/0017-runners-machines-and-accounts.md), "Transport"): `https://` with a
certificate the caller checks, or a network that already limits who can connect (loopback, a VPN such as WireGuard or
Tailscale, a private network). The relay is never used for runners. A runner on the server's own machine needs
nothing more (`listen` stays loopback); on another machine, set `listen` to its address on that network (or
`0.0.0.0` behind a firewall that allows only the server) and pair with `--advertise http://<that address>:8806`.

## Security: what the runner can touch

The runner is a remote control for coding agents on your machine, so treat its token like an SSH key.

- **Sessions have full access as your user.** Claude Code runs with permissions bypassed
  (`bypassPermissions`), in the folder a session was opened in, with your user's files, tools and network, as it
  would in your own terminal with that flag. Whoever can open sessions on a runner (the server, and through it every
  signed-in device of yours and the assistant) can run code on that machine as you. Install a runner only on machines
  where that is what you want, under an account whose reach you're comfortable with.
- **Only the server, with the token.** Every route but `GET /health` needs the runner's token, compared in constant
  time; the token is written at pairing, readable by you only, and never logged. The runner listens on loopback unless
  you set `listen`.
- **Logins and keys stay where they are.** A subscription's login stays in its Claude Code folder (or the macOS
  Keychain); the runner checks only that one exists. Provider keys are read from `secrets/` when a session starts and
  passed to it in its environment, never to the server. `GET /accounts` reports use and errors, never a login or a key.
- **Files.** The server can put files into a session's inbox (in the runner's home) and read files back only from a
  session's own folders (where it was opened, its worktree, its inbox) and the temporary folder: never `.ssh`,
  `.gnupg`, `.aws`, `.kube`, `.docker`, key and certificate files, `.env` files, `.netrc`, `.npmrc`, credential files,
  or the runner's own data. Links are resolved first, `..` is refused, both directions are capped (`maxFileBytes`), and
  every transfer and refusal is recorded.
- **Folders.** The new-session sheet lists folders inside your home folder only, without hidden ones.
- **What it reports.** Sessions' titles, prompts, answers and steps (as the apps show them), and a machine sample
  every 20 s: CPU, memory, GPU, disk and battery figures, the host name, and the runner's own sessions. Nothing else
  leaves the machine.
- **Jobs.** `POST /jobs/stop` stops only a `codex exec` or `claude -p` process it lists, never anything else.

## Develop

```sh
pnpm install
pnpm --filter @emberhut/runner run check   # types, lint, format, tests
pnpm --filter @emberhut/runner start       # the daemon, with the settings in EMBERHUT_RUNNER_HOME
```

The tests drive a real runner over HTTP with a stand-in agent and a stand-in server. `test/contract.test.ts` holds
what the runner sends to the server's contract fixtures (`crates/emberhut-api/tests/fixtures/runner_*.json`) and keeps
a record of it in `crates/emberhut-api/tests/fixtures/runner/`, which the Rust contract tests read with the server's
own types and the server's tests report again as runner events. After a deliberate change to what the runner sends,
write the record again with `EMBERHUT_BLESS=1 pnpm --filter @emberhut/runner test` and check the difference in.

`node scripts/package.ts --target <os-arch> --version <v>` builds a release download on that platform (CI's
`runner-build` workflow runs it for each one).

| Path                            | What it is                                                                                  |
| ------------------------------- | ------------------------------------------------------------------------------------------- |
| `src/cli.ts`                    | The command line: `run`, `pair`, `service`, `accounts`, `status`.                           |
| `src/runner.ts`                 | One runner, put together.                                                                   |
| `src/http.ts`                   | The API the server calls.                                                                   |
| `src/sessions/`                 | Sessions and turns over the Agent SDK, the live stream, folders and worktrees, the prompts. |
| `src/accounts/`                 | The accounts list, logins and each session's environment, use and picking.                  |
| `src/report.ts`                 | Events to the server, in order, kept until taken.                                           |
| `src/machine.ts`, `src/jobs.ts` | The machine sample and the background jobs.                                                 |
| `src/files.ts`                  | Files in and out, and their rules.                                                          |
| `src/service.ts`, `src/pair.ts` | The service on each platform, and pairing.                                                  |
| `src/wire.ts`                   | The protocol's words.                                                                       |
| `install/`                      | The one-line installers.                                                                    |
