//! A call as it happens: while the Mac records, it sends each track in chunks of about ten
//! seconds (`POST /api/calls/live`); a resident speech-to-text process turns them into lines,
//! kept as the call's rolling transcript (the owner's lines and the other side's). A copilot
//! reads the new lines every 45 seconds or so and answers only when it adds something (a fact
//! or a number, a promise or a deadline, a sharp question, a correction); its hints show in
//! the Mac's panel (`GET /api/calls/{id}/live`) and as a quiet line in the call's room. The
//! final transcript doesn't depend on any of this ([`crate::services::calls`]).
//!
//! Two seams: [`LiveTranscriber`] hears a chunk ([`ResidentTranscriber`] keeps the configured
//! program running with `--live` and speaks JSON lines with it; [`NoLiveTranscriber`] stands in
//! when none is set up), and [`Copilot`] gives the hints ([`NoCopilot`] until an agent runtime
//! provides one). Tests swap in fakes.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::body::Bytes;
use emberhut_api::calls::{CallLive, ChunkQuery, LiveCallHint, LiveCallLine};
use emberhut_core::calls::{
    CallId, HeardLine, LiveLine, TrackKind, clock, cut, hint_of, last_chars, unecho,
};
use emberhut_core::event::NewEvent;
use emberhut_core::message::{Author, NewMessage};
use emberhut_core::{Clock, RoomId};
use emberhut_db::{CallStore, DbError, EventStore, MessageStore, RoomStore};
use futures_util::Stream;
use futures_util::future::BoxFuture;
use jiff::{SignedDuration, Timestamp};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::{oneshot, watch};
use tokio_util::sync::{CancellationToken, DropGuard};

use crate::convert::timestamp_to_api;
use crate::services::calls::{CallError, make_private_dir, recording, remove_file, write_private};

/// How long a chunk waits for the transcriber's answer.
const HEAR_WAIT: Duration = Duration::from_secs(60);
/// How long a transcriber may take to load its model before a chunk it doesn't answer has it
/// stopped as hung.
const START_WAIT: Duration = Duration::from_secs(600);
/// The longest chunk taken: 4 MiB. The Mac's recorder sends about ten seconds of 16-bit mono
/// WAV at 48 kHz, under 1 MB.
const MAX_CHUNK: u64 = 4 << 20;
/// How often the copilot looks at the calls.
const TICK_EVERY: Duration = Duration::from_secs(15);
/// A call whose last line is older than this is not live any more.
const LIVE_FOR: SignedDuration = SignedDuration::from_mins(10);
/// The time between two of a copilot's turns.
const TURN_EVERY: SignedDuration = SignedDuration::from_secs(45);
/// A turn with fewer new words than this waits for more, up to [`WAIT_FOR_WORDS`].
const ENOUGH_WORDS: usize = 15;
/// How long a turn waits for enough new words.
const WAIT_FOR_WORDS: SignedDuration = SignedDuration::from_mins(2);
/// How long a copilot's turn may take.
const TURN_WAIT: Duration = Duration::from_secs(60);
/// The longest a hint is kept, in characters.
const HINT_LENGTH: usize = 300;
/// The copilot's state once the owner turned it off on the Mac.
const OFF_ON_THE_MAC: &str = "off: turned off on the Mac";

/// A chunk to hear: its file, the language most of the call is in, and the last words heard
/// (so a sentence cut between two chunks reads on).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    /// The chunk's audio (WAV).
    pub path: PathBuf,
    /// The call's language so far, when known.
    pub lang: Option<String>,
    /// The end of what was heard before it.
    pub prompt: String,
}

/// What a chunk said.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Heard {
    /// The words; none when nothing was said.
    pub text: Option<String>,
    /// Their language.
    pub lang: Option<String>,
    /// The transcriber was sure of the language.
    pub sure: bool,
}

/// Why a chunk was not heard.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LiveError {
    /// The transcriber can't start (no model, not set up): said once per call in its room.
    #[error("the live transcriber can't start: {0}")]
    NoStart(String),
    /// This chunk failed.
    #[error("{0}")]
    Failed(String),
}

/// What hears a call's chunks.
pub trait LiveTranscriber: fmt::Debug + Send + Sync {
    /// Hears `chunk`.
    fn hear(&self, chunk: Chunk) -> BoxFuture<'_, Result<Heard, LiveError>>;
}

/// No live transcriber set up.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoLiveTranscriber;

impl LiveTranscriber for NoLiveTranscriber {
    fn hear(&self, _chunk: Chunk) -> BoxFuture<'_, Result<Heard, LiveError>> {
        Box::pin(async {
            Err(LiveError::NoStart(
                "no live transcriber is set up on this server".to_owned(),
            ))
        })
    }
}

/// The running transcriber process: its input, the chunks waiting for an answer, and how it is.
/// Dropping it stops the process.
#[derive(Debug)]
struct Worker {
    stdin: tokio::process::ChildStdin,
    waiting: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    exited: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    started: tokio::time::Instant,
    _stop: DropGuard,
}

/// The configured program, kept running as `<program> --live` (with `HF_HUB_OFFLINE=1`), one
/// JSON line per chunk on its input (`{ id, path, lang, prompt }`) and one per answer on its
/// output (`{ id, text, lang, sure }` or `{ id, error }`; `{ ready }` once its model is loaded).
/// A process that stops answers every waiting chunk with an error (and, when it never got
/// ready, why it couldn't start); the next chunk starts a new one. A chunk with no answer in
/// 60 s fails; when the process was ready (or has been loading for ten minutes) it is taken as
/// hung and stopped, so the next chunk starts a new one too.
#[derive(Debug)]
pub struct ResidentTranscriber {
    program: PathBuf,
    worker: tokio::sync::Mutex<Option<Worker>>,
    next_id: AtomicU64,
    hear_wait: Duration,
    start_wait: Duration,
}

impl ResidentTranscriber {
    /// The transcriber that keeps `program` running.
    #[must_use]
    pub fn new(program: PathBuf) -> Self {
        Self::with_waits(program, HEAR_WAIT, START_WAIT)
    }

    /// The transcriber that keeps `program` running, a chunk waiting `hear_wait` for its
    /// answer, and a process loading for `start_wait` before it may be taken as hung.
    fn with_waits(program: PathBuf, hear_wait: Duration, start_wait: Duration) -> Self {
        Self {
            program,
            worker: tokio::sync::Mutex::new(None),
            next_id: AtomicU64::new(1),
            hear_wait,
            start_wait,
        }
    }

    /// Stops the worker whose waiting chunks are `waiting`, when it is still the running one
    /// and is hung: ready (its model loaded) or loading for longer than it may.
    async fn stop_if_hung(&self, waiting: &Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>) {
        let mut worker = self.worker.lock().await;
        let hung = worker.as_ref().is_some_and(|w| {
            Arc::ptr_eq(&w.waiting, waiting)
                && (w.ready.load(Ordering::SeqCst) || w.started.elapsed() >= self.start_wait)
        });
        if hung {
            tracing::warn!(
                "the live transcriber gave no answer: it is stopped, and the next chunk starts a new one"
            );
            *worker = None;
        }
    }

    /// A running worker, started when there is none.
    fn start(&self) -> std::io::Result<Worker> {
        let mut child = tokio::process::Command::new(&self.program)
            .arg("--live")
            .env("HF_HUB_OFFLINE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("no input"))?;
        let waiting: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> = Arc::default();
        let exited = Arc::new(AtomicBool::new(false));
        let ready = Arc::new(AtomicBool::new(false));
        let why: Arc<Mutex<Option<String>>> = Arc::default();
        let answers = child
            .stdout
            .take()
            .map(|stdout| read_answers(stdout, Arc::clone(&waiting), Arc::clone(&ready)));
        let errors = child.stderr.take().map(|mut stderr| {
            let why = Arc::clone(&why);
            tokio::spawn(async move {
                let mut buffer = vec![0_u8; 8192];
                while let Ok(n) = stderr.read(&mut buffer).await {
                    if n == 0 {
                        break;
                    }
                    let said = String::from_utf8_lossy(&buffer[..n]).into_owned();
                    if let Some(line) = said.trim().lines().last() {
                        *lock(&why) = Some(line.to_owned());
                    }
                    let lower = said.to_lowercase();
                    if lower.contains("error") || lower.contains("traceback") {
                        tracing::warn!(said = %last_chars(said.trim(), 300), "the live transcriber");
                    }
                }
            })
        });
        let stop = CancellationToken::new();
        {
            let (waiting, exited, ready, why, stop) = (
                Arc::clone(&waiting),
                Arc::clone(&exited),
                Arc::clone(&ready),
                Arc::clone(&why),
                stop.clone(),
            );
            tokio::spawn(async move {
                let status = tokio::select! {
                    status = child.wait() => status,
                    () = stop.cancelled() => {
                        if let Err(error) = child.start_kill() {
                            tracing::debug!(%error, "the live transcriber was not stopped");
                        }
                        child.wait().await
                    }
                };
                // Its last words: why it stopped, and whether it got ready before.
                let last_words = async {
                    if let Some(answers) = answers {
                        let _read = answers.await;
                    }
                    if let Some(errors) = errors {
                        let _read = errors.await;
                    }
                };
                let _heard = tokio::time::timeout(Duration::from_secs(1), last_words).await;
                exited.store(true, Ordering::SeqCst);
                let no_start = if ready.load(Ordering::SeqCst) {
                    Value::Null
                } else {
                    Value::from(lock(&why).clone().unwrap_or_else(|| match &status {
                        Ok(status) => format!("exit code {}", status.code().unwrap_or(-1)),
                        Err(error) => error.to_string(),
                    }))
                };
                for (id, done) in lock(&waiting).drain() {
                    let _sent = done.send(json!({
                        "id": id, "error": "the transcriber stopped", "noStart": no_start
                    }));
                }
            });
        }
        Ok(Worker {
            stdin,
            waiting,
            exited,
            ready,
            started: tokio::time::Instant::now(),
            _stop: stop.drop_guard(),
        })
    }
}

/// Reads a worker's answers from `stdout`: `{ ready }` sets `ready`, and an answer goes to the
/// chunk waiting for its id. Ends when the process closes its output.
fn read_answers(
    stdout: tokio::process::ChildStdout,
    waiting: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    ready: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(Value::Object(answer)) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if answer
                .get("ready")
                .is_some_and(|r| !r.is_null() && r != &json!(false))
            {
                ready.store(true, Ordering::SeqCst);
                tracing::info!("the live transcriber is ready");
            }
            if let Some(id) = answer.get("id").and_then(Value::as_u64)
                && let Some(done) = lock(&waiting).remove(&id)
            {
                let _sent = done.send(Value::Object(answer));
            }
        }
    })
}

/// A mutex's guard, poisoned or not.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl LiveTranscriber for ResidentTranscriber {
    fn hear(&self, chunk: Chunk) -> BoxFuture<'_, Result<Heard, LiveError>> {
        Box::pin(async move {
            let id = self.next_id.fetch_add(1, Ordering::SeqCst);
            let (done, answer) = oneshot::channel();
            let mut done = Some(done);
            let waiting = {
                let mut worker = self.worker.lock().await;
                let mut fresh = false;
                loop {
                    if worker
                        .as_ref()
                        .is_none_or(|w| w.exited.load(Ordering::SeqCst))
                    {
                        *worker = Some(self.start().map_err(|error| {
                            LiveError::NoStart(format!("{}: {error}", self.program.display()))
                        })?);
                        fresh = true;
                    }
                    let Some(running) = worker.as_mut() else {
                        return Err(LiveError::Failed("the transcriber is gone".to_owned()));
                    };
                    // A process that stops marks itself exited, then answers the chunks
                    // waiting: checked under the same lock, this chunk is either answered by
                    // that or sees it stopped (and goes to a new one).
                    let queued = {
                        let mut waiting = lock(&running.waiting);
                        let alive = !running.exited.load(Ordering::SeqCst);
                        if alive && let Some(done) = done.take() {
                            waiting.insert(id, done);
                        }
                        alive
                    };
                    if queued {
                        break;
                    }
                    if fresh {
                        return Err(LiveError::NoStart(format!(
                            "{} stopped as it started",
                            self.program.display()
                        )));
                    }
                    *worker = None;
                }
                let Some(running) = worker.as_mut() else {
                    return Err(LiveError::Failed("the transcriber is gone".to_owned()));
                };
                let line = json!({
                    "id": id,
                    "path": chunk.path.display().to_string(),
                    "lang": chunk.lang,
                    "prompt": chunk.prompt,
                });
                // A write that fails means it stopped: its exit answers the waiting chunks.
                if let Err(error) = running
                    .stdin
                    .write_all(format!("{line}\n").as_bytes())
                    .await
                {
                    tracing::debug!(%error, "the live transcriber took no chunk");
                }
                let _flushed = running.stdin.flush().await;
                Arc::clone(&running.waiting)
            };
            let answer = match tokio::time::timeout(self.hear_wait, answer).await {
                Ok(Ok(answer)) => answer,
                Ok(Err(_gone)) => {
                    return Err(LiveError::Failed("the transcriber stopped".to_owned()));
                }
                Err(_late) => {
                    lock(&waiting).remove(&id);
                    self.stop_if_hung(&waiting).await;
                    return Err(LiveError::Failed(format!(
                        "no answer in {} s",
                        self.hear_wait.as_secs()
                    )));
                }
            };
            heard_from(&answer)
        })
    }
}

/// A transcriber's answer as what was heard, or why not.
fn heard_from(answer: &Value) -> Result<Heard, LiveError> {
    let text = |key: &str| answer.get(key).and_then(Value::as_str).map(str::to_owned);
    if let Some(error) = answer
        .get("error")
        .filter(|e| !e.is_null() && *e != &json!(false))
    {
        if let Some(why) = text("noStart") {
            return Err(LiveError::NoStart(why));
        }
        return Err(LiveError::Failed(
            error
                .as_str()
                .map_or_else(|| error.to_string(), str::to_owned),
        ));
    }
    Ok(Heard {
        text: text("text").filter(|t| !t.is_empty()),
        lang: text("lang"),
        sure: answer
            .get("sure")
            .is_some_and(|sure| sure.as_bool().unwrap_or(!sure.is_null())),
    })
}

/// Why the copilot did not answer.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct CopilotError(pub String);

/// What gives a live call's hints: one conversation per call, given the call's rules and
/// context when it opens and then the new lines, a few at a time.
pub trait Copilot: fmt::Debug + Send + Sync {
    /// Whether there is a copilot at all; without one no call gets hints.
    fn available(&self) -> bool;

    /// Opens `call`'s conversation with `instructions`.
    fn open(&self, call: &str, instructions: String) -> BoxFuture<'_, Result<(), CopilotError>>;

    /// Tells `call`'s conversation `text`: its answer (`-` for nothing to add).
    fn say(&self, call: &str, text: String) -> BoxFuture<'_, Result<String, CopilotError>>;

    /// Ends `call`'s conversation.
    fn close(&self, call: &str);
}

/// No copilot: calls get their live transcript and no hints.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoCopilot;

impl Copilot for NoCopilot {
    fn available(&self) -> bool {
        false
    }

    fn open(&self, _call: &str, _instructions: String) -> BoxFuture<'_, Result<(), CopilotError>> {
        Box::pin(async { Err(CopilotError("there is no copilot".to_owned())) })
    }

    fn say(&self, _call: &str, _text: String) -> BoxFuture<'_, Result<String, CopilotError>> {
        Box::pin(async { Err(CopilotError("there is no copilot".to_owned())) })
    }

    fn close(&self, _call: &str) {}
}

/// What the copilot is told it is, before the call's context.
pub const RULES: &str = "\
You are the user's silent copilot during a live call. You get the call's transcript as it \
happens, a few lines at a time (\"Me:\" is the user's microphone; \"Others:\" is the other side; \
when the user is on speakers their line can echo the other side's). Whatever you reply shows on \
their screen while they talk, so most of the time you say nothing.

Reply with exactly \"-\" (one hyphen), unless you have one of these, and it's worth their eyes \
right now:
- Fact: a fact or number from the context that bears on what's being said (a date, a price, a \
figure, a decision already made).
- Promise: a commitment or deadline just made on the call that they should keep track of (who \
owes what, by when).
- Before: what the other side said, agreed or asked for earlier (on this call or before), when \
it differs from now.
- Ask: one sharp question they could ask now, that moves the call forward.
- Correction: something just said that contradicts what the context says.

Never: restate or summarise what was said, comment on small talk or greetings, give generic \
advice, praise, hedge, or repeat or rephrase any earlier hint of yours (if it's close to one \
you gave, reply \"-\"). When unsure, \"-\".
When someone commits to something in the new lines, that is the hint: \"Promise: <who> <what> \
by <when>\" (one, the most important).
Format: one line, at most 25 words, starting with its kind (\"Fact: ...\", \"Ask: ...\"), in \
English (names and terms in other languages as they are).
Use only what's in the transcript and the context below; never invent a number.";

/// A call's copilot conversation, as the service keeps it.
#[derive(Clone, Debug)]
struct Session {
    room: RoomId,
    title: String,
    /// The last line's time the copilot has seen.
    up_to: f64,
    /// When its last turn began.
    last: Option<Timestamp>,
    busy: bool,
}

/// Live calls: their chunks, their lines and the copilot.
#[derive(Clone, Debug)]
pub struct LiveCallService {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    calls: CallStore,
    rooms: RoomStore,
    messages: MessageStore,
    events: EventStore,
    transcriber: Arc<dyn LiveTranscriber>,
    copilot: Arc<dyn Copilot>,
    clock: Arc<dyn Clock>,
    dir: PathBuf,
    sessions: Mutex<HashMap<String, Session>>,
    /// Each call's copilot: `on`, `ended`, or `off: <why>`.
    states: Mutex<HashMap<String, String>>,
    /// Chunks and turns still running.
    pending: watch::Sender<usize>,
}

/// What the live call service is built from.
#[derive(Debug)]
pub struct LiveCallParts {
    /// The calls.
    pub calls: CallStore,
    /// The rooms.
    pub rooms: RoomStore,
    /// Where the hints' quiet lines go.
    pub messages: MessageStore,
    /// Where a transcriber that can't start is said.
    pub events: EventStore,
    /// What hears the chunks.
    pub transcriber: Arc<dyn LiveTranscriber>,
    /// What gives the hints.
    pub copilot: Arc<dyn Copilot>,
    /// What time it is.
    pub clock: Arc<dyn Clock>,
    /// The folder that keeps the calls.
    pub dir: PathBuf,
}

/// Counts a running chunk or turn until it is dropped.
struct Running(watch::Sender<usize>);

impl Running {
    fn new(pending: &watch::Sender<usize>) -> Self {
        pending.send_modify(|count| *count += 1);
        Self(pending.clone())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.0.send_modify(|count| *count = count.saturating_sub(1));
    }
}

impl LiveCallService {
    /// The service from its parts.
    #[must_use]
    pub fn new(parts: LiveCallParts) -> Self {
        let LiveCallParts {
            calls,
            rooms,
            messages,
            events,
            transcriber,
            copilot,
            clock,
            dir,
        } = parts;
        Self {
            inner: Arc::new(Inner {
                calls,
                rooms,
                messages,
                events,
                transcriber,
                copilot,
                clock,
                dir,
                sessions: Mutex::default(),
                states: Mutex::default(),
                pending: watch::Sender::new(0),
            }),
        }
    }

    /// Resolves once no chunk is being heard and no copilot turn runs.
    pub async fn idle(&self) {
        let mut pending = self.inner.pending.subscribe();
        if let Err(closed) = pending.wait_for(|count| *count == 0).await {
            tracing::debug!(%closed, "the live counter is gone: nothing runs");
        }
    }

    fn state(&self, call: &str) -> Option<String> {
        lock(&self.inner.states).get(call).cloned()
    }

    /// Takes a chunk of a call being recorded (its audio is `body`): stored, heard in the
    /// background, its words kept as the call's next line. The call must be one being recorded,
    /// or one the Mac's recorder started itself (`recording`). `copilot=0` turns the call's copilot off (the owner's switch on the Mac),
    /// `copilot=1` back on.
    ///
    /// # Errors
    ///
    /// [`CallError::Refused`] for a call, track or time it doesn't take, and the database's or
    /// the files' failure.
    pub async fn receive_chunk<S, E>(&self, query: &ChunkQuery, body: S) -> Result<(), CallError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: std::error::Error + Send + Sync + 'static,
    {
        let refused = || CallError::Refused("call, track (mic or sys) and t".to_owned());
        let id = query
            .call
            .as_deref()
            .and_then(CallId::parse)
            .ok_or_else(refused)?;
        let track = query
            .track
            .as_deref()
            .and_then(TrackKind::from_name)
            .ok_or_else(refused)?;
        let t = match query.t.as_deref().map(str::trim) {
            None | Some("") => 0.0,
            Some(t) => t.parse::<f64>().unwrap_or(f64::NAN),
        };
        if !t.is_finite() {
            return Err(refused());
        }
        let now = self.inner.clock.now();
        recording(&self.inner.calls, &id, query.title.as_deref(), now).await?;
        let key = id.as_str().to_owned();
        match query.copilot.as_deref() {
            Some("0") if self.state(&key).as_deref() != Some(OFF_ON_THE_MAC) => {
                self.end(&key);
                lock(&self.inner.states).insert(key.clone(), OFF_ON_THE_MAC.to_owned());
            }
            Some("1") if self.state(&key).as_deref() == Some(OFF_ON_THE_MAC) => {
                lock(&self.inner.states).remove(&key);
            }
            _ => {}
        }
        let dir = self.inner.dir.join(id.as_str()).join("live");
        make_private_dir(&dir).await?;
        let tenths = (t * 10.0 + 0.5).floor();
        let file = dir.join(format!("{}-{tenths}.wav", track.as_str()));
        write_private(&file, body, MAX_CHUNK).await?;
        let last: Vec<String> = self
            .inner
            .calls
            .last_lines(&id, 2)
            .await?
            .into_iter()
            .map(|line| line.text)
            .collect();
        let chunk = Chunk {
            path: file.clone(),
            lang: self.inner.calls.call_lang(&id).await?,
            prompt: last_chars(&last.join(" "), 300),
        };
        let running = Running::new(&self.inner.pending);
        let service = self.clone();
        tokio::spawn(async move {
            let _running = running;
            let heard = service.inner.transcriber.hear(chunk).await;
            remove_file(&file).await;
            if let Err(error) = service.keep(&id, track, t, heard).await {
                tracing::error!(%error, call = %id, "a live line could not be kept");
            }
        });
        Ok(())
    }

    /// Keeps what a chunk said as a line, or says once in the call's room that the
    /// transcriber can't start.
    async fn keep(
        &self,
        id: &CallId,
        track: TrackKind,
        t: f64,
        heard: Result<Heard, LiveError>,
    ) -> Result<(), DbError> {
        match heard {
            Ok(Heard {
                text: Some(text),
                lang,
                sure,
            }) => {
                self.inner
                    .calls
                    .add_line(
                        id,
                        HeardLine {
                            line: LiveLine { track, t, text },
                            lang,
                            sure,
                        },
                        self.inner.clock.now(),
                    )
                    .await
            }
            Ok(_) => Ok(()),
            Err(LiveError::NoStart(why)) => {
                tracing::warn!(call = %id, %why, "the live transcriber can't start");
                let call = self.inner.calls.get(id).await?;
                let room = call.as_ref().map_or(RoomId::MAIN, |call| call.room);
                let title = call.map_or_default(|call| call.title);
                let text = format!(
                    "Live transcription of the call \"{title}\" isn't working: the transcriber \
                     can't start ({}). The Mac's recording doesn't depend on it, but the final \
                     transcript may use the same model and fail too until this is fixed; its \
                     audio is kept.",
                    cut(&why, 300)
                );
                self.inner
                    .events
                    .add(
                        NewEvent::new(room, "call", text).once(format!("live-down:{id}")),
                        self.inner.clock.now(),
                    )
                    .await
                    .map(drop)
            }
            Err(LiveError::Failed(why)) => {
                tracing::warn!(call = %id, track = track.as_str(), t, %why, "a live chunk failed");
                Ok(())
            }
        }
    }

    /// The call's last 40 lines (the echo left out) and the copilot's hints, for the Mac's
    /// panel.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn live(&self, call: &str) -> Result<CallLive, DbError> {
        let id = CallId::parse(call);
        let (lines, hints, found) = match &id {
            Some(id) => (
                self.inner.calls.last_lines(id, 60).await?,
                self.inner.calls.hints(id).await?,
                self.inner.calls.get(id).await?,
            ),
            None => (Vec::new(), Vec::new(), None),
        };
        let lines = unecho(&lines);
        let skip = lines.len().saturating_sub(40);
        let copilot = self.state(call).unwrap_or_else(|| {
            if self.inner.copilot.available() {
                "waiting".to_owned()
            } else {
                "off".to_owned()
            }
        });
        Ok(CallLive {
            call: call.to_owned(),
            title: found.as_ref().map(|c| c.title.clone()),
            status: found.as_ref().map(|c| c.status.as_str().to_owned()),
            copilot,
            lines: lines
                .into_iter()
                .skip(skip)
                .map(|line| LiveCallLine {
                    t: line.t,
                    who: line.track.who().to_owned(),
                    text: line.text,
                })
                .collect(),
            hints: hints
                .into_iter()
                .map(|hint| LiveCallHint {
                    t: hint.t,
                    text: hint.text,
                    at: timestamp_to_api(hint.at),
                })
                .collect(),
        })
    }

    /// Ends the call's copilot (its last track came, or it was turned off).
    pub fn end(&self, call: &str) {
        if lock(&self.inner.sessions).remove(call).is_some() {
            self.inner.copilot.close(call);
        }
        let mut states = lock(&self.inner.states);
        if states.get(call).map(String::as_str) == Some("on") {
            states.insert(call.to_owned(), "ended".to_owned());
        }
    }

    /// The context a call's copilot gets: where the call is.
    async fn background(&self, room: RoomId) -> Result<String, DbError> {
        let name = self
            .inner
            .rooms
            .get(room)
            .await?
            .map_or_else(|| "Main".to_owned(), |room| room.name);
        Ok(format!("The call is in the assistant's {name} room."))
    }

    /// One look at the live calls: each with new lines whose copilot's last turn is 45 s old
    /// gets a turn; a call quiet for ten minutes (or ended) loses its copilot.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn tick(&self) -> Result<(), DbError> {
        if !self.inner.copilot.available() {
            return Ok(());
        }
        let now = self.inner.clock.now();
        let since = now.checked_sub(LIVE_FOR).unwrap_or(Timestamp::UNIX_EPOCH);
        let live = self.inner.calls.live_since(since).await?;
        for call in &live {
            let key = call.id.as_str().to_owned();
            if self
                .state(&key)
                .is_some_and(|state| state.starts_with("off"))
            {
                continue;
            }
            let opened = lock(&self.inner.sessions).contains_key(&key);
            if !opened && let Err(error) = self.open(&call.id, call.room, &call.title).await {
                self.stopped(&key, call.room, &call.title, &error.0).await?;
                continue;
            }
            let due = {
                let mut sessions = lock(&self.inner.sessions);
                let Some(session) = sessions.get_mut(&key) else {
                    continue;
                };
                let rested = session
                    .last
                    .is_none_or(|last| now.duration_since(last) >= TURN_EVERY);
                !session.busy && rested
            };
            if due {
                let running = Running::new(&self.inner.pending);
                let service = self.clone();
                let (id, room, title) = (call.id.clone(), call.room, call.title.clone());
                tokio::spawn(async move {
                    let _running = running;
                    if let Err(error) = service.turn_or_stop(&id, room, &title).await {
                        tracing::error!(%error, call = %id, "a copilot's turn could not be kept");
                    }
                });
            }
        }
        let ended: Vec<String> = lock(&self.inner.sessions)
            .keys()
            .filter(|key| !live.iter().any(|call| call.id.as_str() == key.as_str()))
            .cloned()
            .collect();
        for call in ended {
            self.end(&call);
        }
        Ok(())
    }

    /// Opens the call's copilot, with its rules, context and the hints it gave before a
    /// restart.
    async fn open(&self, id: &CallId, room: RoomId, title: &str) -> Result<(), CopilotError> {
        let background = self
            .background(room)
            .await
            .map_err(|error| CopilotError(error.to_string()))?;
        self.inner
            .copilot
            .open(id.as_str(), format!("{RULES}\n\n# Context\n{background}"))
            .await?;
        lock(&self.inner.sessions).insert(
            id.as_str().to_owned(),
            Session {
                room,
                title: title.to_owned(),
                up_to: -1.0,
                last: None,
                busy: false,
            },
        );
        lock(&self.inner.states).insert(id.as_str().to_owned(), "on".to_owned());
        let earlier: Vec<String> = self
            .inner
            .calls
            .hints(id)
            .await
            .map_err(|error| CopilotError(error.to_string()))?
            .into_iter()
            .map(|hint| format!("- {}", hint.text))
            .collect();
        if !earlier.is_empty() {
            self.inner
                .copilot
                .say(
                    id.as_str(),
                    format!(
                        "(The server restarted mid-call.) Your hints so far, don't repeat \
                         them:\n{}\nReply \"-\".",
                        earlier.join("\n")
                    ),
                )
                .await?;
        }
        Ok(())
    }

    /// A turn; a copilot that fails is ended, and says so once in the call's room.
    async fn turn_or_stop(&self, id: &CallId, room: RoomId, title: &str) -> Result<(), DbError> {
        let result = self.turn(id).await;
        if let Some(session) = lock(&self.inner.sessions).get_mut(id.as_str()) {
            session.busy = false;
        }
        match result {
            Ok(()) => Ok(()),
            Err(TurnError::Database(error)) => Err(error),
            Err(TurnError::Copilot(error)) => {
                self.stopped(id.as_str(), room, title, &error.0).await
            }
        }
    }

    /// The copilot of `call` stopped, for `why`: it is off, and the room hears it once.
    async fn stopped(
        &self,
        call: &str,
        room: RoomId,
        title: &str,
        why: &str,
    ) -> Result<(), DbError> {
        tracing::warn!(call, why, "a call's copilot stopped");
        self.end(call);
        let why = cut(why, 120);
        lock(&self.inner.states).insert(call.to_owned(), format!("off: {why}"));
        self.inner
            .messages
            .append(
                NewMessage {
                    room,
                    author: Author::Event,
                    text: format!(
                        "The live copilot for {title} stopped ({why}); the live transcript \
                         carries on."
                    ),
                    reply_to: None,
                },
                self.inner.clock.now(),
            )
            .await
            .map(drop)
    }

    /// One turn: the new lines since the last one; a hint when the copilot has one.
    async fn turn(&self, id: &CallId) -> Result<(), TurnError> {
        let key = id.as_str();
        let Some(session) = lock(&self.inner.sessions).get(key).cloned() else {
            return Ok(());
        };
        let near = self
            .inner
            .calls
            .lines_after(id, session.up_to - 12.0)
            .await?;
        let lines: Vec<LiveLine> = unecho(&near)
            .into_iter()
            .filter(|line| line.t > session.up_to)
            .collect();
        let (Some(first), Some(last)) = (lines.first(), lines.last()) else {
            return Ok(());
        };
        let now = self.inner.clock.now();
        let words: usize = lines
            .iter()
            .map(|line| line.text.split_whitespace().count())
            .sum();
        let recent = session
            .last
            .is_some_and(|last| now.duration_since(last) < WAIT_FOR_WORDS);
        if words < ENOUGH_WORDS && recent {
            return Ok(());
        }
        let up_to = last.t;
        if let Some(session) = lock(&self.inner.sessions).get_mut(key) {
            session.busy = true;
            session.last = Some(now);
            session.up_to = up_to;
        }
        let said: Vec<String> = lines
            .iter()
            .map(|line| format!("{}: {}", line.track.who(), line.text))
            .collect();
        let text = format!("[{}–{}]\n{}", clock(first.t), clock(up_to), said.join("\n"));
        let answer = tokio::time::timeout(TURN_WAIT, self.inner.copilot.say(key, text))
            .await
            .map_err(|_| TurnError::Copilot(CopilotError("no answer in 60 s".to_owned())))??;
        if let Some(hint) = hint_of(&answer) {
            let hint = cut(&hint, HINT_LENGTH);
            let now = self.inner.clock.now();
            self.inner
                .calls
                .add_hint(id, Some(up_to), &hint, now)
                .await?;
            // A quiet line: no push, and nothing wakes the assistant.
            self.inner
                .messages
                .append(
                    NewMessage {
                        room: session.room,
                        author: Author::Event,
                        text: format!("💡 {} ({}): {hint}", session.title, clock(up_to)),
                        reply_to: None,
                    },
                    now,
                )
                .await?;
        }
        Ok(())
    }

    /// Runs [`Self::tick`] every 15 seconds until `stopping`.
    pub async fn keep_ticking(self, stopping: CancellationToken) {
        loop {
            tokio::select! {
                () = stopping.cancelled() => return,
                () = tokio::time::sleep(TICK_EVERY) => {}
            }
            if let Err(error) = self.tick().await {
                tracing::error!(%error, "the live calls' tick failed");
            }
        }
    }
}

/// Why a copilot's turn ended early.
#[derive(Debug, thiserror::Error)]
enum TurnError {
    /// The copilot failed: it is stopped.
    #[error(transparent)]
    Copilot(#[from] CopilotError),
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transcriber_s_answer_reads_as_heard_or_why_not() {
        let heard = heard_from(&json!({ "id": 1, "text": "hello", "lang": "en", "sure": true }));
        assert_eq!(
            heard.unwrap(),
            Heard {
                text: Some("hello".to_owned()),
                lang: Some("en".to_owned()),
                sure: true
            }
        );
        let silent = heard_from(&json!({ "id": 2, "text": "" })).unwrap();
        assert_eq!(silent.text, None);
        let down = heard_from(&json!({ "id": 3, "error": "stopped", "noStart": "no model" }));
        assert!(matches!(down, Err(LiveError::NoStart(why)) if why == "no model"));
        let failed = heard_from(&json!({ "id": 4, "error": "stopped", "noStart": null }));
        assert!(matches!(failed, Err(LiveError::Failed(why)) if why == "stopped"));
    }

    /// A fake transcriber program in `dir`: each start counts itself in `dir/starts` and
    /// writes its process id to `dir/pid-<n>`; the first one runs `first`, the later ones
    /// answer every chunk with `"second"`.
    #[cfg(unix)]
    fn fake_program(dir: &std::path::Path, first: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join("transcribe");
        let script = format!(
            r#"#!/bin/sh
cd "$(dirname "$0")"
n=$(( $(cat starts 2>/dev/null || echo 0) + 1 ))
echo "$n" > starts
echo "$$" > "pid-$n"
if [ "$n" = 1 ]; then
{first}
fi
echo '{{"ready":true}}'
while read -r line; do
  id=$(printf '%s' "$line" | sed 's/.*"id":\([0-9]*\).*/\1/')
  echo "{{\"id\":$id,\"text\":\"second\",\"lang\":\"en\",\"sure\":true}}"
done
"#
        );
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[cfg(unix)]
    fn chunk(dir: &std::path::Path) -> Chunk {
        Chunk {
            path: dir.join("mic-0.wav"),
            lang: None,
            prompt: String::new(),
        }
    }

    /// `transcriber.hear(chunk)`, started again while the fake program can't run yet because
    /// another test's process, forked as it was written, still holds it open.
    #[cfg(unix)]
    async fn hear(transcriber: &ResidentTranscriber, chunk: Chunk) -> Result<Heard, LiveError> {
        for _ in 0..50 {
            match transcriber.hear(chunk.clone()).await {
                Err(LiveError::NoStart(why)) if why.contains("Text file busy") => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                heard => return heard,
            }
        }
        transcriber.hear(chunk).await
    }

    /// What `dir/starts` says, once the fake program has written it: on a busy machine a
    /// process can be started and not yet have run that far.
    #[cfg(unix)]
    async fn starts(dir: &std::path::Path) -> String {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let read = std::fs::read_to_string(dir.join("starts"));
            let written = read.as_ref().is_ok_and(|n| !n.trim().is_empty());
            if written || std::time::Instant::now() >= deadline {
                return read.unwrap().trim().to_owned();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_hung_transcriber_is_stopped_and_the_next_chunk_starts_a_new_one() {
        let dir = tempfile::tempdir().unwrap();
        // Ready, then reads every chunk and never answers.
        let program = fake_program(dir.path(), r#"echo '{"ready":true}'; exec cat > /dev/null"#);
        // The second chunk's wait also covers starting the new process, which a busy machine can
        // take seconds over, so the wait is generous rather than tight.
        let transcriber = ResidentTranscriber::with_waits(
            program,
            Duration::from_secs(5),
            Duration::from_secs(600),
        );
        let late = hear(&transcriber, chunk(dir.path())).await;
        assert!(matches!(late, Err(LiveError::Failed(why)) if why.starts_with("no answer in")));
        let heard = hear(&transcriber, chunk(dir.path())).await.unwrap();
        assert_eq!(heard.text.as_deref(), Some("second"));
        assert_eq!(starts(dir.path()).await, "2");
        #[cfg(target_os = "linux")]
        {
            let pid = std::fs::read_to_string(dir.path().join("pid-1")).unwrap();
            let proc = PathBuf::from(format!("/proc/{}", pid.trim()));
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while proc.exists() && std::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            assert!(!proc.exists(), "the hung transcriber was stopped");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_transcriber_still_loading_is_given_its_time() {
        let dir = tempfile::tempdir().unwrap();
        // Never ready: still loading its model.
        let program = fake_program(dir.path(), "exec cat > /dev/null");
        let transcriber = ResidentTranscriber::with_waits(
            program,
            Duration::from_millis(300),
            Duration::from_secs(600),
        );
        for _ in 0..2 {
            let late = hear(&transcriber, chunk(dir.path())).await;
            assert!(matches!(late, Err(LiveError::Failed(_))));
        }
        assert_eq!(
            starts(dir.path()).await,
            "1",
            "the same process, still loading"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_transcriber_that_stopped_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        // Ready, then stops at the first chunk without answering it.
        let program = fake_program(dir.path(), r#"echo '{"ready":true}'; read -r line; exit 1"#);
        let transcriber = ResidentTranscriber::with_waits(
            program,
            Duration::from_secs(20),
            Duration::from_secs(600),
        );
        let stopped = hear(&transcriber, chunk(dir.path())).await;
        assert!(matches!(stopped, Err(LiveError::Failed(why)) if why == "the transcriber stopped"));
        let heard = hear(&transcriber, chunk(dir.path())).await.unwrap();
        assert_eq!(heard.text.as_deref(), Some("second"));
    }
}
