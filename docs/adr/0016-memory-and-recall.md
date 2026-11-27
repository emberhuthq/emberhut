# 0016. Memory: facts in files, quick notes, learning from corrections, and recall over FTS5 and vectors

- Status: Proposed
- Date: 2026-11-26

## Context

The assistant's memory has five parts, and the apps speak their contract:

- **Facts.** One small markdown file per fact in a context folder, grouped by category
  (`people/ada-lovelace.md`), with a front matter (`title`, `source`, `updated`, and for a
  person's card `now`, `nowAt`, `chats`, `important`, `project`). The assistant keeps them with
  the `facts`, `fact_set`, `fact_drop` and `person_set` tools; the app's "About you" page reads
  them (`GET /api/facts`) and corrects, forgets and marks them (`factSet`, `factDrop`,
  `personSet` in `POST /api/settings`).
- **Quick notes and the diary.** The owner writes short notes from the app, offline first, each
  with its own id (`GET`/`POST /api/notes`); the assistant writes one entry a day
  (`diary_write`) and reads the diary (`notes`, `note_search`). New notes ride along with what
  next wakes the main room's assistant.
- **Learning.** Corrections are logged (`correction_log`, and by the server for a message that
  sounds like one); each lesson is a trial rule for 14 days, kept by the weekly review as a
  standing rule (`lesson_set`) or dropped (`trial_drop`, or `trialDrop` from the app). The
  review reads `corrections` and the chat (`chat_log`).
- **Awareness.** The `context` tool shows the assistant's open loops, notes, routines and quiet
  hours; the state's `contextAt` says when its picture of the owner's chats was last brought up
  to date. Other areas write these (the loops, the scout, the routines).
- **Recall.** One search over everything seen (`recall`): agent session transcripts, the
  chat, tasks, facts, voice notes and the notes, by words (SQLite FTS5, BM25) and by meaning
  (a local embedding model's vectors), fused by rank. Its index needs `docs`, `docs_fts` (FTS5,
  kept in step by triggers), `vecs`, `marks` (how far each source is indexed) and how far each
  transcript is read.

ADR 0002 keeps SQL to the migrations; ADR 0010 already gave the name `files` to the chat's
uploads; ADR 0012 allows one statement outside the schema builder (`VACUUM INTO`) where SeaORM
has no form for it.

## Decision

### Facts stay files

- The folder is `context/` beside the database (already in the backup's `DATA_ENTRIES`), one
  `<category>/<slug>.md` per fact, with the same front matter, so an existing folder is used as
  it is. The rules (reading and writing a file, slugs, one spelling of a name, free ids, a
  person's name cut from a description) are `emberhut_core::memory::facts`; the file work is
  `services/facts.rs` (`FactService`), on blocking threads, every change under one lock.
- The categories are `people`, `work`, `money`, `preferences`, `health`, `home` and
  `projects`.
- The folder keeps no git history of its own: the server runs no `git`, and the backup already
  keeps the folder. An owner who wants a history can make the folder a repository; the server
  only writes files.
- An app change tells the main room's assistant through a `fact` event (the events store).

### Notes, corrections and lessons are rows; days are the server's time zone

- `notes` (one row per note: `uid` unique, `who`, `day`, `at`, `text`, `created`, `updated`)
  and `corrections` (with each lesson's trial: `trial`, `trial_by`, `trial_at`, `rule`) are
  tables of their own migration; the stored words of `who`, `source` and `trial_by` (`owner`,
  `assistant`) and the diary entry's `uid` prefix are named once in
  `emberhut_core::memory::wire`. The standing rules are the `lessons` setting
  (`[{ id, text, at }]`), and the notes the assistant has seen the `notesSeen` setting.
- A note's day is its time's day in the server's time zone (`TimeZone::system()`, so `TZ`
  sets it; a test passes its own through `Dependencies::time_zone`), rather than one fixed
  zone.
- The awareness is read, not owned: `services/awareness.rs` reads the `context`, `routines`
  and `quiet` settings in their shapes, and the areas that keep the loops,
  the scout and the routines write them.
- There are no default routines: they are one installation's own, so `context` answers
  `routines: []` until something sets the `routines` setting.

### Recall: its tables in the server's database, FTS5 through one narrow allowance

- `docs`, `docs_fts`, `vecs` and `marks` are made by the memory migration in the server's own
  database, not a second file: one file to back up and open, and the index is rebuilt from its
  sources if it is ever dropped. The transcripts' read offsets are a table named
  **`transcript_offsets`** (`path`, `offset`, `line`), not `files`, because `files` is the
  chat's uploads (ADR 0010).
- **FTS5 is the one exception to the schema builder.** A virtual table and its triggers have
  no SeaORM form, so the memory migration runs them as statements (`CREATE VIRTUAL TABLE
  docs_fts USING fts5(title, text, content='docs', content_rowid='id', tokenize='porter
  unicode61 remove_diacritics 2')` and the insert, delete and update triggers, which also drop a
  changed piece's vector), and `emberhut-db/src/recall.rs` holds the one query that names it (the
  word search: `MATCH`, ranked by `bm25` with titles weighing twice). Every write to `docs` is an
  ordinary SeaORM one, and the triggers keep the index in step. No other file uses SQL.
- **`docs_fts` is tested in `crates/emberhut-db/tests/stores/memory.rs`**
  (`the_index_finds_words_and_keeps_its_pieces_in_step`: words found stemmed, a changed piece's
  words re-indexed by the update trigger, a shortened or dropped piece's words gone) and end to
  end through the `recall` and `note_search` tools in `crates/emberhut-server/tests/server/memory.rs`,
  including a query full of FTS5 syntax that is searched as plain words. Being a virtual table
  made by a statement, it has no `Table::create()` for a schema check to find.
- Text is indexed in pieces of about 1,500 characters, cut at a paragraph, line or sentence;
  a text that gets shorter loses its later pieces. The chat is indexed from its new messages
  (the `chat` mark), the notes and the facts whole each time (so an edit or a deletion shows),
  and the transcripts from what they gained. A transcript is read at most 64 MiB a pass, in
  8 MiB slices, whole lines only; a line longer than a slice is tool output and skipped; a
  transcript that got shorter was rewritten and is read again from the start. Only what the
  user typed, the agent's answers and compaction summaries are kept
  (`services/transcripts.rs`).
- **Meaning comes from an outside model behind a trait.** `Embedder` has a real
  `HttpEmbedder` for an OpenAI-style `POST <url>/v1/embeddings` (llama.cpp's and Ollama's
  servers serve it), configured with `--embed-url` / `EMBERHUT_EMBED_URL` and optionally
  `--embed-model` / `EMBERHUT_EMBED_MODEL`; `NoEmbedder` is the default, and tests use a
  deterministic fake. Vectors are normalised, stored as little-endian `f32`s, and kept in memory
  once loaded (tens of MB for tens of thousands of pieces). Without a model, or when it doesn't
  answer, recall searches by words alone and says so in its answer; the pieces without a vector
  are embedded when the model answers again.
- Both searches take their best 50 that pass the filters (source, since, until) and are fused
  by reciprocal rank (`1 / (60 + rank)`); the same words said twice show once.
- The index is brought up to date every minute (`RecallService::keep_indexing`, started by
  `serve`), and a search first indexes the chat, notes and facts up to that moment. Agent
  transcripts are indexed only when `--transcripts` / `EMBERHUT_TRANSCRIPTS` names their folder.

### What waits for other areas

- Tasks and voice notes are recall sources too; their areas index them through
  `RecallStore::index` (sources `task` and `voice`) when they land.
- The owner's notes are to stay out of other people's rooms (and their tools out of workers'
  sessions). This server has no person rooms yet, so every room is the owner's;
  `Filter::without_notes` is there for the rooms that will need it.
- The assistant's loop calls `NoteService::take_new` (new notes riding along),
  `LearningService::rules_text` (the rules in its prompt) and `LearningService::notice` (a
  message that sounds like a correction); they are here, tested, for that loop to call.

## Consequences

- An existing context folder, notes and corrections carry over as they are; the recall index
  doesn't need to, as it is rebuilt from its sources.
- The FTS5 table is SQLite's: a move to another database needs its own full-text index. The
  exception is two places, the migration and one query, each saying so.
- The database grows with the index (pieces and vectors), and so does its backup; the
  transcripts themselves are never copied in.
- Meaning search is off until an embedding model is configured, and only as good as the model.
