# 0009. The MCP endpoint, the settings actions and the state's sections: one registry each

- Status: Proposed
- Date: 2026-11-26

## Context

The server has three places that every feature area plugs into:

- **Its MCP endpoint** (`POST`, at the path `emberhut_api::wire::mcp::ROOM_PATH` names): the tools a room's agent session calls (about a
  hundred: tasks, check-ins, facts, messages, the browser...). It is the MCP Streamable HTTP
  transport, stateless: one JSON-RPC message per request, `initialize`, `tools/list` and
  `tools/call`, the caller let in by its room session's own token (401 `unknown caller`
  otherwise). Each tool has a name, a description, an argument schema it checks the call
  against (the checking library's messages reach the agent), and an answer of text or
  pictures; a call to a tool that isn't there, or with bad arguments, is a result with
  `isError`, not a JSON-RPC error.
- **`POST /api/settings`**: one route for the app's small changes. The body's fields name the
  actions (`ui`, `taskDecide` with `choice`, `pinFocus`, about 25 of them); several in one body
  run in a fixed order, fields no action takes are ignored, the answer is `{ ok: true }` with the
  whole state spread in, and a refused action is 400 `{ ok: false, error }`.
- **The state** (`GET /api/state`): one object with a key per area (`ui`, `theme`, `issues`,
  `working`, ...).

The server's areas are built in parallel, one part each. The three places must take an
area's addition as a small, additive change, so the parts merge without stepping on each other,
and the server's design (ADR 0002: constructor injection, typed errors, no SQL outside emberhut-db)
must hold in each.

## Decision

### The MCP endpoint

- **Route.** `POST` at `emberhut_api::paths::MCP_ROOM` (`/mcp/emberhut`: agent
  configurations name it) in `routes/mcp.rs`; the protocol in `crates/emberhut-server/src/mcp/`;
  the JSON-RPC and MCP types in `emberhut-api`'s `mcp` module, with the server's name
  (`emberhut_api::mcp::SERVER_NAME`, `emberhut`) and version `1`. The endpoint's wire words are
  kept in one place, `emberhut_api::wire::mcp`.
- **Answers are `application/json`.** The transport lets a server answer a request either as JSON or
  as a one-event SSE stream. A stateless server never has anything to stream, every MCP client must
  take both, and JSON is simpler to test and to read in a log, so this server answers JSON. A
  notification (and a client's answer) is 202 with no body. A body that is not JSON is 400 with the
  JSON-RPC parse error, one that is not a single JSON-RPC 2.0 message (a batch, a wrong `jsonrpc`,
  no `method`) is 400 with "Invalid Request", and a body over the server's limit is 413 with the
  same error: the server says no clearly, as ADR 0002 does for the app's routes.
- **Revisions.** `initialize` answers with the client's revision when it is one of
  `2025-11-25`, `2025-06-18`, `2025-03-26` or `2024-11-05`, and the newest otherwise, and
  announces `{ tools: { listChanged: true } }`. `ping` answers `{}`;
  any other method is the JSON-RPC error -32601.
- **Callers: a token per room, in memory.** `AgentTokens` hands out one token per room (256
  random bits, as the device tokens), the same each time it is asked until it is revoked or the
  server restarts; it keeps the tokens' SHA-256 and finds a presented one by its hash. The
  token names the room, and the tools act in it. The agent runtime asks for its room's token
  each time it starts a session and writes it into the session's MCP configuration, so keeping
  tokens across a restart buys nothing. A device's session token is not an agent's: it is 401
  here, and an agent token is not a device's.
- **Tools: a trait, and one list.** A tool implements `mcp::tools::Tool`: `name`,
  `description`, `call(&ToolContext, Args)`, and an `Args` type that derives serde's
  `Deserialize`, utoipa's `ToSchema` and garde's `Validate`. The JSON Schema the agent reads is
  derived from `Args` by utoipa, which the server already uses for its OpenAPI document, so a
  schema cannot drift from the code that reads the arguments; serde reads the arguments
  (unknown fields ignored) and garde checks them. An
  argument error reaches the agent as `MCP error -32602: Input validation error: Invalid
  arguments for tool <name>: <reason>`, where the reason is serde's or garde's. `ToolContext`
  carries the `AppState` and the caller's room, so a tool calls its area's service and holds no
  state of its own; the endpoint builds the list per request (`room_tools()`), which leaves room
  for per-room lists later (some rooms may get fewer tools). A tool's own
  refusal is an error result with its text; a failure underneath is logged with its causes and
  reaches the agent as its message.
- **Tool calls run inside the server's request timeout** (`EMBERHUT_REQUEST_TIMEOUT`). A tool that
  waits longer (waiting on a worker, a browser) must either
  answer early and report later, or come with its own decision about the timeout.
- **The first tool, `theme`,** reads the theme choice,
  or changes the parts given, through the theme service. It is the endpoint's working example
  and gives the assistant a way to change the apps' colours when asked to. Tool names are part
  of the wire contract: once shipped, a tool keeps its name.

### The settings actions

`POST /api/settings` reads its body as JSON and hands it to `crate::settings`. An action implements
`settings::Action`: the `key` (the body field that asks for it) and `apply(&ActionContext, &Body)`,
where the context carries the `AppState` and the body's room (read leniently, Main when it names
none or one that doesn't exist). The dispatcher runs every registered action whose key the body has,
in a fixed order, which `settings::ORDER` lists in full for every action, so an action's place in
the run doesn't depend on where its line is in the list. A field no action takes is ignored: there
is no "unknown action" error, so a client sending a field this server doesn't take yet gets `{ ok:
true }` and the state. An action reads its own field and any that go with it (`choice` with
`taskDecide`), does nothing for a field of the wrong kind, and refuses with `ActionError::Refused`
(400 `{ ok: false, error }`; the actions before it stay done). The `ui` action is the first.

### The state's sections

`StateService` takes a `Sections` value: the services the state's keys are read from besides
the rooms and the chat (the display and theme services today). A key is filled by adding its
service to `Sections`, its line where `AppState::new` makes it, and reading it in the `State`
that `StateService::state` builds, in place of its empty stand-in. The services stay injected
in the constructor, as ADR 0002 has it, rather than the state reaching into the whole
`AppState`.

### Settings, the theme and what the app hides

- The `settings` table (one JSON value per key, the contract's shape for each key),
  the activity log and the events table are emberhut-db stores; an area declares its keys once with
  their type (`SettingKey<T>`).
- The theme choice is the setting `theme` (`{ preset, appearance, accent, at }`); its colours are
  worked out by `emberhut-core`'s theme rules on each request. `POST /api/theme` reads a change
  leniently (a preset or an appearance in any case, with spaces around; an accent as `#RGB` or
  `#RRGGBB`, `null` or blank for the preset's own) and answers its refusals in its words; an empty
  change saves the choice again with a new `at`. The per-person themes of the family area
  (`/family/theme`) are that area's to add, on the same service with a key per person.
- What the app hides is the setting `ui` (`{ hideGoals, hideProjects }`), both hidden until
  changed. Each change is a line in the activity log.

## Consequences

- An area adds a tool, a settings action or a state key with a module of its own and one line
  in a list (two or three for a state key); docs/architecture.md says how. Two parts adding a
  line at the end of the same list meet in a trivial merge conflict, resolved by keeping both.
- A restart makes new agent tokens: a session started before it must be started again (the
  agent runtime does that at its next turn).
- Argument errors are worded by serde and garde; an agent reads them as text, and no caller
  matches on their wording.
- MCP answers are JSON rather than a one-event stream; a client that only takes the stream
  would not work, and none is known (the transport requires clients to take both).
