# 0014. The agent runtime: a CLI session per room, the wake rule, and the assistant's own tools

- Status: Proposed; its second agent with its own persona is superseded by [ADR 0027](0027-agents-and-approvals.md)
- Date: 2026-11-26

## Context

Until now `POST /api/talk` stored the message and ran a turn on the `Assistant` trait, whose
only implementation answered nothing (`Silent`, ADR 0002). The assistant is an agent CLI
(Claude Code, speaking stream-json on its standard input and output) kept running as one
long-lived session per room. Around it:

- **The session.** Started with the room's first turn and kept: each turn is one user message
  written to the CLI, read back until its `result`. The session id is kept so a restart (or a
  session closed to make room) resumes the same conversation. Every session gets the server's
  MCP endpoint with its room's token (ADR 0009). Sessions open at once are capped; a room's
  idle session closes after 30 minutes (Main's never).
- **Pause and interrupt.** A switch (`on`) pauses the assistant: every room's turn stops and
  nothing wakes it. The app's stop button interrupts one room's turn (the CLI's `interrupt`
  control request) and the chat says `Stopped.`.
- **Failover.** Sessions run on several accounts; when one runs out of usage mid-turn (a
  rejected rate-limit notice, an error result saying so), the session moves to the next
  account with room, resuming the same conversation, and the turn carries on. A setting
  (`testLimit`) pretends a room's next answer ran out, to test this.
- **The wake rule.** Programs and the server tell the assistant things as events
  (`POST /api/events`, check-ins coming due, watches, routines, another room handing work over,
  a followed coding thread ending). A room's waiting events become one turn (`Event: - [kind]
  text` lines): the assistant's own appointments and news someone waits on at once, the rest
  after a 90-second burst (or ten events), at most twenty wakes an hour for all rooms, never
  while the room is in a turn. A turn reads the oldest ten waiting events at most; the rest
  wait for the next wake.
- **Its own tools** on the MCP endpoint: `wake_me_at`, `list_checkins`, `cancel_checkin`,
  `watch`, `watch_note`, `list_watches`, `stop_watch`, `room_send`, `rooms`, `send_file`,
  `set_routines` and `tell_owner`; routines (the morning brief, the evening wrap) and quiet
  hours; the activity log (`did`); how fresh each source is (`awareness`); an answer's chips
  (`refs`); and what the other rooms said about the task a turn names (same-task memory).

## Decision

### An `Assistant` with a CLI behind a spawner

`services::claude::ClaudeAssistant` implements `Assistant`. It does not start processes
itself: a `CliSpawner` does, given a `Launch` (program, arguments, environment changes,
directory), and hands back the CLI's input and output as lines (`CliPipes`). The real spawner
(`ProcessSpawner`) runs a tokio child process in a process group of its own; stopping a
session ends its input and sends `SIGTERM` to the group (the CLI and whatever it started, such
as its own MCP servers), then `SIGKILL` five seconds later if the CLI is still there (through
`rustix`, without `unsafe`). When the server stops, after its turns ended, it closes the
sessions kept between turns and waits for them (`Assistant::close_all`), so idle sessions get
the same grace. The server's tests use a scripted spawner that answers like the CLI, so no test
runs a real CLI or reaches the network. The protocol the server
speaks is the agent SDK's, as far as a turn needs it (`services::claude::stream`): the
`initialize` control request with the server's system prompt after the CLI's own, a user
message per turn, `interrupt`, and an error answer to any control request the server does not
serve (it registers no hooks and no permission callback).

Nothing names an account, a machine or a person. The configuration (`EMBERHUT_AGENT_*`, flags of
the same names) gives the command, extra arguments, the model, the working directory, the
permission mode, the accounts (`ID` or `ID=CONFIG_DIR`, the CLI's configuration directory set
as `CLAUDE_CONFIG_DIR`), a file of extra system prompt, and the cap on sessions. Without a
command the server keeps `Silent`. The sessions are unattended: nobody answers a permission
prompt. Every session allows its own MCP server's tools (`--allowedTools=mcp__<server>__*`), and
the permission mode (`EMBERHUT_AGENT_PERMISSION_MODE`) decides the rest:

- `dontAsk`, the default: any other tool that would ask is refused without asking. Out of the
  box the assistant acts only through the server's own tools (and the CLI's tools that never
  ask, such as reading files).
- `bypassPermissions`, an explicit opt-in: every tool runs
  without asking, the shell and file edits included, as the user the server runs as, in the
  working directory and wherever that user can write. Only for an installation that wants the
  assistant to do work on the machine itself.
- Any other mode the CLI knows is passed through as it is.

The session id (and the account it last ran on) is a setting per room (`agentSession:<room>`). The
cap (`EMBERHUT_AGENT_MAX_SESSIONS`, four) counts every open session, those in a turn too: a new one
closes the room session idle longest, never Main's and never one in a turn. The wake tick closes
rooms' sessions idle for 30 minutes. The MCP endpoint is passed as `--mcp-config`; its URL is the
listen address (`127.0.0.1` when the server listens on every address). The room's token is not in
that JSON: its header reads `Bearer ${EMBERHUT_MCP_TOKEN}`, which the CLI expands from its
environment, and the server sets `EMBERHUT_MCP_TOKEN` for the child only.

### The turn carries what the runtime needs

`Turn` gains its `prompt` (the message with the one it replies to and its files described, or
`Event: ...`), `from_owner`, an `interrupt` token and the room's agent token. `TalkService`
gains `post` (an event's turn: an event line in the chat, then the turn), `interrupt`,
`stop_all` (pausing), `send_file` (an outbox the next reply carries), `busy`, and `alert`
(`tell_owner`). A reply that is exactly `NO_REPLY` puts nothing in the chat, only a line in the
activity log; a reply to an event's turn reaches the phones only outside quiet hours. The
`Assistant` trait gains `test_limit`, a no-op by default. These are additions: `Silent` and the
tests' scripted assistant answer as before.

### Failover is a rule in emberhut-core

`emberhut_core::failover::AccountPool` keeps the accounts in their configured order and which are
out until when; a session stays on its account while it can take work, else moves to the first
that can. An account out without a reset time is out for an hour. The CLI's wordings for
running out are matched by hand (no regex dependency). With every account out the answer
reaches the chat as it came. Which accounts the owner pins and the accounts' use for the app
(`account`, `assistantAccount` in the state) are the accounts part's; this runtime only rotates.

### The wake rule

`services::wake::WakeService` keeps the switch (`on`), the check-ins (`checkins`, the 20
soonest) and the watches (`watches`, `watchSeq`) as settings (their shapes in api.md),
and ticks every 30 seconds: due routines and check-ins become events, each room not in a turn
wakes by `emberhut_core::wake`'s rule, and due watches remind their room (a watch on a coding thread
only after that thread changed since its last note). It also takes over polling the coding
threads (ADR 0013), so a followed thread's end or question becomes a `thread` event in Main.
Before a room wakes for its events, the owner's messages that never got an answer (a restart
dropped their turn, or it failed) are sent to it again once, when a minute old: those after the
room's last answer and after its last stop (the settings `stoppedThrough` and
`stoppedThrough:<room>`). A kept session that died between turns
is started again for the next turn, on the same conversation; a shutdown interrupts the turns
running, so they end within the grace period.
`POST /api/events` stores an event in the room a `project` names (a room's name), Main
otherwise; a project given by number goes to Main until projects have rooms of their own. Its
`text` is 1 to 100 000 characters, as a message's, and its `kind` at most 32 of `a-z`, `0-9`
and `_`: the kind is written into the turn's `- [kind] text` lines, so a bracket or a line
break in it would forge another event. Events still pile up while the assistant is paused or
the hourly ceiling is reached, so a wake reads one bounded batch, the oldest ten
(`emberhut_core::wake::PER_WAKE`, `EventStore::waiting`'s limit), marks only those read, and leaves
the rest for the next wake: a turn's prompt, the chat line it is stored as and the state that
serves it stay bounded however many events a script or a webhook sends.
An event wakes the assistant, which then acts with its tools, so `POST /api/events` needs a
signed-in device in either auth mode: a program posting events uses a paired device's token.

Routines are a setting (`routines`, `quiet`), times of day in the server's time zone
(`EMBERHUT_TIME_ZONE`, the system's by default). Until the owner sets their own, the routines are
the morning brief and the evening wrap (`emberhut_core::briefs`), worded for any installation; the
brief also carries what each room said since the day before. A routine wakes Main once a day
(an event with the key `routine:<local day>:<time>`).

### Rooms, files, alerts, chips and same-task memory

- `room_send` stores a `room` event in the target room (`From the Main room: ...`); `rooms`
  reads the last twelve lines of each room since a time, and each room's line as the switcher
  shows it.
- `send_file` keeps a file on this machine through `UploadService::keep` and adds it to the
  room's outbox; files on another machine (a `machine:/path`) are not
  fetched. The agent can be steered by text it reads (an event from a webhook, a note it
  recalls), and a kept file reaches the chat and every signed-in device, so `keep` resolves the
  path to its real path first and refuses what never leaves the server
  (`services::private::PrivatePaths`, the one set the runners' file pushes refuse too, ADR
  0017: the secrets folder, the database and its journals, the machine's key, the configured
  token and key files, the agent accounts' configuration directories and the home's key
  folders), a file in the uploads folder, and a
  hidden file or one in a hidden folder (where keys, tokens and tools' configuration live, such
  as `~/.ssh` or the CLI's default configuration directory). It opens the file once and copies
  from that handle, at most 500 MB, so the size checked is the size copied.
- `tell_owner` pushes an alert through the push service, whatever the quiet hours.
- An answer's chips are found in its text without a model (`emberhut_core::refs`): a GitHub pull
  request, a Linear ticket, and a report page (an `.html` address with no secret in its query).
  They are kept in a table of their own (`message_refs`, one JSON list per answer) and put back
  on the line wherever the chat is shown. Chips for tasks (the tasks a turn touched) come with
  the tasks part.
- Same-task memory reads the keys a turn names (`TASK-12`, `ENG-42`: capitals, a dash, a
  number) and puts after the message what every room said naming them in the last 48 hours
  (the turn's own room before its last two hours), each line once per room.

### Left out, and why

These parts of an agent loop belong to one installation or to other areas of the server.

- The per-person rooms and their rules (a family member's room with its own tool set) and a
  second agent with its own persona: personal to one installation. The generic mechanism, one
  session per room with its own token, is here.
- The tool gate that blocks file edits and caps the assistant's quick commands per turn (a
  "dispatcher, not worker" rule): it presumes the tasks and workers that
  take the work instead, which are other areas; without them it would leave the assistant
  unable to act.
- The memory block put in front of a fresh or compacted session (goals, tasks, loops, people,
  facts): its contents are the tasks, plan and facts areas'. The session's own transcript is
  resumed.
- The ask guard (a second model call that holds a reply asking for an OK already given),
  account pinning, cloud-provider accounts, the MCP servers copied from the CLI's own
  configuration and the switches that turn them off: other areas, or tied to one installation.
- Retrying a turn a minute later when the service could not be reached, as its own timer: the
  turn fails with the usual note, and the redelivery below sends the message again once.

## Consequences

- An installation gets a working assistant by setting `EMBERHUT_AGENT_COMMAND` (and, for failover,
  `EMBERHUT_AGENT_ACCOUNTS`); without it nothing changes. It acts only through the server's tools
  until `EMBERHUT_AGENT_PERMISSION_MODE=bypassPermissions` lets it run commands and edit files.
- The session's MCP token is not on the CLI's command line (which any user of the machine can
  read, through `ps`), only in the child's environment, which only its own user (and root) can
  read. It is valid only until the server restarts and only for that room's tools. A CLI that
  did not expand `${VAR}` in `--mcp-config` would send the placeholder and be refused, not leak
  the token.
- Tests drive the whole loop (talk, events, ticks, interrupts, failover) through the scripted
  spawner and the manual clock; the real spawner is tested with `/bin/sh` standing in for the
  CLI (lines in and out, a stop reaching what it started, a child ignoring `SIGTERM` killed
  after the grace).
- A watch's or check-in's room, a routine's day and an event's room are kept in the settings'
  shapes, so an imported archive keeps its appointments.
