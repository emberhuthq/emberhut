# The emberhut HTTP/SSE API

This is the contract between the server and its apps. The types are in `crates/emberhut-api`;
`crates/emberhut-client` is a client of them. A route marked **partly covered** reads more than
the contract models so far.

## Conventions

- **JSON.** Bodies are UTF-8 JSON (`Content-Type: application/json`). Field names are camelCase;
  ids are bare numbers (rooms, messages, tasks, devices, projects) or strings (machines, sessions,
  work rows). An empty request body reads as `{}`; a body that isn't JSON (or is `null` where an
  object is read) fails with 500.
- **Null and absent.** A field the server always writes is present, as `null` when it has no
  value. Some fields are written only when they have something (`event` on a chat message, `refs`,
  `reply`, a waiting work row's `need`...): each type's docs say which. Readers ignore fields they
  don't know.
- **Times** are text: an instant is JavaScript's `toISOString()` (`2026-11-24T21:57:53.410Z`, UTC,
  always milliseconds); a day is `2026-11-24`. They sort as text. A chat message's `time` is a
  relative label (`now`, `4m`, `3h`, `2d`) as of when the server answered.
- **Numbers.** A floating-point value that happens to be whole may be written `1` or `1.0`:
  both are one JSON number, and a reader takes either.
- **Compression.** A body over 16 KiB is gzipped when the request has `Accept-Encoding: gzip`
  (`Content-Encoding: gzip`, `Vary: Accept-Encoding`). The state is 1 to 2 MB of JSON, ~150 KB
  gzipped.
- **Rooms.** Main is room `1`. A `room` (query or body) that is missing, not a positive integer, or
  not one of the person's own rooms means Main. There is no error for an unknown room.

## Errors

| Status | Body | When |
|---|---|---|
| 400 | `{ "error": "..." }`, or `{ "ok": false, "error": "..." }` on routes that answer `ok` | a value the route refuses |
| 401 | `{ "error": "Sign in with Apple first.", "signIn": true }` | sign-in is enforced and the request has no valid session |
| 404 | `{ "error": "not found" }` | no such route |
| 405 | `{ "error": "GET or POST" }` (`/api/theme`) | wrong method |
| 429 | an `AuthReply` refusal (pairing) | too many wrong pairing codes |
| 500 | `{ "error": "<the exception's message>" }` | the server failed |

Pairing and renewal answer with their own shape (`AuthReply`, below) for every status. A client
tells "sign in" from "not let in" by `signIn` on a 401 (`emberhut_api::error::ErrorBody`).

## Authentication

**The gates, in order:**

1. **Pairing** (`/api/auth/pair/new`, `/api/auth/pair`) comes before the session gate: the device
   pairing has no session yet.
2. **Session gate.** A device session is an access token sent as `Authorization: Bearer <access>`
   (the `Bearer` prefix is matched case-insensitively). Media a player opens itself may pass it
   as `?token=<access>` instead. What happens without a valid one depends on the mode, shown in
   the state's `auth.mode`:
   - `report`: the request passes, and the server notes it (once an hour per address) as what
     enforcing would refuse. A client in report mode works signed out.
   - `enforce`: 401 with `signIn: true`.

**Tokens.** The access token lasts an hour (`expires` says when). The refresh token renews it and
**turns over on every use**: the old one never works again. A device's session ends when it is
signed out in Settings, or after 14 days unseen. A client must run **one renewal at a time** and,
when a renewal is refused, sign out only if the refresh token it holds is still the refused one
(overlapping renewals from the poll, the live stream and a post would send a used token, get
401, and wipe the session). A renewal's answer applies only to the session it renewed: a
sign-out or a new pairing made meanwhile stands. `emberhut-client` renews when under a minute is left.

### POST /api/auth/pair/new

Makes a one-time pairing code, on a signed-in device (its Bearer access token) or with the server's
key. No body.

- 200 `{ "ok": true, "code": "ABCD-EFGH", "expires": "<instant>" }` (`PairingCode`). The code is 8
  characters from `ABCDEFGHJKMNPQRSTUVWXYZ23456789` (no 0/O, 1/I/L), shown with a dash after 4. It
  works once, for 10 minutes; only its SHA-256 is stored.
- A server with a machine id adds `link`, the code's pairing link (below), which a signed-in
  device shows as text and as a QR code beside the code. A legacy host leaves it out.
- 401 `{ "ok": false, "error": "Only a signed-in device or the server's key can make a pairing code." }`
  (a bare report-mode request can't make one).

### POST /api/auth/pair

```json
{ "code": "ABCD-EFGH", "install": "zr-…", "name": "Emberhut on a laptop", "model": "linux" }
```

`PairRequest`. `code` is read without case, spaces or dashes (`abcd efgh` works). `name` is kept to
60 UTF-16 units (none: "A device"), `model` to 40, `install` to 64 (none: a new device row each
time). The same `install` pairing again replaces its device row, so its old refresh token stops.
Answer (`AuthReply`, the status repeated in the body):

| Status | Body |
|---|---|
| 200 | `{ "ok": true, "status": 200, "access", "refresh", "device", "expires" }` |
| 400 | `{ "ok": false, "status": 400, "error": "Type the pairing code." }` (no code, or not a string) |
| 401 | `"That code isn't right. Check it, or make a new one."`, `"That code was already used. Make a new one."`, `"That code has expired. Make a new one."` |
| 429 | `"Too many wrong codes. Try again in N minutes."`: after 5 wrong codes in 10 minutes, for every caller, the right code too, until the oldest is 10 minutes old (in memory: a restart clears it) |

`device` is the device's row id; `expires` the access token's expiry.

### POST /api/auth/refresh

```json
{ "refresh": "<refresh token>", "install": "zr-…" }
```

`RefreshRequest`; `install` is optional (a device that signed in before sending one gets it
recorded). 200 is the same `AuthReply` as pairing, with a **new** refresh token; 401
`{ "ok": false, "status": 401, "error": "Sign in again." }` when the token is unknown, used or
revoked. A 401 without `status` is not this answer: a client must not sign out on it.

### GET /api/auth/status

With or without a session, in either mode (`AuthStatus`): the state's `auth` and whether this
request's access token is a signed-in device's.

```json
{ "mode": "enforce", "bound": true, "devices": [ { "id": 3, "name": "…", "model": "linux", "created": "<instant>", "seen": "<instant>" } ], "signedIn": true }
```

The server sends `devices` only to a signed-in device (an empty list otherwise).

## Setup

What the server has set up, for the apps' onboarding. Before pairing, an app needs only
`GET /api/auth/status` (whether the server is in use: `bound`); the checklist says what the owner
has connected, so it is read with a session.

### GET /api/setup

Behind the session gate (`SetupPage`). Every item once, in the order the apps show it:

```json
{
  "items": [
    { "id": "agent", "state": "ready", "detail": "The assistant and its workers run on the agent CLI." },
    { "id": "devices", "state": "ready", "detail": "2 devices signed in." },
    { "id": "mac", "state": "off", "detail": "No Mac helper is set up (EMBERHUT_MAC_HELPER_URL)." }
  ]
}
```

- `state` is `ready` (set up, and working as far as the server can tell), `missing` (the owner
  can set it up from the apps: pair, sign in, save a token, log an account in), `off` (it takes
  the server's own configuration) or `error` (set up, but failing now).
- `detail` is a line for the owner: how many there are, or what to do. It may be left out.
- `id` and `state` are open words: a newer server may add items, and a client shows or skips
  the ones it doesn't know.

| `id` | `ready` | otherwise |
|---|---|---|
| `agent` | `EMBERHUT_AGENT_COMMAND` is set | `off` |
| `provider` | the agent runs on configured accounts, or on a saved provider it can run on (below) | `off` without an agent CLI, or for `openai`/`openai-compatible` on a server without the OpenAI bridge; `missing` with nothing saved, with `claude` saved and no account (only the CLI can tell whether it is signed in), or when the saved key's file is gone |
| `accounts` | a runner reports an enabled account that is logged in | `off` without a runner; `missing` with no account, or none logged in |
| `devices` | a device is signed in | `missing` |
| `push` | APNs or FCM is ready | `off` |
| `public-url` | `EMBERHUT_PUBLIC_URL` is set (`detail` is it) | `off` |
| `mac` | the Mac helper answers its health, within 5 s | `off` without a helper; `error` when it doesn't answer or refuses |
| `chrome` | a Chrome profile is paired (`detail` says how many are online) | `missing` |
| `gmail` | the server's Google OAuth client is saved and an account is signed in | `missing` without either; `error` when an account's last look failed |
| `slack` | a user token is saved for a workspace | `missing` |
| `whatsapp` | the daemon says the number is connected (`open`) | `off` without a daemon (`EMBERHUT_WHATSAPP_URL`); `error` when it reports an error; `missing` otherwise (not linked) |

Nothing is started to answer: no CLI is run, and only the Mac helper is asked anything. What the
server can't know isn't an item: whether the machine is linked to an account is the account
directory's to say (directory.md's `GET /v1/machines`), and whether the server's own agent CLI is
logged in only the CLI can tell (the runners report theirs).

## Agent provider

What the agent thinks with, chosen from a signed-in device so an owner never needs a terminal:
Claude Code (the agent CLI) on the owner's own Claude login, an Anthropic API key, another
provider Claude Code can reach through an Anthropic-compatible endpoint, or an `OpenAI`-style
endpoint through the server's OpenAI bridge. Every route takes a
signed-in device, in either auth mode (401 `signIn` otherwise), and no answer carries a key.

| `provider` | `apiKey` | `baseUrl` | `model` | the agent CLI gets |
|---|---|---|---|---|
| `claude` | – | – | optional | its own login |
| `anthropic` | needed (`sk-ant-…`) | – | optional | `ANTHROPIC_API_KEY` |
| `deepseek` | needed | optional (`https://api.deepseek.com`) | optional (`deepseek-v4-pro`, quick: `deepseek-v4-flash`) | `ANTHROPIC_BASE_URL=<base>/anthropic`, `ANTHROPIC_AUTH_TOKEN` |
| `openrouter` | needed | optional (`https://openrouter.ai/api`) | needed | `ANTHROPIC_BASE_URL=<base>`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_API_KEY` empty |
| `ollama` | – | optional (`http://localhost:11434`) | needed | `ANTHROPIC_BASE_URL=<base>`, `ANTHROPIC_AUTH_TOKEN=ollama` |
| `anthropic-compatible` | needed | needed | needed | `ANTHROPIC_BASE_URL=<base>`, `ANTHROPIC_AUTH_TOKEN` |
| `openai` | needed | optional (`https://api.openai.com/v1`) | needed | `ANTHROPIC_BASE_URL=<the bridge>`, `ANTHROPIC_AUTH_TOKEN=<a token of the launch's own>`, `ANTHROPIC_API_KEY` empty |
| `openai-compatible` | optional | needed (`https://…/v1`) | needed | as `openai` |

- **Models.** With a model (given, or the provider's default) the CLI's every model is mapped to
  it: `ANTHROPIC_MODEL`, `ANTHROPIC_DEFAULT_OPUS_MODEL`, `ANTHROPIC_DEFAULT_SONNET_MODEL`,
  `ANTHROPIC_DEFAULT_FABLE_MODEL` and `CLAUDE_CODE_SUBAGENT_MODEL` to `model`;
  `ANTHROPIC_DEFAULT_HAIKU_MODEL` and `ANTHROPIC_SMALL_FAST_MODEL` to `fastModel` (or `model`).
  The launch's `--model` (`EMBERHUT_AGENT_MODEL`, a task's model) is dropped then. Without one,
  `claude` and `anthropic` keep the configured model.
- **The OpenAI bridge.** `openai` and `openai-compatible` run through a bridge the server serves
  on loopback only (ADR 0029): it speaks Anthropic's Messages API to the CLI and sends each
  request on as `POST <base>/chat/completions` with the key as bearer. The CLI never gets the
  key: each launch gets a token for the bridge alone, revoked once the CLI has exited. Thinking,
  prompt caching and Anthropic's server tools are not available there; other models may use
  Claude Code's tools less well than Claude.
- **Only the saved provider.** Every launch of the CLI (a room's session, a configured agent's
  room, a worker, the guard's judge) gets these in its environment only, and none of the server's
  own `ANTHROPIC_*`, `CLAUDE_CODE_USE_BEDROCK`/`VERTEX`/`FOUNDRY` or `CLAUDE_CODE_SUBAGENT_MODEL`
  (nor `CLAUDE_CODE_OAUTH_TOKEN`, but for `claude`). With nothing saved the environment is left
  as it is. A session started on another provider is restarted on the new one at its next turn,
  on the same conversation.
- **Accounts first.** While `EMBERHUT_AGENT_ACCOUNTS` configures accounts (each a CLI login, with
  failover between them), the saved provider isn't used: `inUse` is `false`.
- **Errors elsewhere.** On another endpoint than Anthropic's, a turn's error fails the turn; it is
  never read as a Claude account's usage limit.
- **Runners** on other machines keep their own credentials: nothing saved here is sent to them.

### GET /api/agent/provider

What is saved (`AgentProvider`), without the key:

```json
{ "provider": "deepseek", "keySet": true, "keyHint": "x7Qa", "checkedAt": "<instant>", "usable": true, "inUse": true }
```

`provider` is left out when nothing is saved; `keyHint` is the key's last four characters;
`baseUrl`, `model` and `fastModel` are there when given; `checkedAt` is when the provider last
accepted what is saved (left out for `claude`, and for a key put in the secrets folder by hand);
`usable` is `true` for every saved provider (`openai` and `openai-compatible` run through the
bridge); `inUse` says whether the agent's sessions run on it now.

### PUT /api/agent/provider

`ProviderRequest`: `{ "provider": "openrouter", "apiKey": "sk-or-…", "model": "z-ai/glm-5", "fastModel": "z-ai/glm-5-air" }`.
The provider is checked first, as the test below does, and only what passes is saved: the key in
the secrets folder (`agent-<provider>-key`, the server's user's only), the rest in the settings.
One provider is saved at a time: the others' keys are removed. 200 is the `AgentProvider` saved.

| Status | When (nothing is saved) |
|---|---|
| 400 | the provider isn't known, or doesn't take what was sent: a key, base URL or model missing or malformed, the sentence says which |
| 422 | the provider refused the key, or is too old to run the agent (Ollama before 0.14) |
| 502 | the provider didn't answer within 15 s, or not with a model list |

### DELETE /api/agent/provider

Removes what is saved and every key: the agent CLI runs on its own login again. 200 is
`{ "keySet": false, "usable": false, "inUse": false }`, also when nothing was saved.

### POST /api/agent/provider/test

The same body and refusals as `PUT`, but nothing is saved and a model isn't needed: for choosing
a model, and for finding a local Ollama. 200 is `ProviderTest`:

```json
{ "checked": true, "models": [ { "id": "qwen3-coder:latest" }, { "id": "claude-opus-5-5", "name": "Claude Opus 5.5" } ] }
```

| `provider` | asked |
|---|---|
| `anthropic` | `GET <EMBERHUT_ANTHROPIC_URL>/v1/models` with `x-api-key` and `anthropic-version: 2023-06-01` |
| `anthropic-compatible` | `GET <base>/v1/models` with the key as bearer (a 404: no list, nothing refused) |
| `deepseek`, `openai`, `openai-compatible` | `GET <base>/models` with the key as bearer |
| `openrouter` | `GET <base>/v1/key` with the key as bearer, then `GET <base>/v1/models` |
| `ollama` | `GET <base>/api/version` (0.14 or later), then `GET <base>/api/tags` |
| `claude` | nothing (`checked: false`): only the CLI can check its own login |

## The state

### GET /api/state?room=

Everything the app shows, for the room it has open (`State`). The server builds it on each
request and fingerprints it:

- The answer has `ETag: "<base64url SHA-1 of the body>"`. A request with `If-None-Match` equal to
  it gets **304** with the `ETag` and no body. An `ETag` is only good for the same room.
- **`x-emberhut-looking: <room id>`**: send it while that room is on screen and the person is in
  front of the device (a phone app in the foreground; a desktop app frontmost with input in the last
  few minutes). The server marks the room looked at; for the next 10 seconds that room's replies
  don't push to the phone, since they are being read. It changes nothing in the answer.
- 401 `{ signIn: true }` in enforce mode without a session.

`chat`, `earlier`, `unread`, `busy` and `live` are the requested room's; the rest is everyone's.

| Field | Type | Notes |
|---|---|---|
| `room` | room id | the room this state is for |
| `rooms` | `RoomRow[]` | Main first, then by `lastAt`, then rooms with nothing said by name |
| `on` | bool | the assistant isn't paused |
| `busy` | bool | it is working in this room |
| `live` | `Live` or null | what it is doing in this room |
| `unread` | bool | the room has answers since it was last seen: `POST /api/seen` once shown |
| `pending` | int | events the assistant hasn't handled |
| `chat` | `ChatMessage[]` | the room's newest 60, oldest first |
| `earlier` | bool | older messages exist |
| `issues` | `Issue[]` | every task, open and this week's done (board fields `deadline`, `hardDeadline`, `score` added) |
| `working` | `WorkRow[]` | work in flight, waiting on the person first |
| `sessions` | `RunnerSession[]` | runner sessions, every room's: running and queued first, then the newest, 30 at least |
| `today` | task id[] | Today, ranked |
| `projects` | `Project[]` | with health from hard deadlines |
| `goals` | `Goal[]` | ranked; each with `at` and `week`: its steps done this week, counted by owner id |
| `todos` | `Todo[]` | each goal's list this week (open tasks, then this week's done ones): `id`, `goal`, `text`, `who`, `today`, `done`, `parkedUntil`, `at`; a goal's ring is its done of all |
| `areas` | `Area[]` | each project's areas: `status`, the `state` rolled up from its tasks, `counts`, `questions`, `findings` and its `tree` of tasks with their steps; a done area comes bare |
| `report` | `WeekReport` | the weekly report: `concern`, `weeks` (finished per week, this one last), `weekStarts`, `since`, `goals` with their projects, `done` |
| `theme` | `ThemeChoice` | the theme choice only: a newer `at` than the client's means `GET /api/theme` |
| `ui` | `{ hideGoals, hideProjects }` | screens to hide (by default both `true`) |
| `account` | `Account` | the assistant's account and its use (`used5h`, `used7d` 0 to 1); every field `null` until the first turn |
| `auth` | `{ mode, bound, devices[] }` | sign-in mode and the signed-in devices (`id`, `name`, `model`, `created`, `seen`; no tokens) |
| `builds` | `{ ios, mac }` | the newest build each store has ready (`null` where none was): an older app offers the update |
| `push` | `boolean` | the server can push alerts to phones: it has the keys of APNs, FCM or both |
| `threads` | `CodingThread[]` | every coding thread the server knows (runner sessions and the coding-thread app's), helpers' left out: the newest 40, most recently changed first |
| `followed` | `FollowedThread[]` | the threads the assistant handed work to and follows (`id`, `machine`, `why`, `at`, `thread`: its latest look or `null`), the longest followed first |
| `t3Running` | `ExternalThread[]` | the coding-thread app's threads still `running`, `queued` or `waiting`, newest first, 30 at most; Main lists every room's |
| `tasks` | `BackgroundTask[]` | the assistant's background work, the running ones first, then the last few that ended; `[]` when left out (not the person's tasks: those are `issues`) |
| `chrome` | `ChromeProfile[]` | the paired Chrome profiles: name, label, account, machine, extension and Chrome versions, online, the assistant's grant, the user's pause, who drives it |
| `chromePairing` | `ChromePairing[]` | pairing links of the last while: `code`, `link`, `label`, `device` (a re-pair), `until`, `paired` |
| `chromeLatest` | string | the extension's newest version (`0` when unknown) |
| `browser` | `BrowserInfo` | the assistant's own browser: `connected`, the page's `url` and `title`, `driver`, `task`, `control` (`agent` or `owner`, the person), `handoff` (why it was handed over), `at`, `tabs[]` (`id`, `url`, `title`, `active`), `chromeUI`, `device` (the device that drives; `""` handed over and not taken yet), `fileRequest` (`{ multiple, at }`: the page asks for a file) |
| `checkins` | `Checkin[]` | the assistant's check-ins, soonest first: `{ at, why, room? }` (`room` left out for Main) |
| `watches` | `Watch[]` | its watches by number: `{ id, title?, thread, what, every, until, nextAt, note, noteAt, fails, room?, checkAt? }` |
| `did` | `DidLine[]` | the activity log's newest 40 lines, newest first: `{ time, at, text }` |
| `awareness` | `AwarenessSource[]` | each source the assistant reads: `{ id, name, detail, at, error, state }`, `state` `ok`, `stale` or `down` |
| `mySessions` | `RunnerSession[]` | the person's own runner sessions, every room's, newest first, 40 at most |
| `machines` | `{ [machine]: string \| null }` | each coding-thread app machine's and runner's error at the last look (`null`: it answered) |
| `accounts` | `AccountRow[]` | every account the runners use, by machine, with its use (0 to 100) |
| `usage` | `UsageRow[]` | the Usage card: one row per account any work may run on, names and percentages only, `best` the roomiest |
| `assistantAccount` | `AssistantAccount` | the account the assistant runs on (`on`), the person's `pin`, `mode` (`auto`, `pinned`), `pinOut`, `rooms` |

**Rows** (each type's docs in `emberhut-api` list every field):

- `RoomRow`: `id`, `name`, `project` (null for Main), `goal`, `active`, `status` (`active`,
  `paused`, `archived`, `done`; Main `active`), `pausedUntil`, `unread` (0 while paused or
  archived), `needs`, `ask`, `askId`, `busy`, `doing`, `last` (one line; `Sent a file` for a file
  alone), `lastAt`, `lastMine`. The switcher shows Main and the active rooms, folds the paused and
  done ones, and lists archived ones apart (`emberhut_api::rooms::Switcher`).
- `ChatMessage`: `id`, `mine`, `event` (only when true: the server's own line), `text` (Markdown),
  `time`, `at`, `files` (always an array), `reply` (only on a reply: `{ id, mine, at, text, file }`,
  the text cut to ~160 characters, `file` its first non-`file` attachment or null), `refs` (only
  when there are some: `{ kind, label, task?, title?, url?, name? }`, kind `task`, `pr`, `report`,
  `card`, `ticket` or `file`).
- `ChatFile`: `{ name, url, kind, path }`; `kind` is `image`, `video` or `file` for uploads; `url`
  is `/uploads/<name>` behind the same sign-in.
- `Issue`: the fields the desktop reads; `status` `todo`, `working`, `waiting`, `done`,
  `canceled`; `requested` (`{ at, msg, room }`, the person's message that asked for it) and
  `requestedBy` (`{ who, at, how }`, someone else's ask), each the task's own or else its
  nearest parent's; `send` (`{ state, at, why?, via, to }`, the person's last tap on its send
  card); `decision` (only its given fields: `question`, `options`, `draft`, `kind`, `brief`,
  `notes`, `recommend`, `context`, `asked`, `checked`, and `choices`, `chat`, not modelled).
- `WorkRow`: `state` `waiting`, `running`, `stuck`, `limit`, `stopped`, `queued`, `stalled`,
  `scheduled`. A waiting row adds `need` (`approve`, `choose`, `do`), `asked`, `checked` (written
  together, `null` included), and on a task row `decideBy` (may be null) and `hardBy`; `paused`
  only when true; `area` only when set (`emberhut_api::work::Waiting`). A task row also has `thumb`
  (its newest picture's thumbnail) and `reply` (an `x_reply` artifact whose `x` is an X post with
  the replies drafted for it, `emberhut_api::x_reply`), each `null` when it has none.
- `BackgroundTask`: `id` (a string, as the assistant's tools gave it), `description` (one line),
  `type` (its kind as the tools name it: a shell command, an agent), `status`,
  `startedAt`, `endedAt` and `summary` (each may be null), and `room` (the room it was started
  from; left out for Main by older servers). `status` is `running` while it runs, and the tools'
  own word once it ended (`completed`, say): a client takes `running` as running and shows any
  other word as it came.

Words of these vocabularies may grow: a client keeps a word it doesn't know (`Other` in Rust).

### GET /api/history?room=&before=&after=

A page of a room's chat, `{ "chat": ChatMessage[] }`, oldest first, at most 60: the **newest** 60
of the range. `before=<message id>`: older than that message (for "Show earlier"; fewer than 60
means the start). `after=<message id>`: newer than it (to catch up). Neither: the newest page. A
full page after `after` may leave a gap behind it, so a client joins pages into what it holds by
the rule in `emberhut_api::chat::join`.

## The live stream

### GET /api/live?room=

`text/event-stream` (`Cache-Control: no-cache`, `X-Accel-Buffering: no`), open until the client
leaves. Each event is one `data:` line and a blank line; no `event:` or `id:` fields:

```text
data: {"text":"","doing":"Reading your new messages"}

: ping

data: null

```

- The first event, at once, is the room as it is now.
- `data: {"text": "...", "doing": "..."}` (`LiveEvent::Working`): the assistant works; `doing` is
  its current step (null right at the start). `text` is `""`: only the final answer reaches the
  chat.
- `data: null` (`LiveEvent::Idle`): nothing is being written. Its turn has ended and the answer is
  in the chat: fetch the state.
- `: ping` every 15 s keeps proxies from closing a quiet stream. A client's read timeout should be
  longer than that (`emberhut-client`: 40 s).

Only that room's changes are sent. Auth is the session gate's, like any route.

## Actions

### POST /api/talk

```json
{ "text": "Can you look at the example?", "files": [ChatFile], "replyTo": 39, "room": 2 }
```

`TalkRequest`. `text` may be empty when files come with it. `files` must be ones
`POST /api/uploads` stored (the whole object sent back, `path` included); the server drops any
whose `path` isn't in its uploads folder (the Rust server also drops a file it didn't note
storing; [ADR 0010](adr/0010-chat-files-history-and-health.md)). The Rust server refuses a blank
text with no file it took with 400 `Say something.`. `replyTo` (optional) quotes that message
when it exists.
200 `{ "ok": true, "started": true }` at once (`TalkReply`); the message is in the chat and the
reply comes with the next polls and on the live stream. A body that isn't an object fails with
500.

### POST /api/uploads?name=

Stores a file for the chat. The body is the **raw file** (not JSON, not a form;
`Content-Type: application/octet-stream`, which the server doesn't read) with its
`Content-Length`; `name` (percent-encoded, `UploadQuery`) is the file's name with its extension.
A client uploads each file as it is attached, then sends the answers' `file`s with
`POST /api/talk`.

- 200 `{ "ok": true, "file": { "name", "url", "kind", "path" } }` (`UploadReply`): `name` as
  given (no `name` in the query: `file`); `url` `/uploads/<stored name>`, the stored name being
  8 random characters, `-`, and the name's last 120 characters once each run of anything but
  ASCII letters and digits, `_ . -` and spaces has become one `_` (`file` when nothing is left); `kind` from the extension (`png jpg jpeg gif heic webp`: `image`;
  `mov mp4 m4v`: `video`; anything else `file`); `path` where it is on the server.
- 400 `{ "ok": false, "error": "file is larger than 500 MB" }` when `Content-Length` is over
  500 MiB (`emberhut_api::uploads::MAX_SIZE`, 524 288 000 bytes); the body isn't read then. Any other
  failure to store it is 400 with its message too.

### GET /uploads/<stored name>

The stored file, behind the same sign-in (`?token=` works for players), `Content-Type` from its
extension (`application/octet-stream` for one the server doesn't know), with byte ranges
(`Range: bytes=…`: 206, or 416 past the end). An HTML file comes with a sandboxing
`Content-Security-Policy`. A name with `..`, or no such file: 404 with no body. Clients fetch it
by the `url` of a `ChatFile` (`emberhut_client::Client::file`).

### POST /api/seen

`{ "room": 2 }` → 200 `{ "ok": true }`. The room's messages up to now are seen: its `unread` clears.

### POST /api/settings

Every small change the user makes, one route for all of them: the body is one object of
optional keys and the server does each action whose key it finds. A client sends one action at
a time (`emberhut_api::settings::SettingsAction`, written as the wire's `SettingsRequest`):

| Keys | Action |
| --- | --- |
| `{ "on": false }` | pause (`false`) or resume (`true`) the assistant; paused, it wakes for nothing and stops its turn, and still answers in the chat |
| `{ "interrupt": true, "room": 2 }` | stop the turn in that room |
| `{ "cancelCheckin": "<its time>" }`, `{ "stopWatch": 4 }`, `{ "stopTask": "b1a2" }` | cancel a check-in, stop a watch, stop a background job |
| `{ "todoDone": 12, "done": false }`, `{ "todoPark": 12, "until": "<time>" }` | tick (or untick) a step, park it (no `until`: back now) |
| `{ "taskArchive": 12 }`, `{ "taskUnarchive": 12 }`, `{ "taskStop": 12 }` | archive a task, bring it back, stop its worker |
| `{ "taskDecide": 12, "choice": "Report A" }` | answer an ask (`AnswerRequest`) |
| `{ "taskCreate": "Title", "goal": 3, "project": 2 }` | add a task (blank titles are ignored) |
| `{ "taskSet": { "id": 12, ... } }` | change a task: `deadline`, `hard`, `size`, `project`, `body`, `ultra`, `decideBy`; a key left out keeps its value, `null` clears it |
| `{ "projectSet": { "id": 2, ... } }` | change a project: `priority` (0 to 4), `deadline`, `hard`, `status`, `pausedUntil` |
| `{ "factSet": { "id", "category", "title", "text" } }` | correct a fact; its source becomes `you corrected it, <day>` and the assistant hears of it |
| `{ "personSet": { "id": "people/…", "important": true } }` | whether that person's messages wake the assistant at once |
| `{ "factDrop": "work/…" }`, `{ "trialDrop": 252 }`, `{ "trialKeep": 252 }` | forget a fact, drop a rule on trial, keep one now as a standing rule (before the weekly review) |
| `{ "xPosted": 40, "option": 0 }` | one of a post's drafted replies was posted (its task is done) |
| `{ "authRevoke": 3 }` | sign a device out |
| `{ "ui": { "hideGoals": false } }`, `{ "build": { "platform": "ios", "number": 12 } }` | what the app hides, a build a store has ready (below) |

200 is `{ "ok": true, ...the whole state of body.room (Main by default) }`; a change that can't be
made (no such task, project or fact) is 400 `{ "ok": false, "error": "…" }`. Pausing or
resuming, stopping a turn, a check-in, a watch or a job, and keeping or dropping a rule that
isn't on trial answer 200 whatever came of them; a server older than `trialKeep` answers 200 to
it too and leaves the rule on trial, so a client reads `GET /api/facts` after to tell. An answer to an ask:
`choice` is one of the decision's `options` (or `Done` for an action). `Leave it`, `Not now` and
`No` (any case) cancel the task; other answers carry it on, and the work resumes at once. It
can't be taken back, so a client holds an answer a few seconds for an Undo before sending it.

Several fields in one body run in a fixed order; a field no action takes is left
alone (still 200). A body that isn't JSON is 400 `{ "error": "..." }`; an action that refuses, or
fails underneath, is 400 `{ "ok": false, "error": "..." }`, the ones before it done.
**Partly covered:** the server also reads `testLimit`; the contract leaves it out.

### POST /api/settings: what the app hides

```json
{ "ui": { "hideGoals": false } }
```

The switches given (`hideGoals`, `hideProjects`) change, the others stay; a switch that isn't
`true` or `false`, or a `ui` that isn't an object, changes nothing. Both start hidden. 200 is
`{ "ok": true, ...the state }`, whose `ui` has the switches as they are now.

### POST /api/settings: build

The release step reports a build a store has ready:

```json
{ "build": { "platform": "ios", "number": 12 } }
```

`SettingsRequest`'s `build` (`BuildReport`). `platform` is `ios` or `mac`, `number` a whole
number from 1. A platform's number only moves up (a lower one changes nothing), and the
state's `builds` carries it. 200 is
`{ "ok": true, ...the whole state of body.room (Main by default) }`; 400
`{ "ok": false, "error" }` for a `build` object that is not a platform and a number.

### POST /api/settings: a project's status

```json
{ "projectSet": { "id": 2, "status": "paused", "pausedUntil": "2026-12-01" } }
```

`ProjectSetRequest`. `id` is the project; only the fields given change. `status` is `active`,
`paused` or `archived`; any status but `paused` clears the day it was paused until.
`pausedUntil` (`YYYY-MM-DD`), with `paused`, is the day it comes back by itself. A paused
project's room folds away in the switcher until that day; an archived one leaves the switcher
and its open tasks are parked; `active` brings it back (an archived project's tasks with it).
200 is `{ "ok": true, ...the whole state }`; 400 `{ "ok": false, "error": "<reason>" }` for a
project that doesn't exist.

### POST /api/settings: sign a device out

```json
{ "authRevoke": 7 }
```

The device's tokens stop working at once (it pairs again to come back), it leaves the
state's `auth.devices`, and the push addresses it registered are forgotten. A value that isn't
a number, or an id no device has, changes nothing. 200 `{ "ok": true, ...the state }`. Only a
signed-in device may sign one out, in either auth mode: without a session it is 401
`{ "error": "Sign in first.", "signIn": true }`, report mode included. A client that signs out
its own device forgets its session at once rather than wait for its next renewal to be refused
(`RevokeRequest`).

### GET /api/switches, POST /api/switches

The MCP servers and skills the assistant's sessions have, each with the user's switch
(`SwitchPage`): `{ at, account, live, mcp: [{ name, state, error, source, kind, on, fixed,
note }], skills: [{ name, description, source, on }], note }`. A server's `state` is
`connected`, `failed`, `needs-auth`, `pending`, `disabled` (off in the session's own settings),
`off` (off here) or `unknown`; `fixed` is the assistant's own server, always on. A skill's
`source` is `yours`, `plugin` or `built in`.

`POST` takes `{ "kind": "mcp" | "skill", "name", "on" }` or `{ "kind": "reconnect", "name" }`
(`SwitchAction`) and answers with the page as it is after. A request it can't do (no name, the
assistant's own server, reconnecting one that is off) is 400 `{ "error": "…" }`; a server
without these routes answers 404.

### GET /api/facts

What the assistant knows about the user (`FactsPage`): `facts` (each `{ id, category, title,
text, source, updated }`, a person's with `now`, `nowAt`, `chats`, `important`, some with
`project`), the standing rules `lessons` (`{ id, text, at }`), and the rules on trial `trials`
(`{ id, text, at, ends, said, lesson? }`: a correction's lesson, followed for 14 days until a
weekly review keeps or drops it; `lesson` is the same lesson as one short line to the user, in
the second person, when the assistant gave one). Categories: `people`, `work`, `money`, `preferences`, `health`,
`home`, `projects`, `proof`. Fixes go through `POST /api/settings`.

### POST /api/chrome

The user's Chrome profiles (`ChromeAction`): `{ "kind": "add", "label" }` (a link for a new
profile), `{ "kind": "repair", "id" }`, `{ "kind": "rename", "id", "label" }`,
`{ "kind": "unpair", "id" }`, `{ "kind": "cancel", "code" }` (a link not used yet),
`{ "kind": "pause", "id", "minutes"? }` (1 to 10, 10 when left out or 0; `"stop"` is read
as `"pause"`) and `{ "kind": "handback", "id" }`. A name (`label`) is 1 to 24 characters once
its runs of whitespace are one space, with a letter or digit for its short name. 200 is
`{ "ok": true, "chrome", "chromePairing", "chromeLatest", "pairing"? }` (`ChromeReply`: the
new link for `add` and `repair`); a request it can't do (no such profile, one that isn't
connected, a name taken) is 409 `{ "ok": false, "error": "…" }`.

### GET /api/browser/session?device=

The live view of the assistant's browser for this device (`BrowserSessionReply`):
`{ "ok": true, "url", "user", "password", "mode", "screen" }`. `mode` is `control` while this
device drives (the state's `browser.control` is `owner` and its `device` is this one or `null`)
and `view` otherwise; the stream checks the password, so a device asks again whenever control
changes. `screen` is the server's screen size (`{ width, height }`, `null` when unknown), for a
viewer while nobody drives. A machine without the live view is 503 `{ "ok": false, "error" }`.

### POST /api/browser/input

`BrowserInput`: `{ "kind": "takeover", "device" }` (the agents wait), `{ "kind": "handback" }`
(wakes the assistant), and while the person drives: `back`, `forward`, `reload`,
`{ "kind": "go", "url" }` (a site's name becomes `https://…`, other words a search),
`{ "kind": "tab", "id" }`, `newtab`, `{ "kind": "closetab", "id" }` and
`{ "kind": "chrome", "show" }` (the browser's own controls). 200 is `{ "ok": true, …browser }`
(`BrowserReply`); anything but `takeover` or `handback` before taking over is 409.

### POST /api/browser/upload?name=

The raw file as the body, into the page's file field once it asks for one (the state's
`browser.fileRequest`). 200 is `{ "ok": true, …browser }`; with no field asking, 409.

### GET /api/browser/frame.jpg

The latest picture of the page (JPEG, a few seconds old at most), for a card. 404 while the
person drives (a password may be on screen) or before the first picture.

### POST /api/settings: the assistant

```json
{ "on": false }
{ "interrupt": true, "room": 2 }
{ "cancelCheckin": "2026-11-26T10:30:00.000Z" }
{ "stopWatch": 3 }
```

- `on` (`true` or `false`): pause or resume the assistant. Paused, the turn running in every
  room stops and nothing wakes it (check-ins and watches wait); the state's `on` says which.
- `interrupt: true`: the app's stop button. The turn running in `room` (Main by default) stops
  and the chat says `Stopped.`; no reply comes for it.
- `testLimit: true`: `room`'s next answer counts as its account running out, so its session
  moves to the next account (a test of the failover; ADR 0014).
- `cancelCheckin` (a check-in's `at`, as the state gives it) and `stopWatch` (a watch's `id`):
  the app's cancel buttons.

A value of another kind changes nothing. 200 `{ "ok": true, ...the state }`.

### POST /api/events

```json
{ "text": "The nightly backup finished", "kind": "backup", "project": "Atlas" }
```

A program (a script when a long job ends, a webhook) tells the assistant something with a paired
device's token or, from the server's machine, the server's program key as the bearer
(`EMBERHUT_PROGRAM_KEY_FILE`), in either auth mode (`EventRequest`). It waits in the room `project`
names until the wake rule wakes that room: a room's name (`"Atlas"`), or a number (`4` or `"4"`):
the room of the project with that id, else the room with that id; Main when none is given or none
matches (a family member's room never does). The wake rule wakes it at once for the assistant's own
appointments and urgent kinds, else after a 90-second burst or ten events, at most twenty wakes
an hour for every room; the turn reads `Event: - [kind] text`, the oldest ten waiting events at
most (the rest wait for the room's next wake). `text` is 1 to 100 000 characters (a number is
read as its digits); `kind` is at most 32 of `a-z`, `0-9` and `_` (it is written into the
`- [kind]` line, so nothing else may forge one), `note` when left out.
200 `{ "ok": true }`; 400 `{ "ok": false, "error": "text is required" }` without a text, 400
`{ "error": "<field>: <rule>" }` for a text over the limit or another kind; 401 with neither a
session nor the program key.

## Push

### POST /api/devices

Where the server pushes this phone's alerts (`PushRegistration`), for a signed-in device in
either auth mode (401 `signIn` without a session, report mode included):
an iPhone's `{ "token": "<64 to 200 hex digits>", "env": "production" }` (`env` left out, or
anything else, is the sandbox: a development build), or an Android phone's
`{ "token": "<FCM registration token>", "platform": "android" }`. The same token again updates
it. 200 `{ "ok": true }`; 400 `{ "error": "not a device token" }` or
`{ "error": "not an FCM registration token" }`. The address belongs to the device that
registered it: signing that device out (`authRevoke`) forgets it.

Each of the assistant's replies then reaches every registered phone, unless its room was on screen
within the last 10 s (`x-emberhut-looking` on `GET /api/state`). The alert is titled `Emberhut`
(`Emberhut · <room>` outside Main), its text the reply as one plain line, at most 240 UTF-16 units;
a tap opens `emberhut://chat?room=<id>`. iPhones get `link` and `room` beside `aps` (whose
`thread-id` is `emberhut-<id>`); Android phones get `link`, `thread` and `room` (strings) in `data`,
on the notification channel `emberhut`. The state's `push` is `true` when the server has the keys of
APNs, FCM or both.

## Secret links

A one-time page where the person types secrets the assistant asked for (the `secret_link` tool
makes the link), so they land in the server's secrets directory and never in a chat. The link
is the key, so neither route needs a session; it works once, for 24 hours. Both answer HTML
(`text/html; charset=utf-8`, `Cache-Control: no-store`, `Referrer-Policy: no-referrer`).

### GET /s/{token}

200: a form with one password field per secret (named as the secret) and the line saying why.
404: the link expired, was used, or never existed.

### POST /s/{token}

The form, `application/x-www-form-urlencoded`. Each of the link's secrets whose field isn't
blank is saved, trimmed, to its file, and the link is used up (even when nothing was filled in).
200 says what was saved; 404 as above. Main gets an event line naming what was saved, never the
values.

## Runners

A runner is a program on one of the person's machines that runs coding-agent sessions (ADR
0017). The server reaches each one's API at `--runner NAME=URL`; each runner has its own token
(a `NAME=TOKEN` line of `--runner-tokens`), which the server sends it and it presents back.

The routes below that act (opening a session, a reply, stop, interrupt and settings, a
worker's reply and stop, stopping a job, pinning the assistant's account, a switch), and
`POST /api/settings`'s `{ "stopTask": "<id>" }` (stop one of the assistant's own background
tasks), need a signed-in device in either auth mode: without a session they are 401 `{ "error": "Sign in
first.", "signIn": true }`, report mode included. Reading, and "Check now"
(`POST /api/usage/check`, `POST /api/environments/check`), passes the session gate as
anything else does. A session's id is `r-` and lowercase hex digits; the server puts no other
id into a runner's path, from a route or from the assistant's tools.

### POST /api/runner/events

A runner's event about one of its sessions: `{ machine, kind, at, session: { id, title,
project, status, account, cwd, branch, model, effort, meta: { direct?, task? } }, turn?: { id,
by, prompt, started, outcome, last }, doing?, text?, to?, from?, why?, resumeAt? }`. `kind` is
`turn_queued`, `turn_started`, `activity`, `turn_finished`, `question`, `note`, `account` or
another the server takes as a plain update. The session's thread gets its new look; taking an
event twice changes nothing. 200 `{ ok: true }`; 400 `{ error }` when it has no session id or
names another runner (`not a runner event (<machine>)`); 401 `{ error: "runners use their
token" }` without a runner's token (a device's session is not one).

### POST /api/runner/machine

A runner's sample of its machine, every 20 s: `{ machine, host, platform, cpu: { cores,
load1, load5, usage }, memory: { total, available, used, swapTotal, swapUsed, pressure },
gpus, disks, battery, runner: { sessions, queued, open, claude, claudeRss, rss, cgroup },
sessions: [{ id, title, status, running, account }] }`. 200 `{ ok: true }`; 400 when it isn't
an object or names another runner; 401 as above.

### GET /api/runner/mcp

The MCP servers the runners' sessions get too: `{ mcpServers: { name: { ... } } }`, the
`mcpServers` of the JSON file `--runner-mcp` (none without one). 401 as above.

### GET /api/runner/sessions?task=&room=&direct

`{ sessions: RunnerSession[] }`, newest first, 50 at most: a task's (none is linked yet), a
room's (Main: every room's), or the person's own (`direct`).

### POST /api/runner/sessions

One of the person's own sessions: `{ machine, cwd, text, files, account?, model?, effort?,
worktree?, title? }` (`machine` `runner` by default). 200 `{ ok: true, id, turn, session }`, the
runner's session; 409 `{ ok: false, error }` for a machine that isn't a runner (`no runner
called <machine>`), nothing to start with (`say something to start it with`), or the runner's
refusal.

### GET /api/runner/folders?machine=&path= and GET /api/runner/branches?machine=&cwd=

The runner's listing passed through: where to start picking a folder (`{ machine, home,
recent, documents, projects }`, `projects` the projects' folders there, none yet), a folder's
folders (`{ machine, home, path, git, folders }`), or a repository's branches (`{ repo,
current, branches }`). 400 `{ ok: false, error: "no such runner" }`; 502 `{ ok: false, error }`
with the runner's error.

### GET /api/runner/sessions/r-{id}/transcript?after=&limit=

The runner's `{ session, events, earlier }`, the session with `place`, `accountName`, `tasks`,
`doing` and `project` added. 404 (no body) for an id that isn't one; 502 `{ ok: false, error }`.

### GET /api/runner/sessions/r-{id}/live?after=

The runner's own event stream for the session, passed through as it comes (`head`, `step`,
`delta`, `think`, `tokens`). 502 `{ ok: false, error: "the runner on <machine> didn't answer" }`.

### POST /api/runner/sessions/r-{id}/send, /stop, /interrupt, /settings

The person's reply (`{ text, files }`, its files put in the session's inbox first; 400 `{
error: "nothing to send" }`), stopping the session and its queued turns, stopping its turn, or
`{ model?, effort?, title? }`. 200 `{ ok: true, ...the runner's answer }` (`settings`: `{ ok,
session }`); 502 `{ ok: false, error }`.

### The sessions w-{task}

A worker of the assistant's, watched the same way: `GET .../w-{task}/transcript` (`{ session,
events, earlier }`, 400 steps by default, 1000 at most), `GET .../w-{task}/live` (`step` and
`head` events, looked at every second), `POST .../w-{task}/send` (409 `{ ok: false, error }`
when the worker doesn't take it), `POST .../w-{task}/stop` and `/interrupt` (both stop it). 404
`{ ok: false, error: "no such task" }`.

### GET /api/accounts, POST /api/accounts/use, POST /api/usage/check

`{ accounts: AccountRow[], assistantAccount }` (`assistantAccount`, the account the assistant
runs on); `{ account: "<id>" | "auto" }` pins the assistant to an account or lets it pick
(200 `{ ok: true, ...assistantAccount, waits, usage }`, 400 `{ ok: false,
error }` for an account no runner lists or one the assistant doesn't run on); and a look at
every account's use now (`{ ok: true, asked }`, `asked` false within 2 minutes of the last).

### GET /api/jobs?fresh, POST /api/jobs/stop

`{ at, jobs, runners }`: the runners' sessions with a turn on and what each runner lists of
its machine (`session:<id>`, `<machine>:pid:<pid>`, `<machine>:launchd:<label>`), failed first,
each runner's `{ ok, error? }`; a look is kept 4 s unless `fresh`. `{ id }` stops one (400 `{
ok: false, error }` when it is gone or can't be stopped from here). There is no
`POST /api/jobs/clear` (ADR 0017).

### GET /api/environments, POST /api/environments/check

`{ at, summary: { ok, warn, down }, groups: [{ id, title, footer, items }] }`: the runners, the
coding-thread app's machines (group `t3`) and the assistant's accounts, each `{ id, group,
name, icon, tint, state, status, detail, at, error, hint, facts, lists, actions, lastError }`.
`{ id, kind: "check" }` looks at one again and answers the page; 400 `{ error: "unknown action
<kind>" }` for another kind.

## Machines

### GET /api/machines

Every runner machine's capacity, warnings, sessions and its last day (`MachinesPage`), from
the samples of `POST /api/runner/machine`:
`{ "at", "thresholds", "machines": [Machine] }`. A machine has `id`, `name`, `host`, `platform`,
`state` (`ok`, `warn`, `low`, `stale`, `offline`, `unknown`), `status` (in words), `seen`, `age`
(s), `placeable`, `skipWhy`, `warnings [{ level, text }]`, `cpu`, `memory`, `gpus`, `disks`,
`battery`, `runner` (its sessions, queue, Claude processes and memory `cap`), `sessions` and
`history` (5-minute points of the last 24 h, each the worst of its samples; `[]` when there are
none). Figures come from the runner's own sample: a runner that doesn't measure something
leaves it out (inside `cpu`, `gpus`, `disks`, `battery`, history points) or sends `null`
(`memory`, `runner`, `cap`). Bytes are bytes; shares are 0 to 1. `?history=0`
leaves `history` out (`emberhut-client` always asks for it).

A GPU also has `procs`, the processes holding its memory (NVIDIA GPUs; left out, or `[]`, for one
that shares the system's memory): each `{ pid, name, mem, unit, user }`, every field optional,
`mem` in bytes and `unit` the service it runs under (a systemd unit). The Rust server doesn't
serve `/api/machines` yet.

## Coding threads

The coding-thread app is another coding tool on the person's machines. The server follows it
over its MCP endpoint, one per machine (`--coding-app NAME=URL`, tokens in the file
`--coding-app-tokens` as `NAME=TOKEN` lines): every 30 seconds it lists each machine's threads
into the state's `threads` and `t3Running`, and reads what the changed ones were asked and
answered. A machine that can't be reached keeps its last list.

### GET /api/t3/activity?machine=&thread=

A coding-thread app thread's latest steps, read from the app now, read-only: `{ "title", "status",
"updatedAt", "items": [{ "type", "title", "text", "at" }] }`. `items` are its newest 40, oldest
first, checkpoints and blank ones left out, each text cut to 1200 characters; `title` and `status`
are `""` and `updatedAt`, an item's `title` and `at` `null` when the app gave none. 400 `{ ok:
false, error }` when no coding-thread app is configured on `machine` (or none is named); 502 `{ ok:
false, error }` with the app's error (`<machine>: <reason>`) when it can't be reached or refuses (an
unknown thread); 401 `{ signIn: true }` without a session.

## Theme

### GET /api/theme

`ThemePage`: `{ version, choice, tokens: { light, dark }, contrast: { light, dark, aa }, presets: [{ id, name, light, dark }], names }`.

- `choice`: `{ preset, appearance (system | light | dark), accent ("#RRGGBB" or null), at (null: never set) }`.
  The default is `paper`, `system`; a saved choice keeps the preset it names.
- `tokens`: the choice's 14 colours per appearance, `#RRGGBB`: `accent`, `accentSoft`,
  `onAccent`, `background`, `surface`, `surfaceRaised`, `border`, `text`, `muted`, `faint`,
  `success`, `warn`, `danger`, `info` (`names` lists them in this order).
- `contrast`: per appearance `{ accentOnBackground, accentOnSurface, onAccent, adjusted, asked }`
  (WCAG ratios rounded down to 2 places; `adjusted`: the asked accent was darkened or lightened to
  reach 4.5:1), and `aa` when every ratio is 4.5 or more.
- `version` is the presets' version: a client keeps its cache per version.

### POST /api/theme

```json
{ "preset": "teal", "appearance": "dark", "accent": null, "preview": true }
```

`ThemeChange`: only the fields given change. `accent` left out keeps it, `null` goes back to the
preset's own, a colour (`#RGB` or `#RRGGBB`, `#` optional) sets it. `preview: true` works the page
out without saving. 200 `{ "ok": true, ...ThemePage }` (`ThemeSetReply`); 400
`{ "ok": false, "error": "preset: one of paper, graphite, blue, ..." }`,
`"appearance: system, light or dark"` or `"accent: a hex colour like #3366FF (or null for the preset's own)"`.

## The MCP endpoint

### POST /mcp/emberhut

The tools a room's agent session calls, over MCP's Streamable HTTP transport, stateless: each
request is one JSON-RPC 2.0 message (`RpcRequest`), answered in the same response as
`application/json` (`RpcResponse`, its keys in the order `{ result, jsonrpc, id }` or
`{ jsonrpc, id, error }`). The bearer is the room's agent token, which the server hands to the
room's session (not a device's token): without one, 401 `{ "error": "unknown caller" }`. A
family member's room's session is 501 `{ "error": "family rooms are not available yet" }`
(ADR 0022). Any
other method than `POST` is 405 `{ "error": "POST only" }`, token or not. Each tool's
`inputSchema` stands alone: the named types it uses are in its own `$defs`.

- `initialize`: `{ protocolVersion, capabilities: { tools: { listChanged: true } }, serverInfo: {
  name: "emberhut", version: "1" } }`, the client's revision when the server speaks it.
- `ping`: `{}`. `tools/list`: `{ tools: [{ name, description, inputSchema }] }`.
- `tools/call` `{ name, arguments }`: `{ content: [{ type: "text", text }], isError? }`. A tool
  that isn't there (`MCP error -32602: Tool <name> not found`), arguments its schema refuses
  (`MCP error -32602: Input validation error: Invalid arguments for tool <name>: ...`) and a
  tool's own refusal are results with `isError: true`.
- The tools a room's session has besides the areas' own: `wake_me_at`, `list_checkins`,
  `cancel_checkin`, `watch`, `watch_note`, `list_watches`, `stop_watch` (its appointments: every
  room's from the owner's rooms, only its own from a configured agent's),
  `room_send`, `rooms` (the other rooms), `send_file` (a file with its reply), `set_routines`
  (routines and quiet hours) and `tell_owner` (an alert on the owner's phone).
  [ADR 0014](adr/0014-agent-runtime-and-wake-rule.md) has what each does.
- Another method: the JSON-RPC error -32601. A notification: 202, no body. A body that isn't
  JSON (-32700), isn't one JSON-RPC 2.0 message (-32600), or is over the limit (413, -32600):
  the error, `id` null.

## Party

Two to five AI personas talk about the person's topic turn by turn; the person jumps in, steers,
pauses, resumes and stops them (`emberhut_api::party`).

### GET /api/parties

`PartiesPage`: `{ parties: [Party], presets: [Persona], recent: [Persona], models: [{ id, name,
in, out, note }], defaults: { maxTurns, maxTokens, model } }`. Parties come the one changed last
first; `recent` is the person's own personas from earlier parties, one per name. A `Party` is
`{ id, title, topic, status (running | paused | done | stopped), why, personas, turns, maxTurns,
tokens, maxTokens, cost, created, updated, speaking, last }`; a `Persona` `{ id, name, role,
model, color }` (`p1`…`p5` and a colour in a party, empty elsewhere); `speaking` is `{ who, name,
text }` while a persona speaks; `last` (the list only) is `Name: what they said`.

### POST /api/parties

```json
{ "topic": "Tea or coffee", "maxTurns": 12, "maxTokens": 150000,
  "personas": [{ "name": "Nora", "role": "An optimist.", "model": "claude-opus-5-5" }] }
```

`NewParty`, started at once. 200 `{ "ok": true, "party": Party }`; 400 `{ "ok": false, "error" }`
for no topic, a topic over 4,000 characters, other than 2 to 5 named personas, or two of one name.
The server keeps `maxTurns` within 1 to 100 and `maxTokens` within 5,000 to 2,000,000.

### GET /api/parties/{id}/live

Server-sent events (`PartyEvent`, by `type`): `snapshot { party, messages }` first, then `party
{ party }`, `message { message }` and `speaking { speaking }` (a persona's words so far, every
120 ms at most), with a `: ping` comment every 15 s. A line is `{ id, party, who, name, kind
(turn | say | steer), text, model, tokens, cost, cut, at }`. 404 for a party the server lacks.

### POST /api/parties/{id}/say, /pause, /resume, /stop, /delete

`say` takes `{ "text", "kind": "say" | "steer" }` (a party that waits starts again to answer);
the others take `{}`. 200 `{ "ok": true, "party": Party }` (`null` after `delete`); 400 or 404
`{ "ok": false, "error" }`. `resume` gives a party at its caps 10 more turns and half its token cap
again.

## The Dark room

A plain chat with a model off the record: the assistant never sees it (`emberhut_api::dark`).

- `GET /api/dark`: `{ model, where, busy, chat: [{ id, role (user | assistant), text, at }] }`;
  `busy` while an answer is being written.
- `POST /api/dark` `{ "text" }`: the answer as server-sent events, `{ "thinking": true }` while
  the model reasons, `{ "delta" }` as it is written, then `{ "done": message }` or `{ "error" }`.
  400 `{ "error": "Say something." }`, 409 `{ "error": "A reply is still being written." }`.
- `DELETE /api/dark`: everything said is deleted; 200 `{ "ok": true }`, 409 while an answer is
  being written.

## Agents and approvals

Configured agents and the approvals of their outward steps ([ADR 0027](adr/0027-agents-and-approvals.md);
types `emberhut_api::agents`, `emberhut_api::approvals`). Every route here but the last group needs a
signed-in device in either auth mode; a refusal is `{ "error" }` with 400 (a rule broken), 401,
403, 404 or 409.

- `GET`/`POST /api/agents`, `GET`/`PATCH`/`DELETE /api/agents/{id}`: an agent answers as
  `AgentPage` (the agent and its lists); a new one without `room` gets a room of its own. The
  default agent (`main`) is not deleted, renamed, given a persona or narrowed (400); an agent
  with approval history is not deleted (409: disable it).
- `PUT /api/agents/{id}/channels`, `/tools`, `/accounts`, `/policies`: the whole list, in one
  transaction. A tool that acts as the owner, or a fixed-core kind (`payment`, `credential`,
  `secret`, `account_security`, `as_owner`) given to anyone but `owner`, is 400.
- `GET`/`POST /api/approvers`, `PATCH`/`DELETE /api/approvers/{id}` (409 while a policy names
  one); `GET`/`PUT /api/trusted-requesters`.
- `GET /api/approvals?state=&agent=`, `GET /api/approvals/{id}`: `Approval`s, with
  `waitsOnOwner`, and `amountCurrency` beside a payment's `amountCents`.
- `POST /api/approvals/{id}/decision` `{ "decision": "approve" | "reject", "payloadHash" }`: the
  owner's, on a request that waits on the owner (403 for one that waits on an approver, 409 for a
  hash that isn't the request's or a request no longer pending). `POST /api/approvals/{id}/withdraw`.
- `POST /api/approvals/{id}/use` `{ "text", "files" }` with an agent's room token: 200
  `{ "used", "message" }`; nothing goes out unless `used`.
- Proving themselves instead: `POST /api/approvals/slack` (Slack's signature, the approver's team
  and user), `POST /api/approvals/{id}/webhook` (the approver's HMAC), and the signed link's page,
  `GET`/`POST /approve/{token}` (HTML; 410 once used, expired or withdrawn).

- `POST /api/agents/{id}/inbound` `InboundMessage` with the agent's bridge's bearer (the secret
  `agent-<id>-inbound`; none saved: 401 for everyone): 200 `{ "turn", "skipped" }` (a direct
  message, a mention or a reply in a thread the agent is in is a turn in its room, framed as a
  channel message with the words quoted), 403 for a channel that isn't the agent's, 404 for no
  such agent (and for the default agent, which takes no bridge). The bearer is checked before
  the body is read; the header fields are one line each. `kind` is `slack`, `whatsapp` or `app`
  (a chat in another program, answered through its bridge's own tools; its messages are always
  direct). Never an approval.
- `POST /api/agents/{id}/notes` `InboundNote` `{ "text" }` (up to 2,000 bytes) with the same
  bearer: the bridge's own note about itself (its connection was back after a gap, a button on
  one of the agent's messages was tapped), 200 `{ "turn", "skipped" }`, a turn framed as the
  bridge's (its own word) with its words quoted; 401 and 404 as above. Never anyone's request,
  never an approval.
- `POST /api/agents/{id}/slack-actions` with the same bearer: a click on one of the agent's
  approval buttons, Slack's `block_actions` payload as a Socket Mode connection receives it
  (such an app's clicks reach only its connection, unsigned). It counts as a signed click on
  `POST /api/approvals/slack` does: 200 decided; 400 not an approval's button; 401 as above; 403
  not the approver's team and user, or not on the request's own message; 404 no such request of
  this agent's (another agent's included); 409 another hash, or decided already.

The state has `agents` (when there is one besides the default) and `approvals` (the pending
ones); the live stream sends `event: approvals` with `{ id, state, pending }` when a request
changes.

## Releases

The feed of each app's published builds, and the desktop's payloads (ADR 0005). The server
doesn't serve these routes yet: a client reads a `404` as "nothing published". The types are
`emberhut_api::releases`.

| Route | Who | What |
| --- | --- | --- |
| `GET /api/releases/v{format}/{platform}` | a signed-in device, or the publish key | the feed; `404` when nothing is published |
| `PUT /api/releases/v{format}/{platform}` | the publish key | replaces the feed |
| `GET /api/releases/files/{name}` | a signed-in device | a payload (`Content-Length`, `Range`), never compressed |
| `PUT /api/releases/files/{name}` | the publish key | stores a payload |

### GET /api/releases/v{format}/{platform}

`ReleaseFeed`: `{ format, product, platform, published, releases: [Release] }`, `platform` one of
`macos`, `ios`, `android`, `releases` newest first (the last few). A `Release` is
`{ version, build, date, notes, minimumOs, assets: [Asset] }`; `assets` is empty or left out on a
phone's feed. An `Asset` is `{ arch, url, size, sha256, signature }`: `url` a path on this server,
`sha256` 64 hex digits, `signature` a minisign signature file as text, whose trusted comment is
`emberhut-desktop macos <arch> <version> build <build> sha256:<hex> notes:<hex>` (the last the SHA-256
of the notes' UTF-8 bytes as they decode). A reader reads any `format` up to its own, asks for its
own first, and on a `404` for each lower one; a breaking change is a new format at a new path.

### GET /api/releases/files/{name}

The payload's bytes as they are, with no `Content-Encoding` whatever the request's
`Accept-Encoding` (the gzip rule above doesn't apply), so `Content-Length` is its size.
`emberhut-client` sends `Accept-Encoding: identity` and stops past the size the feed lists. Names match
`Emberhut_<version>_<build>_<platform>_<arch>.dmg`.

### The publish key's routes

A `PUT` of a payload writes a temporary file and renames it into place, and refuses to overwrite a
name whose bytes differ. A `PUT` of a feed is refused unless every asset it lists is stored with
that size and `sha256`; once it succeeds, the payloads no served feed lists are deleted. The
publish key can do these two `PUT`s and the feed `GET`, nothing else.

## Connect

How a client finds and checks the routes to a machine, from
[ADR 0007](adr/0007-connectivity.md) and [ADR 0008](adr/0008-end-to-end-encryption.md). Only
emberhut's server has these routes and fields. A server without them is a **legacy host**: its
pairing answer has no `machine`, and the probe answers 404 or 405. The client's rules for
choosing a route are in [connect.md](connect.md), the relay's protocol in [relay.md](relay.md),
the sealed sessions relay routes carry in [sealed.md](sealed.md), and the account directory's
API in [directory.md](directory.md). The types are in `emberhut_api::connect`, and the test vectors
in `crates/emberhut-api/tests/fixtures/connect/`.

### Encodings

- **Binary in JSON** (keys, nonces, signatures) is base64url without padding (RFC 4648,
  section 5): 32 bytes are 43 characters, 64 bytes are 86. A decoder refuses padding and
  non-zero trailing bits, so every value has one spelling.
- **The machine id** is the first 160 bits of
  `SHA-256(statement("emberhut machine id v1", [public key]))`, in RFC 4648 base32, lowercase,
  without padding: 32 characters from `a-z2-7`.
- **Times in JSON** are numbers: whole milliseconds since the Unix epoch, from 0 to
  2^53 − 1, so a JavaScript number holds them exactly. A larger one doesn't read.
- **A host** (`host` in the probe, a route's host) is compared in a normal form:
  - a DNS name: lowercase, with internationalised labels as IDNA A-labels (UTS 46 with the
    STD3 rules but `_` allowed, hyphens checked and DNS lengths verified). So only letters,
    digits, hyphens and underscores (container and `.local` names use them), no hyphen at
    either end of a label nor in its third and fourth places (but `xn--`), labels of 1 to 63
    characters, at most 253 characters, and no trailing dot;
  - or an IPv4 address in dotted decimal. A name whose last label is a number must be one;
  - or an IPv6 literal in brackets, in RFC 5952 form, without a zone id, and not
    IPv4-mapped (`[::ffff:a.b.c.d]` is refused: the IPv4 address names that host);
  - the port, 1 to 65535, dropped when it is the scheme's default (`:443` for `https`, `:80`
    for `http`) and kept otherwise.

  The machine normalizes a request's host against the scheme of the route it came in on:
  `https` when relayed (the host is then the relay domain, with no port when the relay serves
  on 443, its default); on a direct listener, `http`, unless the listener sits behind a TLS
  proxy, in which case it is configured with its public scheme (`EMBERHUT_PUBLIC_URL`'s).

  These rules differ from the WHATWG URL Standard's host parser (what `new URL()` does), so a
  client can't take a URL's host as it is. Each difference has a vector in `hosts.json`
  (`whatwgDifferences`); all are refused here:
  - IPv4 addresses in short, hexadecimal or octal forms (`127.1`, `0x7f.1`, `192.0.2.010`);
  - IPv4-mapped IPv6 literals, in either spelling;
  - characters other than letters, digits, hyphens and underscores (`;`, `,`, `=`, `"`,
    `*`...);
  - hyphens at a label's ends or in its third and fourth places;
  - labels over 63 characters and names over 253;
  - a trailing dot;
  - percent-encoding (`ex%61mple.com`);
  - user information (`user@example.com`): a host is not an authority;
  - port 0, and an empty port (`example.com:`).
- **A relay domain** is a host name in that normal form with no port and no `_` (strict
  STD3), of at most 220 characters.
- **A socket address** (`local`) is text, `a.b.c.d:port` or `[v6]:port`:
  - the IPv6 address in RFC 5952 form (lowercase, shortest);
  - no zone id;
  - an IPv4-mapped IPv6 address (`::ffff:a.b.c.d`) written as the IPv4 address.
- **A machine's name** (`name` in the route list, the link statement, the relay's `auth` and the
  directory) is 1 to 60 Unicode scalar values with no white space at either end, and no control
  character (`Cc`) or bidirectional control (U+061C, U+200E, U+200F, U+202A to U+202E, U+2066 to
  U+2069) anywhere. White space is Unicode's `White_Space` property: U+0009 to U+000D, U+0020,
  U+0085, U+00A0, U+1680, U+2000 to U+200A, U+2028, U+2029, U+202F, U+205F and U+3000. U+FEFF is not
  white space (JavaScript's `trim()` removes it, and keeps U+0085). The sender trims, cuts after 60
  characters and trims the end again, before it signs; a receiver refuses a name that isn't already
  so, and never cuts it. A name that still holds such a control character after trimming can't be
  sent: the sender reports an error rather than dropping the character.
- **A route kind** in a probe is 1 to 32 characters from `a-z`, `0-9` and `-`.
- **An account's user id** is 1 to 255 bytes of visible ASCII (`!` to `~`).

### Statements

Every signed statement, and the input of the machine id's hash, is
`statement(prefix, fields)`:

- the ASCII prefix, then a `0x00` byte;
- then each field as a 4-byte big-endian length followed by its bytes.

What a field's bytes are:

- a time is 8 bytes: big-endian milliseconds since the Unix epoch;
- a key or nonce is its raw bytes;
- any text field (the machine id, `kind`, `via`, an account's user id, a relay domain, a
  machine's name, a host, a socket address) is its UTF-8 bytes, in the normal form above. The
  machine id is its 32 characters, not the 20 bytes they encode.

Signatures are Ed25519 (RFC 8032), and every implementation verifies them by one rule, so that
whether a signature verifies never depends on the library. A signature `R || s` by key `A`
over statement `M` verifies when:

1. `s < L` (the group order), as a little-endian integer;
2. `A` and `R` are canonical encodings (`y < p`, and no negative zero) of points in the
   prime-order subgroup (`[L]P` is the identity), and neither has small order;
3. `[s]B = R + [k]A`, without the cofactor, where `k = SHA-512(R || A || M) mod L`, `R` and
   `A` taken as their 32 bytes, compared as encodings.

A public key that fails the checks on `A` doesn't read at all. This is ed25519-dalek's
`verify_strict` with keys checked for the subgroup when they are read. A TypeScript
implementation on `@noble/curves` sets `zip215: false`, checks that `A` and `R` are in the
prime-order subgroup itself, and uses the cofactorless equation: noble's default is the
cofactored ZIP 215 check, which accepts what this rule refuses. `signatures.json` has a case
for each check (`s + L`, small-order and non-canonical `R`, `R` with a small-order part,
mixed-order, small-order and non-canonical `A`), the twelve cases of "Taming the many EdDSAs",
and a signature of one statement presented as another; each says whether it verifies.

| Prefix | Fields, in order | Signed by | Where |
| --- | --- | --- | --- |
| `emberhut machine id v1` | public key | (hashed, not signed) | the machine id |
| `emberhut probe v1` | nonce, machine id, kind, via, local, host | the machine | `GET /api/connect/probe` |
| `emberhut relay auth v1` | relay domain, nonce, machine name, time | the machine | relay.md's handshake |
| `emberhut relay ticket v1` | machine id, relay domain, ticket id, expires (a time) | the machine | sealed.md's `accept`, shown to the relay (relay.md) |
| `emberhut link v1` | account user id, relay domain, machine name, time | the machine | directory.md's link |
| `emberhut unlink v1` | machine id, relay domain, time | the machine | directory.md's unlink |
| `emberhut device key v1` | machine id, code hash | the device | `PairRequest.keySignature` |
| `emberhut key handover v1` | role (`machine` or `device`), old public key, new public key, time | the old key | defined now, used later (ADR 0008's open question 4) |
| `emberhut sealed static v1` | role (`machine` or `device`), static key, expires (a time) | the machine, or the device | sealed.md's hellos |
| `emberhut sealed prologue v1` | machine id, route kind, route host | (the Noise prologue, not signed) | sealed.md's handshake |
| `emberhut sealed pair v1` | machine id | (CPace's channel identifier, not signed) | sealed.md's pairing |
| `emberhut sealed confirm v1` | role, slot (one byte), handshake hash | (HMAC-SHA-256 under the slot's key) | sealed.md's pairing |

### Pairing: the machine and the device key

- `POST /api/auth/pair` answers, on success, `machine: { id, publicKey }` as well. A device
  that paired from a link, or already holds a key for this host, refuses an answer without
  `machine`, or with another id, as "This server didn't prove it is the machine you meant".
  Only a first, typed pairing may make a legacy host.
- `PairRequest` takes an optional `publicKey`: the device's Ed25519 key, which the machine
  stores on the device's row, and, with it, `keySignature`: the key's signature over the
  `emberhut device key v1` statement, naming the machine id and the SHA-256 of the normalised code
  (the hash the machine stores). The machine stores the key only when the signature verifies
  (by the rule above). The signature is checked against this request's code hash before the
  code is looked up: a `publicKey` without a valid `keySignature` is refused with 400 "Pairing
  failed. Try again.", counts on the wrong-code brake like a wrong code, and uses nothing up:
  no code, row or key is touched. This rule is added by
  ADR 0008: before it the key was taken as sent, and no released client sends the field yet.
- **Until the proof is implemented** (`keySignature` and its statement, ADR 0007's parts 16 and
  13), the server stores no device key: it ignores `publicKey`, whatever it holds, and pairs
  the device as it would without it. A key is stored only once its proof is checked.
- A key is on at most one active row: a pairing that proves a key already on a row replaces
  that row, and a key without proof replaces nothing. Signing the device out clears the key
  from its row.
- Through the relay, a device pairs with the sealed pairing routes below instead, and its key is
  the sealed session's, never a body field.
- A wrong code typed while no code is outstanding is answered as wrong (401) but not counted
  against the brake, since no guess can be right then.

### The pairing link

`https://emberhut.com/pair#id=<machine id>&code=<code>&route=<url>…`, a universal link.

- **The fragment** is `application/x-www-form-urlencoded`: each value percent-encoded, so a
  route's `:`, `/`, `&` and `#` are safe.
- **`id`** appears once.
- **`code`** appears once, as shown (`ABCD-EFGH`). It is read like a typed code: case, spaces
  and the dash don't matter.
- **`route`** appears 0 to 8 times, in the order of the machine's route list, and is never
  empty.
- Unknown keys are ignored. The whole link is at most 2,000 characters, so its QR code stays
  scannable.
- Everything is in the fragment, which browsers never send to a server.
- `emberhut-server pair` and "Pair a device" show the link as text and as a QR code. The QR code
  is scanned with Emberhut's own scanner, so the system never routes it. "Pair a device" gets the
  link as `PairingCode.link` from `POST /api/auth/pair/new`.
- The routes are the ones the machine's configuration gives, best first: its listen address
  when other devices can reach it (not loopback, not unspecified, not link-local), then
  `EMBERHUT_PUBLIC_URL` under the same rule (a public URL on `localhost`, loopback or a link-local
  address is left out of the link). With neither, the link names no route, and the code still
  works typed.
- The device pairs over any of the routes, and checks the answer's `machine.id` against the
  link's `id`.

### GET /api/connect

A signed-in device. `RouteList`:

```json
{
  "id": "<machine id>",
  "publicKey": "<base64url>",
  "name": "Main computer",
  "sealed": [1],
  "routes": [
    { "kind": "lan", "url": "http://<address>:18990", "label": null, "up": true },
    { "kind": "relay", "url": "wss://<relay domain>/v1/sealed/<machine id>", "label": null, "up": false }
  ]
}
```

- `kind` is one of `loopback`, `lan`, `vpn`, `public`, `ssh` and `relay`. It is an open enum:
  readers skip kinds they don't know.
- `up` says whether the machine can serve the route now.
- `sealed` lists the versions of sealed sessions the machine speaks (sealed.md), in ascending
  order. A machine lists a `relay` route only when the list isn't empty, and a client skips
  this list's relay routes when it speaks none of its versions. A relay route learned without a
  list (a pairing link, the account's list) is tried at the client's highest version
  (connect.md).
- Every configured route is listed, whatever its state.

### GET /api/connect/probe?nonce=&kind=

No session is needed, and the probe carries no token. It checks direct routes: through the
relay, the sealed handshake proves the machine and binds the route instead (sealed.md), so a
race sends no probe there. In a sealed session of a paired device the probe still answers, with
`via: relayed`; in a pairing session it gets 401 like every route but pairing's.

- `nonce` is 32 random bytes, `kind` the route kind the client is checking. A probe whose
  `kind` isn't 1 to 32 characters from `a-z`, `0-9` and `-`, or whose `nonce` isn't 32 bytes,
  is answered 400, and so is one whose `Host` (or `:authority`) isn't a host in the encodings
  above.
- The probe is answered with `Access-Control-Allow-Origin: *`, whatever its status.
- While the server shuts down it answers 503.

The answer is `ProbeAnswer`:

```json
{
  "id": "<machine id>",
  "publicKey": "<base64url>",
  "via": "direct",
  "local": "<address>:18990",
  "host": "<the request's host>",
  "signature": "<base64url>"
}
```

- `id` is `publicKey`'s machine id; an answer whose `id` isn't doesn't read.
- `via` is `direct` or `relayed`.
- `local` is the local socket address that accepted a direct connection, and empty when
  relayed, and only then.
- `host` is the request's `Host` (or `:authority`), or, when relayed, the relay domain, in the
  normal form.
- The signature covers the `emberhut probe v1` statement.

A route **answers** when, within 2.5 s, the answer is a 200 with the pinned key and a
signature that verifies over this nonce and kind, and its binding matches:

- an address route (`loopback`, `lan`, a `vpn` address) needs `via = direct` and `local` equal
  to the route's address and port;
- a name route (`public`, a VPN's HTTPS name) must be `https`, and needs `via = direct` and
  `host` equal to the route's host, both in the normal form;
- an `ssh` route, probed through its forward to `127.0.0.1:<local port>`, needs
  `via = direct`, `local` a loopback address on the machine port the forward targets, and
  `host` equal to `127.0.0.1:<local port>`;
- the route's kind is the probe's `kind`; kinds a client doesn't know never match.

A `relay` route answers when its sealed session opens (connect.md).

### Which `Host` a direct listener answers

A request on a direct (TCP) connection is answered only when its `Host` is one of these:

- a loopback name or address;
- the machine's host name, or `<host name>.local`;
- any address of its listening interfaces;
- a name in `EMBERHUT_ROUTES` or `EMBERHUT_PUBLIC_URL`;
- a name in `EMBERHUT_HOSTS`, the list for any other alias.

Any other `Host` is answered 421
`{ "error": "This server doesn't answer to <host>. Add it to EMBERHUT_HOSTS." }`. Requests in
sealed sessions are not checked this way: the handshake has already bound the session to this
machine.

### CORS on direct listeners

The web app (ADR 0008, section 6) calls a machine's `https` direct routes from its own origin,
so a direct listener answers it with CORS. Relayed requests travel in sealed sessions, where CORS
plays no part.

- **Allowed origins**: `https://app.emberhut.com`, and the origins in `EMBERHUT_WEB_ORIGINS` (a
  comma-separated list, for a self-hoster's copy of the web app). A request whose `Origin` is
  any other value gets no CORS headers, and the browser keeps the answer from the page.
- **Preflight** (`OPTIONS` with `Access-Control-Request-Method`): 204 with
  `Access-Control-Allow-Origin` set to the request's origin, `Access-Control-Allow-Methods: GET,
  POST`, `Access-Control-Allow-Headers: Authorization, Content-Type, If-None-Match,
  x-emberhut-looking` (the presence header of "The state") and `Access-Control-Max-Age: 600`. It
  needs no session. These are every header a client sets itself; the others it sends (`Accept`) are
  safelisted, or set by the browser.
- **Answers** carry `Access-Control-Allow-Origin` set to the request's origin, `Vary: Origin`,
  and `Access-Control-Expose-Headers: ETag, Date, Retry-After`: the headers the client reads
  beyond the safelisted ones (`ETag` for the state's `If-None-Match`, `Date` for the clock
  check, `Retry-After` on the pairing brake's 429).
- **No credentials**: no `Access-Control-Allow-Credentials`. The web app sends its token as
  `Authorization` and uses no cookies.
- The probe keeps its `Access-Control-Allow-Origin: *`.

### Sealed sessions

Through the relay, every request travels inside a sealed session (sealed.md): one HTTP/1.1
exchange per inner stream, with the contract unchanged.

- **Tokens** go inside, as on direct routes. In a session of a paired device, a token whose
  device row isn't the session's gets 401 with `signIn`.
- **A pairing session** (a device whose key is on no active row) is served only the two routes
  below; everything else gets 401 with `signIn`.
- `POST /api/auth/pair/sealed/start` takes four CPace shares and answers four shares and four
  confirmation tags; `POST /api/auth/pair/sealed/finish` takes the device's slot and tag with
  `install`, `name` and `model`, and answers `AuthReply` with `machine` and a relay `ticket`.
  Each attempt counts against the wrong-code brake from its `start`, and the code itself is
  never sent. sealed.md has the bodies, the answers and the checks.
- The relay sees no header, so the machine tells it about a session on the uplink instead
  (relay.md's `mark`).

### POST /api/connect/link-statement

```json
{ "account": "<the account's user id>", "relayDomain": "<relay domain>" }
```

A signed-in device, on a TCP connection from loopback only. The server serves it only when the
request came over a connection its own TCP listener accepted, from a loopback peer (`127.0.0.1`,
`::1`, or `::ffff:127.0.0.1`), carrying none of the headers a proxy, tunnel or forward adds
(`Forwarded`, `X-Forwarded-For`, `X-Forwarded-Host`, `X-Forwarded-Proto`, `X-Forwarded-Port`,
`X-Real-IP`, `X-Client-IP`, `True-Client-IP`, `CF-Connecting-IP`, `Via`); those headers are never
trusted to name a peer. Any other request, including one a relayed or sealed listener serves, gets
403 before its session or body is read. A forward on the same computer that adds no header (a
layer-4 port forward, or a proxy set to add none) looks local and is served: ADR 0007, section 5,
says why that is accepted. Then: 401 without a signed-in device (in either auth mode), 400 for an
account id or relay domain not in its normal form, 413 for a body over the limit, and 404 from a
server without an identity of its own. The 200 answer is `SignedLink`, made now under the
machine's name (`EMBERHUT_MACHINE_NAME`, or the host name):

```json
{
  "publicKey": "<base64url>",
  "name": "Main computer",
  "account": "<user id>",
  "relayDomain": "<relay domain>",
  "time": 1795561073000,
  "signature": "<base64url>"
}
```

The signature covers the `emberhut link v1` statement, `name` included. The desktop posts it, as it
is, as the body of directory.md's `POST /v1/machines`, with its own account token.

## Health

### GET /health

`{ "ok": true, "lag": { "ms", "p95_1m", "max_1m", "p95_5m", "max_5m" } }` (`Health`), without
sign-in: the server answers, and how late its runtime is, in whole milliseconds (the latest
sample, then the 95th percentile and the longest of the last minute and of the last five).

### GET /api/loop

`{ "lag": {…}, "slow": [{ "what", "n", "ms", "max" }] }` (`LoopReport`), behind sign-in: the lag,
and the work that held the runtime 50 ms or more in the last five minutes, by name (a request's
method and path, ids as `:x`), the longest in all first.

## The Mac

### GET /api/mac

The owner's Mac, through the helper on it (`MacStatus`): `{ "online": false }` when no helper is
set up or it doesn't answer in 5 s, else `{ "online": true, "locked": bool, "idle": seconds | null }`.

### POST /api/mac/lock

Locks the Mac's screen now: 200 `{ "ok": true }`; 400 `{ "error" }` with the helper's words,
or why it couldn't be reached. Nothing in the API unlocks it.

The helper's own routes for the browser extension's folder, which the server calls, are in
[mac-helper.md](mac-helper.md).

## The browser

The assistant's own browser (a Chrome the server drives over the `DevTools` protocol,
`EMBERHUT_BROWSER_CDP`), and the owner's own Chrome profiles through the browser extension. ADR 0021.
`session`, `input`, `upload` and `POST /api/chrome` hand out the live view's password, drive the
logged-in browser or mint a pairing link, so they need a signed-in device in either auth mode
(401 `signIn` without one); the frame is read with any session report mode lets through.

### GET /api/browser/session?device=

The live view (the browser's screen, streamed by the machine it runs on, `EMBERHUT_BROWSER_VIEW`):
200 `{ ok, url, user, password, mode, screen }`. `mode` is `control`, with the
`browser-control` secret's password, for the device that took over (or any, when it didn't
say which); else `view`, with `browser-view`'s. 503 `{ ok: false, error }` while it isn't set up.

### GET /api/browser/frame.jpg

The tab's latest picture (refreshed at most every 5 s), keys blacked out: 200 `image/jpeg`,
`no-store`. 404 while the owner drives (a password may be on screen) or with no browser.

### POST /api/browser/input

`{ kind }`: `takeover` (`device`), `handback`; while the owner drives, the toolbar: `back`,
`forward`, `reload`, `go` (`url`: an address, or words searched), `tab` (`id`), `newtab`,
`closetab` (`id`, or the tab on screen), `chrome` (`show`: Chrome's own controls). 200
`{ ok: true, ...browser }`; 409 `{ ok: false, error }` (`Take over first`, `unknown input <kind>`).

### POST /api/browser/upload?name=

The file the page asked for (`fileRequest`), as the raw body (100 MB at most): streamed into the
data folder's `browser/uploads`, then put into the page's field. 200 `{ ok: true, ...browser }`;
409 when the page isn't asking for one, or the file is over 100 MB or didn't arrive whole.

### GET /api/chrome/ws

The extension's WebSocket (no app session: its first message is `pair` with a link's code, or
`hello` with its own token, its params carrying `"protocol": 1`; anything else, or nothing in 10 s,
closes it with 1008). Then the extension's protocol (`emberhut.start`, `emberhut.hello`,
`emberhut.paired`, `emberhut.pause`, `emberhut.resume`, `emberhut.name`, `emberhut.step`,
`emberhut.handoff`, `emberhut.handback`, `emberhut.update`, `emberhut.stopped`, `emberhut.unpair`,
`emberhut.cut`, `emberhut.blocked`, `emberhut.framed`: `emberhut_api::chrome::extension`), and its
`ping` is answered with `{"method":"pong"}`. `emberhut.update` `{ version }` tells a profile on the
Mac that the extension's folder there holds a newer version (ADR 0023,
[mac-helper.md](mac-helper.md)). A request that isn't an upgrade is 400.

### GET /chrome/pair, /chrome/pair/status?code=, /chrome/emberhut-extension.zip

No sign-in (the code is the key). The pairing page (its code after `#`) polls its status:
`{ state: "waiting" | "paired" | "expired", label?, name?, email?, machine? }`. The zip is the
extension's build (`EMBERHUT_CHROME_EXTENSION_ZIP`), 404 without one, named
`emberhut-extension-<version>.zip`.

## Backup

### GET /api/backup

A signed-in device only (a session is needed even in report mode): 200
`application/gzip`, `Content-Disposition: attachment; filename="emberhut-backup-<UTC day>.tar.gz"`,
streamed. The archive holds `emberhut.db`, a consistent snapshot of the database, and the folders
the server keeps next to it; not the secrets directory, which is kept and restored on its own.
It is never gzipped again. A failure after the answer has started shows as a cut-off archive.
One backup is made at a time: while one is, another request is 503 `{ error }`.

## Tasks

Tasks, their projects and areas, and the person's plan (ADR 0015). The state carries `issues`
(the board's tasks with their `deadline`, `hardDeadline` and `score`), `today`, `projects`,
`goals` (with this week's steps, `week: { owner, assistant }`), `todos`, `weekStart`, `focus`,
`areas`, `report`, `loops` and `tasks` (the assistant's background jobs).

### GET /api/tasks?q=&project=&closed=&archived=&area=&status=&owner=&limit=

Any task, done and canceled ones too (`TasksQuery`, `TasksPage`): `q` a key, a bare number or words
of the title and description; `project` an id or `none`; `closed=1` only done and canceled ones,
`archived=1` only canceled ones; `status` statuses, comma separated (unknown ones left out); `owner`
is `owner` or `assistant`; `limit` 50 by default, 200 at most. A key first, then tasks with every
word in their title, then the newest. A list with no `q` has no findings.

### GET /api/tasks/{id}

One task (`TaskDetail`): its fields, `history` (oldest first, `{ at, who, text }`), `subtasks`,
`findings`, `links` (`{ kind, ref, machine, title, status, at, line }`), `runs` (newest first),
`sessions` and `ultraRun`. 404 `{ "error": "no such task" }`.

### POST /api/tasks/{id}/message

The person's reply on a task's page, `{ text, files }`: it goes to the task, never Main. To its
worker when it is the assistant's with a brief (`{ ok, to: "worker", queued }`: queued while one
runs), else to the assistant, told to answer on the task (`{ ok, to: "assistant" }`). The task's ask
is cleared. 400 `{ "error": "nothing to send" }` for no text and no files.

### POST /api/tasks

A program on the server's machine hands the assistant a job the person asked it for:
`{ title, body, from }`, with the server's program key as the bearer (`EMBERHUT_PROGRAM_KEY_FILE`).
200 `{ ok, id, key }`; 400 `{ "error": "a task needs a title and a body (the brief)" }`; 401
`{ "error": "programs use the key" }`, a device's session too.

### GET /api/artifacts/{id}, GET /api/artifacts/{id}/thumb

A file a task produced, by its artifact's id, in byte ranges (as `/uploads/`); `/thumb` a small
JPEG of a picture or a video. A link, an X reply, or a file that is gone, private or of a kind
the app doesn't open is 404 with no body.

### GET /api/scheduled

`{ items }` (`ScheduledPage`): everything with a next run, every room's, soonest first, each
with `kind` (`watch`, `checkin`, `routine`, `prep`, `unit`), `nextAt`, `repeat` in words, its
`room` and the open `tasks` it keeps an eye on.

### POST /api/settings: tasks and the plan

`SettingsRequest`'s fields, each doing nothing for a value of the wrong kind:

- `pinFocus: id` (with `pinned: false` to unpin), `dropFocus: id`, `parkFocus: id` with
  `until` (`null` brings it back);
- `todoDone: id` (with `done: false` to take the tick back), `todoPark: id` with `until`;
- `taskArchive: id` (with its open subtasks; a done task is refused), `taskUnarchive: id`;
- `taskDecide: id` with `choice` (above);
- `xPosted: artifact` with `option`: the reply posted, kept on the post, and its task done;
- `taskCreate: title` with `goal` and `project`: the person's quick add;
- `projectSet: { id, priority, deadline, hard, status, pausedUntil }` and
  `taskSet: { id, deadline, hard, size, project, decideBy, body, ultra }`: the fields given
  change, `null` clears one; a deadline that isn't a day is 400.

## Calls

The recorder on the owner's Mac signs in as a device, like the apps. Recording (`/record`,
`/in`, `/live`) needs a signed-in device in either auth mode: without a session it is 401
`{ "error": "Sign in first.", "signIn": true }`, report mode included. Reading the calls passes
the session gate as anything else does.

### POST /api/calls/record

`{ "action": "start" | "stop", "room"? }` (`RecordCallRequest`): the app's Record button. `start`
(anything but `stop`) makes a call (`2026-11-25-1a2b3c`, titled `Call on <UTC time> UTC`) whose
transcript goes to `room` (Main when left out) and asks the Mac helper's recorder to start
(`POST <helper>/record/start` with `{ id, title }`); `stop` asks it to stop. Always 200:
`{ "ok": true, "text" }` with what the recorder said, or `{ "ok": false, "error" }` when the Mac
didn't do it or the server couldn't keep the call (a start that fails leaves no call).

### POST /api/calls/in?call=&track=mic|sys&last=&seconds=&title=

One track of a recorded call, its audio (m4a, at most 256 MiB) as the raw body: 200 `{ "ok":
true, "call", "bytes" }` (`ReceivedTrack`). The call is one being recorded, or one the Mac's
recorder started itself, which is made, in Main, titled `title` or by its time: its id must then
be a new call's, `<UTC day>-<6 lowercase hex digits>` of today or yesterday. `last` (not empty)
queues the call to be transcribed and ends its live copilot. 400 `{ "error": "call (its id) and
track (mic or sys)" }` for an id that isn't 6 to 40 letters, digits, `_` and `-`, or another
track; 400 `{ "error": "not a call being recorded" }` for an id the server didn't make and that
isn't a new call's, or a call no longer being recorded; 400 `{ "error": "The audio did not
arrive whole. Try again." }` for audio over the limit or cut short. The transcript (Markdown, kept
beside the database in `calls/<id>/`) comes to the call's room as a `call` event; a failure too,
with its reason. The audio is deleted after `--call-audio-days` days.

### POST /api/calls/live?call=&track=&t=&title=&copilot=0|1

About ten seconds of a track while it records (WAV, at most 4 MiB), from `t` seconds: 200
`{ "ok": true }`, and heard in the background as a line of the live transcript. The call is
taken as by `/api/calls/in`. `copilot=0`: the owner turned the call's copilot off; `copilot=1`:
back on. 400 `{ "error": "call, track (mic or sys) and t" }`, or `not a call being recorded`.

### GET /api/calls, GET /api/calls/{id}/transcript, GET /api/calls/{id}/live

`{ "calls": [{ id, title, status, room, at }] }`, the latest 8 (`CallList`); a call's transcript
as `text/markdown` (404 `{ "error": "no transcript" }`); and the live panel (`CallLive`):
`{ call, title, status, copilot, lines: [{ t, who, text }], hints: [{ t, text, at }] }`, the
last 40 lines with the microphone's echo of the other side left out (404 for an id that isn't
one). The state's `recording` is the call being recorded now, `{ id, title, room, at }`. Not
working yet: the copilot's hints need an agent session outside the rooms, which this server
doesn't run yet (ADR 0019), so `hints` is empty and `copilot` is `off`.

## Voice

Every voice route needs a signed-in device in either auth mode (401 `signIn` without a
session, report mode included): a call's key and dictation are paid for with the provider's
account, and a call's request and end speak to the assistant for the owner.

### POST /api/voice/session

`{ "ok": true, "key", "expiresAt", "model" }` (`VoiceSession`): a key for one realtime voice call
(10 minutes), with its instructions and its one tool, which the phone answers with `POST
/api/voice/ask`. The tool is `ask_assistant` (`emberhut_api::wire::VOICE_ASK_TOOL`). 502 `{ "ok":
false, "error" }` when the provider isn't set up or refuses.

### POST /api/voice/ask

`{ "request", "room"? }` (`VoiceAsk`): said in the room's chat as `(by voice) <request>`;
`{ "ok": true, "answer" }` with the assistant's first answer after it (two minutes at most, then
a line saying it will be in the chat). 502 when it can't be said.

### POST /api/voice/done

`{ "seconds"?, "diag"?, "transcript"?, "room"? }` (`VoiceDone`): `{ "ok": true }`. The phone's
report is kept as the state's `lastCall` (`{ report, transcript }`); a transcript with a line of the
person's (starting with one of the server's person labels: by default `Owner:` and `Me:`) goes to
the room's assistant as a `voice` event to follow up.

### POST /api/transcribe?name=&words=

Dictation: the recording as the raw body, at most 25 MiB (413 `{ "ok": false, "error":
"recording too long" }`); `{ "ok": true, "text" }` (`TranscribeReply`). `words`: names it may
have. 502 when the provider isn't set up or fails. The state's `voice` says whether it works.

## Calendar

### GET /api/calendar?from=&to=

`{ from, to, events, items, note, at }` (`CalendarView`): the events of the days `from..to`
(both `YYYY-MM-DD`, at most 62 days apart; 400 `{ "error" }` otherwise), each as the Mac helper
gave it with `id`, `day` and `kind` (`meeting`, `event`, `focus`), by start; and what there is
to work on, each on its day. The Mac is read once a minute per range at most (`GET
<helper>/calendar?days=&from=&to=`); each day it covered is kept, and when it doesn't answer
`note` says the events are what was last seen. `items` comes from the server's tasks: their
deadlines, decide-by days and today's plan (ADR 0019).

## Health and screen time

Both routes store the owner's data, so both need a signed-in device in either auth mode (401
`signIn` without a session, report mode included); the phone and the Mac helper sign in as
devices.

### POST /api/health

`{ "days": [{ "date", ... }], "why"?, "tz"? }` (`HealthUpload`) from the phone: each day with a
`YYYY-MM-DD` date is kept as it came, in place of what that day had; `why`, the reason it sent
none, is kept until days come; `tz` is the phone's zone. `{ "ok": true, "saved" }` (`Saved`).
The state's `healthAt` is the last time days came, and `health` today's figures
(`HealthNow`: `date`, `steps`, `exerciseMin`, `moveMin`, `stepsTarget`, `at`, `project`,
`tasks`): the walk tasks are the active `Health` project's tasks (ADR 0019).

### POST /api/screentime

`{ "days": [{ "date", "device", "minutes", "apps" }] }` (`ScreenTimeUpload`) from the Mac helper:
each entry with a date and a device is kept, in place of what that day and device had;
`{ "ok": true, "saved" }`. The state's `screentimeAt` is the last time it came.

## Messaging

The owner's messages from outside the app and the assistant's own WhatsApp number (ADR 0020).
The reads and sends are MCP tools; these are the routes.

### GET /api/whatsapp

A signed-in device: the number's state (`WhatsAppState`): `on` (the kill switch), `state` (the
daemon's connection), `number`, `error`, `linking`, every chat it knows (`chats`, the allowed
ones first) and the last 10 messages it sent (`sent`).

### POST /api/whatsapp

A signed-in device, `{ "kind", "by"?, "task"?, "phone"? }`: `on` / `off` answer
`{ "ok": true, "text" }`; `link` (a pairing code for `phone`) and `qr` (a one-time QR page)
answer `{ "need" | "code" | "qr" | "error", "text" }`; `link-task` (the task linking is asked on)
answers `{ "ok": true }`; any other kind answers the state. A body that isn't one is 400.

### POST /api/whatsapp/in

The WhatsApp daemon only, its token (the secret `whatsapp-token`) as the bearer; anything else
is 401. `{ "kind": "status" | "group" | "removed" | "message", ... }` (`WhatsAppEvent`) answers
`{ "ok": true }`, or for a message `{ "dropped": why }` or `{ "kept": true, "asked"?, "off"?,
"capped"? }`. An unknown kind is 400 `{ "error": "unknown kind <kind>" }`.

### GET /wa/{token}

The one-time QR page that links the number (HTML, no session: the link is the key, for 10
minutes): the live QR, refreshing itself; "Linked" once the number is linked; 404 once it
expired or was used.

### GET /gmail/connect/{token} and GET /gmail/callback?state=&code=&error=

Signing a Gmail account in, read only (HTML, no session: the one-time link is the key, for a
day). The first sends the browser on to Google's consent screen (302). The second is where
Google sends it back: 200 once the account's sign-in is saved on the server; 400 when Google
refused, the reading scope wasn't allowed, or another address signed in; 404 for a link that
expired or was used; 500 when the sign-in couldn't be saved.

## Parties

Two to five AI personas talking about a topic, turn by turn, through the model at
`--party-model-url` (ADR 0022). Every route needs a session, and the ones that change a party
(`POST`) a signed-in device in either auth mode; a refusal is `{ ok: false, error }`,
400 for what the rules don't take and 404 `"no such party"`.

### GET /api/parties

`PartiesPage`: `{ parties, presets, recent, models, defaults: { maxTurns, maxTokens, model } }`.
A `Party` is `{ id, title, topic, status (running | paused | done | stopped), why, personas:
[{ id (p1…p5), name, role, model, color }], turns, maxTurns, tokens, maxTokens, cost (USD, 4
places), created, updated, speaking: { who, name, text } | null, last }` (`last`, the list's
preview, only in the list).

### POST /api/parties

`{ topic, personas: [{ name, role, model }], title?, maxTurns?, maxTokens? }`: it starts talking
at once. 200 `{ ok: true, party }`; 400 `"a party needs a topic or an opening line"`, `"a party
has 2 to 5 personas"`, `"each persona needs a name of its own"`.

### GET /api/parties/{id} and GET /api/parties/{id}/live

`{ party, messages }`, a message `{ id, party, who ("owner" for the person, or a persona's id),
name, kind (turn | say | steer), text, model, tokens, cost, cut, at }`. The live stream sends
`{ type: "snapshot", party, messages }` first, then `{ type: "party", party }`,
`{ type: "message", message }` and `{ type: "speaking", speaking }`.

### POST /api/parties/{id}/say, pause, resume, stop, caps, delete

`say { text, kind: say | steer }` (a party that waits talks again), `pause` (after the turn
being said), `resume` (an ended one gets 10 more turns and half its token cap again), `stop`
(cuts the turn in flight), `caps { maxTurns?, maxTokens? }`, `delete`: 200
`{ ok: true, party }` (`party: null` after a delete).

## The off-the-record room

A chat with a model that the assistant never sees, kept in `dark.db` beside the server's
database and left out of backups (ADR 0022). A session is needed, and a signed-in device in
either auth mode to write in it or clear it.

### GET /api/dark

`{ model, where, busy, chat: [{ id, role (user | assistant), text, at }] }`.

### POST /api/dark

`{ text }`: the reply as server-sent events, `{ thinking: true }`, `{ delta }` as it is written,
then `{ done: message }` or `{ error }`. 400 `{ error: "Say something." }`; 409
`{ error: "A reply is still being written." }`. A reply the cloud model wrote because the local
one was off starts with a line that says so.

### DELETE /api/dark

Clears it: 200 `{ ok: true }`; 409 while a reply is being written.

## Family members

**Not enabled until scoped tools exist** (ADR 0022): every `/family/...` request is 501
`{ error: "family rooms are not available yet" }`, and the OpenAPI document lists that one
refusal (`GET` and `POST /family/{route}`). What follows is the contract once they are on.

A household server on the same machine passes a family member's requests on with their device
key as the bearer; the server asks it who the key is (`--family-directory-url`, its
`GET /api/whoami`: `{ role: "family", person }`). 401 `{ error }` for any other key, 403 from
another machine. `?room=` picks one of their rooms; none (or another's) is their Home room, made
the first time.

- `GET /family/state`: `{ me, room, rooms: [{ id, name }], chat: [{ id, mine, text, at }],
  working, live, reminders: [{ id, due, text }], theme }`, with an `ETag` (`If-None-Match`:
  304).
- `GET /family/history?before=`: `{ chat }`, the 60 lines before `before`.
- `GET /family/live`: as `GET /api/live`, for their room.
- `POST /family/talk { text }`: `{ ok, id }`; 400 `{ error: "say something" }`.
- `POST /family/devices { token, env }`: their iPhone, kept apart from the owner's; 400
  `{ error: "not a device token" }`.
- `POST /family/seen`: `{ ok, at }`.
- `GET` and `POST /family/theme`: their own theme, as `/api/theme`.

## Not in this contract

Sign in with Apple (`/api/auth/apple`; ADR 0011), and any route this page doesn't list.
