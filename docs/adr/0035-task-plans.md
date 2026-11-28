# 0035. Task plans: a task's steps, ticked by its agent as it goes

- Status: Proposed
- Date: 2026-11-28
- Builds on: [ADR 0015](0015-tasks-projects-and-plan.md) (tasks, their history and runs, one
  transaction per change), [ADR 0017](0017-runners-machines-and-accounts.md) (runner sessions)
  and [ADR 0018](0018-workers-ultra-and-guard-rails.md) (workers and ultra runs)

## Context

A task worked on by an agent (a worker, a runner session the assistant started, the assistant
itself) shows a status, a live state and its history, but not where the work is: which part is
done, which part runs now, what is stuck. Agents made up for it with progress pages of their own
(HTML files kept beside the work and attached to the task), which every agent wrote differently,
which went stale when the agent forgot them, and which the apps can only open, not read.

The apps need the same thing on every task, in a shape they can draw: the task list at a glance
(done of total, the step now, what blocks it, the estimate, when it last changed), the task's page
step by step.

## Decision

### The model

- **A plan is an ordered list of steps**, each with a title (one short line), a state (`todo`,
  `doing`, `done`, `blocked`, `skipped`), an optional note (what it came to, or what blocks it), an
  optional link (what it produced) and its times (started, finished, created, updated).
- **Steps nest one level.** A top-level step may have sub-steps; a sub-step has none. One level is
  enough to break a large step into its parts, keeps the list readable on a phone, and keeps the
  rules simple: a top-level step with sub-steps counts as its sub-steps in the summary, and there
  is no rolling state up through levels. Deeper structure is subtasks, which tasks already have.
- **Ids are stable.** A step's id is unique within its task and never given to another of its
  steps (the task keeps the last id it gave out), so an agent can name a step by id across turns.
- **The summary is a rule, not stored**: done and total (skipped steps in neither), the current
  step (the first `doing`, else the first `blocked`, else the first `todo`), the blocker (the first
  blocked step's note, else its title). The plan's estimate is the task's existing `eta`.
- **Writing the whole plan again keeps the ticks.** `plan_set` replaces the plan, but a step whose
  title matches one already there (spaces and case aside) keeps its id, state, note and times, so
  an agent can re-state its plan as the work changes shape without losing what was done.
- **States date the steps**: `doing` sets the start (the first time), `done` and `skipped` set the
  finish, reopening clears the finish.

The rules are pure, in `emberhut-core` (`task::plan`), and unit-tested there.

### Storage: a row per step

The steps are rows of their own (`plan_steps`: the task, the step's id, its parent's id, its
position, title, state, note, link and times; going with their task) rather than one JSON value on
the task. Rows let the state read the plans of every listed task in one query and let a change be
checked against the schema; a step's times and state are columns the apps' summary is computed
from, not text to parse. A change rewrites the task's rows in the same transaction as its history
line, which at a hundred steps at most costs nothing. The task gets three nullable columns: when
the plan last changed, the last step id given out, and when a turn last ended with no plan.
Existing tasks need nothing: no rows is an empty plan ("No plan yet"), and nothing is made up for
them.

### History, and how the apps learn of a change

Every change writes one line of the task's history, so its history reads as the work went:
`Plan set: 7 steps`, `Started: Run the tests`, `Ticked: Build the Mac app`,
`Blocked: Sign the build — needs the certificate`, `Skipped: …`, `Added step: …`. One call is one
line (a whole plan written is one line, not one per step), so the history stays readable.

The history line changes the task's `updated` and its last line, and the plan at a glance is on
every task the state lists, so the state's fingerprint (its `ETag`) changes: the apps' state poll,
kicked at the end of every turn by the live stream, sees the change, and an open task page fetches
the page again when its task's `updated` changes. No new channel was needed.

### Turns with no plan

When a turn of work on a task ends (a worker's session, a runner session's turn) and its plan is
still empty, the task records when, and the first such turn in a row writes `A turn ended with no
plan`. Writing a plan clears it. This makes a missing plan visible without nagging on every turn.

### Who writes a plan

The tools are `plan_set`, `plan_step` and `plan_read`. A worker writes its own task's plan; a
runner session writes the plan of the task it is linked to. Runner sessions share one token,
so each names itself with a request header (`x-emberhut-session`), which the server lists in the
runners' shared MCP configuration with the variable the runner sets per session; a session the
server does not know, or a value left unexpanded, is as no header (see how runner sessions reach
the tools, below). A session the assistant starts
for a task is linked to the task from its meta, and its turns are the task's runs, so the task
shows running while it works. The assistant's own sessions may write any task's plan.

### How runner sessions reach the tools

A runner gives each session's agent CLI the MCP servers `GET /api/runner/mcp` lists. That list
was only the operator's `--runner-mcp` file, so a runner session had no tools of the server's
unless the operator listed the room endpoint with a room token by hand, and a room token is made
at a room's session start and lost at a restart, so a file can't hold one for long. With the
flag on, the list now always carries the server's own entry (named `emberhut`, as the server
names itself to MCP clients):

- **Its own endpoint, `POST /mcp/runner`, and a token kind of its own.** The runner sessions'
  token is one random token per server process, made the first time the list hands it out and
  held in memory as its hash, like the rooms' and the workers' tokens. Only the list hands it out,
  and only a runner's token reaches the list. One path per kind of token is how the server
  already tells callers apart (rooms on `/mcp/emberhut`, workers on `/mcp/worker`): each route
  reads one token store and picks its tools from it alone, so a room token on the runner path, or
  this token on the room path, is an unknown caller, and the room endpoint's checks (whose agent,
  a family member's room) stay as they are.
- **A short list of tools**: `task` (one task in full, read only), `plan_set`, `plan_step` and
  `plan_read`. Nothing that sends a message, drives a browser, reads threads or reaches past
  tasks: the token is shared by every session on every runner.
- **Writes need a linked session.** The entry sends the session header with the variable the
  runner sets per session. A call whose header names a session linked to tasks writes only those
  tasks' plans (the task may be left out when it is one); a call with no header, a value left
  unexpanded, or a session the server doesn't know (or linked to no task) is refused writes, with
  a message naming the header, and reads any task's plan. A room token keeps its rights as
  before.
- **Who and where.** The history line of a change names the assistant, as for a runner session
  calling with a room token: a runner session works for the assistant. The calls act in Main,
  as every runner session is Main's.
- **The URL** is the one the server gives its own agent sessions: its listen address
  (`127.0.0.1` for one listening on every address) and the path. Where the runners reach the server
  at another address, the operator lists the entry in the file instead: a file entry for one of the
  server's endpoints (by its URL's path) or named `emberhut` is kept as written, gets the session
  header, and, pointing at `/mcp/runner` without an `Authorization`, gets the token.
- **After a restart** the token is new and the old one is an unknown caller (401). Runners fetch
  the list when they start and every ten minutes after, and give each new turn the list they last
  had, so a session works again from its first turn after the next fetch; a runner that can't
  reach the list keeps the one it had.

With the flag off the list is the file's alone, no token is made, and every call to
`/mcp/runner` is an unknown caller.

### A session's runs, and an end that was missed

A session is linked to its task when a turn of it is sent or starts (or the server first sees
it), not on every event. A run stays open until the session's turn ends, so an end the server
never heard (it was down, the session was deleted) would keep the task running for good. The
tasks' tick (every five minutes, and at start-up) closes such a run as `lost`, judged by what the
server already keeps of each session, the record the runners' events (and the coding-thread
app's listing) leave, with no new call to the runners: a record that says the session is not
working, or no record any more, ten minutes after it was last heard of; a record that still says
it works, after twelve hours with no event from it (a working session reports every few seconds,
and the record cannot be checked against anything fresher). A turn's end that comes late finds
its run closed already, and leaves it as it is.

### The flag

Plans are behind a server flag (`--task-plans`, `EMBERHUT_TASK_PLANS`, off by default). With it
off, no `plan` is sent, the tools are absent, the briefs don't mention plans, and runner sessions
are neither linked to their tasks nor their turns kept as runs (a session's `tasks` stays empty, as
before plans); with it on, the display settings say so (`ui.taskPlans`, the flag's and never a
saved setting's), so the apps can show tasks as the work running. Plans written while it was on
are kept when it goes off. The flag lets the server, the desktop app and the phone app move to
plans together.

## Consequences

- Agents are asked, in every brief, to keep the plan instead of making progress pages; the
  `STATUS` line a runner session ends its replies with is asked for and read as before.
- A plan is only as true as its agent keeps it; the unplanned-turn note and the plan's `updated`
  time show when it is not kept.
- Matching steps by title means renaming a step through `plan_set` makes it a new step; renaming
  through `plan_step` keeps it.
