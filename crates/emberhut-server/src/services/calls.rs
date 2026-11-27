//! The owner's calls as text: recorded on their Mac when they ask (their microphone and the
//! Mac's sound as two tracks, so the other side of a call is heard even in headphones), or a
//! recording they share in the chat; transcribed on this server, one call at a time, and
//! handed to the assistant as a `call` event in the room it is for, so it pulls out the
//! decisions and to-dos.
//!
//! The recorder on the Mac is reached through the Mac helper ([`MacHelper::record`]); it sends
//! each track here when a recording stops (`POST /api/calls/in`). A call's files are kept
//! beside the database, in `calls/<id>/` (the folder for the server's user only, the files
//! too): its audio is deleted after [`CallsConfig::audio_days`] days, its transcript is kept.
//!
//! [`CallTranscriber`] is the seam that turns a call's audio into a transcript:
//! [`CommandTranscriber`] runs a program configured on the server (a local speech-to-text model,
//! so no recording leaves the machine), [`NoTranscriber`] stands in when none is set up, and
//! tests swap in a fake.
//!
//! [`MacHelper::record`]: crate::services::mac::MacHelper::record

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use emberhut_api::calls::{CallRecording, CallRow, ReceivedTrack, TrackQuery};
use emberhut_core::calls::{
    Call, CallId, CallStatus, NewCall, Track, TrackKind, audio_extension, cut, last_chars,
    title_or_untitled,
};
use emberhut_core::event::NewEvent;
use emberhut_core::{Clock, RandomError, RandomSource, RoomId};
use emberhut_db::{CallStore, DbError, EventStore, RoomStore};
use futures_util::future::BoxFuture;
use futures_util::{Stream, StreamExt as _};
use jiff::{SignedDuration, Timestamp};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::convert::{room_to_api, timestamp_to_api};
use crate::services::calendar::text_of;
use crate::services::mac::{MacError, MacService, RecorderCommand};

/// The folder beside the database that keeps the calls.
pub const FOLDER: &str = "calls";

/// The longest a call's transcript is quoted in its event; the rest is in the file.
const QUOTED: usize = 6000;

/// How long after start-up the first sweep and the first transcription run.
const FIRST_SWEEP: Duration = Duration::from_secs(3);

/// How often old audio is looked for.
const SWEEP_EVERY: Duration = Duration::from_secs(3600);

/// A call being recorded for longer than this is not shown as recording any more.
const RECORDING_FOR: SignedDuration = SignedDuration::from_hours(6);

/// How many calls the latest calls' list has.
const LATEST: u64 = 8;

/// How many calls the `mac_record` tool's status lists.
const STATUS_CALLS: u64 = 5;

/// The longest track taken: 256 MiB. The Mac's recorder writes AAC at 64 kbit/s, so a
/// recording of [`RECORDING_FOR`] (six hours) is about 175 MB.
const MAX_TRACK: u64 = 256 << 20;

/// How long a track may stop arriving before it is given up on.
const IDLE: Duration = Duration::from_secs(60);

/// How the call service is configured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallsConfig {
    /// A call's audio is deleted this many days after it was transcribed (or failed).
    pub audio_days: u32,
}

impl Default for CallsConfig {
    /// 14 days of audio.
    fn default() -> Self {
        Self { audio_days: 14 }
    }
}

/// A call to transcribe: where its transcript goes, its title, and its tracks (each with who
/// speaks on it, when known).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptionJob {
    /// Where the transcript (Markdown) is to be written.
    pub out: PathBuf,
    /// The call's title.
    pub title: String,
    /// Its audio files and who speaks on each.
    pub tracks: Vec<(PathBuf, Option<String>)>,
}

/// Why a call was not transcribed: the last line the transcriber said.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct TranscribeFailure(pub String);

/// What turns a call's audio into a transcript.
pub trait CallTranscriber: fmt::Debug + Send + Sync {
    /// Transcribes `job`, writing its transcript to `job.out`.
    fn transcribe(&self, job: TranscriptionJob) -> BoxFuture<'_, Result<(), TranscribeFailure>>;
}

/// No transcriber set up: every call fails, saying so, and keeps its audio.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoTranscriber;

impl CallTranscriber for NoTranscriber {
    fn transcribe(&self, _job: TranscriptionJob) -> BoxFuture<'_, Result<(), TranscribeFailure>> {
        Box::pin(async {
            Err(TranscribeFailure(
                "no call transcriber is set up on this server".to_owned(),
            ))
        })
    }
}

/// How much of what a transcriber says on its error output is kept for the reason.
const SAID_KEPT: usize = 4000;

/// A program on the server that transcribes a call, run as
/// `<program> <transcript.md> <title> <file>[=<who>]...`. Its arguments are by position, with no
/// options:
///
/// 1. where to write the transcript (Markdown);
/// 2. the call's title;
/// 3. then one argument per track: its audio file, followed by `=<who>` when who speaks on it
///    is known (`.../calls/<id>/mic.m4a=Me`); a program splits each at its last `=`.
///
/// The paths are absolute (under the data directory), and a leading `-` in the title is written
/// as `‐` (U+2010), so no argument reads as an option. A data directory whose path contains `=`
/// makes the `<file>=<who>` form ambiguous (a track without `who` reads as one with it): keep
/// `=` out of it.
///
/// It runs with no input and `HF_HUB_OFFLINE=1` (a local model is not downloaded mid-call), for
/// at most its time limit (a run past it is killed and the call fails, so one hung run doesn't
/// hold up the calls after it). Exit status 0 and the transcript written mean done; anything
/// else failed, for the last line it said.
#[derive(Clone, Debug)]
pub struct CommandTranscriber {
    program: PathBuf,
    limit: Duration,
}

impl CommandTranscriber {
    /// The transcriber that runs `program`, each run for at most `limit`.
    #[must_use]
    pub fn new(program: PathBuf, limit: Duration) -> Self {
        Self { program, limit }
    }
}

/// The arguments [`CommandTranscriber`] runs its program with for `job`, by position: the
/// transcript's path, the title (a leading `-` written as `‐`), and each track as
/// `<file>[=<who>]`. A relative path starting with `-` is given as `./-...`.
#[must_use]
pub fn transcriber_args(job: &TranscriptionJob) -> Vec<OsString> {
    let mut args = Vec::with_capacity(job.tracks.len() + 2);
    args.push(not_an_option(&job.out));
    args.push(title_arg(&job.title).into());
    args.extend(job.tracks.iter().map(|(file, who)| {
        let mut arg = not_an_option(file);
        if let Some(who) = who.as_deref().filter(|who| !who.is_empty()) {
            arg.push("=");
            arg.push(who);
        }
        arg
    }));
    args
}

/// `title` with each leading `-` written as `‐` (U+2010), which reads the same.
fn title_arg(title: &str) -> String {
    let rest = title.trim_start_matches('-');
    let dashes = title.len() - rest.len();
    format!("{}{rest}", "\u{2010}".repeat(dashes))
}

/// `path` as an argument that can't read as an option.
fn not_an_option(path: &Path) -> OsString {
    if path.as_os_str().as_encoded_bytes().starts_with(b"-") {
        Path::new(".").join(path).into_os_string()
    } else {
        path.as_os_str().to_owned()
    }
}

impl CallTranscriber for CommandTranscriber {
    fn transcribe(&self, job: TranscriptionJob) -> BoxFuture<'_, Result<(), TranscribeFailure>> {
        Box::pin(async move {
            let mut command = tokio::process::Command::new(&self.program);
            command
                .args(transcriber_args(&job))
                .env("HF_HUB_OFFLINE", "1")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            let mut child = command.spawn().map_err(|error| {
                TranscribeFailure(format!("Error: spawn {} {error}", self.program.display()))
            })?;
            let (mut out, mut err) = (String::new(), String::new());
            let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
            let finished = {
                let read_out = async {
                    if let Some(mut stdout) = stdout {
                        let _read = stdout.read_to_string(&mut out).await;
                    }
                };
                let read_err = async {
                    if let Some(mut stderr) = stderr {
                        let _read = stderr.read_to_string(&mut err).await;
                    }
                };
                let run = async { tokio::join!(read_out, read_err, child.wait()) };
                tokio::time::timeout(self.limit, run).await
            };
            let Ok(((), (), status)) = finished else {
                if let Err(error) = child.kill().await {
                    tracing::warn!(%error, "a call transcriber past its time limit was not stopped");
                }
                return Err(TranscribeFailure(format!(
                    "the transcriber was stopped after {}, its time limit",
                    limit_text(self.limit)
                )));
            };
            let said = last_chars(&format!("{out}{err}"), SAID_KEPT);
            match status {
                Ok(status)
                    if status.success()
                        && tokio::fs::try_exists(&job.out).await.unwrap_or(false) =>
                {
                    Ok(())
                }
                Ok(_) => Err(TranscribeFailure(last_line(&said))),
                Err(error) => Err(TranscribeFailure(format!("Error: {error}"))),
            }
        })
    }
}

/// `limit` in minutes (`120 min`), or in seconds when under one (`1 s`), rounded up.
fn limit_text(limit: Duration) -> String {
    if limit >= Duration::from_secs(60) {
        format!("{} min", limit.as_secs().div_ceil(60))
    } else {
        format!("{} s", limit.as_millis().div_ceil(1000))
    }
}

/// The last line of `said`, trimmed.
fn last_line(said: &str) -> String {
    said.trim().lines().last().unwrap_or_default().to_owned()
}

/// Why a call could not be taken.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CallError {
    /// The request is not one the route takes (400, with these words).
    #[error("{0}")]
    Refused(String),
    /// The Mac helper didn't do it.
    #[error(transparent)]
    Mac(#[from] MacError),
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
    /// A call's files could not be written or read.
    #[error("the call's files could not be written")]
    Storage(#[from] std::io::Error),
    /// No id could be made.
    #[error("no id could be made")]
    Random(#[from] RandomError),
}

/// Calls: recording on the Mac, the tracks it sends, and their transcription.
#[derive(Clone, Debug)]
pub struct CallService {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    calls: CallStore,
    rooms: RoomStore,
    events: EventStore,
    mac: MacService,
    transcriber: Arc<dyn CallTranscriber>,
    clock: Arc<dyn Clock>,
    random: Arc<dyn RandomSource>,
    dir: PathBuf,
    data_dir: PathBuf,
    uploads_dir: PathBuf,
    config: CallsConfig,
    /// A call is being transcribed: one at a time (the machine's speech model is shared).
    busy: tokio::sync::Mutex<bool>,
    /// `true` while a transcription runs.
    working: watch::Sender<bool>,
}

/// What the call service is built from.
#[derive(Debug)]
pub struct CallParts {
    /// The calls.
    pub calls: CallStore,
    /// The rooms (a tool's room by name).
    pub rooms: RoomStore,
    /// Where the transcripts go, as events.
    pub events: EventStore,
    /// The Mac's recorder.
    pub mac: MacService,
    /// What transcribes.
    pub transcriber: Arc<dyn CallTranscriber>,
    /// What time it is.
    pub clock: Arc<dyn Clock>,
    /// Where call ids get their randomness.
    pub random: Arc<dyn RandomSource>,
    /// The database's folder: the calls are kept in its [`FOLDER`].
    pub data_dir: PathBuf,
    /// The chat's uploads folder (a shared recording's source is deleted with its audio).
    pub uploads_dir: PathBuf,
    /// How long audio is kept.
    pub config: CallsConfig,
}

impl CallService {
    /// The service from its parts.
    #[must_use]
    pub fn new(parts: CallParts) -> Self {
        let CallParts {
            calls,
            rooms,
            events,
            mac,
            transcriber,
            clock,
            random,
            data_dir,
            uploads_dir,
            config,
        } = parts;
        let data_dir = std::path::absolute(&data_dir).unwrap_or(data_dir);
        Self {
            inner: Arc::new(Inner {
                calls,
                rooms,
                events,
                mac,
                transcriber,
                clock,
                random,
                dir: data_dir.join(FOLDER),
                data_dir,
                uploads_dir,
                config,
                busy: tokio::sync::Mutex::new(false),
                working: watch::Sender::new(false),
            }),
        }
    }

    /// The folder a call's files are in, made (for the server's user only) when missing.
    async fn dir_of(&self, id: &CallId) -> std::io::Result<PathBuf> {
        let dir = self.inner.dir.join(id.as_str());
        make_private_dir(&dir).await?;
        Ok(dir)
    }

    /// The folder that keeps the calls.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.inner.dir
    }

    /// The call `id`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn get(&self, id: &CallId) -> Result<Option<Call>, DbError> {
        self.inner.calls.get(id).await
    }

    /// The room called `name` (any case, or by its number), if there is one.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn room_named(&self, name: &str) -> Result<Option<RoomId>, DbError> {
        let name = name.trim();
        Ok(self
            .inner
            .rooms
            .list()
            .await?
            .into_iter()
            .find(|room| {
                room.name.eq_ignore_ascii_case(name)
                    || room.name.to_lowercase() == name.to_lowercase()
                    || room.id.get().to_string() == name
            })
            .map(|room| room.id))
    }

    /// Starts recording a call on the Mac (`title`, for `task`, its transcript to `room`), or
    /// stops the one recording: what the recorder said. A start the Mac refuses leaves no call.
    ///
    /// # Errors
    ///
    /// [`CallError::Mac`] when the Mac's recorder doesn't do it, and the database's or the
    /// files' failure.
    pub async fn record(
        &self,
        action: RecorderCommand,
        title: Option<&str>,
        task: Option<i64>,
        room: RoomId,
    ) -> Result<Option<String>, CallError> {
        if action != RecorderCommand::Start {
            let answer = self.inner.mac.record(action, json!({})).await?;
            return Ok(answer.get("text").map(|text| text_of(Some(text))));
        }
        let now = self.inner.clock.now();
        let title = title_or_untitled(title.unwrap_or_default().trim(), now);
        let id = CallId::new(now, self.inner.random.as_ref())?;
        self.inner
            .calls
            .create(
                NewCall {
                    id: id.clone(),
                    title: title.clone(),
                    task: task.filter(|task| *task != 0),
                    room,
                    status: CallStatus::Recording,
                },
                now,
            )
            .await?;
        if let Err(error) = self.dir_of(&id).await {
            self.inner.calls.delete(&id).await?;
            return Err(error.into());
        }
        match self
            .inner
            .mac
            .record(action, json!({ "id": id.as_str(), "title": title }))
            .await
        {
            Ok(answer) => Ok(answer.get("text").map(|text| text_of(Some(text)))),
            Err(error) => {
                self.inner.calls.delete(&id).await?;
                remove_dir(&self.inner.dir.join(id.as_str())).await;
                Err(error.into())
            }
        }
    }

    /// The `mac_record` tool's status: whether the Mac records, and the last calls.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn status(&self) -> Result<String, DbError> {
        let mac = match self
            .inner
            .mac
            .record(RecorderCommand::Status, json!({}))
            .await
        {
            Ok(answer) => match answer.get("text") {
                Some(text) => text_of(Some(text)),
                None => "(the Mac said nothing)".to_owned(),
            },
            Err(error) => format!("Mac: {error}"),
        };
        let lines: Vec<String> = self
            .inner
            .calls
            .latest(STATUS_CALLS)
            .await?
            .iter()
            .map(status_line)
            .collect();
        let last = if lines.is_empty() {
            "- none".to_owned()
        } else {
            lines.join("\n")
        };
        Ok(format!("{mac}\nLast calls:\n{last}"))
    }

    /// Takes one track of a recorded call (its audio is `body`), as the Mac sends it when a
    /// recording stops; the last one queues the call to be transcribed. The call must be one
    /// being recorded, or one the Mac's recorder started itself (`recording`).
    ///
    /// # Errors
    ///
    /// [`CallError::Refused`] for a call id or track it doesn't take, and the database's or the
    /// files' failure.
    pub async fn receive_track<S, E>(
        &self,
        query: &TrackQuery,
        body: S,
    ) -> Result<ReceivedTrack, CallError>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: std::error::Error + Send + Sync + 'static,
    {
        let refused = || CallError::Refused("call (its id) and track (mic or sys)".to_owned());
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
        let now = self.inner.clock.now();
        recording(&self.inner.calls, &id, query.title.as_deref(), now).await?;
        let file = self
            .dir_of(&id)
            .await?
            .join(format!("{}.m4a", track.as_str()));
        let bytes = write_private(&file, body, MAX_TRACK).await?;
        let file_text = file.display().to_string();
        let mut tracks: Vec<Track> = self
            .inner
            .calls
            .get(&id)
            .await?
            .map_or_default(|call| call.tracks);
        tracks.retain(|kept| kept.file != file_text);
        tracks.push(Track {
            file: file_text,
            who: Some(track.who().to_owned()),
        });
        let seconds = query
            .seconds
            .as_deref()
            .map(|s| {
                if s.trim().is_empty() {
                    0.0
                } else {
                    s.trim().parse().unwrap_or(f64::NAN)
                }
            })
            .filter(|s| s.is_finite() && *s != 0.0);
        self.inner.calls.set_tracks(&id, &tracks, seconds).await?;
        if query.last.as_deref().is_some_and(|last| !last.is_empty()) {
            self.queue(&id).await?;
        }
        Ok(ReceivedTrack {
            ok: true,
            call: id.as_str().to_owned(),
            bytes,
        })
    }

    /// The `call_transcribe` tool: transcribes a recording the person shared (a file in the
    /// server's data folder: a chat upload), titled `title`, for `task`, its transcript to
    /// `room`. What it says to the assistant.
    ///
    /// # Errors
    ///
    /// The database's or the files' failure.
    pub async fn transcribe_file(
        &self,
        path: &str,
        title: &str,
        task: Option<i64>,
        room: RoomId,
    ) -> Result<String, CallError> {
        let source = PathBuf::from(path);
        let inside = source.is_absolute()
            && source.starts_with(&self.inner.data_dir)
            && !path.contains("/../");
        let extension = audio_extension(path);
        let exists = tokio::fs::try_exists(&source).await.unwrap_or(false);
        let (true, Some(extension), true) = (inside, extension, exists) else {
            return Ok(format!("No audio file at {path}."));
        };
        let now = self.inner.clock.now();
        let id = CallId::new(now, self.inner.random.as_ref())?;
        let title = title_or_untitled(title, now);
        self.inner
            .calls
            .create(
                NewCall {
                    id: id.clone(),
                    title: title.clone(),
                    task: task.filter(|task| *task != 0),
                    room,
                    status: CallStatus::Queued,
                },
                now,
            )
            .await?;
        let file = self.dir_of(&id).await?.join(format!("call{extension}"));
        tokio::fs::copy(&source, &file).await?;
        private_file(&file).await?;
        let size = tokio::fs::metadata(&file).await?.len();
        self.inner
            .calls
            .set_source(
                &id,
                &[Track {
                    file: file.display().to_string(),
                    who: None,
                }],
                path,
            )
            .await?;
        self.queue(&id).await?;
        #[expect(
            clippy::cast_precision_loss,
            reason = "a file's size in megabytes, shown with one decimal"
        )]
        let megabytes = size as f64 / 1e6;
        Ok(format!(
            "Transcribing {title} ({megabytes:.1} MB) on this server, call {id}; its transcript \
             comes as a \"call\" event."
        ))
    }

    /// The call being recorded on the Mac now (started in the last six hours), for the state.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn recording(&self) -> Result<Option<CallRecording>, DbError> {
        let since = self
            .inner
            .clock
            .now()
            .checked_sub(RECORDING_FOR)
            .unwrap_or(Timestamp::UNIX_EPOCH);
        Ok(self
            .inner
            .calls
            .recording_since(since)
            .await?
            .map(|call| CallRecording {
                id: emberhut_api::ids::CallId::new(call.id.as_str()),
                title: call.title,
                room: room_to_api(call.room),
                at: timestamp_to_api(call.at),
            }))
    }

    /// The latest calls, newest first.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn latest(&self) -> Result<Vec<CallRow>, DbError> {
        Ok(self
            .inner
            .calls
            .latest(LATEST)
            .await?
            .into_iter()
            .map(|call| CallRow {
                id: call.id.as_str().to_owned(),
                title: call.title,
                status: call.status.as_str().to_owned(),
                room: room_to_api(call.room),
                at: timestamp_to_api(call.at),
            })
            .collect())
    }

    /// A call's transcript, when it has one and its file is there.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn transcript(&self, id: &str) -> Result<Option<String>, DbError> {
        let Some(id) = CallId::parse(id) else {
            return Ok(None);
        };
        let Some(path) = self
            .inner
            .calls
            .get(&id)
            .await?
            .and_then(|call| call.transcript)
            .filter(|path| !path.is_empty())
        else {
            return Ok(None);
        };
        Ok(tokio::fs::read_to_string(path).await.ok())
    }

    /// Queues the call to be transcribed, and starts on the queue.
    async fn queue(&self, id: &CallId) -> Result<(), DbError> {
        self.inner.calls.set_status(id, CallStatus::Queued).await?;
        self.next().await;
        Ok(())
    }

    /// Starts transcribing the queue, unless a transcription runs already.
    async fn next(&self) {
        let mut busy = self.inner.busy.lock().await;
        if *busy {
            return;
        }
        *busy = true;
        self.inner.working.send_replace(true);
        let service = self.clone();
        tokio::spawn(async move { service.work().await });
    }

    /// Transcribes queued calls, oldest first, until none is left. The queue is read under the
    /// lock, so a call queued as the last one ends is either found here or starts its own run.
    async fn work(&self) {
        loop {
            let mut busy = self.inner.busy.lock().await;
            let call = match self.inner.calls.next_queued().await {
                Ok(call) => call,
                Err(error) => {
                    tracing::error!(error = %error_chain(&error), "the queue of calls could not be read");
                    None
                }
            };
            let Some(call) = call else {
                *busy = false;
                self.inner.working.send_replace(false);
                return;
            };
            drop(busy);
            if let Err(error) = self.transcribe(call).await {
                tracing::error!(error = %error_chain(&error), "a call's transcription could not be recorded");
                *self.inner.busy.lock().await = false;
                self.inner.working.send_replace(false);
                return;
            }
        }
    }

    /// Resolves once no call is being transcribed.
    pub async fn idle(&self) {
        let mut working = self.inner.working.subscribe();
        if let Err(closed) = working.wait_for(|working| !*working).await {
            tracing::debug!(%closed, "the transcription flag is gone: nothing runs");
        }
    }

    /// Transcribes `call`, and says how it went in its room.
    async fn transcribe(&self, call: Call) -> Result<(), CallError> {
        self.inner
            .calls
            .set_status(&call.id, CallStatus::Transcribing)
            .await?;
        let out = self.dir_of(&call.id).await?.join("transcript.md");
        let job = TranscriptionJob {
            out: out.clone(),
            title: call.title.clone(),
            tracks: call
                .tracks
                .iter()
                .map(|track| (PathBuf::from(&track.file), track.who.clone()))
                .collect(),
        };
        match self.inner.transcriber.transcribe(job).await {
            Ok(()) => self.done(&call, &out).await,
            Err(failure) => self.failed(&call, &failure.0).await,
        }
    }

    async fn done(&self, call: &Call, out: &Path) -> Result<(), CallError> {
        private_file(out).await?;
        let now = self.inner.clock.now();
        let path = out.display().to_string();
        self.inner.calls.mark_done(&call.id, &path, now).await?;
        let text = tokio::fs::read_to_string(out).await?;
        let body = if text.chars().count() > QUOTED {
            format!("{}\n… (the rest is in the file)", cut(&text, QUOTED))
        } else {
            text
        };
        let task = call.task.map_or_default(|task| format!(" for task {task}"));
        let days = self.inner.config.audio_days;
        let news = format!(
            "Call transcript{task}: \"{}\" ({path}, kept; the audio is deleted after {days} \
             days). Pull out the decisions, to-dos and dates (who owes what), add them where \
             they belong, and tell the user in a few lines.\n\n{body}",
            call.title
        );
        self.tell(call.room, news, format!("call:{}", call.id))
            .await
    }

    async fn failed(&self, call: &Call, why: &str) -> Result<(), CallError> {
        let now = self.inner.clock.now();
        self.inner
            .calls
            .mark_failed(&call.id, &cut(why, 500), now)
            .await?;
        let task = call.task.map_or_default(|task| format!(" (task {task})"));
        let dir = self.inner.dir.join(call.id.as_str());
        let news = format!(
            "Couldn't transcribe the call \"{}\"{task}: {}. Its audio is kept in {}; \
             call_transcribe with its file tries again.",
            call.title,
            cut(why, 300),
            dir.display()
        );
        self.tell(call.room, news, format!("call-failed:{}", call.id))
            .await
    }

    /// A `call` event for `room`'s assistant, once per `key`.
    async fn tell(&self, room: RoomId, text: String, key: String) -> Result<(), CallError> {
        let room = if self.inner.rooms.get(room).await?.is_some() {
            room
        } else {
            RoomId::MAIN
        };
        self.inner
            .events
            .add(
                NewEvent::new(room, "call", text).once(key),
                self.inner.clock.now(),
            )
            .await?;
        Ok(())
    }

    /// Deletes the audio of calls transcribed (or failed) more than the configured days ago,
    /// and the shared recordings in the uploads folder they were copied from; the transcripts
    /// stay. How many calls lost their audio.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn sweep(&self) -> Result<usize, DbError> {
        let before = self
            .inner
            .clock
            .now()
            .checked_sub(SignedDuration::from_hours(
                24 * i64::from(self.inner.config.audio_days),
            ))
            .unwrap_or(Timestamp::UNIX_EPOCH);
        let old = self.inner.calls.audio_older_than(before).await?;
        for call in &old {
            let dir = self.inner.dir.join(call.id.as_str());
            if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let name = entry.file_name();
                    if audio_extension(&name.to_string_lossy()).is_some() {
                        remove_file(&entry.path()).await;
                    }
                }
            }
            if let Some(source) = &call.source {
                let source = PathBuf::from(source);
                if source.starts_with(&self.inner.uploads_dir) {
                    remove_file(&source).await;
                }
            }
            self.inner.calls.mark_audio_gone(&call.id).await?;
        }
        Ok(old.len())
    }

    /// Runs until `stopping`: puts back in the queue the calls a restart cut off, then (a few
    /// seconds after start-up, and every hour) deletes old audio, and transcribes what is
    /// queued.
    pub async fn keep_working(self, stopping: CancellationToken) {
        match self.inner.calls.requeue_transcribing().await {
            Ok(0) => {}
            Ok(count) => tracing::info!(count, "calls cut off by a restart are queued again"),
            Err(error) => {
                tracing::error!(error = %error_chain(&error), "the calls could not be queued again");
            }
        }
        let mut wait = FIRST_SWEEP;
        loop {
            tokio::select! {
                () = stopping.cancelled() => return,
                () = tokio::time::sleep(wait) => {}
            }
            if let Err(error) = self.sweep().await {
                tracing::error!(error = %error_chain(&error), "the calls' sweep failed");
            }
            self.next().await;
            wait = SWEEP_EVERY;
        }
    }
}

/// A call as the `mac_record` tool's status lists it.
fn status_line(call: &Call) -> String {
    let task = call.task.map_or_default(|task| format!(" task {task}"));
    let transcript = call
        .transcript
        .as_deref()
        .filter(|t| !t.is_empty())
        .map_or_default(|t| format!(" → {t}"));
    let error = call
        .error
        .as_deref()
        .filter(|e| !e.is_empty())
        .map_or_default(|e| format!(" ({e})"));
    format!(
        "- {} \"{}\" {}{task}{transcript}{error}",
        call.id, call.title, call.status
    )
}

/// An error and its causes, joined with ": ", for a log line.
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// Makes `dir` (and its parents) for the server's user only.
pub(crate) async fn make_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = tokio::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(dir).await
}

/// Makes `file` readable by the server's user only.
async fn private_file(file: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        tokio::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600)).await?;
    }
    #[cfg(not(unix))]
    let _unused = file;
    Ok(())
}

/// Writes `body` to `file` (in place of what it had), for the server's user only; its size. A
/// body over `limit` bytes, or one that stops arriving, is an error, and no part of it stays.
pub(crate) async fn write_private<S, E>(
    file: &Path,
    mut body: S,
    limit: u64,
) -> std::io::Result<u64>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut out = options.open(file).await?;
    let written = async {
        let mut size = 0_u64;
        loop {
            let next = tokio::time::timeout(IDLE, body.next())
                .await
                .map_err(|elapsed| std::io::Error::new(std::io::ErrorKind::TimedOut, elapsed))?;
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(std::io::Error::other)?;
            size += u64::try_from(chunk.len()).unwrap_or(u64::MAX);
            if size > limit {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::FileTooLarge,
                    "over the limit",
                ));
            }
            out.write_all(&chunk).await?;
        }
        out.flush().await?;
        Ok(size)
    }
    .await;
    if written.is_err() {
        remove_file(file).await;
    }
    written
}

/// Makes sure the call `id` a track or a live chunk is for is being recorded. A call the server
/// doesn't know is one the Mac's recorder started itself (from its menu), made now in Main,
/// titled `title` or by its time, when its id is a new call's ([`CallId::is_new_on`]). Any other
/// id is refused, and so is a call that is no longer being recorded: a client can't keep audio
/// under ids of its own making, or add to a call already transcribed.
///
/// # Errors
///
/// [`CallError::Refused`] for such an id; [`CallError::Database`].
pub(crate) async fn recording(
    calls: &CallStore,
    id: &CallId,
    title: Option<&str>,
    now: Timestamp,
) -> Result<(), CallError> {
    let refused = || CallError::Refused("not a call being recorded".to_owned());
    if let Some(call) = calls.get(id).await? {
        return if call.status == CallStatus::Recording {
            Ok(())
        } else {
            Err(refused())
        };
    }
    if !id.is_new_on(now) {
        return Err(refused());
    }
    calls
        .create_if_missing(
            NewCall {
                id: id.clone(),
                title: title_or_untitled(title.unwrap_or_default(), now),
                task: None,
                room: RoomId::MAIN,
                status: CallStatus::Recording,
            },
            now,
        )
        .await?;
    Ok(())
}

/// Removes `file`; one already gone is fine, any other failure is logged.
pub(crate) async fn remove_file(file: &Path) {
    if let Err(error) = tokio::fs::remove_file(file).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(path = %file.display(), %error, "could not remove a call's file");
    }
}

/// Removes `dir` and what is in it; a failure is logged.
async fn remove_dir(dir: &Path) {
    if let Err(error) = tokio::fs::remove_dir_all(dir).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(path = %dir.display(), %error, "could not remove a call's folder");
    }
}

/// The parts of a tool's arguments that were given, as the Mac helper is sent them.
#[must_use]
pub(crate) fn pick(fields: &[(&str, Option<&str>)]) -> Map<String, Value> {
    fields
        .iter()
        .filter_map(|(key, value)| Some(((*key).to_owned(), Value::from((*value)?))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_line_names_the_call_and_what_became_of_it() {
        let call = Call {
            id: CallId::parse("2026-10-31-abcdef").unwrap(),
            title: "Weekly".to_owned(),
            task: Some(12),
            room: RoomId::MAIN,
            status: CallStatus::Failed,
            tracks: Vec::new(),
            source: None,
            transcript: None,
            error: Some("no model".to_owned()),
            seconds: None,
            at: Timestamp::UNIX_EPOCH,
            done_at: None,
            audio_gone: false,
        };
        assert_eq!(
            status_line(&call),
            "- 2026-10-31-abcdef \"Weekly\" failed task 12 (no model)"
        );
    }

    #[test]
    fn the_calls_folder_is_backed_up() {
        assert!(crate::services::backup::DATA_ENTRIES.contains(&FOLDER));
    }

    fn job(title: &str, tracks: Vec<(PathBuf, Option<String>)>) -> TranscriptionJob {
        TranscriptionJob {
            out: PathBuf::from("/data/calls/c1/transcript.md"),
            title: title.to_owned(),
            tracks,
        }
    }

    #[test]
    fn the_transcriber_s_arguments_are_by_position() {
        let args = transcriber_args(&job(
            "Weekly = sync",
            vec![
                (
                    PathBuf::from("/data/calls/c1/mic.m4a"),
                    Some("Me".to_owned()),
                ),
                (PathBuf::from("/data/calls/c1/sys.m4a"), Some(String::new())),
                (PathBuf::from("/data/calls/c1/shared.m4a"), None),
            ],
        ));
        assert_eq!(
            args,
            [
                "/data/calls/c1/transcript.md",
                "Weekly = sync",
                "/data/calls/c1/mic.m4a=Me",
                "/data/calls/c1/sys.m4a",
                "/data/calls/c1/shared.m4a",
            ]
        );
    }

    #[test]
    fn no_argument_reads_as_an_option() {
        let args = transcriber_args(&TranscriptionJob {
            out: PathBuf::from("-out.md"),
            title: "--help me -now".to_owned(),
            tracks: vec![(PathBuf::from("-mic.m4a"), Some("Me".to_owned()))],
        });
        assert_eq!(
            args,
            ["./-out.md", "\u{2010}\u{2010}help me -now", "./-mic.m4a=Me"]
        );
        assert_eq!(title_arg("-"), "\u{2010}");
        assert_eq!(title_arg("a-b"), "a-b");
        assert_eq!(title_arg(""), "");
    }

    /// A program at `dir/name` that runs `body` (a shell script).
    #[cfg(unix)]
    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    /// `transcriber.transcribe(job)`, run again while the fake program can't run yet because
    /// another test's process, forked as it was written, still holds it open.
    #[cfg(unix)]
    async fn transcribe(
        transcriber: &CommandTranscriber,
        job: TranscriptionJob,
    ) -> Result<(), TranscribeFailure> {
        for _ in 0..50 {
            match transcriber.transcribe(job.clone()).await {
                Err(failure) if failure.0.contains("Text file busy") => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                done => return done,
            }
        }
        transcriber.transcribe(job).await
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_program_gets_the_arguments_and_writes_the_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let program = script(
            dir.path(),
            "transcribe",
            r#"printf '%s|' "$@" > "$1"; echo "$HF_HUB_OFFLINE" >> "$1""#,
        );
        let out = dir.path().join("transcript.md");
        let transcriber = CommandTranscriber::new(program, Duration::from_secs(30));
        transcribe(
            &transcriber,
            TranscriptionJob {
                out: out.clone(),
                title: "-Weekly".to_owned(),
                tracks: vec![(dir.path().join("mic.m4a"), Some("Me".to_owned()))],
            },
        )
        .await
        .unwrap();
        let written = std::fs::read_to_string(&out).unwrap();
        assert_eq!(
            written,
            format!(
                "{}|\u{2010}Weekly|{}=Me|1\n",
                out.display(),
                dir.path().join("mic.m4a").display()
            )
        );

        let failing = script(
            dir.path(),
            "fails",
            "echo loading; echo 'Error: no model' >&2; exit 3",
        );
        let failing = CommandTranscriber::new(failing, Duration::from_secs(30));
        let failed = transcribe(&failing, job("Weekly", Vec::new()))
            .await
            .unwrap_err();
        assert_eq!(failed.0, "Error: no model");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_run_past_its_time_limit_is_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let program = script(dir.path(), "hangs", "exec sleep 30");
        let hangs = CommandTranscriber::new(program, Duration::from_millis(300));
        let started = std::time::Instant::now();
        let failed = transcribe(&hangs, job("Weekly", Vec::new()))
            .await
            .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(
            failed.0,
            "the transcriber was stopped after 1 s, its time limit"
        );
        assert_eq!(limit_text(Duration::from_secs(7200)), "120 min");
        assert_eq!(limit_text(Duration::from_secs(90)), "2 min");
    }

    #[test]
    fn the_last_line_is_the_reason() {
        assert_eq!(last_line("loading\nError: no model\n"), "Error: no model");
        assert_eq!(last_line(""), "");
    }

    #[test]
    fn only_given_fields_are_picked() {
        let picked = pick(&[("title", Some("Lunch")), ("notes", None)]);
        assert_eq!(Value::Object(picked), json!({ "title": "Lunch" }));
    }
}
