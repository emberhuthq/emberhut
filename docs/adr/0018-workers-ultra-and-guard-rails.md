# 0018. Workers, ultra runs and the guard rails: a CLI session per task, a gate per step, one OK per job

- Status: Proposed; its delegated approver and trusted requesters are superseded by [ADR 0027](0027-agents-and-approvals.md)
- Date: 2026-11-26

## Context

The assistant does its own tasks with **workers**: a task the assistant
owns, with a brief, gets its own session of the agent CLI, whose tools reach only that task
(notes into its history, one question to the owner, done). The owner's answer resumes the same
session. The apps watch a worker as the session `w-<task>` (its steps), stop it (`taskStop`),
see every worker and sub-agent running now (the state's `agentsNow`) and everything in flight
(`working`).

A task can be an **ultra run**: its worker is the lead of many sub-agents in parallel, a verify
agent per part. The lead posts a plan first (`ultra_plan`); a big plan waits for the owner; a
gate before every new sub-agent and shell command holds the plan, the parallel cap and the
spending cap; parts can run as their own sessions on other accounts (`ultra_lane`,
`ultra_wait`).

Around all the assistant does are **guard rails**: one job, one OK at most (the owner's request
or one approval covers a task's ordinary steps; a message or post needs its exact text OK'd
once, for any channel; payments, passwords, codes and account security ask every time); the
owner's "send it" in the chat as the OK for a text they were shown; a card's Send it sending
there and then; Go ahead cards for a job's final steps in the browser or on the owner's Mac;
outward steps (push, merge, release, deploy) held until the owner's words cover them, read from
a command's verbs by a shell reader; an **ask guard** that sends back an ask the owner's words
already cover; and keys masked wherever they would show.

ADR 0014 runs the assistant's room sessions and lets the tasks' area (ADR 0015) reach workers
through `TaskWorkers`. ADR 0017 left the `w-{id}` routes and `stopTask` waiting on a worker
runtime (`WorkerSessions`). ADR 0020 left messaging's other OKs (`SendOks`) as a stand-in.

## Decision

### Rules in the core, state in the settings

`emberhut_core::guard` holds the rules without I/O, each with its own tests on its
cases: `shell` (a command line cut into its simple commands as bash runs them: words unquoted,
the folder each runs in, what heredocs feed them; a line it can't read is an error), `outward`
(the outward steps of a line, read through `bash -c`, `ssh`, `sudo`, `env`, `xargs`, `eval`,
`su -c`, `script -c`, `tmux`, `trap`, `npm exec`, `uv run`...;
the owner's words that name a step; a repository's name), `key_mask` (token shapes masked, the
sites keys are never saved from), `go` (a clear go, a clear yes, a stop), `grants` (what a
step is, a text's key, what a tap grants), `chat_ok`, `send_tap`, `go_ahead` and `ask_guard`
(what always asks, a reply that asks, the judge's instructions and how its answer is checked:
a quote must be the owner's own words, word for word). `emberhut_core::ultra` holds the estimate,
the caps, a sub-agent's kind and part from its name, the parts done and the lead's brief.

The server's `services::guard` keeps the OKs in the settings, one key each (`jobGrants`, `textOks`,
`postOks`, `stepCards`, `chatOksUsed`, `outwardOks`, `browserPending`, `browserPlans`,
`browserByRequest`, `roomRequests`, `sendTaps`, `askGuard`), so an imported archive keeps its OKs. A
history line's keys are masked as it is written (`TaskService::note`), which ADR 0015 left to this
area.

### Workers: children of the server, on the worker endpoint

- A worker is the agent CLI started through the runtime's `CliSpawner` (ADR 0014), with the
  brief as its first message and read until its result. It is a child of the server, not a unit
  of the machine's service manager: one server need not run on a machine that has one (a
  container, another system). A task has one worker at most: a start checks and takes the
  task's slot under one lock before it reads or spawns anything, and gives it back on every way
  out, so the tick and `task_work` racing on a fresh task start one CLI.
- A server that stops ends its workers before its background tasks are dropped
  (`WorkerService::close_all`, awaited in `serve`): no worker starts any more, each one running
  is stopped (its CLI's whole process group gets `SIGTERM`, then `SIGKILL` after the spawner's
  grace), and the server waits for each to exit and be wrapped up. The tick in flight is
  dropped at the stop. The task's session is kept, and the tick picks the task up again (the
  owner's answer, or its brief).
- Each worker reaches `POST /mcp/worker` with its task's token (held in memory as a hash, like
  the rooms'; in the child's environment only, as `EMBERHUT_MCP_TOKEN`). Its tools are the room's
  read tools (messages, calendar, facts, recall, the plan and tasks; nothing that acts), its own
  task's (`task`, `task_note`, `task_findings`, `task_ask`, `task_runner`, `task_done`, which
  take another task only when the brief names it), and an ultra lead's (`ultra_plan`,
  `ultra_lane`, `ultra_wait`). The browser's and the Mac's tools join the set when those areas
  bring them.
- Its permission mode is ADR 0014's (`dontAsk` by default, its own MCP server's tools allowed).
  Unless the owner's words cover the task's outward steps, every worker gets the gate as a
  `PreToolUse` hook before each shell command (`Bash`): `curl` to `POST /ultra/gate` with its
  token from its environment; when the server doesn't answer, the hook refuses. The gate runs
  the outward rule on the command for any worker's task. An ultra lead gets the gate before
  each new sub-agent as well (`Agent`, `Task`, `Workflow`: the plan and the caps). Its push, PR,
  release and API commands are also refused by prefix (`--disallowedTools`); prefixes are easy
  to get around (`git -C . push`, `env git push`), so they back the gate up and never stand in
  for it.
- `POST /mcp/worker` and `POST /ultra/gate` have no request time limit, as uploads and the
  backup don't: `ultra_wait` waits minutes for its lanes, `task_ask` waits on the ask guard's
  judge, and each bounds its own wait (the hook's `curl` gives the gate 20 seconds).
- Its stream goes to a turn file in the task's folder (`work/<task key>/turn-<ms>.json` beside
  the database, scratch the backup leaves out). The steps the app shows as `w-<task>` are read
  back from those files (`services::workers::steps`), so nothing is copied; an ultra lead's
  sub-agents are seen in the same lines (the CLI's `task_started`, `task_progress`,
  `task_notification`, `task_updated`) and kept in `agents`, with how far each stream was read
  and the lead's step and tokens in `streams`.
- Every 30 seconds the tick sweeps sub-agents whose run ended, resumes the tasks the owner
  answered (their `next`), starts every assistant task with a brief and no worker yet (a ceiling
  with `EMBERHUT_WORKER_MAX`), and tells the assistant once of a task with a brief that sat three
  minutes with no worker. A worker that ends without `task_done` or `task_ask` asks the owner
  whether to try again; the assistant hears what came of the task (`ask` or `work` events).
- With no agent command configured the server runs no workers, and `Dependencies::workers` and
  `worker_sessions` stand in (the tests' fakes, or `NoWorkers`).

### Ultra runs

The run is kept on the task (`issues.ultra_run`, as JSON; `issues.turn_file`
for the stream), its caps from the task (`ultra_parallel`, `ultra_budget`) or the server's
(`EMBERHUT_ULTRA_PARALLEL` 4, `EMBERHUT_ULTRA_BUDGET` 3M tokens, a plan over `EMBERHUT_ULTRA_ASK_TOKENS`
1.5M tokens or `EMBERHUT_ULTRA_ASK_AGENTS` 12 sub-agents asks). A big plan the owner's own OK covers
(the job's grant, their OK relayed) starts without a card; their Go ahead raises the cap to fit.
A lane is a runner session (ADR 0017) on `EMBERHUT_ULTRA_LANE_RUNNER` or the first runner, on
another account the runner picks; `ultra_wait` reads each lane's state from its thread, and
only for this run's own lanes (another session id is named as not one of them, never read). A
runner's session has neither the gate nor the deny rules, and the runner's protocol has no way
to give it them, so a lane starts only once the owner's words cover the task's outward steps;
until then the lead runs the part as a sub-agent of its own session, behind the gate. No
account is named in the code. The owner's answers reach the run through the tasks' answer
takers, which now hear every taker in turn (the guard rails', messaging's, the ultra runs').

### The guard rails' services

- `GuardOks` is messaging's `SendOks` (ADR 0020) over the guard rails, layered over the
  dependencies' own (the stand-in by default, the tests' fakes in theirs): the tap just made,
  the owner's go in the chat on the exact text, an exact text's OK on another card, the standing
  OK for a 1:1 chat (`EMBERHUT_SEND_WITHOUT_OK`, a file read at every send), and spending them.
  The owner's Send it on a Slack or WhatsApp card is done there and then through the channel's
  own guard (`answer_card`), the result kept for the task (`sendTaps`) and the room told. A tap
  is a send card's only when the ask it answers (the task's open ask, by its id) is the one
  that shows the card: another card on the task, a re-check or one with the same question,
  sends nothing (ADR 0020). The answer's history line is masked like every other line.
- The outward rule holds a worker's step until the owner's words cover it: their request naming
  it, their OK (relayed, or their Go ahead on the outward card the server puts on the task), or
  every repository it touches being one of `EMBERHUT_ALWAYS_MERGE` (`owner/name` or `*/name`, none by
  default), told from the folders' git remotes (`git remote -v`, behind `RepoRemotes`).
- The reader fails closed: what it can't be sure of is held like an outward step, with no
  repository (so `EMBERHUT_ALWAYS_MERGE` never lets it through). That is a line it can't take apart,
  more than nine words before a command (`sudo`, `env`, `nice`...), a command whose name is a
  variable or a substitution, a script file (`sh p.sh`, `./p.sh`, `source`) or a shell reading a
  pipe, an interpreter's inline code or script (`python -c`, `node -e`, a heredoc, a file), a
  command whose words decide what it does run by `xargs` or `find -exec`, git with a `-c` or
  environment setting that can change what it runs (aliases, `url.*.insteadOf`, hooks) or a
  command that isn't one of its own (an alias, `send-pack`), a `gh` alias or extension, an
  HTTP write (`curl`, `wget`, httpie: a method other than GET or HEAD, or a body) to anything
  but this machine, and a command on another machine (`ssh`) that isn't one of a short list
  that only reads there (`ls`, `cat`, `journalctl`, `systemctl status`, `docker ps`...; no
  file written), or `ssh` running what its input sends. A quoted string or a heredoc's body
  fed to a command that isn't a shell is still only text. The reader reads the line, not what
  it runs: a build script, a test runner or a git hook is code it never sees.
- Read-only work isn't held. Python's inline code (`python3 -c`, or a heredoc the shell
  expands nothing in) passes when it only reads and prints: its modules are from a fixed list
  of ones that don't run, write or reach the network (`json`, `re`, `html`, `csv`, `sys`...;
  `os` and `pathlib` only for what reads), every `open` opens to read, and no name of it runs
  code from a string or reaches what can't be seen (`exec`, `eval`, `getattr`, a dunder,
  `subprocess`, `system`). The code is read as Python's tokens, f-strings' fields as code; what
  it can't read that way is held as before. A shell that `xargs` or `find -exec` runs is read
  through when its code is written out (`sh -c '…'`) and the replacement sits only inside its
  double quotes: the input's words are read as unknown words, so a command whose words decide
  what it does is still held. The input is taken as names (a listing's lines); one holding a
  double quote would end the quotes, which the line alone can't rule out. `docker push`, `cargo publish`, `twine upload`,
  `dotnet nuget push`, and `scp` or `rsync` to another machine are outward steps.
- What the reader can't be sure of goes to the assistant before the owner. Its judge model
  (the ask guard's, below) reads the command with its own instructions: anything outward
  (a push, a deploy, a change on another machine, a paid API call, code it isn't shown) is not
  read-only, the command's text is data, and in doubt it isn't. A clear yes lets it run; its
  verdicts are kept in memory so a command run again isn't read again. A no, no judge, or no
  answer within 25 seconds puts the outward card to the owner as before. A step the reader
  knows is outward never goes to the judge.
- Wrappers are read through to the command they run (`sudo`, `env`, `nice`, `timeout`,
  `setsid`, `taskset`, `strace`, `systemd-run`, `unshare`, `chroot`, `runuser`, `firejail`,
  `npx`, `npm`/`pnpm` `exec`, `uv run`, `poetry run`...), each matched by its name, and by its
  path only in the system's own folders (`/usr/bin/time`; `./time` is a script). The code given
  to `sh -c`, `su -c`, `script -c`, `sudo -s`, `trap`, `bun`/`yarn exec` or a `tmux`/`screen`
  command is read as a line of its own; a shell `xargs` or `find -exec` runs, a word holding
  `xargs -I`'s replacement, keys typed into `tmux` or `screen`, and `parallel` are held.
- A backstop holds what that reading misses: a command whose own words (never what a heredoc
  feeds it) hold `git` or `gh` followed by a push or a `gh` write, or a package manager
  followed by `publish`, is held, whatever runs it and whether or not the reader read it
  through.
- The owner's words name an outward step sentence by sentence, normalised first (case,
  accents, curly and straight apostrophes, punctuation): a question names none, and nor does a
  sentence with a no, a wait or a condition in it ("don’t merge", "hold off on merging", "wait
  before you push", "merge it if the tests pass").
- The Go ahead cards of final steps are `Guard::step_gate` (and `plan_job` for a job's steps
  asked once). The browser's `StepGate` is an adapter over it: its gate calls
  `step_gate(task, label, page, card, composed)` and lets the click through on
  `GateResult::By(line)` (logging the line), holding it and telling the agent the text on
  `GateResult::Text`; the default `CardGate`'s one card per step becomes the grant model's.
- The ask guard's judge is a trait (`Judge`): one CLI turn with no tools and no MCP servers on
  the first configured account (`CliJudge`, model `EMBERHUT_ASK_GUARD_MODEL` or the agent's; `off`
  turns it off), none without an agent command. With no judge every ask reaches the owner;
  merging to main is still sent back when the owner's request names it (the outward rule).

### Left out, and why

- Systemd units for workers, an account rotation of their own (a pinned
  provider, logins refused and retried on the next account, a provider's outage), and the
  machine-specific folders a worker starts in: a worker here runs on the first configured
  account in the agent's working directory; rotation stays the runtime's (ADR 0014).
- Work handed for another person's OK (a delegated approver's OK through another room), the list
  of people whose asks count as the owner's, and one installation's client and repository names:
  personal to one installation. The generic parts (the job's grant, the standing OK list, the
  always-merge list) are configuration.
- A task's ask with a draft and a chat that turns into a send card by itself: the draft is on
  the task's card as before; the card's Send it for a `slack_send` or `whatsapp_send` card is
  here.
- Holding the assistant's own reply in a room (`Guard::reply`) is built; the talk service does
  not call it in the turn yet.

## Consequences

- With `EMBERHUT_AGENT_COMMAND` set, the assistant's tasks with a brief are worked by workers on the
  same CLI and accounts; without it nothing changes.
- Every worker's shell command and every ultra sub-agent pass a gate that is the server's code,
  not the prompt; a gate the server can't answer refuses, and a command it can't read is held.
  Inline code that only reads runs, and a script or inline code the reader can't read waits on
  the assistant's judge, then on a card only when the judge can't say it only reads. The gate
  answers within the worker's hook limit (a minute; the card goes up in the background when the
  database is slow).
- An ultra run's lanes need the owner's OK for the task's outward steps first.
- The state's `working` rows follow the `WorkRow` type; its document now describes the waiting
  group's fields as optional (a row that doesn't wait has none), which the rows the server now
  sends need.
- New configuration: `EMBERHUT_WORKER_MODEL`, `EMBERHUT_WORKER_MAX`, `EMBERHUT_ULTRA_PARALLEL`,
  `EMBERHUT_ULTRA_BUDGET`, `EMBERHUT_ULTRA_ASK_TOKENS`, `EMBERHUT_ULTRA_ASK_AGENTS`, `EMBERHUT_ULTRA_LANE_RUNNER`,
  `EMBERHUT_ALWAYS_MERGE`, `EMBERHUT_SEND_WITHOUT_OK`, `EMBERHUT_ASK_GUARD_MODEL`.
