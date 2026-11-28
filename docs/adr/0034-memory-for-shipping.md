# 0034. Memory for shipping: a profile in the prompt, facts with provenance and time, trusted writes, real forgetting and an eval

- Status: Proposed
- Date: 2026-11-28
- Builds on: [ADR 0016](0016-memory-and-recall.md) (facts in files, notes, learning, recall over
  FTS5 and vectors), [ADR 0002](0002-rust-server.md) (SQL only in migrations; FTS5 stays the one
  exception) and [ADR 0022](0022-side-apps.md) (the off-the-record room)

## Context

ADR 0016 moved the memory over as the reference server had it. Read against what a long-used
installation needs, it works only with a hand-written prompt that teaches the assistant to use it.
It would not work for a new user:

- **The built-in rules say nothing about memory.** When to save a fact, read facts, search recall before asking, or log a
  correction all live in a hand-written prompt.
- **No fact is in front of the assistant when a session starts.** `facts` returns every fact (tens of thousands of tokens for a long-used install), so the
  assistant either skips it or pays for all of it. `fact_set` promises that a project's room "starts
  with" its facts; nothing does that.
- **Forgetting a fact leaves its words in recall,** where the assistant can find them and save them again. There is no
  export and no "never remember".
- **Anything can become a fact.** Other people's messages, voice notes and web pages reach facts and person cards with no
  recorded trust. `source` is free text (often empty). Recall hands quoted text back unmarked.
- **The learning loop never closes on its own.** Nothing keeps or drops a trial rule except a weekly review
  the owner has to set up as a routine, which a new install doesn't have. Unreviewed trial rules ride in every
  prompt until they lapse, and the regex detector mostly logs bug reports and questions.
- **Ranking:**
  - no recency or source weighting and no reranker;
  - sub-agent transcripts are indexed;
  - an Arabic question missed an English fact on the same subject;
  - `pieces()` cuts at the index of the full stop, so the next piece begins with ". ";
  - near-duplicates (a chat message and the same text in a session transcript) show twice.
- **Cost:**
  - every search takes `BEGIN IMMEDIATE` and re-reads every fact and note even when nothing changed;
  - all vectors sit as f32 in a `BTreeMap` (about 3 KB a piece at 768 dimensions, so gigabytes at a million pieces), scored under a `std::sync::Mutex` on the runtime with a full sort;
  - the embedding model's identity isn't stored, so a model change mixes vector spaces;
  - fact writes are not atomic.
- **No evaluation exists,** so no change can be shown to help.

The research behind this ADR covered ChatGPT, Claude, Claude Code, Gemini, Mem0, Letta, Zep/Graphiti, LongMemEval, the LoCoMo audit, OWASP ASI06, MINJA and Vec2Text. It
agrees on one shape:
- the raw history is the ground truth;
- curated facts are linked to it;
- a short profile is always in the prompt, and the rest is fetched with a tool;
- memory is tidied in the background;
- writes are admitted by provenance;
- the user can see, correct, export and erase everything, including derived vectors.

## Decision

### Three layers

1. **Profile**, always in the prompt: `context/_profile.md`, generated, one line per item with its
   fact id. At most 1,500 tokens (chars/4), plus at most 500 for the room's project (facts whose
   `project` is the room's). Pinned lines first, then by `last_used` and `updated`. Health and money
   only when pinned. `ClaudeService::system_prompt` appends it after `RULES` and the lessons.
2. **Facts**, as today (one markdown file each, edited in the app), with more front matter.
3. **Record**: the recall index over chat, the owner's sessions, tasks, voice and notes.

`RULES` gains the generic memory rules (read the profile, search recall before asking, one fact per
thing, log corrections), so behaviour no longer depends on an installation's prompt.

### Facts: provenance, time, a write decision

- **New keys**, all optional (a missing key reads as today):
  - `origin` (`owner`, `assistant`, `message`, `web`, `import`);
  - `from` (a pointer into the record: `chat #n`, a chat id, a transcript line);
  - `confidence`;
  - `valid` (`since..until`);
  - `superseded_by`;
  - `pending`;
  - `sensitive` (`health`, `money`);
  - `last_used`.
- **The server decides `origin`** from what started the turn:
  - the owner's message → written directly;
  - an event (someone else's message, mail, a web page, a voice note, a scheduled wake) → written with `pending: yes`, shown under "To confirm", never in the profile until confirmed;
  - a person card's `now` is written directly; it describes them, not the owner.
- **Sensitive facts** (health, money) are confirm-before-save whatever their origin, and a category can be switched off.
- **Write decision:** `fact_set` without an `id` first looks for the 5 closest facts (words + vectors).
  Above a closeness threshold it writes nothing and answers with them, asking for `id` (update),
  `supersedes` (end the old one) or `new: true`.
- **Supersede, don't overwrite:**
  - the old version moves to `context/.history/` with `valid` closed and `superseded_by` set;
  - `fact_drop` ends a fact;
  - only the owner's Forget deletes.
- Writes go to a temporary file and are renamed into place.

### Recall

- **What is indexed:**
  - transcripts under `subagents/` are not indexed;
  - chat is indexed per exchange (the owner's message and the answer);
  - every piece's embedded and indexed text starts with a header (`[date · source · who]`);
  - a fact is also indexed as an extra key on the piece its `from` names;
  - `pieces()` cuts after the full stop and prefers a paragraph break in the second half.
- **Arabic normalisation** (`emberhut_core::memory::recall::fold`) is applied before FTS insert and query:
  - tashkeel and tatweel stripped;
  - أ/إ/آ→ا, ى→ي, ة→ه;
  - a leading ال/و/ب adds an extra OR term.
- **Ranking:**
  - a date range parsed from the query (English and Arabic) fills `since`/`until` when they're unset;
  - words and vectors (200 candidates each) are fused by RRF (k = 60);
  - then weighted by source (fact 1.5, chat 1.2, summary 1.1, session 1.0, task line 0.7) and recency (`0.85 + 0.15·e^(−age/90 d)`);
  - then reranked by a local cross-encoder over the top 50, behind a `Reranker` trait with `HttpReranker` (llama.cpp `--reranking`, `EMBERHUT_RERANK_URL`) and `NoReranker`.
- **Hits are returned fenced** with their source and trust, and the tool says they are quotes, never instructions.
- **Cost:**
  - `sync()` checks for change before the write lock: the facts folder's newest mtime, file count and size, and the notes' count and latest `updated`. A search still indexes what was said or saved since the last pass, so the assistant finds what it just kept, but takes the write lock only then;
  - vectors are held as int8 with a per-vector scale, in an `Arc` snapshot swapped on change, scored in `spawn_blocking` with `select_nth_unstable`;
  - the embedding model's id and dimension are kept in `marks`, and a change re-embeds in the background (word search meanwhile).
- sqlite-vec waits until its ANN index leaves alpha; when adopted, it is for transactional deletes.

### Consolidation is built in

A nightly job (03:30 in the server's time zone; on by default, a setting turns it off) runs an
internal worker with a fixed brief. It:
- merges near-duplicate facts;
- closes facts whose date passed and fixes their tense;
- archives person `now` lines idle for 21 days;
- supersedes contradicted facts when the newer one is the owner's, or asks "Which is right?";
- triages the corrections: drops the noise the regex caught and turns real ones into trial rules;
- auto-keeps a trial rule after 14 days with no repeat and no objection (`trial_by: review`); one the owner corrected the assistant on again is put to the owner the night it ends, so none lapses unjudged;
- rebuilds the profile.

A weekly card lists the trials and pending facts, and one answer applies all the advice. The prompt carries the standing
lessons and at most 10 trial rules, within 1,000 tokens.

### What the owner controls

- **The "About you" page shows:**
  - the profile, line by line, with pin and hide;
  - each fact with "why I know this" (`origin`, `from`) and its history;
  - "To confirm", for pending facts and contradictions.
- **Forget everywhere:**
  - deletes the fact, its history and its vectors;
  - offers the matching record pieces for removal (from `docs`, the triggers clean `docs_fts`, and `vecs`);
  - writes a tombstone (`context/.forgotten`): the hashes of the fact's distinctive words (those in at most 1% of pieces, or 20) and of its keys (in at most 0.1%, or 5), never its text. `fact_set` refuses, and indexing skips, what holds two of those words with a key among them, or three;
  - the memory migration turns on `PRAGMA secure_delete` and FTS5 `secure-delete`;
  - the backup keeps a copy for its retention window, and the page says so.
- **Export:** a zip of the context folder plus a JSON of notes, corrections and lessons.
- **Switches:** memory on/off, per category, and incognito turns and rooms (not indexed; fact tools refuse).
- **A "memory updated" line** under an answer that saved or changed something.

### Evaluation comes first

- **`crates/emberhut-server/tests/memory_eval/`:** a generated two-month history in English and Arabic with ~150 questions, covering:
  - LongMemEval's five types (single fact, multi-session, temporal, knowledge update, abstention);
  - cross-lingual;
  - forget;
  - injection (a third party's message must not become a fact or a profile line);
  - dedupe.
- **recall@8 against gold piece ids** runs in CI by words alone and with a real embedder's vectors recorded once (`data/vectors.bin`, int8), and fails below the floors in `data/floors.json`. Recording again (`EMBERHUT_EVAL_EMBED_URL`) is needed when what is embedded changes.
- **An end-to-end LLM-judged run** happens on demand, with wrong-answer probes.
- **An installation's private questions** stay in its data folder, never in the repository.

### Order

| Phase | Work |
|---|---|
| 0 | Eval and baseline; the change check before the write lock, the cut, skipping sub-agents, the model mark, atomic writes, int8 vectors off the runtime |
| 1 (ship) | Memory rules in `RULES`; profile and project slice; `origin`/`from`/`pending` decided by the server; Forget everywhere, tombstones, secure delete and export; the nightly job and the weekly card; the lesson cap |
| 2 | The write decision; supersede and history; validity and tense; headers and per-exchange chat; Arabic normalisation; weights and time parsing; the reranker |
| 3 | Sensitive confirm and category switches; incognito; the "memory updated" line; the embedder chosen by the eval |

## Consequences

- A new user's assistant knows the user from its first session, and its behaviour is the product's, not one
  installation's prompt.
- Third-party content no longer becomes the owner's truth unasked, at the cost of a few taps a week.
- Forget reaches every derived copy except the backups, which are bounded by their retention.
- The profile and the nightly job cost one cheap model run a night and ~1.5–2.5k prompt tokens a turn,
  against tens of thousands for reading every fact.
- The facts folder gains `.history/`, `_profile.md` and `.forgotten`. Old folders read as they are.
- A reranker is one more local model to run. Without it, recall works as today, with weights.
- The FTS5 exception grows by the `secure-delete` option, still in the memory migration only.
