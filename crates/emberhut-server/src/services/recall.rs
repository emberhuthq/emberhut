//! Recall: one search over everything the assistant has seen (`emberhut_core::memory::recall`),
//! so what never became a fact can still be found, in other words than the ones used then.
//!
//! Indexed: the chat with the assistant (new messages only), the owner's quick notes and the
//! assistant's diary, the facts (both whole each time, so an edit or a deletion shows), and,
//! when a folder of agent transcripts is configured, what was said in those sessions and their
//! compaction summaries ([`super::transcripts`]). Two searches are fused: words (SQLite FTS5,
//! BM25) and meaning (the nearest vectors of an outside embedding model, behind [`Embedder`]).
//! Without a model it searches by words alone. ADR 0016 has the reasons.
//!
//! The index is brought up to date every minute ([`RecallService::keep_indexing`]): the chat,
//! notes and facts, the transcripts that grew (each read at most 64 MiB a minute, in 8 MiB
//! slices, whole lines only), and the vectors of the pieces that changed. A search first
//! indexes the chat, notes and facts up to that moment.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use emberhut_core::RoomId;
use emberhut_core::memory::notes::{NoteAuthor, hour_minute};
use emberhut_core::memory::recall::{
    EACH, Filter, Found, NewDoc, closeness, embed_query, embed_text, fuse, match_query, normalised,
    query_words, sameness, source,
};
use emberhut_db::{
    ConversationStore, DbError, IndexBatch, NoteStore, RecallStore, TranscriptOffset,
};
use futures_util::future::BoxFuture;
use jiff::tz::TimeZone;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};
use tokio_util::sync::CancellationToken;

use super::facts::{FactError, FactService};
use super::transcripts::{Role, may_say, place, said};
use crate::http::read_body;

/// How often the index is brought up to date.
pub const RECALL_EVERY: Duration = Duration::from_secs(60);

/// The mark under which the chat is indexed: the last message's id.
const CHAT_MARK: &str = "chat";
/// How far the tasks are indexed: the last change.
const TASKS_MARK: &str = "issues";
/// How far the tasks' history is indexed: its last line.
const LINES_MARK: &str = "events";
/// How far the voice notes are indexed: the last one transcribed.
const VOICE_MARK: &str = "voice";

/// Messages indexed per transaction.
const CHAT_BATCH: u64 = 500;

/// A transcript is read in slices this big.
pub const SLICE_BYTES: usize = 8 << 20;

/// At most this much of one transcript is read per pass.
pub const MAX_PER_PASS: u64 = 64 << 20;

/// Pieces embedded per call to the model.
const EMBED_BATCH: u64 = 32;

/// The longest embeddings answer read: a batch's vectors take a few megabytes at most.
const ANSWER_LIMIT: usize = 64 << 20;

/// How many of the word search's best are read before the filters.
const WORD_CANDIDATES: u64 = 200;

/// An outside embedding model: a vector for each text, in order. A fake answers in tests.
pub trait Embedder: fmt::Debug + Send + Sync {
    /// The texts' vectors, in order (not yet normalised).
    fn embed<'a>(&'a self, texts: &'a [String])
    -> BoxFuture<'a, Result<Vec<Vec<f32>>, EmbedError>>;
}

/// Why the model gave no vectors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EmbedError {
    /// No model is configured.
    #[error("no embedding model is configured")]
    NotConfigured,
    /// The model could not be reached.
    #[error("the embedding model could not be reached")]
    Unreachable(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The model's answer was not vectors for the texts.
    #[error("the embedding model answered something else: {0}")]
    Answer(String),
}

/// No model: recall searches by words alone.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoEmbedder;

impl Embedder for NoEmbedder {
    fn embed<'a>(
        &'a self,
        _texts: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<Vec<f32>>, EmbedError>> {
        Box::pin(async { Err(EmbedError::NotConfigured) })
    }
}

/// A model behind an OpenAI-style embeddings endpoint (`POST <url>/v1/embeddings` with
/// `{ input: [...], model? }`, answering `{ data: [{ embedding: [...] }, ...] }`), such as a
/// local llama.cpp or Ollama server.
pub struct HttpEmbedder {
    client: reqwest::Client,
    url: String,
    model: Option<String>,
}

impl fmt::Debug for HttpEmbedder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpEmbedder")
            .field("url", &self.url)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl HttpEmbedder {
    /// The model at `url` (`http://127.0.0.1:8795`), asked for `model` when given, reached
    /// with `client`.
    #[must_use]
    pub fn new(client: reqwest::Client, url: &str, model: Option<String>) -> Self {
        Self {
            client,
            url: url.trim_end_matches('/').to_owned(),
            model,
        }
    }
}

impl Embedder for HttpEmbedder {
    fn embed<'a>(
        &'a self,
        texts: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<Vec<f32>>, EmbedError>> {
        Box::pin(async move {
            let mut body = json!({ "input": texts });
            if let Some(model) = &self.model {
                body["model"] = Value::from(model.as_str());
            }
            let response = self
                .client
                .post(format!("{}/v1/embeddings", self.url))
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.to_string())
                .timeout(Duration::from_secs(120))
                .send()
                .await
                .map_err(|error| EmbedError::Unreachable(Box::new(error)))?;
            let status = response.status();
            let body = read_body(response, ANSWER_LIMIT)
                .await
                .map_err(|error| EmbedError::Unreachable(Box::new(error)))?;
            if !status.is_success() {
                return Err(EmbedError::Answer(status.to_string()));
            }
            if body.cut {
                return Err(EmbedError::Answer("an answer over 64 MiB".to_owned()));
            }
            let answer: Value = serde_json::from_slice(&body.bytes)
                .map_err(|error| EmbedError::Answer(error.to_string()))?;
            vectors_of(&answer, texts.len())
        })
    }
}

/// The vectors in an embeddings answer, one per text.
fn vectors_of(answer: &Value, count: usize) -> Result<Vec<Vec<f32>>, EmbedError> {
    let data = answer
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| EmbedError::Answer("no data".to_owned()))?;
    let vectors: Vec<Vec<f32>> = data
        .iter()
        .map(|item| {
            item.get("embedding")
                .and_then(Value::as_array)
                .map_or_default(|parts| {
                    parts
                        .iter()
                        .map(|part| {
                            #[expect(
                                clippy::cast_possible_truncation,
                                reason = "embeddings are f32s sent as JSON numbers"
                            )]
                            let part = part.as_f64().unwrap_or(0.0) as f32;
                            part
                        })
                        .collect()
                })
        })
        .collect();
    if vectors.len() == count {
        Ok(vectors)
    } else {
        Err(EmbedError::Answer(format!(
            "{} vectors for {count} texts",
            vectors.len()
        )))
    }
}

/// Why recall could not index or search.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RecallError {
    /// The database failed.
    #[error(transparent)]
    Database(#[from] DbError),
    /// The facts could not be read.
    #[error(transparent)]
    Facts(#[from] FactError),
    /// A transcript could not be read.
    #[error("a transcript could not be read")]
    Transcript(#[source] std::io::Error),
}

/// What recall is built from.
#[derive(Clone, Debug)]
pub struct RecallParts {
    /// The index.
    pub store: RecallStore,
    /// The chat, read back.
    pub conversation: ConversationStore,
    /// The notes.
    pub notes: NoteStore,
    /// The facts.
    pub facts: FactService,
    /// The tasks and their history, indexed as `task`.
    pub tasks: emberhut_db::TaskStore,
    /// The voice notes people sent, indexed as `voice` once transcribed.
    pub voice: emberhut_db::VoiceNoteStore,
    /// The embedding model.
    pub embedder: Arc<dyn Embedder>,
    /// The folder of agent transcripts (`.jsonl` files, in folders of their own), if any.
    pub transcripts: Option<PathBuf>,
    /// The time zone the notes' times are shown in.
    pub zone: TimeZone,
}

/// Indexes and searches everything said.
#[derive(Clone, Debug)]
pub struct RecallService {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    parts: RecallParts,
    /// One indexing pass at a time.
    syncing: tokio::sync::Mutex<()>,
    /// Every vector, by piece id, once loaded.
    vectors: Mutex<Option<BTreeMap<i64, Vec<f32>>>>,
    /// A piece changed since the vectors were last brought up to date.
    changed: AtomicBool,
}

impl RecallService {
    /// Recall over `parts`.
    #[must_use]
    pub fn new(parts: RecallParts) -> Self {
        Self {
            inner: Arc::new(Inner {
                parts,
                syncing: tokio::sync::Mutex::new(()),
                vectors: Mutex::new(None),
                changed: AtomicBool::new(true),
            }),
        }
    }

    fn store(&self) -> &RecallStore {
        &self.inner.parts.store
    }

    /// Indexes the chat (its new messages), the notes and the facts up to now.
    ///
    /// # Errors
    ///
    /// [`RecallError`].
    pub async fn sync(&self) -> Result<(), RecallError> {
        let _one = self.inner.syncing.lock().await;
        self.sync_chat().await?;
        self.sync_tasks().await?;
        self.sync_task_lines().await?;
        self.sync_voice().await?;
        let parts = &self.inner.parts;
        let notes = parts.notes.list(None, None).await?;
        let mut docs = Vec::with_capacity(notes.len());
        let mut kept = HashSet::new();
        for note in notes {
            let reference = format!("note:{}", note.uid);
            kept.insert(reference.clone());
            let title = match note.who {
                NoteAuthor::Assistant => format!("The assistant's diary entry for {}", note.day),
                NoteAuthor::Owner => format!(
                    "Quick note, {} {}",
                    note.day,
                    hour_minute(note.at, &parts.zone)
                ),
            };
            docs.push(NewDoc {
                source: source::NOTE.to_owned(),
                reference,
                at: Some(note.at),
                title,
                place: format!("note {} (notes tool)", note.day),
                text: note.text,
            });
        }
        let facts = parts.facts.facts(None).await?;
        let mut fact_ids = HashSet::new();
        for fact in facts {
            fact_ids.insert(fact.id.clone());
            let text = [Some(fact.text.as_str()), fact.now.as_deref()]
                .into_iter()
                .flatten()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            docs.push(NewDoc {
                source: source::FACT.to_owned(),
                at: fact.updated.parse().ok(),
                title: format!("{}: {}", fact.category, fact.title),
                place: format!("fact {}", fact.id),
                reference: fact.id,
                text,
            });
        }
        let batch = IndexBatch {
            docs,
            only: vec![
                (source::NOTE.to_owned(), kept),
                (source::FACT.to_owned(), fact_ids),
            ],
            marks: Vec::new(),
        };
        if self.store().index(&batch).await? {
            self.inner.changed.store(true, Ordering::Release);
        }
        Ok(())
    }

    /// The instant mark `key` holds, if it holds one.
    async fn instant_mark(&self, key: &str) -> Result<Option<jiff::Timestamp>, RecallError> {
        Ok(self
            .store()
            .mark(key)
            .await?
            .and_then(|mark| mark.parse::<jiff::Timestamp>().ok()))
    }

    /// Indexes `docs`, and moves mark `key` to `mark`.
    async fn put(&self, docs: Vec<NewDoc>, key: &str, mark: String) -> Result<(), RecallError> {
        let batch = IndexBatch {
            docs,
            only: Vec::new(),
            marks: vec![(key.to_owned(), mark)],
        };
        if self.store().index(&batch).await? {
            self.inner.changed.store(true, Ordering::Release);
        }
        Ok(())
    }

    /// Indexes the tasks changed since they were last indexed: each one's title, status and
    /// brief, as `task`.
    async fn sync_tasks(&self) -> Result<(), RecallError> {
        loop {
            let after = self.instant_mark(TASKS_MARK).await?;
            let tasks = self
                .inner
                .parts
                .tasks
                .changed_after(after, CHAT_BATCH)
                .await?;
            let Some(last) = tasks.last().map(|task| task.updated) else {
                return Ok(());
            };
            let docs = tasks
                .into_iter()
                .map(|task| {
                    let key = task.key();
                    NewDoc {
                        source: source::TASK.to_owned(),
                        reference: key.clone(),
                        at: Some(task.updated),
                        title: format!("{key} {} ({})", task.title, task.status.as_str()),
                        place: format!("task {key}"),
                        text: format!("{}\n{}", task.title, task.body.unwrap_or_default()),
                    }
                })
                .collect();
            self.put(docs, TASKS_MARK, emberhut_core::memory::iso(last))
                .await?;
        }
    }

    /// Indexes the lines written in the tasks' history since it was last indexed, as `task`.
    async fn sync_task_lines(&self) -> Result<(), RecallError> {
        loop {
            let after = self
                .store()
                .mark(LINES_MARK)
                .await?
                .and_then(|mark| mark.parse::<i64>().ok())
                .unwrap_or(0);
            let lines = self
                .inner
                .parts
                .tasks
                .lines_after(after, CHAT_BATCH)
                .await?;
            let Some(last) = lines.last().map(|line| line.id) else {
                return Ok(());
            };
            let docs = lines
                .into_iter()
                .map(|line| {
                    let key = line.task.key();
                    NewDoc {
                        source: source::TASK.to_owned(),
                        reference: format!("{key}@{}", line.id),
                        at: Some(line.line.at),
                        title: format!("{key} {} · {}", line.title, line.line.who.as_str()),
                        place: format!("task {key}"),
                        text: line.line.text,
                    }
                })
                .collect();
            self.put(docs, LINES_MARK, last.to_string()).await?;
        }
    }

    /// Indexes the voice notes transcribed since they were last indexed, as what was said, as
    /// `voice`.
    async fn sync_voice(&self) -> Result<(), RecallError> {
        loop {
            let after = self.instant_mark(VOICE_MARK).await?;
            let notes = self.inner.parts.voice.done_after(after, CHAT_BATCH).await?;
            let Some(last) = notes.last().and_then(|note| note.done_at) else {
                return Ok(());
            };
            let docs = notes.into_iter().map(voice_doc).collect();
            self.put(docs, VOICE_MARK, emberhut_core::memory::iso(last))
                .await?;
        }
    }

    /// Indexes the messages said since the chat was last indexed.
    async fn sync_chat(&self) -> Result<(), RecallError> {
        loop {
            let after = self
                .store()
                .mark(CHAT_MARK)
                .await?
                .and_then(|mark| mark.parse::<i64>().ok())
                .map(emberhut_core::MessageId::new);
            let said = self
                .inner
                .parts
                .conversation
                .said_after(after, CHAT_BATCH)
                .await?;
            let Some(last) = said.last().map(|message| message.id) else {
                return Ok(());
            };
            let docs = said
                .into_iter()
                .map(|message| {
                    let who = if message.mine {
                        "User to assistant"
                    } else {
                        "Assistant to user"
                    };
                    let room = if message.room == RoomId::MAIN {
                        String::new()
                    } else {
                        format!(" ({} room)", message.room_name)
                    };
                    NewDoc {
                        source: source::CHAT.to_owned(),
                        reference: format!("chat:{}", message.id.get()),
                        at: Some(message.at),
                        title: format!("{who}{room}"),
                        place: format!("chat #{}", message.id.get()),
                        text: message.text,
                    }
                })
                .collect();
            let batch = IndexBatch {
                docs,
                only: Vec::new(),
                marks: vec![(CHAT_MARK.to_owned(), last.get().to_string())],
            };
            if self.store().index(&batch).await? {
                self.inner.changed.store(true, Ordering::Release);
            }
        }
    }

    /// Reads every transcript that grew since it was last read (none without a folder). One
    /// that fails is logged and left for the next pass.
    ///
    /// # Errors
    ///
    /// [`RecallError::Database`].
    pub async fn read_transcripts(&self) -> Result<(), RecallError> {
        let Some(root) = self.inner.parts.transcripts.clone() else {
            return Ok(());
        };
        let files = tokio::task::spawn_blocking(move || {
            let mut files = Vec::new();
            walk(&root, &root, &mut files);
            files
        })
        .await
        .unwrap_or_default();
        let read = self.store().transcript_offsets().await?;
        for (path, folder, size) in files {
            let key = path.to_string_lossy().into_owned();
            if read.get(&key).is_some_and(|known| known.offset == size) {
                continue;
            }
            let _one = self.inner.syncing.lock().await;
            if let Err(error) = self.read_transcript(&path, &key, &folder, size).await {
                self.inner.changed.store(true, Ordering::Release);
                tracing::warn!(path = %key, error = %error, "recall: a transcript was not read");
            }
        }
        Ok(())
    }

    /// Reads the new whole lines of the transcript at `path` (`size` bytes long now).
    async fn read_transcript(
        &self,
        path: &Path,
        key: &str,
        folder: &str,
        size: u64,
    ) -> Result<(), RecallError> {
        let store = self.store();
        let mut at = store.transcript_offset(key).await?.unwrap_or_default();
        if size < at.offset {
            store.forget_transcript(key).await?;
            at = TranscriptOffset::default();
        }
        if size == at.offset {
            return Ok(());
        }
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(RecallError::Transcript)?;
        let mut slice = vec![0_u8; SLICE_BYTES];
        let mut budget = MAX_PER_PASS;
        let place_name = place(folder);
        while at.offset < size && budget > 0 {
            let n = read_at(&mut file, &mut slice, at.offset, size).await?;
            if n == 0 {
                break;
            }
            budget = budget.saturating_sub(n as u64);
            let Some(end) = slice[..n].iter().rposition(|byte| *byte == b'\n') else {
                if n < slice.len() {
                    break; // the last line, still being written
                }
                // A line over a slice is tool output: skipped to its end, and its place saved.
                let Some(after) =
                    line_end(&mut file, &mut slice, at.offset + n as u64, size).await?
                else {
                    break;
                };
                budget = budget.saturating_sub(after - at.offset - n as u64);
                at = TranscriptOffset {
                    offset: after,
                    line: at.line + 1,
                };
                store.read_transcript(key, at, &[]).await?;
                continue;
            };
            let mut docs = Vec::new();
            let mut line = at.line;
            for raw in slice[..end].split(|byte| *byte == b'\n') {
                line += 1;
                if !may_say(raw) {
                    continue;
                }
                let Ok(value) = serde_json::from_slice::<Value>(raw) else {
                    continue;
                };
                let Some((role, text)) = said(&value) else {
                    continue;
                };
                let reference = format!("{key}:{line}");
                docs.push(NewDoc {
                    source: if role == Role::Summary {
                        source::SUMMARY
                    } else {
                        source::SESSION
                    }
                    .to_owned(),
                    at: value
                        .get("timestamp")
                        .and_then(Value::as_str)
                        .and_then(|at| at.parse().ok()),
                    title: format!("{} · {place_name}", role.as_str()),
                    place: reference.clone(),
                    reference,
                    text,
                });
            }
            at = TranscriptOffset {
                offset: at.offset + end as u64 + 1,
                line,
            };
            if store.read_transcript(key, at, &docs).await? {
                self.inner.changed.store(true, Ordering::Release);
            }
        }
        Ok(())
    }

    /// Embeds the pieces with no vector yet; `true` when every piece has one now (`false`: the
    /// model didn't answer, or there is none).
    ///
    /// # Errors
    ///
    /// [`RecallError::Database`].
    pub async fn embed_new(&self) -> Result<bool, RecallError> {
        self.load_vectors().await?;
        loop {
            let batch = self.store().unembedded(EMBED_BATCH).await?;
            if batch.is_empty() {
                return Ok(true);
            }
            let texts: Vec<String> = batch
                .iter()
                .map(|doc| embed_text(&doc.title, &doc.text))
                .collect();
            let Ok(vectors) = self.inner.parts.embedder.embed(&texts).await else {
                return Ok(false);
            };
            let vectors: Vec<(i64, Vec<f32>)> = batch
                .iter()
                .zip(vectors)
                .map(|(doc, vector)| (doc.id, normalised(vector)))
                .collect();
            self.store().store_vectors(&vectors).await?;
            let mut cache = self
                .inner
                .vectors
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(cache) = cache.as_mut() {
                cache.extend(vectors);
            }
        }
    }

    /// Loads every vector, once.
    async fn load_vectors(&self) -> Result<(), RecallError> {
        let loaded = self
            .inner
            .vectors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some();
        if !loaded {
            let all: BTreeMap<i64, Vec<f32>> = self.store().vectors().await?.into_iter().collect();
            *self
                .inner
                .vectors
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(all);
        }
        Ok(())
    }

    /// Forgets the vectors whose pieces changed or went (their rows are gone with them).
    async fn prune_vectors(&self) -> Result<(), RecallError> {
        let loaded = self
            .inner
            .vectors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some();
        if loaded {
            let live = self.store().vector_ids().await?;
            if let Some(cache) = self
                .inner
                .vectors
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_mut()
            {
                cache.retain(|id, _| live.contains(id));
            }
        }
        Ok(())
    }

    /// One indexing pass: the chat, notes and facts, the transcripts, and the new vectors.
    ///
    /// # Errors
    ///
    /// [`RecallError`]; what failed is tried again next pass.
    pub async fn index(&self) -> Result<(), RecallError> {
        let done = async {
            self.sync().await?;
            self.read_transcripts().await?;
            if self.inner.changed.swap(false, Ordering::AcqRel) {
                self.prune_vectors().await?;
                if !self.embed_new().await? {
                    self.inner.changed.store(true, Ordering::Release);
                }
            }
            Ok(())
        }
        .await;
        if done.is_err() {
            self.inner.changed.store(true, Ordering::Release);
        }
        done
    }

    /// Indexes every `every` until `stop`, starting a few seconds in.
    pub async fn keep_indexing(self, every: Duration, stop: CancellationToken) {
        let mut ticks =
            tokio::time::interval_at(tokio::time::Instant::now() + Duration::from_secs(5), every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                () = stop.cancelled() => return,
                _ = ticks.tick() => {
                    if let Err(error) = self.index().await {
                        tracing::warn!(error = %error, "recall: the index was not brought up to date");
                    }
                }
            }
        }
    }

    /// Searches for `query`: at most `limit` hits that pass `filter`, best first, the same
    /// words said twice shown once. The chat, notes and facts are indexed up to now first.
    ///
    /// # Errors
    ///
    /// [`RecallError::Database`].
    pub async fn search(
        &self,
        query: &str,
        limit: usize,
        filter: &Filter,
    ) -> Result<Found, RecallError> {
        if let Err(error) = self.sync().await {
            tracing::warn!(error = %error, "recall: searching the index as it is");
        }
        let mut lists = Vec::new();
        let words = query_words(query);
        if !words.is_empty() {
            let found = self
                .store()
                .search_words(&match_query(&words), WORD_CANDIDATES)
                .await?;
            lists.push(
                found
                    .iter()
                    .filter(|doc| filter.keeps(doc))
                    .take(EACH)
                    .map(|doc| doc.id)
                    .collect(),
            );
        }
        let asked = vec![embed_query(query)];
        let query_vector = self
            .inner
            .parts
            .embedder
            .embed(&asked)
            .await
            .ok()
            .and_then(|vectors| vectors.into_iter().next())
            .map(normalised);
        let meaning = query_vector.is_some();
        if let Some(query_vector) = query_vector {
            self.load_vectors().await?;
            let mut near: Vec<(i64, f64)> = self
                .inner
                .vectors
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .flatten()
                .map(|(id, vector)| (*id, closeness(vector, &query_vector)))
                .collect();
            near.sort_by(|a, b| b.1.total_cmp(&a.1));
            let mut ids = Vec::new();
            for chunk in near.chunks(200) {
                if ids.len() >= EACH {
                    break;
                }
                let chunk: Vec<i64> = chunk.iter().map(|(id, _)| *id).collect();
                let docs = self.store().docs(&chunk).await?;
                for id in chunk {
                    if ids.len() >= EACH {
                        break;
                    }
                    if docs.get(&id).is_some_and(|doc| filter.keeps(doc)) {
                        ids.push(id);
                    }
                }
            }
            lists.push(ids);
        }
        let ranked = fuse(&lists);
        let docs = self.store().docs(&ranked).await?;
        let mut seen = HashSet::new();
        let mut hits = Vec::new();
        for id in ranked {
            if hits.len() >= limit {
                break;
            }
            let Some(doc) = docs.get(&id) else {
                continue; // dropped since
            };
            if seen.insert(sameness(&doc.text)) {
                hits.push(doc.clone());
            }
        }
        let embedded = self
            .inner
            .vectors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map_or(0, BTreeMap::len);
        Ok(Found {
            hits,
            meaning,
            indexed: self.store().count().await?,
            embedded,
        })
    }
}

/// Every `.jsonl` file under `dir` (`root`'s), with the first folder under `root` it is in and
/// its size, folders before what is in them, each folder in name order.
fn walk(root: &Path, dir: &Path, found: &mut Vec<(PathBuf, String, u64)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    let mut folders = Vec::new();
    for entry in entries {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            folders.push(path);
        } else if kind.is_file() && path.extension().is_some_and(|ext| ext == "jsonl") {
            let folder = path
                .strip_prefix(root)
                .ok()
                .and_then(|relative| relative.components().next())
                .map_or_default(|first| first.as_os_str().to_string_lossy().into_owned());
            // Read from the file, not the folder's listing: on Windows the listing's size of a
            // file still open for writing (a transcript being written) lags behind.
            let size = std::fs::symlink_metadata(&path).map_or(0, |meta| meta.len());
            found.push((path, folder, size));
        }
    }
    for folder in folders {
        walk(root, &folder, found);
    }
}

/// Reads up to a slice of `file` from `offset` (not past `size`); how much was read.
async fn read_at(
    file: &mut tokio::fs::File,
    slice: &mut [u8],
    offset: u64,
    size: u64,
) -> Result<usize, RecallError> {
    let want = usize::try_from(size - offset).map_or(slice.len(), |left| left.min(slice.len()));
    file.seek(std::io::SeekFrom::Start(offset))
        .await
        .map_err(RecallError::Transcript)?;
    let mut read = 0;
    while read < want {
        let n = file
            .read(&mut slice[read..want])
            .await
            .map_err(RecallError::Transcript)?;
        if n == 0 {
            break;
        }
        read += n;
    }
    Ok(read)
}

/// Where the line going on at `from` ends (just after its newline); none when its end isn't
/// written yet.
async fn line_end(
    file: &mut tokio::fs::File,
    slice: &mut [u8],
    mut from: u64,
    size: u64,
) -> Result<Option<u64>, RecallError> {
    while from < size {
        let n = read_at(file, slice, from, size).await?;
        if n == 0 {
            break;
        }
        if let Some(newline) = slice[..n].iter().position(|byte| *byte == b'\n') {
            return Ok(Some(from + newline as u64 + 1));
        }
        from += n as u64;
    }
    Ok(None)
}

/// A transcribed voice note as recall keeps it: who sent it, where, and what they said.
fn voice_doc(note: emberhut_core::messaging::voice::VoiceNote) -> NewDoc {
    use emberhut_core::messaging::voice::VoiceSource;
    let from = match note.source {
        VoiceSource::Slack => "Slack",
        VoiceSource::WhatsApp => "WhatsApp",
        VoiceSource::IMessage => "iMessage",
    };
    let sender = note.sender.clone().unwrap_or_else(|| "someone".to_owned());
    let chat = note.chat_name.clone().filter(|chat| !chat.is_empty());
    let within = chat
        .as_ref()
        .filter(|chat| **chat != sender)
        .map_or_else(String::new, |chat| format!(" in {chat}"));
    let place = chat
        .as_ref()
        .map_or_else(String::new, |chat| format!(" ({from}: {chat})"));
    NewDoc {
        source: source::VOICE.to_owned(),
        reference: note.id.clone(),
        at: note.msg_at.or(note.done_at),
        title: format!("Voice note from {sender}{within} ({from})"),
        place: format!("voice note {}{place}", note.id),
        text: note.text.unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_embeddings_answer_has_a_vector_per_text() {
        let answer = json!({ "data": [ { "embedding": [1.0, 2.0] }, { "embedding": [0.5] } ] });
        assert_eq!(
            vectors_of(&answer, 2).unwrap(),
            vec![vec![1.0, 2.0], vec![0.5]]
        );
        assert!(vectors_of(&answer, 3).is_err());
        assert!(vectors_of(&json!({ "error": "x" }), 1).is_err());
    }
}
