# emberhut-runner

The runner is the program on each of your machines (each hut) that runs coding-agent sessions for your Emberhut
server. The server and the apps decide what to do; the runner does it on the machine where the code, the tools and the
logins are: it starts Claude Code sessions in a folder (through Claude Code's own command line, `claude -p` with
stream-json in and out), sends them each turn, picks an account with room and moves a session to another one when an
account runs out, and reports every change to the server as it happens. The server's side of this protocol is
[ADR 0017](../../docs/adr/0017-runners-machines-and-accounts.md) and [api.md, "Runners"](../../docs/api.md#runners);
this crate is the other side ([ADR 0033](../../docs/adr/0033-the-runner-in-rust.md)).

It is one static binary per platform, with no runtime to install besides Claude Code itself.
The macOS desktop app carries it too, at `Emberhut.app/Contents/MacOS/emberhut-runner`, signed with the identifier
`com.emberhut.runner`.

## Install

You need [Claude Code](https://docs.claude.com/en/docs/claude-code/overview) on the machine (`claude` on the `PATH`,
or set `agentCommand`). Then one command. Get a pairing code first: in the app, **Machines → Add a hut** (it fills
in the command for you), or on the server's machine `emberhut-server pair-runner`.

**macOS and Linux**

```sh
curl -fsSL https://github.com/emberhuthq/emberhut/releases/download/v<version>/install-runner.sh | sh -s -- --server https://your-server.example.com --pair ABCD-1234
```

**Windows** (PowerShell, as an administrator: it installs a Windows service)

```powershell
$env:EMBERHUT_SERVER="https://your-server.example.com"; $env:EMBERHUT_PAIR_CODE="ABCD-1234"
irm https://github.com/emberhuthq/emberhut/releases/download/v<version>/install-runner.ps1 | iex
```

The installer downloads the release's binary for your platform, checks its SHA-256 against the release's
`SHA256SUMS`, and runs `emberhut-runner install --server URL --pair CODE`, which pairs the runner with your server and
installs it as a service: a launchd agent on macOS, a systemd user unit on Linux, a Windows service. Running the
installer again upgrades in place. On a Linux machine you aren't logged in to, keep the user unit running with
`loginctl enable-linger`.

With the binary already on the machine, the same step is one command:

```sh
emberhut-runner install --server https://your-server.example.com --pair ABCD-1234
```

Its options: `--name` (the runner's name on the server, default the machine's), `--place` (where the machine is, in
words), `--advertise URL` (the address the server calls the runner at, see [Network](#network)), `--force` (replace
a service file the runner didn't write).

After installing:

```sh
emberhut-runner status             # does it answer, and is it paired
emberhut-runner accounts           # its accounts, their use and logins
emberhut-runner accounts login default
emberhut-runner service status     # what the service manager says (its log is runner.log in the runner's home)
```

On Windows the service runs as `LocalSystem`, so the Claude Code logins it uses are that account's: sign them in with
`emberhut-runner accounts login <id>` from a shell running as that account, or use an `api` or `bedrock` account.

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

To add a subscription, `emberhut-runner accounts add` makes the next `claude-N` folder in the runner's home and runs
Claude Code's own browser sign-in in it (`claude auth login --claudeai`); the runner never sees the login.
`emberhut-runner accounts list --json` lists them with their email, plan and use, and `accounts remove <id>` takes one
off the list (its folder stays).

How a session gets its account:

- **The most room first.** The runner reads each subscription's use (the 5-hour and weekly windows Claude Code's
  usage view shows, asked of Claude Code itself) every 10 minutes, and from every rate-limit event a running session
  sees. A new session goes to the account that may take its work with the most room; `order` breaks ties (lower
  first), and an account with less than 10% left goes after the others.
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

## Sessions

Each open session is one Claude Code process (`claude -p --input-format stream-json --output-format stream-json`),
started with the account's `CLAUDE_CONFIG_DIR`, `--session-id` (or `--resume` once it has a transcript), the
session's `--model` and `--effort`, the MCP servers the server hands out (`--mcp-config`) and the runner's
instructions (`--append-system-prompt`). Turns queue one behind the other. An interrupt is Claude Code's own
`interrupt` control request; a stop also cancels what is queued. A session with nothing to do closes its process after
10 minutes, and its next turn resumes it. A runner restart, or a process that dies mid-turn, resumes the turn where it
was. A running turn whose agent says nothing for 30 minutes (and waits on no background job or sub-agent) is carried
on in a new process. Your own sessions (opened from the app) can ask you a question: it shows in the app as a card and
the session waits for your answer.

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
| `model`          | Claude Code's default       | The model a session runs on when it names none.                                                                                                                                                            |
| `instructions`   |                             | `{ "work": "...", "direct": "...", "machine": "..." }`: text added to sessions' system prompts (for the assistant's sessions, for your own, about this machine). `work` and `direct` replace the defaults. |
| `skillsDir`      |                             | A folder of skills linked into every account's skills.                                                                                                                                                     |
| `shareMcp`       | `true`                      | Whether sessions get the MCP servers the server hands out.                                                                                                                                                 |
| `jobLabels`      |                             | macOS: a regular expression over launchd labels the Background jobs page lists.                                                                                                                            |
| `maxFileBytes`   | 200 MB                      | The largest file taken in or sent out.                                                                                                                                                                     |
| `machineEvery`   | `20000`                     | How often a machine sample goes to the server, in ms.                                                                                                                                                      |
| `agentCommand`   | `claude`                    | Claude Code's command (a path, or a name on the `PATH`).                                                                                                                                                   |

The environment overrides a few, for service managers and tests: `EMBERHUT_RUNNER_NAME`, `EMBERHUT_SERVER`,
`EMBERHUT_RUNNER_PORT`, `EMBERHUT_RUNNER_TOKEN_FILE`, `EMBERHUT_RUNNER_ACCOUNTS`, `EMBERHUT_RUNNER_AGENT`.
`EMBERHUT_RUNNER_LOG` sets the log's level (`info` by default; `debug` for more).

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
  (`--permission-mode bypassPermissions`), in the folder a session was opened in, with your user's files, tools and
  network, as it would in your own terminal with that flag. Whoever can open sessions on a runner (the server, and
  through it every signed-in device of yours and the assistant) can run code on that machine as you. Install a runner
  only on machines where that is what you want, under an account whose reach you're comfortable with.
- **Only the server, with the token.** Every route but `GET /health` needs the runner's token, compared in constant
  time; the token is written at pairing, readable by you only, and never logged. The runner listens on loopback unless
  you set `listen`.
- **Logins and keys stay where they are.** A subscription's login stays in its Claude Code folder (or the macOS
  Keychain); the runner checks only that one exists. Provider keys are read from `secrets/` when a session starts and
  passed to it in its environment, never to the server. Nothing of the runner's own environment's logins or provider
  keys reaches a session on another account. `GET /accounts` reports use and errors, never a login or a key.
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
cargo test -p emberhut-runner
cargo run -p emberhut-runner -- run      # the daemon, with the settings in EMBERHUT_RUNNER_HOME
```

`tests/contract/` drives a whole runner in-process over its API, with a stand-in agent speaking Claude Code's
stream-json and a stand-in server that keeps what it is sent, and reads every answer and event with the server's own
types (`emberhut-api`). `tests/parity/` plays one scenario of the server's calls against two running runners and
compares the shapes of what they answer and report (ignored by default; its docs say how to run it).
`tests/engine_live.rs` runs the Claude Code driver against the real command line when `EMBERHUT_LIVE_CLAUDE` names it.

The release builds are `.github/workflows/runner-release.yml`: static binaries for Linux (musl, x64 and arm64),
macOS (arm64 and x64) and Windows (x64), each with the installers.

| Path                                       | What it is                                                                     |
| ------------------------------------------ | ------------------------------------------------------------------------------ |
| `src/cli.rs`, `src/main.rs`                | The command line: `run`, `install`, `pair`, `service`, `accounts`, `status`.   |
| `src/runner.rs`                            | One runner, put together.                                                      |
| `src/http.rs`                              | The API the server calls.                                                      |
| `src/engine/`                              | The coding agent behind a trait: Claude Code through its command line.         |
| `src/sessions/`                            | Sessions and turns, the live stream, folders and worktrees, the prompts.       |
| `src/accounts/`                            | The accounts list, logins and each session's environment, use and picking.     |
| `src/store/`                               | The runner's database: sessions, turns, transcript steps, the outbox, files.   |
| `src/report.rs`                            | Events to the server, in order, kept until taken.                              |
| `src/machine/`, `src/jobs.rs`              | The machine sample and the background jobs.                                    |
| `src/files.rs`                             | Files in and out, and their rules.                                             |
| `src/service/`, `src/pair.rs`              | The service on each platform, and pairing.                                     |
| `src/config.rs`, `src/wire.rs`             | The settings, and the protocol's words.                                        |
| `install/`                                 | The one-line installers.                                                       |
