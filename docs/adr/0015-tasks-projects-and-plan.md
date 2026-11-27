# 0015. Tasks, projects, areas and the plan: one transaction per change, rules in the core

- Status: Proposed
- Date: 2026-11-26

## Context

Every piece of work, the person's and the assistant's, is a task:
an owner, a status (`todo` → `working` → `waiting` on the person → `done`, or `canceled`, shown
as Archived), a project, an area of the project, a place in a tree of subtasks, a history, the
one decision it waits on (an ask with plain answers or cards to compare), what it produced
(files, links, X replies) and its runs (every turn of work on it). A scan's or a review's
results are filed under it as findings. Around the tasks sit the person's plan: goals ranked
by what matters, the watch list (Focus items), the open loops, the ultimate concern, and the
weekly report; and the board that ranks Today from the hard deadlines against the work left.

The apps read all of it from the state (`issues`, `today`, `projects`, `goals`, `todos`,
`weekStart`, `focus`, `areas`, `report`, `loops`, and `tasks`, the assistant's own background
jobs), search it (`GET /api/tasks`), open one task (`GET /api/tasks/{id}`), reply on a task's
page (`POST /api/tasks/{id}/message`), open what a task produced (`GET /api/artifacts/{id}`,
`/thumb`), list what is scheduled (`GET /api/scheduled`) and change it through
`POST /api/settings` (`pinFocus`, `dropFocus`, `parkFocus`, `todoDone`, `todoPark`,
`taskArchive`, `taskUnarchive`, `taskDecide`, `xPosted`, `taskCreate`, `projectSet`,
`taskSet`). The assistant works it through MCP tools (`tasks`, `task`, `task_create`,
`task_update`, `task_progress`, `task_note`, `task_ask`, `task_findings`, `task_done`,
`task_work`, `task_stop`, `projects`, `project_set`, `area_set`, `plan`, `goal_set`,
`goal_drop`, `focus_set`, `focus_drop`, `loop_set`, `close_loop`, `concern_set`). A program
on the server's machine hands the assistant a job with `POST /api/tasks` and a key.

ADR 0002 puts rules in `emberhut-core` with no I/O, SQL only in migrations, and one transaction for
anything that changes more than one row. A task change touches many rows: its subtree's area and
project, its parent's rollup (a parent is done when every subtask is), and the tasks whose ask only
pointed at it.

## Decision

### The model

- **`emberhut-core::task`** holds the rules: the statuses, owners and sizes; `settle_change`
  (an action makes a task wait and a cleared one sends it back to work, waiting without an
  action is refused, a count that reaches its total finishes the task, a finding's state gives
  its status); the ask's answers (`choices_of`: plain answers 4 at most, cards 6, one pick, its
  notes from each card's `then`), the action made from a question, the kind guessed from the
  answers, a draft's markdown taken out; the findings' words and the run lines that report them
  (`FINDING: <title> | <state> | <detail>`); the tree (`Family`: ancestors, descendants,
  height, a parent's rollup); the board (projects' health from their hard deadlines, each
  task's score, Today until a day's 4 focus hours are full); the areas' trees rolled up from
  their leaves and the outlines the `tasks` tool answers; a task's live state from its runs;
  searches by key or words; the Scheduled page's words.
- **`emberhut-db`** has seven tables: `issues` (a finding is a row of kind
  `finding`; `last_note` keeps the newest history line so a list reads no history),
  `issue_events`, `artifacts`, `projects`, `areas`, `links` and `runs`. Instants are integer
  milliseconds (ADR 0002); days are `YYYY-MM-DD` text, as the contract writes and sorts them.
  `TaskStore::update` applies a change and everything that follows from it, the parents'
  settling and the pointing asks included, in one `BEGIN IMMEDIATE` transaction; so does
  `create` (a finding filed, its scan settled) and `ProjectStore::set` (an archived project
  parks its open tasks that don't need the person, and bringing it back unparks exactly those).
- **The plan** (goals, Focus items, loops, the concern, the background jobs) is settings, one
  key each (`goals`, `focus`, `context.loops`, `concern`, `tasks`), so an imported archive
  reads as it was. The loops live in the
  `context` setting beside what other areas keep there, which stays as it is.
- **Wire words** (the owners `owner` and `assistant`, a worker's `worker`, the task-key prefix
  `T-`, so task 12 is `T-12`) are written once, in `emberhut_core::task::wire`; the tools'
  arguments use the same words (`owner_asked`, the owner enum).

### The routes and the time

- `GET /api/tasks` takes its query leniently: an unknown status
  is left out of the filter, `project=none` is tasks with no project, `limit` defaults to 50 and
  stops at 200. A key or a bare number comes first, then tasks with every word in their title,
  then the newest; `%` and `_` in a query are matched as themselves.
- `POST /api/tasks` takes only the server's program key as its bearer (a device's session is
  refused, 401 `programs use the key`). The key is read at start-up from the file
  `--program-key-file` / `EMBERHUT_PROGRAM_KEY_FILE` names; without one no program can make a task.
- An artifact is a file only when it is under the server user's home or a temporary folder,
  outside the private folders (the secrets directory, `.ssh`, `.gnupg`, `.aws`, `.config`), of
  a kind the app opens, and still there; it is served only by its id, in byte ranges (the
  chat's files' code). Thumbnails are made once by ffmpeg (`--ffmpeg` / `EMBERHUT_FFMPEG`, a fake
  in tests) and kept beside the database in `thumbs/`, a cache the backup leaves out.
- Days, the week and deadlines are the server's time zone's: the system's (`TZ` is honoured),
  UTC when none can be read. The week starts on Sunday.
- `GET /api/scheduled` lists what its sources schedule, soonest first. This server reads its
  own machine's systemd user timers; the areas that schedule things themselves (the assistant's
  watches, check-ins and routines, a calendar's meeting prep) add a `ScheduleSource` of their
  own when they land.
- A tick every 5 minutes brings paused projects back on their day and hands the assistant
  each ask that has waited 12 hours since it was last checked, once per check.

### Where other areas plug in

- **Workers** (a session doing one task) are another area. Tasks reach it through the
  `TaskWorkers` trait (start, stop; a server with none says so), and the workers' area calls
  `TaskService::worker_started` / `worker_ended`, which keep the task's runs, and reads the
  task's brief and `next` (what its worker carries on with: the person's answers and replies,
  a changed brief).
- **Worker tools.** A worker could have its own tool set, with its task's id implied. Here the
  tools are the room endpoint's (`task`, `task_note`, `task_ask`,
  `task_findings`, `task_done`) with the task's id given; a worker's endpoint can serve them
  with the id fixed when the workers' area brings one.
- **The assistant's background jobs** are kept by `PlanService::jobs_running` and `job_ended`,
  for the assistant's session to call as its jobs change; the state shows the running ones and
  the last 8 finished.
- **Keys in a history line** are masked as the line is written (`TaskService::note`), by the
  guard rails' rules (ADR 0018), before it is stored and shown.
- **Events** for the assistant (an answer, an archive, a reply on a task's page, a stale ask)
  go to Main, and a Scheduled item keeps its source's room (Main for this machine's timers)
  instead of its first task's project's: a project's own room comes with the rooms' area.

### Left out, with the reason

- What a task's page shows of other areas: runner sessions (`sessions`), an ultra run's plan
  and agents (`ultraRun`), the outward-step and go-ahead cards, the send-it card of a draft,
  and the people whose asks count as the owner's: they are those areas', and their fields stay
  empty or out until those areas land. `project_set`'s arguments for where a project's sessions
  and coding threads start belong with the runners and the coding-thread app.
- One installation's own Focus item for an outside ticket system, counted by code: there is no
  generic mechanism behind it.

## Consequences

- A change that cascades (a parent settling, a subtree moving) is atomic; a big tree is read
  whole (`Family`, seven columns of every task) once per change, which is cheap for one
  person's tasks.
- The state is built from the board on every poll; the board reads
  the listed tasks, their runs and artifacts, and every area's tree.
- The OpenAPI document describes an area's subtasks as objects (`kids`), not by a reference to
  the node's own schema, because the document's checking validator can't take a schema that
  refers to itself.
- New configuration: `EMBERHUT_PROGRAM_KEY_FILE`, `EMBERHUT_FFMPEG`; and `TZ` matters.
