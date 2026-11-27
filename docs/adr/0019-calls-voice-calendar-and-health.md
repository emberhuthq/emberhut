# 0019. Calls, voice, the calendar, and health from the owner's devices

- Status: Proposed
- Date: 2026-11-26

## Context

The server reads a good part of the owner's day from their own devices, and the apps speak the
contract:

- **Calls.** The owner's Mac records a call when asked (their microphone and the Mac's sound as
  two tracks), sends both tracks when it stops, and the server transcribes them on its own
  machine with a local speech-to-text model. The transcript becomes a `call` event in the room it
  is for, so the assistant pulls out decisions and to-dos. A recording shared in the chat goes the
  same way (`call_transcribe`). While a call is recorded, ten-second chunks give a live
  transcript, and a copilot reads it and shows short hints on the Mac.
- **Voice.** The phone holds voice calls with a realtime voice model, with a key the server makes
  per call; the voice model hands anything it can't answer to the assistant, through the server.
  Dictation sends a recording and gets its text.
- **The calendar and reminders**, from every account on the Mac, through the Mac helper: the
  app's Day, Week and Month, and the assistant's tools to read and add.
- **Health and screen time.** The phone sends daily Health summaries, the Mac helper screen-time
  totals; the state shows today's steps, and a real walk closes a walk task.

ADR 0002 sets the server's shape (services with constructor injection, stores in `emberhut-db`,
rules in `emberhut-core`), ADR 0009 the registries, ADR 0012 the Mac helper.

## Decision

### Every outside service behind a trait

| Seam | Real implementation | Default | Configured by |
| --- | --- | --- | --- |
| `CallTranscriber` | `CommandTranscriber`: a program on the server, `<program> <out.md> <title> <file>[=<who>]...` | `NoTranscriber` (the call fails, its audio kept) | `--call-transcriber` / `EMBERHUT_CALL_TRANSCRIBER` |
| `LiveTranscriber` | `ResidentTranscriber`: the same program kept running as `--live`, one JSON line per chunk each way | `NoLiveTranscriber` | the same |
| `Copilot` | none yet: the agent runtime keeps one CLI session per room, and a call's copilot is a conversation of its own (no room, its own instructions, closed with the call); that needs a session kind the runtime doesn't have yet | `NoCopilot` (lines, no hints) | wired when the runtime runs sessions outside rooms |
| `SpeechToText`, `RealtimeVoice` | `OpenAiVoice` | `NoVoice` | `--openai-url`, `--voice-model`, `--transcribe-model`, `--voice`; key file `openai-api-key` in the secrets directory |
| `CalendarProvider` | `HttpMacCalendar`: the Mac helper | `NoCalendar` | the Mac helper's URL and token (ADR 0012) |
| `MacHelper::record` | `HttpMacHelper`: `POST /record/start`, `/record/stop`, `/record/status` | `NotSetUp` | ADR 0012 |
| `CalendarItems`, `WalkTasks` | `PersonalTasks`, over the server's tasks (deadlines, decide-by days, today's plan; the active `Health` project's walk tasks) | `NoItems` (no items), `NoWalkTasks` (no walk closes a task) | the default: a test may name others |

They are grouped in `PersonalDependencies` (one field of `Dependencies`), and their services in
``PersonalServices`, so this area adds one line to each shared list. Tests swap every provider
for a fake; no test reaches the network. The two program seams are tested against small shell
scripts standing in for the program (on Unix only).

- The call transcriber is a program, not a library: a local speech model run through a script lets
  an operator use any local model (or none) without the server linking one. `HF_HUB_OFFLINE=1` keeps
  a model from being downloaded mid-call. One call is transcribed at a time; a restart puts the
  cut-off ones back in the queue.
- Its arguments are by position, with no options: (1) the transcript's path, (2) the call's
  title, (3...) one per track, the audio file's path followed by `=<who>` when the speaker is
  known (`mic.m4a=Me`), which a program splits at its last `=`. The paths are absolute, under
  the data directory; a leading `-` in the title is written as `‐` (U+2010), so no argument
  reads as an option. A data directory whose path contains `=` makes the `<file>=<who>` form
  ambiguous (a track without a speaker reads as one with it), so it should not have one.
- A run has a time limit (`--call-transcribe-minutes` / `EMBERHUT_CALL_TRANSCRIBE_MINUTES`, two
  hours by default): one past it is killed and its call fails, saying so, so a hung model can't
  stall the queue. `call_transcribe` can take it again.
- The live transcriber answers a chunk in 60 s or the chunk fails. A process that let a chunk
  time out after it got ready (or after ten minutes still
  loading its model) is taken as hung and killed, so the next chunk starts a new one rather than
  queueing behind it. A chunk that reaches a process as it exits is never left waiting: the exit
  marks the process stopped and answers the chunks waiting under one lock, and a chunk that finds
  it stopped goes to a new process.
- The OpenAI key is a file read at each call, never a flag, and is never sent to an app; the
  phone gets a key that works for one call, ten minutes. The client is `ureq` on a blocking
  thread (already in the tree with rustls); the dictation form is written by hand.
- The calendar is asked through the same Mac helper as the Mac row, on these paths
  (`GET /calendar?days=&from=&to=`, `GET /reminders`, `POST /calendar/add`,
  `POST /reminders/add`).

### What is stored, and where

- `calls`, `call_lines`, `call_hints`, `calendar_days`, `health`, `screentime` (migration
  `m20261126_000011_calls_calendar_health`). A day's health or screen time, and a calendar event,
  are kept as the device sent them (JSON text): the phone's and the Mac's fields go to the app and
  the tools exactly as they came, and a newer phone's field needs no migration. Lines and hints go
  with their call (a foreign key that cascades).
- A call's files are in `calls/<id>/` beside the database, the folder and files for the server's
  user only, and `calls` is in the backup's `DATA_ENTRIES`. Audio is deleted after
  `--call-audio-days` (14) days, with the chat upload it was copied from; transcripts stay.
- The small values are settings: `healthAt`, `healthPhone`,
  `healthTz`, `healthWalks`, `stepsTarget`, `screentimeAt`, `lastCall`, `calendarFrom`, and the
  tools' last reads (`calendar:<days>`, `reminders`). The assistant's blocks are
  `calendarBlocks`.
- A transcript, a failed transcription, a live transcriber that can't start and a voice call's
  end are events in the room they are for (`call`, `voice`), each with a key so it happens once.
  A copilot's hint is a quiet event line in the chat, which wakes nothing.

### The wire

- Paths, fields and statuses are in api.md. The Mac's recorder signs in as a device, with no
  shared key. What records, spends or stores needs a
  signed-in device in either auth mode, report mode included, since report mode's pass for a
  request without a session is for reading: the Record button (it turns the Mac's microphone
  on), the tracks and live chunks, every voice route (a call's key and dictation are billed to
  the provider's account; a call's request and end speak to the assistant), and the phone's
  health days and the Mac's screen time. Reading the calls, a transcript and the calendar
  passes the session gate as any route does.
- A track or a live chunk is taken only for a call being recorded, or for one the Mac's recorder
  started itself from its menu, whose id is then a new call's (`<UTC day>-<6 lowercase hex
  digits>` of today or yesterday, as the server and the recorder make them); a client can't keep
  audio under ids of its own making or add to a call already transcribed. A track is at most
  256 MiB (six hours of the recorder's 64 kbit/s AAC is about 175 MB), a live chunk 4 MiB (ten
  seconds of its 16-bit mono WAV is under 1 MB).
- Days are calendar days in the server's time zone (the system's, `TimeZone::system()`); the
  phone's health days are in the zone the phone sends (`tz`), UTC until it does. Named zones are
  read from the system's time-zone database (`tzdb-zoneinfo`).
- An event's id is the first 16 hex digits of the SHA-256 of its calendar, title, start and
  end. Ids are only compared within one server.
- A track's speakers are `Me` and `Others`. The label a client gives the person's lines in a voice
  call's transcript (`Owner:`) and the name of the voice session's tool (`ask_assistant`) are wire
  words in `emberhut_api::wire` (`VOICE_PERSON_LABEL`, `VOICE_ASK_TOOL`); `emberhut_api::voice`
  names them `PERSON_LABEL` and `ASK_TOOL`. The labels a transcript's person lines may start with
  are configurable (`--voice-person-label` / `EMBERHUT_VOICE_PERSON_LABELS`, comma-separated); by
  default `Owner:` and `Me:`, so a client needs no owner's name.
- A route that waits on something slow by design (a track arriving, a transcription, a voice
  call's request waiting for its answer, the recorder stopping) is let run past the request
  timeout (`routes::takes_long`).

### What is not wired yet

One feature does not work yet: the live-call copilot (`Copilot`), a trait whose stand-in does
nothing. A call being recorded gets its live transcript, but no hints, and its copilot shows as
off: the copilot is a conversation of its own, with no room, and the agent runtime runs only
room sessions so far.

In detail:

- **Tasks.** A call's `task_id` is kept and named in its event. The calendar's items (the
  deadlines, decide-by days and today's plan: `CalendarItems`) and the walk tasks of the active
  `Health` project (`WalkTasks`) come from the server's tasks (`PersonalTasks`).
- **The assistant's runtime.** The voice call's answer is whatever the room's assistant replies
  to `POST /api/voice/ask`, and a call's follow-up is an event that wakes Main by the wake rule
  (ADR 0014). `NoCopilot` gives no hints until the runtime runs sessions outside rooms.
- **Memory.** The copilot and voice instructions say where the call is; the owner's goals,
  plan, facts, open loops and recall are not in them yet.

## Consequences

- A server without a transcriber, a speech key or a Mac helper still takes everything the
  devices send: calls are kept (and say why they weren't transcribed), health and screen time are
  stored, the calendar shows what was last seen.
- New configuration: `EMBERHUT_CALL_TRANSCRIBER`, `EMBERHUT_CALL_AUDIO_DAYS`,
  `EMBERHUT_CALL_TRANSCRIBE_MINUTES`, `EMBERHUT_OPENAI_URL`,
  `EMBERHUT_VOICE_MODEL`, `EMBERHUT_TRANSCRIBE_MODEL`, `EMBERHUT_VOICE`, `EMBERHUT_VOICE_PERSON_LABELS`, and the secrets file
  `openai-api-key`. The server's image needs the system's time-zone data for a phone's named
  zone.
- The live transcriber and the call transcriber share one program; an operator who has neither
  gets recordings without transcripts, which `call_transcribe` can take again later.
