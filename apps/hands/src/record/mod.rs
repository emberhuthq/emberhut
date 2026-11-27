//! Recording the owner's calls when asked (`/record/start`, `/record/stop`, `/record/status`):
//! their microphone and the Mac's sound (what a call plays, even into headphones) as two
//! tracks, `mic.m4a` and `sys.m4a`, so who spoke is known. While it records, the bar with Stop
//! says so. When it stops, both tracks go to the server (`POST /api/calls/in`) to be
//! transcribed and are deleted here once there; one that couldn't be sent is tried again every
//! few minutes. Every ten seconds of each track also goes to the server as it happens
//! (`POST /api/calls/live`) for the live transcript.
//!
//! The capture itself is an [`AudioSource`] (`ScreenCaptureKit` on the Mac), and sending is an
//! [`Uploader`] (the system's URL loading on the Mac), so the rest is tested anywhere.

pub mod live;
pub mod send;

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use serde_json::{Value, json};
use url::Url;

use self::live::Chunker;
use self::send::{Track, chunk_url, track_url};
use crate::fail::{Fail, Reply, field, said};
use crate::hands::Clock;
use crate::text::{clip, clock, push};

/// The call's own file in its folder: its id, title and length.
const CALL_FILE: &str = "call.json";
/// How many live chunks may wait to be sent before new ones are dropped (the recording is never
/// held up by a slow link).
const LIVE_QUEUE: usize = 3;
/// How many times a call is tried (every five minutes: about four hours) before it is set
/// aside.
const MAX_TRIES: u64 = 48;

/// Whether the helper may use the microphone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mic {
    /// Allowed.
    Allowed,
    /// The Mac is asking the owner now; this recording goes on without it.
    Asking,
    /// Not allowed.
    Refused,
}

/// What the recorder does when the Mac stops a capture by itself, told its number.
type WhenLost = Box<dyn Fn(u64) + Send + Sync>;

/// Called when the Mac stops a capture by itself.
pub type Lost = Arc<dyn Fn() + Send + Sync>;

/// Each track's samples as they come (floats, full scale 1): the track, seconds into the call
/// they begin at, their rate in hertz, and the samples.
pub type LiveSink = Arc<dyn Fn(Track, f64, f64, &[f32]) + Send + Sync>;

/// The Mac's sound and microphone.
pub trait AudioSource: Send + Sync {
    /// Whether the helper may capture the Mac's sound (Screen Recording).
    fn permitted(&self) -> bool;
    /// Whether it may use the microphone (asking the owner the first time).
    fn microphone(&self) -> Mic;
    /// Starts writing the Mac's sound (and the microphone, with `mic`) into `dir` as
    /// `sys.m4a` (and `mic.m4a`), handing every buffer to `live` too, and calling `lost` (off
    /// the capture's own thread) if the Mac stops the capture by itself.
    ///
    /// # Errors
    ///
    /// [`Fail`] when the capture doesn't start.
    fn start(
        &self,
        dir: &Path,
        mic: bool,
        live: LiveSink,
        lost: Lost,
    ) -> Result<Box<dyn Capture>, Fail>;
}

/// A capture that runs.
pub trait Capture: Send {
    /// Stops it and closes its files.
    fn finish(self: Box<Self>);
}

/// What is sent.
#[derive(Debug)]
pub enum Upload {
    /// A file's bytes.
    File(PathBuf),
    /// Bytes in memory.
    Bytes(Vec<u8>),
}

/// Why a request to the server failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{text}")]
pub struct UploadError {
    /// The server's HTTP status, when it answered.
    pub status: Option<u16>,
    /// What went wrong.
    pub text: String,
}

impl UploadError {
    /// Whether trying again won't help: the server refused the request itself (a 4xx other
    /// than a timeout, too many requests, or a sign-in the owner can still fix: 401 and 403).
    #[must_use]
    pub fn permanent(&self) -> bool {
        matches!(self.status, Some(400..=499))
            && !matches!(self.status, Some(401 | 403 | 408 | 429))
    }
}

/// Sends a request to the server.
pub trait Uploader: Send + Sync {
    /// `POST`s `body` to `url` with `token` as the bearer.
    ///
    /// # Errors
    ///
    /// [`UploadError`]: the status, or the network's words.
    fn post(&self, url: &Url, token: &str, body: Upload) -> Result<(), UploadError>;
}

/// The server the recordings go to, and the device token it knows the helper by.
#[derive(Clone)]
pub struct Server {
    /// Its address.
    pub url: Url,
    /// The device token (read from a file at start-up).
    pub token: String,
}

impl fmt::Debug for Server {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Server")
            .field("url", &self.url.as_str())
            .finish_non_exhaustive()
    }
}

/// The recording running now.
struct Recording {
    id: String,
    title: String,
    dir: PathBuf,
    started: Instant,
    mic: bool,
    capture: Box<dyn Capture>,
    /// Which recording this is since start-up: a capture lost late names its own.
    number: u64,
}

/// What the recorder is doing.
enum State {
    /// Nothing.
    Idle,
    /// Starting a capture (the lock is not held while it starts).
    Starting,
    /// Recording.
    Recording(Recording),
}

/// The call recorder.
pub struct Recorder {
    source: Arc<dyn AudioSource>,
    uploader: Arc<dyn Uploader>,
    server: Option<Server>,
    calls: PathBuf,
    clock: Arc<dyn Clock>,
    now: Mutex<State>,
    /// What to do when the Mac stops a capture by itself (set once, at start-up).
    lost: Arc<OnceLock<WhenLost>>,
    /// The last recording's number.
    started: std::sync::atomic::AtomicU64,
    /// One sending at a time, so a retry never sends a call twice at once.
    sending: Arc<Mutex<()>>,
    live: Option<SyncSender<(Url, Vec<u8>)>>,
}

impl fmt::Debug for Recorder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Recorder")
            .field("server", &self.server)
            .field("calls", &self.calls)
            .finish_non_exhaustive()
    }
}

/// Locks `mutex`, going on with its data if a panic left it poisoned.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Whether `id` is a call id the server makes: 6 to 40 ASCII letters, digits, `_` and `-`.
#[must_use]
pub fn is_call_id(id: &str) -> bool {
    (6..=40).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Makes `dir` (and its parents) readable by the owner only.
fn private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
}

/// Makes the files in `dir` readable by the owner only.
fn private_files(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    for entry in fs::read_dir(dir)? {
        fs::set_permissions(entry?.path(), fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

impl Recorder {
    /// A recorder capturing from `source`, keeping calls in `calls` until `uploader` has sent
    /// them to `server` (none: recording is refused).
    #[must_use]
    pub fn new(
        source: Arc<dyn AudioSource>,
        uploader: Arc<dyn Uploader>,
        server: Option<Server>,
        calls: PathBuf,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let live = server.as_ref().map(|server| {
            let (tx, rx) = sync_channel::<(Url, Vec<u8>)>(LIVE_QUEUE);
            let uploader = Arc::clone(&uploader);
            let token = server.token.clone();
            std::thread::spawn(move || {
                for (url, wav) in rx {
                    if let Err(why) = uploader.post(&url, &token, Upload::Bytes(wav)) {
                        tracing::warn!(%why, "a live chunk was not sent");
                    }
                }
            });
            tx
        });
        Self {
            source,
            uploader,
            server,
            calls,
            clock,
            now: Mutex::new(State::Idle),
            lost: Arc::new(OnceLock::new()),
            started: std::sync::atomic::AtomicU64::new(0),
            sending: Arc::new(Mutex::new(())),
            live,
        }
    }

    /// The call recorded now: its id and title.
    #[must_use]
    pub fn current(&self) -> Option<(String, String)> {
        match &*lock(&self.now) {
            State::Recording(r) => Some((r.id.clone(), r.title.clone())),
            State::Idle | State::Starting => None,
        }
    }

    /// Sets what happens when the Mac stops a capture by itself (once): it is told the lost
    /// recording's number, for [`Recorder::stop_lost`].
    pub fn when_lost(&self, lost: impl Fn(u64) + Send + Sync + 'static) {
        if self.lost.set(Box::new(lost)).is_err() {
            tracing::warn!("the recorder's lost capture was set twice");
        }
    }

    /// The folder calls that won't be sent are set aside in.
    fn aside(&self) -> PathBuf {
        self.calls.with_file_name("calls-unsent")
    }

    /// `/record/start`: starts recording call `id` (`title`).
    ///
    /// # Errors
    ///
    /// [`Fail`] when one records already, no server is set up, the grant is missing, the id
    /// isn't a call id, or the capture doesn't start.
    pub fn start(&self, body: &Reply) -> Result<Reply, Fail> {
        {
            let mut now = lock(&self.now);
            match &*now {
                State::Recording(r) => {
                    let seconds = self.clock.now().duration_since(r.started).as_secs_f64();
                    return Err(Fail::clash(format!(
                        "Already recording \"{}\" ({}): stop it first.",
                        r.title,
                        clock(seconds)
                    )));
                }
                State::Starting => {
                    return Err(Fail::clash("A recording is starting: wait for it."));
                }
                State::Idle => *now = State::Starting,
            }
        }
        let started = self.start_capture(body);
        let mut now = lock(&self.now);
        match started {
            Ok((recording, reply)) => {
                *now = State::Recording(recording);
                Ok(reply)
            }
            Err(fail) => {
                *now = State::Idle;
                Err(fail)
            }
        }
    }

    /// Starts the capture (the state is `Starting` meanwhile, its lock not held).
    fn start_capture(&self, body: &Reply) -> Result<(Recording, Reply), Fail> {
        let Some(server) = &self.server else {
            return Err(Fail::bad(
                "Emberhut Hands has no server to send recordings to (it starts with --server-url \
                 and --server-token-file).",
            ));
        };
        if !self.source.permitted() {
            return Err(Fail::never(
                "Emberhut Hands needs Screen Recording to hear the Mac's sound.",
            ));
        }
        let id = field(body, "id");
        if !is_call_id(&id) {
            return Err(Fail::bad("id: the call's id from the server"));
        }
        let title = match field(body, "title") {
            t if t.trim().is_empty() => "Call".to_owned(),
            t => clip(&t, 80),
        };
        let dir = self.calls.join(&id);
        private_dir(&self.calls)
            .and_then(|()| private_dir(&dir))
            .map_err(|error| Fail::broke(format!("The call's folder can't be made: {error}")))?;
        let mic = self.source.microphone();
        let lost = Arc::clone(&self.lost);
        let number = self
            .started
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        let capture = self.source.start(
            &dir,
            mic == Mic::Allowed,
            self.sink(server, &id),
            Arc::new(move || {
                if let Some(lost) = lost.get() {
                    lost(number);
                }
            }),
        )?;
        let recording = Recording {
            id: id.clone(),
            title: title.clone(),
            dir,
            started: self.clock.now(),
            mic: mic == Mic::Allowed,
            capture,
            number,
        };
        tracing::info!(%id, %title, ?mic, "started recording");
        let what = match mic {
            Mic::Allowed => "the owner's mic and the Mac's sound".to_owned(),
            Mic::Asking => {
                "the Mac's sound only: the Mac is asking the owner to let Emberhut Hands \
                            use the microphone (the next recording has it too)"
                    .to_owned()
            }
            Mic::Refused => {
                "the Mac's sound only: Emberhut Hands has no microphone access (System \
                             Settings → Privacy & Security → Microphone)"
                    .to_owned()
            }
        };
        let mut reply = said(format!(
            "Recording \"{title}\" on the owner's Mac: {what}. A bar on their screen shows it, \
             with Stop."
        ));
        reply.insert("mic".to_owned(), Value::Bool(mic == Mic::Allowed));
        Ok((recording, reply))
    }

    /// The live sink for call `id`: ten seconds of each track at a time, queued to the server.
    fn sink(&self, server: &Server, id: &str) -> LiveSink {
        let chunkers: Mutex<HashMap<Track, Chunker>> = Mutex::new(HashMap::new());
        let queue = self.live.clone();
        let server = server.url.clone();
        let id = id.to_owned();
        Arc::new(move |track, at, rate, samples| {
            let chunk = lock(&chunkers)
                .entry(track)
                .or_default()
                .feed(at, rate, samples);
            let (Some(chunk), Some(queue)) = (chunk, &queue) else {
                return;
            };
            let url = chunk_url(&server, &id, track, chunk.start);
            if let Err(TrySendError::Full(_)) = queue.try_send((url, chunk.wav)) {
                tracing::info!("a live chunk was dropped (the link is slow)");
            }
        })
    }

    /// Stops recording `number`, whose capture the Mac stopped, if it is still the one
    /// recording (a newer one goes on): what was said, or `None`.
    pub fn stop_lost(&self, number: u64) -> Option<Reply> {
        let current = matches!(&*lock(&self.now), State::Recording(r) if r.number == number);
        current.then(|| self.stop("the Mac stopped the capture"))
    }

    /// `/record/stop` (and Stop on the bar): stops the recording, closes its tracks and sends
    /// them to the server in the background.
    pub fn stop(&self, reason: &str) -> Reply {
        let r = {
            let mut now = lock(&self.now);
            match std::mem::replace(&mut *now, State::Idle) {
                State::Recording(r) => r,
                State::Starting => {
                    *now = State::Starting;
                    return said("Not recording yet: a recording is starting.");
                }
                State::Idle => return said("Not recording."),
            }
        };
        r.capture.finish();
        let seconds = self.clock.now().duration_since(r.started).as_secs_f64();
        if let Err(error) = private_files(&r.dir) {
            tracing::warn!(%error, "a call's files were not made private");
        }
        let meta = json!({ "id": r.id, "title": r.title, "seconds": seconds });
        if let Err(error) = write_private(&r.dir.join(CALL_FILE), meta.to_string().as_bytes()) {
            tracing::warn!(%error, "a call's file was not written");
        }
        tracing::info!(id = %r.id, seconds, %reason, "stopped recording");
        self.send_later(r.dir);
        let mut reply = said(format!(
            "Stopped \"{}\" after {} ({reason}); sending it to the server to be transcribed.",
            r.title,
            clock(seconds)
        ));
        reply.insert("id".to_owned(), Value::String(r.id));
        reply.insert("seconds".to_owned(), json!(seconds));
        reply
    }

    /// `/record/status`: whether it records, and the calls not sent yet.
    #[must_use]
    pub fn status(&self) -> Reply {
        let current = match &*lock(&self.now) {
            State::Recording(r) => Some((r.id.clone(), r.title.clone(), r.started, r.mic)),
            State::Idle | State::Starting => None,
        };
        let waiting = self.waiting(current.as_ref().map(|(id, ..)| id.as_str()));
        let mut text = match &current {
            Some((_, title, started, mic)) => format!(
                "Recording \"{title}\" for {}{}.",
                clock(self.clock.now().duration_since(*started).as_secs_f64()),
                if *mic { "" } else { " (Mac's sound only)" }
            ),
            None => "Not recording.".to_owned(),
        };
        if !waiting.is_empty() {
            push(
                &mut text,
                format_args!(
                    " Not sent to the server yet (tried every 5 minutes): {}.",
                    waiting.join(", ")
                ),
            );
        }
        let aside = folders(&self.aside(), None);
        if !aside.is_empty() {
            push(
                &mut text,
                format_args!(
                    " Set aside, as the server refused them or they failed too often (kept in \
                     Emberhut Hands' calls-unsent folder): {}.",
                    aside.join(", ")
                ),
            );
        }
        let mut reply = said(text);
        reply.insert(
            "recording".to_owned(),
            current.map_or(Value::Null, |(id, ..)| Value::String(id)),
        );
        reply.insert("waiting".to_owned(), json!(waiting));
        reply
    }

    /// The calls kept here, but for `recording`.
    fn waiting(&self, recording: Option<&str>) -> Vec<String> {
        folders(&self.calls, recording)
    }

    /// Sends every call kept here but the one recording, in the background: after a restart,
    /// and every few minutes.
    pub fn resend(&self) {
        let current = self.current().map(|(id, _)| id);
        for id in self.waiting(current.as_deref()) {
            self.send_later(self.calls.join(id));
        }
    }

    /// Sends the call in `dir` on a thread of its own.
    fn send_later(&self, dir: PathBuf) {
        let Some(server) = self.server.clone() else {
            return;
        };
        let uploader = Arc::clone(&self.uploader);
        let sending = Arc::clone(&self.sending);
        let aside = self.aside();
        std::thread::spawn(move || {
            let _one = lock(&sending);
            send_call(uploader.as_ref(), &server, &dir, &aside);
        });
    }
}

/// The folders in `dir` (their names, sorted), but for `except`.
fn folders(dir: &Path, except: Option<&str>) -> Vec<String> {
    let mut ids: Vec<String> = fs::read_dir(dir).map_or_default(|entries| {
        entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|id| Some(id.as_str()) != except)
            .collect()
    });
    ids.sort();
    ids
}

/// Writes `bytes` to a new owner-only file at `path`.
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?
        .write_all(bytes)
}

/// Sends the call in `dir`'s tracks (the last marked so) and deletes it once both arrived. A
/// call the server refuses for good, or that failed 48 times, is moved to `aside`.
pub fn send_call(uploader: &dyn Uploader, server: &Server, dir: &Path, aside: &Path) {
    if !dir.is_dir() {
        return;
    }
    let id = dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    let mut meta: Value = fs::read(dir.join(CALL_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| json!({}));
    let seconds = meta.get("seconds").and_then(Value::as_f64).unwrap_or(0.0);
    let title = meta
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let tracks: Vec<Track> = Track::ALL
        .into_iter()
        .filter(|track| dir.join(format!("{}.m4a", track.name())).is_file())
        .collect();
    for (i, track) in tracks.iter().enumerate() {
        let url = track_url(
            &server.url,
            &id,
            *track,
            seconds,
            &title,
            i + 1 == tracks.len(),
        );
        let file = dir.join(format!("{}.m4a", track.name()));
        if let Err(why) = uploader.post(&url, &server.token, Upload::File(file)) {
            let tries = meta.get("tries").and_then(Value::as_u64).unwrap_or(0) + 1;
            if why.permanent() || tries >= MAX_TRIES {
                set_aside(dir, aside, &id, &why);
                return;
            }
            if let Some(object) = meta.as_object_mut() {
                object.insert("tries".to_owned(), json!(tries));
            }
            if let Err(error) = write_private(&dir.join(CALL_FILE), meta.to_string().as_bytes()) {
                tracing::warn!(%id, %error, "a call's tries were not counted");
            }
            tracing::warn!(%id, track = track.name(), %why, tries, "a track was not sent; tried again later");
            return;
        }
    }
    match fs::remove_dir_all(dir) {
        Ok(()) => tracing::info!(%id, "a call was sent to the server and deleted here"),
        Err(error) => tracing::warn!(%id, %error, "a sent call was not deleted"),
    }
}

/// Moves the call in `dir` to `aside`, as it won't be sent; a call of the same id set aside
/// before keeps its place, and this one gets a name of its own (`id-2`, `id-3`…).
fn set_aside(dir: &Path, aside: &Path, id: &str, why: &UploadError) {
    let place = (1..=10_000_u32)
        .map(|n| {
            if n == 1 {
                aside.join(id)
            } else {
                aside.join(format!("{id}-{n}"))
            }
        })
        .find(|place| !place.exists())
        .unwrap_or_else(|| aside.join(id));
    let moved = private_dir(aside).and_then(|()| fs::rename(dir, place));
    match moved {
        Ok(()) => tracing::warn!(%id, %why, "a call was set aside: the server won't take it"),
        Err(error) => tracing::warn!(%id, %error, "a call that won't be sent was not set aside"),
    }
}

#[cfg(test)]
mod tests;
