# 0022. The side apps: parties, the off-the-record room and family members' rooms

- Status: Proposed
- Date: 2026-11-26
- Specified in: the routes' documents in `crates/emberhut-server/openapi.json` (`parties`, `dark`
  and `family` tags) and the types in `emberhut-api`'s `parties`, `dark` and `family` modules.

## Context

Besides the rooms and the assistant, the server has a few apps of their own:

- **Parties.** Two to five AI personas, each a name, a role and a model, talk about the
  person's topic turn by turn while the person watches it stream in. The person can jump in,
  steer, pause, resume and stop; each party has a turn cap and a token cap, its cost is counted
  per turn at list prices, and parties are kept to reopen. The mobile app's Party screens use
  it.
- **The off-the-record room.** A plain chat with a model that the assistant never sees: its
  own database, left out of backups, no room or chat row, no event, push, memory or tool, and
  nothing logged. A local model is tried first when one is set up, and a cloud model answers
  when it is off, saying so.
- **Family members' rooms.** A household server on the same machine (the one that pairs the
  family's devices) passes a family member's requests on under `/family/...`; the server asks
  it who the device key belongs to, and only a family member gets in, to rooms of their own,
  with their own theme and phones.

## Decision

### Parties

- **The rules are in `emberhut-core::party`**: what a new party is made of (2 to 5 personas with
  names of their own, ids `p1`…, colours, caps clamped to their bounds, a
  title from the topic), who speaks next (the persona the person's latest words name; else
  the first round in cast order; else a supervisor's pick among those who didn't just speak,
  which may end the talk after two rounds and never right after the person spoke; with one
  other persona and no end possible, they simply speak), the prompts, list prices and the
  caps a resumed party gets (10 more turns, half its token cap again).
- **The talk goes through a `ChatModel`** (emberhut-server `services::chat_model`): a system
  prompt and a conversation in, the answer streamed piece by piece, its text and token counts
  out, cancellable. The server's implementation, `OpenAiChat`, speaks the OpenAI-style chat
  completions API with server-sent events over the server's rustls client (no new crate), so
  one implementation reaches a local llama-server, a cloud gateway or a provider's
  compatibility endpoint, rather than one cloud provider's SDK: a self-hosted server should not
  be tied to one provider. The persona's model id is sent as it
  is (`claude-opus-5-5`, ...): the endpoint maps it. Configured with `--party-model-url`
  (`EMBERHUT_PARTY_MODEL_URL`) and an optional key file `party-model-key` in the secrets directory,
  read at every call. Without an endpoint a party starts and pauses at once, saying no model
  is set up.
- **One talk at a time per party, on its own task**; each streamed piece updates the party's
  `speaking` and goes to `GET /api/parties/{id}/live` (a snapshot first; a listener that falls
  behind gets a fresh one), and a turn is stored once it ends. Pausing lets the turn finish;
  stopping cuts it, keeping what was said. At start-up a party left running is paused, so
  nothing runs a bill up on its own after a restart, and a shutdown cuts the turn in flight.
- **Wire values**: the person's lines carry `who: "owner"` (`emberhut_core::party::HOST`).
  Prompts and the transcript call the person "the host".
- `parties` and `party_messages` are tables in the server's database; a persona list is kept
  as JSON in its party's row.

### The off-the-record room

- **Its own database file**, `dark.db` beside the server's, with one table, `dark_messages`,
  made by a migrator of its own (`emberhut-db`'s `DarkStore`), never by the server's. The backup
  copies the server's database and its data folders, so the room is left out without a rule
  of its own. The file keeps no write-ahead log and deletes securely (SQLite's
  `secure_delete`), and clearing the room ends with a `VACUUM`; that `VACUUM` is the one
  statement in `emberhut-db` outside `SeaORM`'s builders besides the backup's `VACUUM INTO`, and
  this ADR allows it. The file is opened the first time the room is used.
- **The models are `ChatModel`s too**: a cloud one (`--dark-model-url`, `--dark-model`,
  `--dark-model-label`; key file `dark-model-key`) and an optional local one tried first
  (`--dark-local-url`, `--dark-local-label`; key file `dark-local-key`), health-checked before
  each reply (llama-server's `/health`). When the local one is off, or fails before it has
  written a word, the cloud one answers and the reply says so at its top; that mark is kept
  with the reply and left out of what the model is sent. The local model gets the latest
  history that fits its context, with sampling suited to a small local model. The provider is
  start-up configuration.
- **Nothing of it is logged** but failures without their text, and the system prompt is
  generic, with no personal lines.
- A reply is written on its own task, so it is kept when the app goes away mid-answer. One
  reply at a time: a message or a clear meanwhile is 409.

### Family members' rooms

- **Not enabled until scoped tools exist.** The tools a family room would get (recall, facts,
  check-ins, the calendar, handing something to another room) act on the whole installation,
  and a family room's agent session would run with the owner's session settings. So
  `services::family::FAMILY_ROOMS` is off: every `/family/...` request is 501 `{ error:
  "family rooms are not available yet" }` before its caller or key is looked at (the document
  lists one refusal for `GET` and `POST /family/{route}`), no family room is made, a family
  room's session gets no tools (the MCP endpoint answers 501), and a reply in one pushes to no
  one. The design below is kept, compiled and tested in parts, for when family-scoped tools
  (a family member's own chat, the `home` facts read-only, their own check-ins, free/busy only,
  a note to Main marked as theirs) and a family session's own settings exist; the tests of the
  routes' behaviour are ignored until then.
- **Who a key belongs to is a `FamilyDirectory`'s answer.** `HttpFamilyDirectory` asks the
  household server's `GET /api/whoami` with the key as the bearer
  (`--family-directory-url`, `EMBERHUT_FAMILY_DIRECTORY_URL`, `http://` only, as the household
  server is on this machine) and keeps the answer a minute, by the key's SHA-256; only role
  `family` with a lowercase name gets in, and a household server that is down refuses without
  being remembered. Without one configured, no one gets in.
- **From this machine only**: a request whose peer is not a loopback address is 403. A request
  served in-process (the tests' router, with no peer) counts as local.
- **Their rooms are rows of `family_rooms`** (a room and the person it belongs to), each a
  plain room in `rooms`; the first request makes their `Home` room. `?room=` picks one of their
  rooms, any other is their Home room. Their state, history and live stream read only their
  room, through the same talk, live and presence services as the owner's.
- **Their theme** is the theme service under the key `theme:<person>`, apart from the owner's.
- **Their phones are rows of `family_devices`**, kept apart from the owner's push addresses so
  that the owner's alerts never reach them.

## Consequences

- The mobile app's Party screens, and any client of the off-the-record room, work for anyone
  who configures a model endpoint.
- Starting, steering, changing or deleting a party, and writing in or clearing the
  off-the-record room, need a signed-in device in either auth mode: they run a model at a
  cost or change the owner's data, and report mode's pass without a session only reads.
- Family rooms are off, so a household server's family members get a clear 501 until scoped
  tools exist. Once on, a family room's replies are pushed to that person's `family_devices`,
  never to the owner's phones; family rooms stay off the owner's room list; and a family
  member's room gets its own short tool set. The family state's `reminders` read the room's
  check-ins through `RoomReminders`, whose stand-in lists none.
- Parties and the off-the-record room depend on an outside model; their tests use a scripted
  one, and the OpenAI-style stream reader is tested on its own.

## Left out

Integrations specific to one deployment are left out; their generic parts belong to the
messaging, tasks and events areas.
