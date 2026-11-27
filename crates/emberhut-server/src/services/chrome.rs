//! The owner's own Chrome profiles, each with the browser extension paired to this server: the
//! extension keeps a WebSocket to `/api/chrome/ws`, and relays the browser tools' `DevTools`
//! commands to `chrome.debugger` in the assistant's own tabs there, so the same tools (and the
//! same guards) work in the owner's real logins. What the assistant may touch there (its own tab
//! group, no blocked sites, nothing browser-wide) is enforced by the extension itself, on the
//! owner's machine; this side keeps the owner's pause (and ends it on time), the hand-off and
//! the task's log.
//!
//! Pairing: the owner asks for a link (the app, or the `chrome_pair` tool) and opens it in that
//! profile (`/chrome/pair#CODE`); the extension sees it and pairs (`pair` with the code), and gets
//! its own token, kept hashed. After that it says `hello` with its token each time it connects.
//! A paired profile is the assistant's to work in any time; the owner's pause lasts at most 10
//! minutes, or until they hand back.
//!
//! The extension (`apps/extension`) speaks the protocol in [`wire`]. The extension's zip is a build artifact this server serves
//! (`EMBERHUT_CHROME_EXTENSION_ZIP`); on the owner's Mac the Mac helper keeps the extension's folder
//! in step with it (the server checks it at start-up and every few minutes, and when a profile
//! there runs an older version), and a profile there that runs an older version is told to
//! reload.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::io::Read as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use emberhut_api::browser::ChromePairStatus;
use emberhut_api::chrome::{
    ChromeGrant, ChromePairing, ChromePause as ChromePaused, ChromeProfile, ChromeReply, extension,
};
use emberhut_api::time::Timestamp as WireTime;
use emberhut_core::RandomSource;
use emberhut_core::browser::{mask, site};
use emberhut_core::chrome::{
    clean_label, code_of, extension_id, new_code, new_token, older, show_code, slug, token_hash,
};
use emberhut_db::{ChromeAbout, ChromeProfileRow, ChromeStore, DbError};
use futures_util::future::BoxFuture;
use jiff::Timestamp;
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use crate::services::browser::Control;
use crate::services::browser::cdp::{CdpEngine, CdpEvent, CdpLink, Target};
use crate::services::browser::desk::{Card, NoteBy};
use crate::services::browser::engine::{BrowserEngine, EngineError, EngineResult, TabId};
use crate::services::browser::surface::{CLOSE_KEPT_MS, Kit, Surface, SurfaceState, TabClose, Who};
use crate::services::mac::{MacError, MacHelper};
use wire::Message;

/// The extension's messages ([`emberhut_api::chrome::extension`]).
pub mod wire {
    use emberhut_api::chrome::extension as native;

    /// A named message of the extension's protocol.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Message {
        /// Server → extension: start working (attach the assistant's tabs, report them).
        Start,
        /// Server → extension, after its hello: its name, label and the latest version.
        Hello,
        /// Server → extension: paired, with its token.
        Paired,
        /// Server → extension: the owner paused the assistant there.
        Pause,
        /// Server → extension: the pause is over.
        Resume,
        /// Server → extension: its new name.
        Name,
        /// Server → extension: a step the assistant took (its popup lists them).
        Step,
        /// Server → extension: the assistant handed the browser to the owner.
        Handoff,
        /// Both ways: the owner's turn is over.
        Handback,
        /// Server → extension: its folder holds a newer version; reload from it.
        Update,
        /// Extension → server: the owner paused it, or it's over (no params).
        Stopped,
        /// Extension → server: the owner unpaired it there.
        Unpair,
        /// Extension → server: Chrome cut the link to a tab (another extension's frame), still
        /// open.
        Cut,
        /// Extension → server: a tab went to a blocked site; the assistant let go of it.
        Blocked,
        /// Extension → server: another extension's frame keeps it out of a tab for now.
        Framed,
    }

    impl Message {
        const ALL: [Self; 15] = [
            Self::Start,
            Self::Hello,
            Self::Paired,
            Self::Pause,
            Self::Resume,
            Self::Name,
            Self::Step,
            Self::Handoff,
            Self::Handback,
            Self::Update,
            Self::Stopped,
            Self::Unpair,
            Self::Cut,
            Self::Blocked,
            Self::Framed,
        ];

        /// Its name on the wire.
        #[must_use]
        pub const fn name(self) -> &'static str {
            match self {
                Self::Start => native::START,
                Self::Hello => native::HELLO,
                Self::Paired => native::PAIRED,
                Self::Pause => native::PAUSE,
                Self::Resume => native::RESUME,
                Self::Name => native::NAME,
                Self::Step => native::STEP,
                Self::Handoff => native::HANDOFF,
                Self::Handback => native::HANDBACK,
                Self::Update => native::UPDATE,
                Self::Stopped => native::STOPPED,
                Self::Unpair => native::UNPAIR,
                Self::Cut => native::CUT,
                Self::Blocked => native::BLOCKED,
                Self::Framed => native::FRAMED,
            }
        }

        /// The message named `name`.
        #[must_use]
        pub fn read(name: &str) -> Option<Self> {
            Self::ALL.into_iter().find(|m| m.name() == name)
        }
    }

    /// Extension → server: ready after a start.
    pub const INITIALIZED: &str = "extension.initialized";
    /// Who closed a tab, or handed back: the assistant itself.
    pub const BY_ASSISTANT: &str = native::BY_ASSISTANT;
    /// The extension's zip's file name, before its version.
    pub const ZIP_NAME: &str = native::ZIP_NAME;
}

/// The server's own browser, as the tools' `browser` argument names it (agents' prompts and
/// tasks carry it), in
/// [`emberhut_api::wire::OWN_BROWSER`].
pub const OWN_BROWSER: &str = emberhut_api::wire::OWN_BROWSER;

/// `POST /api/chrome`'s body, read leniently (the contract's typed form is
/// [`emberhut_api::chrome::ChromeAction`]): what is missing or unusable is refused by
/// [`ChromeService::input`] with a message saying what.
#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize)]
pub struct ChromeInput {
    /// What to do.
    #[serde(default)]
    pub kind: Option<String>,
    /// The profile.
    #[serde(default)]
    pub id: Option<f64>,
    /// The profile, by its short name.
    #[serde(default)]
    pub name: Option<String>,
    /// A label.
    #[serde(default)]
    pub label: Option<String>,
    /// A pairing code.
    #[serde(default)]
    pub code: Option<String>,
    /// How long a pause lasts.
    #[serde(default)]
    pub minutes: Option<f64>,
}

/// How long a pairing link works.
const LINK_LIFE_MS: i64 = 30 * 60_000;
/// How long a used link still shows.
const DONE_SHOWN_MS: i64 = 120_000;
/// The longest pause.
const PAUSE_MS: i64 = 10 * 60_000;
/// How long a call to the extension may take.
const CALL_WAIT: Duration = Duration::from_secs(30);
/// How long a socket may wait before its hello.
const HELLO_WAIT: Duration = Duration::from_secs(10);
/// A profile unused this long is let go (its tabs stay open), so the owner's screen can sleep.
const LET_GO_MS: i64 = 5 * 60_000;
/// How long after a try to bring the folder, or one profile, up to one version the next may
/// start.
const UPDATE_EVERY_MS: i64 = 10 * 60_000;
/// How many tries one version gets: to be installed in the folder, and to bring one profile up
/// to it; then the profile is left showing as out of date.
const UPDATE_TRIES: u8 = 3;
/// How often the server checks the Mac helper's folder against the zip on its own
/// ([`ChromeService::keep_folder`]). Half of the 10 minutes between two tries at one version:
/// a retry after a failed install comes at most 5 minutes after it may, however the rounds fall
/// against the last try, and a fresh setup or a new build reaches the folder within 5 minutes
/// of the helper answering. A round whose folder is current costs one small request to the
/// helper.
pub const FOLDER_EVERY: Duration = Duration::from_mins(5);

/// Why the extension's folder on the Mac wasn't brought up to the zip served.
#[derive(Debug, thiserror::Error)]
enum FolderError {
    /// Asked what its folder holds, the helper isn't set up, didn't answer, or refused.
    #[error(transparent)]
    Mac(#[from] MacError),
    /// No zip is served, or it couldn't be read.
    #[error("no extension zip is set up on this server, or it couldn't be read")]
    NoZip,
    /// The zip's manifest key doesn't give the extension's fixed id.
    #[error(
        "the zip served isn't the extension (its key doesn't give {})",
        extension::ID
    )]
    NotOurs,
    /// The zip's version was tried less than 10 minutes ago.
    #[error("{0} was tried less than 10 minutes ago")]
    Waiting(String),
    /// The zip's version was tried as many times as it may be.
    #[error(
        "{0} was tried {UPDATE_TRIES} times already, so the folder stays as it is until another \
         build is served"
    )]
    Spent(String),
    /// The install didn't go through: the helper refused it, or stopped answering during it.
    #[error("{}", install_words(.0))]
    Install(#[source] MacError),
    /// The helper unpacked an older version than the zip's.
    #[error("the Mac helper's folder holds {0} after the install")]
    Behind(String),
}

impl FolderError {
    /// What went wrong, without its words: failures of one kind are said once per version.
    const fn kind(&self) -> &'static str {
        match self {
            Self::Mac(MacError::NotSetUp) => "no helper",
            Self::Mac(MacError::Unreachable(_)) => "helper off",
            Self::Mac(MacError::Refused(_)) => "helper refused",
            Self::NoZip => "no zip",
            Self::NotOurs => "not ours",
            Self::Waiting(_) => "waiting",
            Self::Spent(_) => "spent",
            Self::Install(MacError::NotSetUp | MacError::Refused(_)) => "install refused",
            Self::Install(MacError::Unreachable(_)) => "install unanswered",
            Self::Behind(_) => "behind",
        }
    }
}

/// Why an install didn't go through, in the activity log's words.
fn install_words(error: &MacError) -> String {
    match error {
        MacError::Unreachable(_) => "the Mac helper didn't answer the install (it is off, asleep, \
                                     or took too long to unpack it)"
            .to_owned(),
        MacError::Refused(words) => format!("the Mac helper refused the install: {words}"),
        MacError::NotSetUp => error.to_string(),
    }
}

/// The helper's folder at the zip's version or newer.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Filled {
    /// The zip's version.
    version: String,
    /// Whether the zip was installed to get there.
    installed: bool,
}

/// Tries at one thing for one version: how many so far, and the last one's time.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Tries {
    count: u8,
    at: i64,
}

/// Why a try can't start now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NotYet {
    /// The last one was less than [`UPDATE_EVERY_MS`] ago.
    Soon,
    /// There were [`UPDATE_TRIES`] already.
    Spent,
}

impl Tries {
    /// Whether a try may start at `now`: at most [`UPDATE_TRIES`], [`UPDATE_EVERY_MS`] apart.
    const fn check(&self, now: i64) -> Result<(), NotYet> {
        if self.count >= UPDATE_TRIES {
            Err(NotYet::Spent)
        } else if self.count > 0 && now - self.at < UPDATE_EVERY_MS {
            Err(NotYet::Soon)
        } else {
            Ok(())
        }
    }

    /// [`Self::check`], noting the try when it may start.
    fn take(&mut self, now: i64) -> Result<(), NotYet> {
        self.check(now)?;
        self.count += 1;
        self.at = now;
        Ok(())
    }
}

/// Installing one version of the zip in the helper's folder, whichever path asks (a profile's
/// hello or tick, or the server's own round), so that two can't install it at once or more
/// often than [`Tries`] allows: the installs tried, and the kinds of failure already logged.
#[derive(Debug, Default)]
struct InstallTry {
    tries: Tries,
    said: HashSet<&'static str>,
}

/// Bringing one profile up to one version: the tries so far, and whether the profile was told
/// to reload (and, after that, warned about).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct UpdateTry {
    tries: Tries,
    told: bool,
    warned: bool,
}

/// What to do now about bringing one profile up to one version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateStep {
    /// Nothing, for now or for good.
    Wait,
    /// Ask the helper (and tell the profile once the folder holds the version).
    Try,
    /// Say the profile loads the extension from another folder.
    Warn,
}

impl UpdateTry {
    /// The next step at `now`, noting it. `hello`: the profile just said hello, so the version
    /// it reports is the one it runs now. A profile told to reload is warned about only when a
    /// hello after that still reports the older version, never on the socket's tick (it may
    /// hold a tab and not have reloaded yet), and once.
    fn next(&mut self, hello: bool, now: i64) -> UpdateStep {
        if self.told {
            if !hello || std::mem::replace(&mut self.warned, true) {
                return UpdateStep::Wait;
            }
            return UpdateStep::Warn;
        }
        if self.tries.take(now).is_ok() {
            UpdateStep::Try
        } else {
            UpdateStep::Wait
        }
    }
}

/// Why a request about the Chrome profiles was refused (409 with its words).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ChromeError {
    /// Refused, with the words for the owner.
    #[error("{0}")]
    Refused(String),
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
}

/// What the Chrome service is built from.
#[derive(Clone)]
pub struct ChromeParts {
    /// The paired profiles.
    pub store: ChromeStore,
    /// What it shares with the browser.
    pub kit: Kit,
    /// Where pairing codes and tokens get their randomness.
    pub random: Arc<dyn RandomSource>,
    /// The address the owner's Chromes reach this server at, without a final `/`.
    pub base_url: String,
    /// The extension's build, served as a zip.
    pub extension_zip: Option<PathBuf>,
    /// The helper on the owner's Mac, which keeps the extension's folder there in step with the
    /// zip.
    pub mac: Arc<dyn MacHelper>,
}

impl fmt::Debug for ChromeParts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChromeParts")
            .field("base_url", &self.base_url)
            .field("extension_zip", &self.extension_zip)
            .finish_non_exhaustive()
    }
}

/// A pairing link.
#[derive(Clone, Debug)]
struct Code {
    label: String,
    name: String,
    device: Option<i64>,
    until: i64,
    done: Option<(i64, i64)>,
}

/// The owner's pause there.
#[derive(Clone, Debug, PartialEq)]
struct Pause {
    why: String,
    until: i64,
    turn: bool,
    task: Option<i64>,
}

/// What goes to the extension.
#[derive(Debug)]
pub enum Outgoing {
    /// A message.
    Text(String),
    /// Close the socket, with a code and a reason.
    Close(u16, String),
}

type Answer = oneshot::Sender<Result<Value, String>>;

#[derive(Debug, Default)]
struct ConnState {
    name: String,
    label: String,
    version: Option<String>,
    os: Option<String>,
    surface: SurfaceState,
    control: Control,
    handoff: Option<String>,
    paused: Option<Pause>,
    current: Option<TabId>,
    closes: Vec<TabClose>,
    blocked: Option<String>,
    used: i64,
    started: bool,
    ready: bool,
    known: BTreeMap<i64, Value>,
    attached: HashSet<i64>,
}

/// One connected profile.
pub struct Conn {
    id: i64,
    out: mpsc::UnboundedSender<Outgoing>,
    calls: Mutex<HashMap<u64, Answer>>,
    next: AtomicU64,
    state: Mutex<ConnState>,
    events: broadcast::Sender<CdpEvent>,
    ready: tokio::sync::Notify,
    engine: Mutex<Option<Arc<CdpEngine>>>,
}

impl fmt::Debug for Conn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Conn")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Conn {
    fn state(&self) -> std::sync::MutexGuard<'_, ConnState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "each message's params, made at its call"
    )]
    fn tell(&self, message: Message, params: Value) {
        drop(self.out.send(Outgoing::Text(
            json!({ "method": message.name(), "params": params }).to_string(),
        )));
    }

    fn close(&self, code: u16, reason: &str) {
        drop(self.out.send(Outgoing::Close(code, reason.to_owned())));
    }

    fn label(&self) -> String {
        self.state().label.clone()
    }

    async fn call(&self, method: &str, params: Value) -> EngineResult<Value> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, tx);
        let label = self.label();
        if self
            .out
            .send(Outgoing::Text(
                json!({ "id": id, "method": method, "params": params }).to_string(),
            ))
            .is_err()
        {
            return Err(EngineError::Unavailable(format!(
                "The owner's {label} Chrome went away"
            )));
        }
        match tokio::time::timeout(CALL_WAIT, rx).await {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(error))) => Err(
                if error.contains("No tab with id") || error.contains("not attached") {
                    EngineError::TabClosed("The tab has been closed.".to_owned())
                } else {
                    EngineError::Failed(error)
                },
            ),
            Ok(Err(_)) => Err(EngineError::Unavailable(format!(
                "The owner's {label} Chrome went away"
            ))),
            Err(_) => {
                self.calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&id);
                Err(EngineError::Timeout(format!(
                    "The owner's {label} Chrome didn't answer ({method})"
                )))
            }
        }
    }

    fn engine(self: &Arc<Self>) -> Arc<CdpEngine> {
        let mut engine = self
            .engine
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(engine) = engine.as_ref() {
            return Arc::clone(engine);
        }
        let made = CdpEngine::new(
            Arc::new(ExtensionLink {
                conn: Arc::clone(self),
            }),
            false,
        );
        *engine = Some(Arc::clone(&made));
        made
    }

    /// Starts working there, once: the extension attaches the assistant's tabs and says so.
    async fn start(&self) -> EngineResult<()> {
        let now = Timestamp::now().as_millisecond();
        {
            let mut state = self.state();
            state.used = now;
            if state.ready {
                return Ok(());
            }
            if !state.started {
                state.started = true;
                drop(state);
                self.tell(Message::Start, json!({}));
            }
        }
        let ready = self.ready.notified();
        if self.state().ready {
            return Ok(());
        }
        if tokio::time::timeout(Duration::from_secs(10), ready)
            .await
            .is_err()
            && !self.state().ready
        {
            self.state().started = false;
            return Err(EngineError::Unavailable(format!(
                "The owner's {} Chrome didn't get ready",
                self.label()
            )));
        }
        Ok(())
    }

    /// Lets go of its tabs (they stay open and the assistant's; the next use takes them again).
    fn let_go(&self) {
        {
            let mut state = self.state();
            state.started = false;
            state.ready = false;
            state.attached.clear();
            state.current = None;
        }
        *self
            .engine
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.tell(Message::Start, json!({}));
    }
}

/// The extension's relay as a CDP link: each command goes to `chrome.debugger` in that tab.
#[derive(Debug)]
struct ExtensionLink {
    conn: Arc<Conn>,
}

impl ExtensionLink {
    async fn attach(&self, tab: i64) -> EngineResult<()> {
        if self.conn.state().attached.contains(&tab) {
            return Ok(());
        }
        self.conn
            .call("chrome.debugger.attach", json!([{ "tabId": tab }, "1.3"]))
            .await?;
        self.conn.state().attached.insert(tab);
        Ok(())
    }
}

fn tab_key(key: &str) -> EngineResult<i64> {
    key.parse()
        .map_err(|_| EngineError::TabClosed("The tab has been closed.".to_owned()))
}

impl CdpLink for ExtensionLink {
    fn connected(&self) -> bool {
        !self.conn.out.is_closed()
    }

    fn targets(&self) -> BoxFuture<'_, EngineResult<Vec<Target>>> {
        Box::pin(async move {
            self.conn.start().await?;
            let known: Vec<(i64, Value)> = self
                .conn
                .state()
                .known
                .iter()
                .map(|(k, v)| (*k, v.clone()))
                .collect();
            let mut targets = Vec::new();
            for (tab, info) in known {
                let mut url = info
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let mut title = info
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                if self.attach(tab).await.is_ok()
                    && let Ok(got) = self
                        .conn
                        .call(
                            "chrome.debugger.sendCommand",
                            json!([{ "tabId": tab }, "Target.getTargetInfo", {}]),
                        )
                        .await
                {
                    if let Some(u) = got.pointer("/targetInfo/url").and_then(Value::as_str) {
                        u.clone_into(&mut url);
                    }
                    if let Some(t) = got.pointer("/targetInfo/title").and_then(Value::as_str) {
                        t.clone_into(&mut title);
                    }
                }
                targets.push(Target {
                    key: tab.to_string(),
                    url,
                    title,
                });
            }
            Ok(targets)
        })
    }

    fn create(&self, url: Option<String>) -> BoxFuture<'_, EngineResult<Target>> {
        Box::pin(async move {
            self.conn.start().await?;
            let mut create = json!({});
            if let Some(url) = &url {
                create["url"] = json!(url);
            }
            let tab = self
                .conn
                .call("chrome.tabs.create", json!([create]))
                .await?;
            let id = tab
                .get("id")
                .and_then(Value::as_i64)
                .ok_or_else(|| EngineError::Failed("Chrome made no tab".to_owned()))?;
            self.conn.state().known.insert(id, tab.clone());
            self.attach(id).await?;
            Ok(Target {
                key: id.to_string(),
                url: url.unwrap_or_default(),
                title: String::new(),
            })
        })
    }

    fn close(&self, key: String) -> BoxFuture<'_, EngineResult<()>> {
        Box::pin(async move {
            let tab = tab_key(&key)?;
            self.conn.call("chrome.tabs.remove", json!([tab])).await?;
            let mut state = self.conn.state();
            state.known.remove(&tab);
            state.attached.remove(&tab);
            Ok(())
        })
    }

    fn activate(&self, _key: String) -> BoxFuture<'_, EngineResult<()>> {
        Box::pin(async { Ok(()) })
    }

    fn send(
        &self,
        key: String,
        method: &'static str,
        params: Value,
    ) -> BoxFuture<'_, EngineResult<Value>> {
        Box::pin(async move {
            self.conn.start().await?;
            let tab = tab_key(&key)?;
            self.attach(tab).await?;
            self.conn.state().used = Timestamp::now().as_millisecond();
            self.conn
                .call(
                    "chrome.debugger.sendCommand",
                    json!([{ "tabId": tab }, method, params]),
                )
                .await
        })
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.conn.events.subscribe()
    }
}

/// Where a browser tool acts.
#[derive(Debug)]
pub enum Pick {
    /// The server's own browser.
    Own,
    /// One of the owner's Chrome profiles.
    Chrome(Arc<dyn Surface>),
    /// Neither: what the agent is told.
    Said(String),
}

/// A file's modification time and length: when they change, it is read again.
type Stamp = (std::time::SystemTime, u64);

/// The extension's build, read when it changes.
#[derive(Debug, Default)]
struct Extension {
    stamp: Option<Stamp>,
    version: String,
    id: Option<String>,
}

/// The extension's zip read again: its stamp, its manifest's version and the id its key gives,
/// and its bytes when it could be read.
struct Build {
    stamp: Option<Stamp>,
    version: String,
    id: Option<String>,
    bytes: Option<Vec<u8>>,
}

/// The zip at `path` read again unless its stamp is still `known` (blocking file work).
fn read_build(path: &std::path::Path, known: Option<Stamp>) -> Option<Build> {
    let stamp = std::fs::metadata(path)
        .ok()
        .and_then(|m| Some((m.modified().ok()?, m.len())));
    if stamp == known {
        return None;
    }
    let bytes = stamp.and_then(|_| std::fs::read(path).ok());
    let manifest = bytes.as_deref().and_then(zip_manifest);
    let field = |name: &str| {
        manifest
            .as_ref()
            .and_then(|m| m.get(name))
            .and_then(Value::as_str)
    };
    Some(Build {
        stamp,
        version: field("version").unwrap_or_default().to_owned(),
        id: field("key").and_then(extension_id),
        bytes,
    })
}

#[derive(Debug, Default)]
struct Book {
    codes: BTreeMap<String, Code>,
    conns: BTreeMap<i64, Arc<Conn>>,
    pauses: HashMap<i64, Pause>,
    which: HashMap<i64, (Timestamp, Vec<i64>)>,
    extension: Extension,
    /// Each profile's tries to be brought up to each version.
    updates: HashMap<(i64, String), UpdateTry>,
    /// Each version's installs in the helper's folder.
    installs: HashMap<String, InstallTry>,
}

/// The owner's Chrome profiles. Cheap to clone.
#[derive(Clone)]
pub struct ChromeService {
    inner: Arc<Inner>,
}

struct Inner {
    parts: ChromeParts,
    book: Mutex<Book>,
}

impl fmt::Debug for ChromeService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChromeService").finish_non_exhaustive()
    }
}

fn wire_time(ms: i64) -> WireTime {
    let at = Timestamp::from_millisecond(ms).unwrap_or(Timestamp::UNIX_EPOCH);
    WireTime::new(at.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
}

fn hhmm_ms(ms: i64) -> String {
    let text = wire_time(ms).as_str().to_owned();
    text.get(11..16).unwrap_or_default().to_owned()
}

fn text_of(value: Option<&Value>, n: usize) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(|s| s.chars().take(n).collect())
}

impl ChromeService {
    /// The service over `parts`.
    #[must_use]
    pub fn new(parts: ChromeParts) -> Self {
        Self {
            inner: Arc::new(Inner {
                parts,
                book: Mutex::new(Book::default()),
            }),
        }
    }

    fn book(&self) -> std::sync::MutexGuard<'_, Book> {
        self.inner
            .book
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn kit(&self) -> &Kit {
        &self.inner.parts.kit
    }

    fn store(&self) -> &ChromeStore {
        &self.inner.parts.store
    }

    fn now_ms(&self) -> i64 {
        self.kit().clock.now().as_millisecond()
    }

    fn online(&self, id: i64) -> Option<Arc<Conn>> {
        self.book().conns.get(&id).cloned()
    }

    // ── The extension's build ──

    /// The extension's version the server serves (`chromeLatest`), from the zip's
    /// `manifest.json`; `0` without one.
    #[must_use]
    pub fn latest(&self) -> String {
        self.read_extension();
        let book = self.book();
        if book.extension.version.is_empty() {
            "0".to_owned()
        } else {
            book.extension.version.clone()
        }
    }

    fn extension_id(&self) -> Option<String> {
        self.read_extension();
        self.book().extension.id.clone()
    }

    fn read_extension(&self) {
        let Some(path) = self.inner.parts.extension_zip.as_deref() else {
            return;
        };
        let known = self.book().extension.stamp;
        if let Some(build) = read_build(path, known) {
            self.keep_build(build);
        }
    }

    /// [`Self::read_extension`] off the async runtime: the zip as read, when it changed.
    async fn refresh_extension(&self) -> Option<Build> {
        let path = self.inner.parts.extension_zip.clone()?;
        let known = self.book().extension.stamp;
        let build = tokio::task::spawn_blocking(move || read_build(&path, known))
            .await
            .ok()
            .flatten()?;
        let mut book = self.book();
        book.extension.stamp = build.stamp;
        book.extension.version.clone_from(&build.version);
        book.extension.id.clone_from(&build.id);
        drop(book);
        Some(build)
    }

    fn keep_build(&self, build: Build) {
        let mut book = self.book();
        book.extension.stamp = build.stamp;
        book.extension.version = build.version;
        book.extension.id = build.id;
    }

    /// The extension's zip and its download name.
    #[must_use]
    pub fn extension_zip(&self) -> Option<(Vec<u8>, String)> {
        let path = self.inner.parts.extension_zip.clone()?;
        let bytes = std::fs::read(path).ok()?;
        Some((bytes, format!("{}-{}.zip", wire::ZIP_NAME, self.latest())))
    }

    // ── Keeping the extension's folder on the Mac current ──

    /// Keeps the Mac helper's folder at the zip served on its own, so a fresh setup needs no
    /// profile running the extension first: a round as soon as it starts, then every `every`,
    /// until `stop`. Each round asks the helper what the folder holds and installs the zip when
    /// the folder is empty or older (under the same limit as a profile's own tries), then tells
    /// the profiles on the Mac still older to reload. A helper that doesn't answer, or refuses to
    /// say (one without `/extension` answers 404), is asked again next round. It ends without a
    /// zip, or without a helper set up on this server (only the stand-in for none says so).
    pub async fn keep_folder(self, every: Duration, stop: CancellationToken) {
        if self.inner.parts.extension_zip.is_none() {
            return;
        }
        let mut ticks = tokio::time::interval(every);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            let filled = tokio::select! {
                () = stop.cancelled() => return,
                _ = ticks.tick() => tokio::select! {
                    () = stop.cancelled() => return,
                    filled = self.fill_folder() => filled,
                },
            };
            match filled {
                Ok(filled) => self.tell_behind(&filled.version),
                Err(FolderError::Mac(MacError::NotSetUp)) => return,
                Err(_) => {}
            }
        }
    }

    /// A profile on the Mac that runs an older version than the zip served: the Mac helper's folder is brought up to the zip (when it is behind)
    /// and the profile told once to reload from it. A try at most every 10 minutes, at most
    /// [`UPDATE_TRIES`] per profile and version (and the installs those tries ask for count
    /// against the version's own limit, shared with [`Self::keep_folder`]); a profile told that
    /// says hello again still running the older version loads the extension from somewhere
    /// else, so it isn't told again (it shows as out of date). `hello`: called for the
    /// profile's hello, not the socket's tick.
    fn keep_current(&self, conn: &Arc<Conn>, hello: bool) {
        let latest = self.latest();
        if latest == "0" || !behind(conn, &latest) {
            return;
        }
        let now = self.now_ms();
        let step = self
            .book()
            .updates
            .entry((conn.id, latest.clone()))
            .or_default()
            .next(hello, now);
        if step == UpdateStep::Wait {
            return;
        }
        let warn = step == UpdateStep::Warn;
        let service = self.clone();
        let conn = Arc::clone(conn);
        if warn {
            let version = conn.state().version.clone();
            tokio::spawn(async move {
                service
                    .kit()
                    .log(format!(
                        "The owner's {} Chrome still runs {} after it was told to reload {latest}: \
                         it loads the extension from another folder than the Mac helper's, so it \
                         shows as out of date",
                        conn.label(),
                        version.as_deref().unwrap_or("an unknown version")
                    ))
                    .await;
            });
            return;
        }
        tokio::spawn(async move { service.update(&conn).await });
    }

    /// Brings the folder up to the zip and tells the profile to reload from it; after an install,
    /// every other profile on the Mac still older too (one may have found the install under way).
    async fn update(&self, conn: &Arc<Conn>) {
        if let Ok(filled) = self.fill_folder().await {
            self.tell_update(conn, &filled.version);
            if filled.installed {
                self.tell_behind(&filled.version);
            }
        }
    }

    /// Tells every profile on the Mac still older than `version` to reload from the helper's
    /// folder, which holds it now.
    fn tell_behind(&self, version: &str) {
        let conns: Vec<Arc<Conn>> = self.book().conns.values().cloned().collect();
        for conn in conns.iter().filter(|conn| behind(conn, version)) {
            self.tell_update(conn, version);
        }
    }

    /// Tells `conn` to reload from the helper's folder, which holds `version` now, unless it was
    /// told already: once per profile and version.
    fn tell_update(&self, conn: &Arc<Conn>, version: &str) {
        let told = std::mem::replace(
            &mut self
                .book()
                .updates
                .entry((conn.id, version.to_owned()))
                .or_default()
                .told,
            true,
        );
        if !told {
            conn.tell(Message::Update, json!({ "version": version }));
        }
    }

    /// [`Self::bring_up`], whichever path asks, with what went wrong logged once per zip version
    /// and kind of failure (an install that went unanswered and a version out of tries too; a line
    /// the activity log failed to keep is said again at the next such failure); a helper that
    /// doesn't answer what its folder holds (it is asked again later) and a version waiting for
    /// its next try only at debug, and no helper set up not at all.
    async fn fill_folder(&self) -> Result<Filled, FolderError> {
        let filled = self.bring_up().await;
        if let Err(error) = &filled {
            self.folder_failed(error).await;
        }
        filled
    }

    /// Says why the folder wasn't brought up to the zip, as [`Self::fill_folder`] says.
    async fn folder_failed(&self, error: &FolderError) {
        match error {
            FolderError::Mac(MacError::NotSetUp) => return,
            FolderError::Mac(MacError::Unreachable(_)) | FolderError::Waiting(_) => {
                tracing::debug!(%error, "the extension's folder on the Mac isn't at the zip yet");
                return;
            }
            _ => {}
        }
        // Claimed before the write, so two paths failing at once say it once; given back when
        // the line isn't kept, so the next failure of the kind says it again.
        let version = self.book().extension.version.clone();
        let first = self
            .book()
            .installs
            .entry(version.clone())
            .or_default()
            .said
            .insert(error.kind());
        if !first {
            return;
        }
        let kept = self
            .kit()
            .log_kept(format!(
                "Couldn't bring the extension's folder on the Mac up to the zip served: {error}"
            ))
            .await;
        if !kept && let Some(install) = self.book().installs.get_mut(&version) {
            install.said.remove(error.kind());
        }
    }

    /// The helper's folder at the zip's version or newer: the zip is installed when the folder
    /// is behind. The helper is asked first; the zip's version and id are those of its last read
    /// (read again, off the async runtime, only when the file changed). Only when the folder is
    /// behind and the version may be tried is the zip read for the install (unless it was just
    /// read), so it is read at most once a call, its id and version coming from those bytes.
    async fn bring_up(&self) -> Result<Filled, FolderError> {
        let mac = &self.inner.parts.mac;
        let folder = mac.extension().await?;
        // Just read: the version and id are that read's own, whatever changed since.
        let fresh = self.refresh_extension().await;
        let (latest, id) = if let Some(build) = &fresh {
            (build.version.clone(), build.id.clone())
        } else {
            let book = self.book();
            (book.extension.version.clone(), book.extension.id.clone())
        };
        if latest.is_empty() {
            return Err(FolderError::NoZip);
        }
        if id.as_deref() != Some(extension::ID) {
            return Err(FolderError::NotOurs);
        }
        let current = |version| Filled {
            version,
            installed: false,
        };
        if !older(folder.version.as_deref(), &latest) {
            return Ok(current(latest));
        }
        let now = self.now_ms();
        self.install_tries(&latest, |tries| tries.check(now))?;
        let (zip, version) = match fresh.and_then(|build| build.bytes) {
            Some(zip) => (zip, latest),
            None => self.read_zip().await?,
        };
        if !older(folder.version.as_deref(), &version) {
            return Ok(current(version));
        }
        self.install_tries(&version, |tries| tries.take(now))?;
        let installed = mac
            .install_extension(zip)
            .await
            .map_err(FolderError::Install)?;
        if older(installed.version.as_deref(), &version) {
            return Err(FolderError::Behind(
                installed.version.unwrap_or_else(|| "nothing".to_owned()),
            ));
        }
        self.kit()
            .log(format!(
                "Brought the extension's folder on the Mac to {version}"
            ))
            .await;
        Ok(Filled {
            version,
            installed: true,
        })
    }

    /// `step` on `version`'s install tries, shared by every path; why a try can't start, as the
    /// error.
    fn install_tries(
        &self,
        version: &str,
        step: impl FnOnce(&mut Tries) -> Result<(), NotYet>,
    ) -> Result<(), FolderError> {
        let done = step(
            &mut self
                .book()
                .installs
                .entry(version.to_owned())
                .or_default()
                .tries,
        );
        done.map_err(|why| match why {
            NotYet::Soon => FolderError::Waiting(version.to_owned()),
            NotYet::Spent => FolderError::Spent(version.to_owned()),
        })
    }

    /// The zip served, read once (its manifest off the async runtime): its bytes and version,
    /// when its key gives the extension's id.
    async fn read_zip(&self) -> Result<(Vec<u8>, String), FolderError> {
        let path = self
            .inner
            .parts
            .extension_zip
            .clone()
            .ok_or(FolderError::NoZip)?;
        let zip = tokio::fs::read(path)
            .await
            .map_err(|_| FolderError::NoZip)?;
        let (zip, manifest) = tokio::task::spawn_blocking(move || {
            let manifest = zip_manifest(&zip);
            (zip, manifest)
        })
        .await
        .map_err(|_| FolderError::NoZip)?;
        let manifest = manifest.ok_or(FolderError::NoZip)?;
        let field = |name: &str| {
            manifest
                .get(name)
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        if field("key").as_deref().and_then(extension_id).as_deref() != Some(extension::ID) {
            return Err(FolderError::NotOurs);
        }
        let version = field("version").ok_or(FolderError::NotOurs)?;
        Ok((zip, version))
    }

    // ── Pairing ──

    fn link_of(&self, raw: &str) -> String {
        format!(
            "{}/chrome/pair#{}",
            self.inner.parts.base_url,
            show_code(raw)
        )
    }

    fn sweep_codes(&self) {
        let now = self.now_ms();
        self.book().codes.retain(|_, c| match c.done {
            Some((_, at)) => now - at <= DONE_SHOWN_MS,
            None => c.until >= now,
        });
    }

    /// A pairing link for a new profile (`label`) or a new token for a paired one (`device`).
    ///
    /// # Errors
    ///
    /// [`ChromeError::Refused`]: no such profile, a label that won't do, a name taken.
    pub async fn pair_code(
        &self,
        label: &str,
        device: Option<i64>,
    ) -> Result<ChromePairing, ChromeError> {
        self.sweep_codes();
        let (label, name) = if let Some(device) = device {
            let row = self
                .store()
                .profile(device)
                .await?
                .ok_or_else(|| ChromeError::Refused("No such Chrome profile".to_owned()))?;
            self.book()
                .codes
                .retain(|_, c| !(c.device == Some(device) && c.done.is_none()));
            (row.label().to_owned(), row.name.clone())
        } else {
            let label = clean_label(label).map_err(|e| ChromeError::Refused(e.to_string()))?;
            let name = slug(&label);
            if let Some(same) = self
                .store()
                .profiles()
                .await?
                .into_iter()
                .find(|r| r.name == name)
            {
                return Err(ChromeError::Refused(format!(
                    "There's already a Chrome profile called {}: re-pair it, or pick another name",
                    same.label()
                )));
            }
            self.book()
                .codes
                .retain(|_, c| !(c.name == name && c.device.is_none() && c.done.is_none()));
            (label, name)
        };
        let raw = new_code(self.inner.parts.random.as_ref())
            .map_err(|e| ChromeError::Refused(format!("No code could be made: {e}")))?;
        let until = self.now_ms() + LINK_LIFE_MS;
        self.book().codes.insert(
            raw.clone(),
            Code {
                label: label.clone(),
                name: name.clone(),
                device,
                until,
                done: None,
            },
        );
        Ok(ChromePairing {
            code: emberhut_api::ids::ChromeCode::new(show_code(&raw)),
            link: self.link_of(&raw),
            label,
            name,
            device: device.map(emberhut_api::ids::ChromeProfileId::new),
            until: wire_time(until),
            paired: false,
        })
    }

    /// The links waiting to be opened, and the ones just used.
    #[must_use]
    pub fn pairings(&self) -> Vec<ChromePairing> {
        self.sweep_codes();
        let now = self.now_ms();
        self.book()
            .codes
            .iter()
            .filter(|(_, c)| {
                c.done
                    .map_or(c.until > now, |(_, at)| now - at < DONE_SHOWN_MS)
            })
            .map(|(raw, c)| ChromePairing {
                code: emberhut_api::ids::ChromeCode::new(show_code(raw)),
                link: self.link_of(raw),
                label: c.label.clone(),
                name: c.name.clone(),
                device: c
                    .device
                    .or(c.done.map(|(id, _)| id))
                    .map(emberhut_api::ids::ChromeProfileId::new),
                until: wire_time(c.until),
                paired: c.done.is_some(),
            })
            .collect()
    }

    /// How a link stands, for its page.
    pub async fn pair_status(&self, code: &str) -> ChromePairStatus {
        let raw = code_of(code);
        let found = self.book().codes.get(&raw).cloned();
        let now = self.now_ms();
        match found {
            None => expired(),
            Some(c) if c.done.is_none() && c.until < now => expired(),
            Some(c) => match c.done {
                None => ChromePairStatus {
                    state: "waiting".to_owned(),
                    label: Some(c.label),
                    name: None,
                    email: None,
                    machine: None,
                },
                Some((id, _)) => {
                    let row = self.store().profile(id).await.ok().flatten();
                    ChromePairStatus {
                        state: "paired".to_owned(),
                        label: Some(
                            row.as_ref()
                                .map_or(c.label.clone(), |r| r.label().to_owned()),
                        ),
                        name: Some(row.as_ref().map_or(c.name.clone(), |r| r.name.clone())),
                        email: row
                            .as_ref()
                            .and_then(|r| r.email.clone())
                            .filter(|e| !e.is_empty()),
                        machine: row.and_then(|r| r.machine),
                    }
                }
            },
        }
    }

    /// A profile by what an agent or the owner calls it: its name, its label, its account or its
    /// id.
    async fn profile(&self, what: &str) -> Result<Option<ChromeProfileRow>, DbError> {
        let w = strip_chrome(what).to_lowercase();
        Ok(self.store().profiles().await?.into_iter().find(|r| {
            r.name == w
                || r.label().to_lowercase() == w
                || r.email
                    .as_deref()
                    .is_some_and(|e| !e.is_empty() && e.to_lowercase() == w)
                || r.id.to_string() == w
        }))
    }

    async fn unpair(&self, id: i64) -> Result<bool, ChromeError> {
        let Some(row) = self.store().profile(id).await? else {
            return Ok(false);
        };
        self.store().revoke(id, self.kit().clock.now()).await?;
        if let Some(conn) = self.online(id) {
            conn.close(1008, "This Chrome profile was unpaired");
        }
        self.kit()
            .log(format!(
                "Unpaired the owner's {} Chrome profile",
                row.label()
            ))
            .await;
        Ok(true)
    }

    async fn rename(&self, id: i64, label: &str) -> Result<ChromeProfileRow, ChromeError> {
        let row = self
            .store()
            .profile(id)
            .await?
            .ok_or_else(|| ChromeError::Refused("No such Chrome profile".to_owned()))?;
        let label = clean_label(label).map_err(|e| ChromeError::Refused(e.to_string()))?;
        let name = slug(&label);
        if let Some(same) = self
            .store()
            .profiles()
            .await?
            .into_iter()
            .find(|r| r.name == name && r.id != id)
        {
            return Err(ChromeError::Refused(format!(
                "There's already a Chrome profile called {}",
                same.label()
            )));
        }
        self.store().rename(id, &name, &label).await?;
        if let Some(conn) = self.online(id) {
            {
                let mut state = conn.state();
                state.label.clone_from(&label);
                state.name.clone_from(&name);
            }
            conn.tell(Message::Name, json!({ "name": name, "label": label }));
        }
        if row.label() != label {
            self.kit()
                .log(format!(
                    "Renamed the owner's {} Chrome profile to {label}",
                    row.label()
                ))
                .await;
            self.kit()
                .desk
                .wake(
                    "chrome",
                    format!(
                        "The owner renamed their Chrome profile \"{}\" to \"{label}\": use browser \
                         \"chrome:{name}\" for it from now on.",
                        row.label()
                    ),
                )
                .await;
        }
        self.store()
            .profile(id)
            .await?
            .ok_or_else(|| ChromeError::Refused("No such Chrome profile".to_owned()))
    }

    // ── The app's view and its input ──

    /// The profiles as the app shows them (`GET /api/state`'s `chrome`).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn summary(&self) -> Result<Vec<ChromeProfile>, DbError> {
        let latest = self.latest();
        let ext_id = self.extension_id();
        let mut out = Vec::new();
        for row in self.store().profiles().await? {
            let conn = self.online(row.id);
            let paused = conn.as_ref().and_then(|c| self.paused_of(c));
            let state = conn.as_ref().map(|c| {
                let s = c.state();
                (s.control, s.handoff.clone(), s.surface.clone())
            });
            let (control, handoff, surface) = state.unwrap_or_default();
            out.push(ChromeProfile {
                id: emberhut_api::ids::ChromeProfileId::new(row.id),
                name: row.name.clone(),
                label: row.label().to_owned(),
                email: row.email.clone().filter(|e| !e.is_empty()),
                machine: row.machine.clone(),
                os: row.os.clone(),
                version: row.version.clone(),
                browser: row.agent.clone(),
                outdated: older(row.version.as_deref(), &latest),
                connected: conn.is_some(),
                seen: row.seen.map(|s| wire_time(s.as_millisecond())),
                created: Some(wire_time(row.created.as_millisecond())),
                ext: row.ext.clone(),
                old_copy: row
                    .ext
                    .as_ref()
                    .zip(ext_id.as_ref())
                    .is_some_and(|(e, id)| e != id),
                grant: (conn.is_some() && paused.is_none()).then(|| ChromeGrant {
                    until: None,
                    task: None,
                    by: "paired".to_owned(),
                }),
                paused: paused.map(|p| ChromePaused {
                    why: p.why,
                    #[expect(
                        clippy::cast_precision_loss,
                        reason = "ms since the epoch, as the wire has it"
                    )]
                    until: p.until as f64,
                }),
                control: control.into(),
                handoff,
                url: surface.url,
                title: surface.title,
                driver: surface.driver,
                task: surface.task.map(emberhut_api::ids::TaskId::new),
                tasks: self
                    .store()
                    .tasks_of(row.id)
                    .await?
                    .into_iter()
                    .map(emberhut_api::ids::TaskId::new)
                    .collect(),
            });
        }
        Ok(out)
    }

    /// The owner's input from the app: add, repair, rename, unpair, cancel, handback, pause.
    ///
    /// # Errors
    ///
    /// [`ChromeError`].
    pub async fn input(&self, input: &ChromeInput) -> Result<ChromeReply, ChromeError> {
        let id = input.id.filter(|n| *n != 0.0 && n.fract() == 0.0).map(|n| {
            #[expect(clippy::cast_possible_truncation, reason = "a whole id")]
            let id = n as i64;
            id
        });
        let text = |v: &Option<String>| v.clone().unwrap_or_default();
        let which = || ChromeError::Refused("Which profile?".to_owned());
        let mut pairing = None;
        match input.kind.as_deref().unwrap_or("undefined") {
            "add" => {
                let made = self.pair_code(&text(&input.label), None).await?;
                self.kit()
                    .log(format!(
                        "The owner asked for a pairing link for a Chrome profile: {}",
                        made.label
                    ))
                    .await;
                pairing = Some(made);
            }
            "repair" => pairing = Some(self.pair_code("", Some(id.ok_or_else(which)?)).await?),
            "rename" => drop(
                self.rename(id.ok_or_else(which)?, &text(&input.label))
                    .await?,
            ),
            "unpair" => {
                if !self
                    .unpair(
                        id.ok_or_else(|| {
                            ChromeError::Refused("No such Chrome profile".to_owned())
                        })?,
                    )
                    .await?
                {
                    return Err(ChromeError::Refused("No such Chrome profile".to_owned()));
                }
            }
            "cancel" => drop(self.book().codes.remove(&code_of(&text(&input.code)))),
            "handback" => {
                let conn = self.connected(id, input.name.as_deref()).ok_or_else(|| {
                    ChromeError::Refused("That Chrome isn't connected".to_owned())
                })?;
                self.hand_back(&conn).await;
            }
            "stop" | "pause" => {
                let conn = self.connected(id, input.name.as_deref()).ok_or_else(|| {
                    ChromeError::Refused("That Chrome isn't connected".to_owned())
                })?;
                let minutes = input
                    .minutes
                    .filter(|m| *m != 0.0 && m.is_finite())
                    .unwrap_or(10.0)
                    .clamp(1.0, 10.0);
                conn.tell(
                    Message::Pause,
                    json!({ "why": "you paused it from the app", "minutes": minutes }),
                );
            }
            other => return Err(ChromeError::Refused(format!("unknown {other}"))),
        }
        Ok(ChromeReply {
            ok: true,
            chrome: self.summary().await?,
            chrome_pairing: self.pairings(),
            chrome_latest: Some(self.latest()),
            pairing,
        })
    }

    fn connected(&self, id: Option<i64>, name: Option<&str>) -> Option<Arc<Conn>> {
        if let Some(conn) = id.and_then(|id| self.online(id)) {
            return Some(conn);
        }
        let name = name.unwrap_or_default().to_lowercase();
        self.book()
            .conns
            .values()
            .find(|c| c.state().name == name)
            .cloned()
    }

    /// `chrome_pair`: the list; a link for a new or paired profile; rename or unpair one.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn pair_tool(
        &self,
        profile: Option<&str>,
        unpair: bool,
        rename: Option<&str>,
    ) -> Result<String, DbError> {
        let what = strip_chrome(profile.unwrap_or_default().trim()).to_owned();
        if what.is_empty() {
            let latest = self.latest();
            let mut lines: Vec<String> = self
                .summary()
                .await?
                .into_iter()
                .map(|p| {
                    let seen = p.seen.as_ref().map_or_default(|s| format!(" (last seen {})", s.as_str().chars().take(16).collect::<String>()));
                    let online = if p.connected { "online".to_owned() } else { format!("offline{seen}") };
                    let outdated = if p.outdated { format!(" ({latest} is out: reload it)") } else { String::new() };
                    let paused = p
                        .paused
                        .as_ref()
                        .map_or_default(|z| {
                            #[expect(clippy::cast_possible_truncation, reason = "ms since the epoch")]
                            let until = z.until as i64;
                            format!(", paused by the owner until {} ({}; it comes back by itself)", hhmm_ms(until), z.why)
                        });
                    let tasks = if p.tasks.is_empty() {
                        String::new()
                    } else {
                        format!(", default for {}", p.tasks.iter().map(|t| format!("task {t}")).collect::<Vec<_>>().join(", "))
                    };
                    format!(
                        "- chrome:{}: \"{}\", {}{}, {online}, extension {}{outdated}{paused}{tasks}",
                        p.name,
                        p.label,
                        p.email.clone().unwrap_or_else(|| "account unknown".to_owned()),
                        p.machine.as_ref().map_or_default(|m| format!(", on {m}")),
                        p.version.clone().unwrap_or_else(|| "?".to_owned()),
                    )
                })
                .collect();
            lines.extend(
                self.pairings()
                    .into_iter()
                    .filter(|p| !p.paired)
                    .map(|p| format!("- waiting: \"{}\" ({})", p.label, p.link)),
            );
            return Ok(if lines.is_empty() {
                "No Chrome profile of the owner's is paired yet. chrome_pair with profile: a name \
                 (Work, Personal…) gives them a link."
                    .to_owned()
            } else {
                lines.join("\n")
            });
        }
        let row = self.profile(&what).await?;
        if unpair {
            return Ok(match row {
                Some(r) if self.unpair(r.id).await.unwrap_or(false) => {
                    format!("Unpaired their {} Chrome profile.", r.label())
                }
                _ => format!("No paired Chrome profile called {what}."),
            });
        }
        if let Some(rename) = rename.filter(|r| !r.is_empty()) {
            let Some(r) = row else {
                return Ok(format!("No paired Chrome profile called {what}."));
            };
            return Ok(match self.rename(r.id, rename).await {
                Ok(n) => format!(
                    "Renamed: now \"{}\", browser \"chrome:{}\".",
                    n.label(),
                    n.name
                ),
                Err(e) => e.to_string(),
            });
        }
        let made = match &row {
            Some(r) => self.pair_code("", Some(r.id)).await,
            None => self.pair_code(&what, None).await,
        };
        Ok(match made {
            Ok(p) => format!(
                "Pairing link for their {}Chrome profile \"{}\": {} (code {}; works once, for 30 \
                 minutes). Tell them: open the link in that Chrome profile; if the extension isn't \
                 in it yet, the page shows how to add it, then it pairs by itself. It's also in \
                 the app under Settings, Chrome profiles.",
                if row.is_some() { "" } else { "new " },
                p.label,
                p.link,
                p.code
            ),
            Err(e) => e.to_string(),
        })
    }

    /// The browser task `task` works in from now on: `assistant` (the server's own),
    /// `chrome:<profile>`, or nothing (none of its own). What it is now, for the agent.
    ///
    /// # Errors
    ///
    /// [`ChromeError::Refused`] for a browser that isn't one.
    pub async fn set_task_browser(&self, task: i64, browser: &str) -> Result<String, ChromeError> {
        let b = browser.trim();
        let now = self.kit().clock.now();
        if b.is_empty() {
            self.store().clear_task_browser(task).await?;
            return Ok(
                "no browser of its own (the server's browser unless a tool says)".to_owned(),
            );
        }
        if b.eq_ignore_ascii_case(OWN_BROWSER) {
            self.store().set_task_browser(task, 0, now).await?;
            return Ok("the server's own browser".to_owned());
        }
        let names = self.names().await?;
        if !is_chrome_profile(b) {
            return Err(ChromeError::Refused(format!(
                "browser is \"{OWN_BROWSER}\" or \"chrome:<profile>\" ({names})"
            )));
        }
        let row = self.profile(b).await?.ok_or_else(|| {
            ChromeError::Refused(format!(
                "No Chrome profile \"{}\" (paired: {names})",
                strip_chrome(b)
            ))
        })?;
        self.store().set_task_browser(task, row.id, now).await?;
        Ok(format!(
            "their {} Chrome (chrome:{})",
            row.label(),
            row.name
        ))
    }

    async fn names(&self) -> Result<String, DbError> {
        let rows = self.store().profiles().await?;
        let names: Vec<String> = rows
            .iter()
            .map(|r| {
                format!(
                    "chrome:{}{}",
                    r.name,
                    if self.online(r.id).is_some() {
                        " (online)"
                    } else {
                        ""
                    }
                )
            })
            .collect();
        Ok(if names.is_empty() {
            "none paired yet".to_owned()
        } else {
            names.join(", ")
        })
    }

    // ── Which browser a tool acts in ──

    /// Where a browser tool acts: what it says (`assistant`, `chrome`, `chrome:<profile>`), else
    /// its task's own browser, else the server's own. `chrome` alone: the task's profile or the one
    /// online; with more than one online it never guesses: it asks the owner on the task.
    pub async fn pick(&self, said: &str, who: &Who, kit: &Kit) -> Pick {
        match self.pick_row(said, who, kit).await {
            Ok(Some(row)) => self.enter(&row),
            Ok(None) => Pick::Own,
            Err(said) => Pick::Said(said),
        }
    }

    async fn pick_row(
        &self,
        said: &str,
        who: &Who,
        kit: &Kit,
    ) -> Result<Option<ChromeProfileRow>, String> {
        let db = |e: DbError| format!("Browser: {e}");
        let own = match who.task {
            Some(task) => self.store().task_browser(task).await.map_err(db)?,
            None => None,
        }
        .filter(|d| *d != 0);
        if said.is_empty() {
            return match own {
                Some(id) => Ok(self.store().profile(id).await.map_err(db)?),
                None => Ok(None),
            };
        }
        if said.eq_ignore_ascii_case(OWN_BROWSER) {
            return Ok(None);
        }
        if said.eq_ignore_ascii_case("chrome") {
            if let Some(id) = own {
                return self.store().profile(id).await.map_err(db);
            }
            return self.choose(who, kit).await.map(Some);
        }
        if is_chrome_profile(said) {
            return match self.profile(said).await.map_err(db)? {
                Some(row) => Ok(Some(row)),
                None => Err(format!(
                    "No Chrome profile \"{}\" of the owner's (paired: {}). chrome_pair lists them.",
                    strip_chrome(said),
                    self.names().await.map_err(db)?
                )),
            };
        }
        Err(format!(
            "browser is \"{OWN_BROWSER}\" (the server's own browser), \"chrome:<profile>\" (one of \
             the owner's Chrome profiles: {}) or \"chrome\".",
            self.names().await.map_err(db)?
        ))
    }

    /// `chrome` with no profile and none on the task: the one online, or ask the owner which.
    #[expect(clippy::too_many_lines, reason = "one pick, step by step in order")]
    async fn choose(&self, who: &Who, kit: &Kit) -> Result<ChromeProfileRow, String> {
        let db = |e: DbError| format!("Browser: {e}");
        let all = self.store().profiles().await.map_err(db)?;
        let on: Vec<ChromeProfileRow> = all
            .iter()
            .filter(|r| self.online(r.id).is_some())
            .cloned()
            .collect();
        if all.is_empty() {
            return Err("No Chrome profile of the owner's is paired yet: chrome_pair gives them a link to pair one.".to_owned());
        }
        if on.is_empty() {
            return Err(format!(
                "None of the owner's Chrome profiles is online now (paired: {}). Use the server's \
                 own browser, or tell them.",
                self.names().await.map_err(db)?
            ));
        }
        if on.len() == 1 {
            return Ok(on[0].clone());
        }
        let listed = on
            .iter()
            .map(|r| format!("chrome:{}", r.name))
            .collect::<Vec<_>>()
            .join(", ");
        let Some(task) = who.task else {
            return Err(format!(
                "{} of the owner's Chrome profiles are online ({listed}): don't guess. Say which \
                 with browser, or ask them which account this is for.",
                on.len()
            ));
        };
        let asked = self.book().which.get(&task).cloned();
        if let Some((at, ids)) = asked {
            let Some(answer) = kit.desk.chose_since(task, at).await else {
                return Err(format!(
                    "Still waiting for the owner to pick a Chrome profile for task {task} (asked \
                     {}). Stop for now; you'll hear when they answer.",
                    crate::services::browser::desk::hhmm(at)
                ));
            };
            self.book().which.remove(&task);
            let now = kit.clock.now();
            for id in ids {
                if let Some(row) = self.store().profile(id).await.map_err(db)?
                    && row.label().eq_ignore_ascii_case(answer.trim())
                {
                    self.store()
                        .set_task_browser(task, row.id, now)
                        .await
                        .map_err(db)?;
                    return Ok(row);
                }
            }
            self.store()
                .set_task_browser(task, 0, now)
                .await
                .map_err(db)?;
            return Err(format!(
                "The owner picked {answer} for task {task}: work in the server's own browser (no \
                 browser)."
            ));
        }
        let options: Vec<ChromeProfileRow> = on.into_iter().take(5).collect();
        self.book().which.insert(
            task,
            (kit.clock.now(), options.iter().map(|r| r.id).collect()),
        );
        let mut answers: Vec<String> = options.iter().map(|r| r.label().to_owned()).collect();
        answers.push("The server's own browser".to_owned());
        kit.desk
            .ask(
                task,
                Card {
                    question: format!(
                        "Which Chrome profile should the assistant use for task {task}? {} are \
                         online, and it doesn't pick one by itself.",
                        options.len()
                    ),
                    options: answers,
                    action: "Pick the Chrome profile for this task".to_owned(),
                    brief: options
                        .iter()
                        .take(4)
                        .map(|r| {
                            format!(
                                "{}: {}{}",
                                r.label(),
                                r.email
                                    .clone()
                                    .filter(|e| !e.is_empty())
                                    .unwrap_or_else(|| "account unknown".to_owned()),
                                r.machine.as_ref().map_or_default(|m| format!(" on {m}"))
                            )
                        })
                        .collect(),
                    kind: "choose",
                },
            )
            .await;
        kit.push(
            "Which Chrome profile?",
            &format!("For task {task}. Pick one in the app."),
        )
        .await;
        Err(format!(
            "Asked the owner which Chrome profile to use for task {task} (online: {}). Stop for \
             now; when they pick, call the same tool again: their pick becomes the task's.",
            options
                .iter()
                .map(ChromeProfileRow::label)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }

    fn enter(&self, row: &ChromeProfileRow) -> Pick {
        let Some(conn) = self.online(row.id) else {
            let seen = row.seen.map_or_default(|s| {
                format!(
                    "; last seen {}",
                    wire_time(s.as_millisecond())
                        .as_str()
                        .chars()
                        .take(16)
                        .collect::<String>()
                )
            });
            return Pick::Said(format!(
                "The owner's {} Chrome profile isn't online now (Chrome closed or the computer \
                 asleep{seen}). Use the server's own browser (browser \"{OWN_BROWSER}\"), or tell \
                 them.",
                row.label()
            ));
        };
        let (control, label) = {
            let s = conn.state();
            (s.control, s.label.clone())
        };
        if control != Control::Owner
            && let Some(pause) = self.paused_of(&conn)
        {
            #[expect(clippy::cast_precision_loss, reason = "minutes left, said rounded up")]
            let minutes = ((pause.until - self.now_ms()) as f64 / 60_000.0)
                .ceil()
                .max(1.0);
            return Pick::Said(format!(
                "The owner paused the assistant in their {label} Chrome ({}): it comes back by \
                 itself at {} (in {minutes} min), or as soon as they hand back. Don't retry before \
                 then and don't ask them to resume it; you'll get an event \"browser\" when it's \
                 over. Meanwhile use the server's own browser (browser \"{OWN_BROWSER}\") or other \
                 work.",
                pause.why,
                hhmm_ms(pause.until)
            ));
        }
        let blocked = conn.state().blocked.take();
        if let Some(host) = blocked {
            conn.state().current = None;
            return Pick::Said(format!(
                "That tab went to {host}, which is blocked in the owner's Chrome: the assistant let \
                 go of it. If the task needs that site, browser_handoff so they do it."
            ));
        }
        Pick::Chrome(Arc::new(ChromeSurface {
            conn,
            service: self.clone(),
        }))
    }

    // ── The owner's pause and turn ──

    fn paused_of(&self, conn: &Arc<Conn>) -> Option<Pause> {
        let pause = conn.state().paused.clone()?;
        if pause.until <= self.now_ms() {
            let service = self.clone();
            let conn = Arc::clone(conn);
            conn.state().paused = None;
            self.book().pauses.remove(&conn.id);
            tokio::spawn(async move { service.unpaused(&conn, pause, "10 minutes are up").await });
            return None;
        }
        Some(pause)
    }

    fn pause_it(&self, conn: &Arc<Conn>, why: String, until: i64, turn: bool) {
        let now = self.now_ms();
        let (was, owner, task) = {
            let s = conn.state();
            (
                s.paused.clone(),
                s.control == Control::Owner,
                s.surface.task,
            )
        };
        let pause = Pause {
            why,
            until: until.min(now + PAUSE_MS),
            turn: turn || owner || was.as_ref().is_some_and(|w| w.turn),
            task: was.as_ref().and_then(|w| w.task).or(task),
        };
        conn.state().paused = Some(pause.clone());
        self.book().pauses.insert(conn.id, pause.clone());
        if was.is_none() {
            let used = {
                let mut s = conn.state();
                s.surface.driver.take()
            };
            conn.let_go();
            let service = self.clone();
            let label = conn.label();
            tokio::spawn(async move {
                service
                    .kit()
                    .log(format!(
                        "The owner paused the assistant in their {label} Chrome until {}: {}",
                        hhmm_ms(pause.until),
                        pause.why
                    ))
                    .await;
                if used.is_some() {
                    let task = pause.task.map_or_default(|t| format!(" for task {t}"));
                    service
                        .kit()
                        .desk
                        .wake(
                            "browser",
                            format!(
                                "The owner paused you in their {label} Chrome ({}){task}. Stop \
                                 working there for now; it comes back by itself at {}, or sooner \
                                 when they hand back, and you'll get an event \"browser\" then. \
                                 Nothing for them to do.",
                                pause.why,
                                hhmm_ms(pause.until)
                            ),
                        )
                        .await;
                }
            });
        }
    }

    async fn unpause(&self, conn: &Arc<Conn>, why: &str) {
        let pause = conn.state().paused.take();
        self.book().pauses.remove(&conn.id);
        if let Some(pause) = pause {
            self.unpaused(conn, pause, why).await;
        }
    }

    async fn unpaused(&self, conn: &Arc<Conn>, pause: Pause, why: &str) {
        conn.tell(Message::Resume, json!({ "why": why }));
        let (label, name) = {
            let s = conn.state();
            (s.label.clone(), s.name.clone())
        };
        self.kit()
            .log(format!(
                "The assistant is back in the owner's {label} Chrome: {why}"
            ))
            .await;
        if let Some(task) = pause.task {
            self.kit()
                .desk
                .wake(
                    "browser",
                    format!(
                        "The pause in the owner's {label} Chrome is over ({why}): you may work \
                         there again for task {task}. Take a browser_snapshot with browser \
                         \"chrome:{name}\" and carry on."
                    ),
                )
                .await;
        }
    }

    async fn hand_back(&self, conn: &Arc<Conn>) {
        if conn.state().paused.is_some() {
            self.unpause(conn, "they handed back").await;
        }
        let (why, task, url, label, name) = {
            let mut s = conn.state();
            if s.control != Control::Owner {
                return;
            }
            s.control = Control::Agent;
            (
                s.handoff.take(),
                s.surface.task,
                s.surface.url.clone(),
                s.label.clone(),
                s.name.clone(),
            )
        };
        conn.tell(Message::Handback, json!({}));
        let asked = why.map_or_default(|w| format!(" (you asked: {w})"));
        let for_task = task.map_or_default(|t| format!(" for task {t}"));
        self.kit()
            .desk
            .wake(
                "browser",
                format!(
                    "The owner handed their {label} Chrome back{asked}{for_task}. Take a \
                     browser_snapshot with browser \"chrome:{name}\" and carry on."
                ),
            )
            .await;
        if let Some(task) = task {
            self.kit()
                .note(
                    task,
                    &format!("The owner handed their {label} Chrome back"),
                    None,
                    &url,
                )
                .await;
            self.kit()
                .desk
                .clear_action(task, format!("In your {label} Chrome"))
                .await;
        }
    }

    // ── The extension's socket ──

    /// Serves one extension's socket: `incoming` are its messages, `out` what goes back.
    /// `origin`: the socket's origin (`chrome-extension://<id>`); `machine`: where it connects
    /// from, when known; `opened`: told once its hello succeeded.
    pub async fn serve(
        &self,
        mut incoming: mpsc::UnboundedReceiver<String>,
        out: mpsc::UnboundedSender<Outgoing>,
        origin: Option<String>,
        machine: Option<String>,
        opened: Option<oneshot::Sender<()>>,
    ) {
        let ext = origin
            .as_deref()
            .and_then(|o| o.strip_prefix("chrome-extension://"))
            .filter(|id| id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b)))
            .map(str::to_owned);
        let first = match tokio::time::timeout(HELLO_WAIT, incoming.recv()).await {
            Ok(Some(first)) => first,
            Ok(None) => return,
            Err(_) => {
                drop(out.send(Outgoing::Close(1008, "No hello".to_owned())));
                return;
            }
        };
        let Ok(Value::Object(hello)) = serde_json::from_str::<Value>(&first) else {
            drop(out.send(Outgoing::Close(1003, "Not JSON".to_owned())));
            return;
        };
        let Some(conn) = self.hello(&hello, &out, ext, machine).await else {
            return;
        };
        if let Some(opened) = opened {
            opened.send(()).ok();
        }
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tokio::select! {
                message = incoming.recv() => {
                    let Some(message) = message else { break };
                    match serde_json::from_str::<Value>(&message) {
                        Ok(Value::Object(m)) => self.on_message(&conn, m).await,
                        _ => self.kit().log(format!("Couldn't read a message from the owner's {} Chrome", conn.label())).await,
                    }
                }
                _ = tick.tick() => self.idle(&conn).await,
            }
        }
        self.gone(&conn).await;
    }

    /// The socket's first message: `pair` (with a link's code) or `hello` (with its token).
    async fn hello(
        &self,
        m: &serde_json::Map<String, Value>,
        out: &mpsc::UnboundedSender<Outgoing>,
        ext: Option<String>,
        machine: Option<String>,
    ) -> Option<Arc<Conn>> {
        let close = |code: u16, why: &str| drop(out.send(Outgoing::Close(code, why.to_owned())));
        let p = m.get("params").cloned().unwrap_or(Value::Null);
        let method = m.get("method").and_then(Value::as_str).unwrap_or_default();
        if method == "pair" {
            match self.pair(&p, ext, machine).await {
                Ok(paired) => drop(out.send(Outgoing::Text(
                    json!({ "method": Message::Paired.name(), "params": paired }).to_string(),
                ))),
                Err(why) => close(1008, &why),
            }
            return None;
        }
        if method != "hello" {
            close(1008, "Not paired yet: open the pairing link from the app");
            return None;
        }
        let token = p.get("token").and_then(Value::as_str).unwrap_or_default();
        let Ok(Some(row)) = self.store().by_token(&token_hash(token)).await else {
            close(
                1008,
                "This Chrome profile isn't paired with the server (or was unpaired): pair it again from the app",
            );
            return None;
        };
        if let Some(before) = self.online(row.id) {
            before.close(1000, "Connected again");
            self.gone(&before).await;
        }
        let kept = self
            .book()
            .pauses
            .get(&row.id)
            .cloned()
            .filter(|p| p.until > self.now_ms());
        let conn = Arc::new(Conn {
            id: row.id,
            out: out.clone(),
            calls: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            state: Mutex::new(ConnState {
                name: row.name.clone(),
                label: row.label().to_owned(),
                version: text_of(p.get("version"), 20),
                os: text_of(p.get("os"), 20),
                paused: kept,
                used: self.now_ms(),
                ..ConnState::default()
            }),
            events: broadcast::channel(256).0,
            ready: tokio::sync::Notify::new(),
            engine: Mutex::new(None),
        });
        self.book().conns.insert(row.id, Arc::clone(&conn));
        drop(self.store().seen(row.id, self.kit().clock.now()).await);
        self.about(row.id, &p, ext, machine).await;
        conn.tell(
            Message::Hello,
            json!({ "name": row.name, "label": row.label(), "latest": self.latest() }),
        );
        self.keep_current(&conn, true);
        Some(conn)
    }

    #[expect(clippy::too_many_lines, reason = "the pairing, step by step in order")]
    async fn pair(
        &self,
        p: &Value,
        ext: Option<String>,
        machine: Option<String>,
    ) -> Result<Value, String> {
        let raw = code_of(p.get("code").and_then(Value::as_str).unwrap_or_default());
        let code = self.book().codes.get(&raw).cloned();
        let now = self.now_ms();
        let Some(code) = code.filter(|c| c.done.is_none() && c.until >= now) else {
            self.book().codes.remove(&raw);
            return Err(
                "That pairing link is used or expired: get a new one in the app".to_owned(),
            );
        };
        if let (Some(ext), Some(id)) = (&ext, self.extension_id())
            && *ext != id
        {
            self.kit()
                .log(format!("An old copy of the extension ({ext}) saw the pairing link for \"{}\": left for the new one", code.label))
                .await;
            return Err(
                "An old copy of the extension: remove it, and load the one from the pairing page"
                    .to_owned(),
            );
        }
        let token = new_token(self.inner.parts.random.as_ref()).map_err(|e| e.to_string())?;
        let hash = token_hash(&token);
        let store = self.store();
        let old = match p
            .get("old")
            .and_then(Value::as_str)
            .filter(|o| !o.is_empty())
        {
            Some(old) => store
                .by_token(&token_hash(old))
                .await
                .ok()
                .flatten()
                .map(|r| r.id),
            None => None,
        };
        let device = if let Some(id) = code.device {
            if store.profile(id).await.ok().flatten().is_none() {
                return Err(
                    "That Chrome profile was unpaired meanwhile: add it again in the app"
                        .to_owned(),
                );
            }
            store
                .set_token(id, &hash)
                .await
                .map_err(|e| e.to_string())?;
            id
        } else {
            let name_taken = store
                .profiles()
                .await
                .map_err(|e| e.to_string())?
                .iter()
                .any(|r| r.name == code.name && Some(r.id) != old);
            if name_taken {
                return Err(format!("A Chrome profile is already called {}", code.label));
            }
            store
                .insert(&code.name, &code.label, &hash, self.kit().clock.now())
                .await
                .map_err(|e| e.to_string())?
                .id
        };
        if let Some(was) = old.filter(|w| *w != device) {
            drop(store.revoke(was, self.kit().clock.now()).await);
            drop(store.move_tasks(was, device).await);
            if let Some(conn) = self.online(was) {
                conn.close(1000, "Paired again under another name");
            }
        }
        if let Some(c) = self.book().codes.get_mut(&raw) {
            c.done = Some((device, now));
        }
        self.about(device, p, ext, machine).await;
        let email = store
            .profile(device)
            .await
            .ok()
            .flatten()
            .and_then(|r| r.email)
            .filter(|e| !e.is_empty())
            .map_or_default(|e| format!(" ({e})"));
        self.kit()
            .log(format!(
                "Paired the owner's {} Chrome profile{email}",
                code.label
            ))
            .await;
        self.kit()
            .desk
            .wake(
                "chrome",
                format!(
                    "The owner {} their Chrome profile \"{}\"{email}: browser \"chrome:{}\" in \
                     the browser tools. You may use it any time (no allow needed); if they pause \
                     you there, it comes back by itself.",
                    if code.device.is_some() {
                        "re-paired"
                    } else {
                        "paired"
                    },
                    code.label,
                    code.name
                ),
            )
            .await;
        Ok(json!({ "token": token, "name": code.name, "label": code.label }))
    }

    /// What the extension says about itself: its version, the profile's account, the OS.
    async fn about(&self, id: i64, p: &Value, ext: Option<String>, machine: Option<String>) {
        let email = p
            .get("email")
            .map(|e| e.as_str().map_or_default(|s| s.chars().take(120).collect()));
        let about = ChromeAbout {
            version: text_of(p.get("version"), 20),
            email,
            os: text_of(p.get("os"), 20),
            agent: text_of(p.get("agent"), 40),
            ext,
        };
        drop(self.store().about(id, about).await);
        if let Some(machine) = machine.filter(|m| !m.is_empty()) {
            drop(self.store().set_machine(id, &machine).await);
        }
    }

    async fn on_message(&self, conn: &Arc<Conn>, m: serde_json::Map<String, Value>) {
        if let Some(id) = m.get("id") {
            let waiting = id.as_u64().and_then(|id| {
                conn.calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&id)
            });
            if let Some(waiting) = waiting {
                let answer = match m.get("error").filter(|e| !e.is_null() && *e != &json!("")) {
                    Some(error) => Err(error
                        .as_str()
                        .map_or_else(|| error.to_string(), str::to_owned)),
                    None => Ok(m.get("result").cloned().unwrap_or(Value::Null)),
                };
                drop(waiting.send(answer));
            }
            return;
        }
        let method = m.get("method").and_then(Value::as_str).unwrap_or_default();
        let p = m.get("params").cloned().unwrap_or(Value::Null);
        if let Some(message) = Message::read(method) {
            return self.on_named(conn, message, &p).await;
        }
        match method {
            "ping" => {
                drop(self.store().seen(conn.id, self.kit().clock.now()).await);
                // The extension counts its pings answered, to tell a socket that died with the
                // computer's sleep from a quiet one.
                drop(
                    conn.out
                        .send(Outgoing::Text(r#"{"method":"pong"}"#.to_owned())),
                );
            }
            wire::INITIALIZED => {
                conn.state().ready = true;
                conn.ready.notify_waiters();
            }
            "chrome.tabs.onCreated" => {
                if let Some(tab) = p.get(0).filter(|t| t.get("id").is_some())
                    && let Some(id) = tab.get("id").and_then(Value::as_i64)
                {
                    conn.state().known.insert(id, tab.clone());
                }
            }
            "chrome.tabs.onRemoved" => {
                if let Some(id) = p.get(0).and_then(Value::as_i64) {
                    {
                        let mut s = conn.state();
                        s.known.remove(&id);
                        s.attached.remove(&id);
                    }
                    if let Some(gone) = p.get(1) {
                        let by = gone
                            .get("by")
                            .and_then(Value::as_str)
                            .unwrap_or("chrome")
                            .to_owned();
                        let url = gone
                            .get("url")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        self.closed(conn, &by, &url, id).await;
                    }
                }
            }
            "chrome.debugger.onDetach" => {
                if let Some(id) = p
                    .get(0)
                    .and_then(|t| t.get("tabId"))
                    .and_then(Value::as_i64)
                {
                    conn.state().attached.remove(&id);
                }
            }
            "chrome.debugger.onEvent" => {
                let tab = p
                    .get(0)
                    .and_then(|t| t.get("tabId"))
                    .and_then(Value::as_i64);
                let child = p
                    .get(0)
                    .and_then(|t| t.get("sessionId"))
                    .filter(|s| !s.is_null());
                let name = p.get(1).and_then(Value::as_str);
                if let (Some(tab), Some(name), None) = (tab, name, child) {
                    drop(conn.events.send(CdpEvent {
                        key: tab.to_string(),
                        method: name.to_owned(),
                        params: p.get(2).cloned().unwrap_or(Value::Null),
                    }));
                }
            }
            _ => {}
        }
    }

    /// A named message from the extension.
    async fn on_named(&self, conn: &Arc<Conn>, message: Message, p: &Value) {
        match message {
            Message::Stopped => self.stopped(conn, p).await,
            Message::Handback => self.hand_back(conn).await,
            Message::Unpair => drop(self.unpair(conn.id).await),
            Message::Cut => {
                if let Some(tab) = p.get("tabId").and_then(Value::as_i64) {
                    let url = p
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    self.closed(conn, "frame", &url, tab).await;
                }
            }
            Message::Blocked => {
                conn.state().blocked = Some(
                    p.get("host")
                        .and_then(Value::as_str)
                        .unwrap_or("a blocked site")
                        .to_owned(),
                );
            }
            Message::Framed => {
                let place = site(p.get("url").and_then(Value::as_str).unwrap_or_default());
                self.kit()
                    .log(format!(
                        "Chrome won't let the assistant into its tab in the owner's {} Chrome yet \
                         ({place}): another extension's frame is in the page; it is offered again",
                        conn.label()
                    ))
                    .await;
            }
            // The server's own words, never the extension's.
            Message::Start
            | Message::Hello
            | Message::Paired
            | Message::Pause
            | Message::Resume
            | Message::Name
            | Message::Step
            | Message::Handoff
            | Message::Update => {}
        }
    }

    /// The owner paused the assistant there (`why`, `until`, `turn`), or the pause is over
    /// (`null`).
    async fn stopped(&self, conn: &Arc<Conn>, p: &Value) {
        if p.is_null() || *p == json!(false) {
            self.unpause(conn, "it's over").await;
            return;
        }
        let why = p
            .get("why")
            .and_then(Value::as_str)
            .unwrap_or("they paused it")
            .to_owned();
        let until = p.get("until").and_then(Value::as_f64).unwrap_or(0.0);
        let now = self.now_ms();
        #[expect(clippy::cast_possible_truncation, reason = "ms since the epoch")]
        let until = until as i64;
        if until > 0 {
            if until > now {
                let turn = p.get("turn").and_then(Value::as_bool).unwrap_or(false);
                self.pause_it(conn, why, until, turn);
            } else {
                conn.tell(Message::Resume, json!({ "why": "10 minutes are up" }));
            }
            return;
        }
        let kept = conn.state().paused.as_ref().map(|p| p.until);
        self.pause_it(conn, why, kept.unwrap_or(now + PAUSE_MS), false);
    }

    /// One of the assistant's tabs there closed, and by whom: kept a few minutes for the tools.
    async fn closed(&self, conn: &Arc<Conn>, by: &str, url: &str, tab: i64) {
        let label = conn.label();
        let place = site(url);
        let why = match by {
            "page" => format!("the page closed it itself ({place})"),
            wire::BY_ASSISTANT => "the assistant closed it".to_owned(),
            "crash" => format!("the page crashed ({place})"),
            "window" => format!("its window was closed in the owner's {label} Chrome"),
            "frame" => "another extension put its own frame in the page (a password manager's \
                        fill-in menu, likely), and Chrome lets the assistant drive no page with \
                        another extension's frame in it"
                .to_owned(),
            _ => format!(
                "it was closed in the owner's {label} Chrome, not by the page or by the assistant \
                 (the owner closed it, or another extension did)"
            ),
        };
        let on = if place.is_empty() {
            String::new()
        } else {
            format!(" on {place}")
        };
        let text = if by == "frame" {
            format!(
                "Chrome cut the assistant's link to its tab in the owner's {label} Chrome{on}, which is still open: {why}"
            )
        } else {
            format!("The assistant's tab in the owner's {label} Chrome{on} closed: {why}")
        };
        let now = self.now_ms();
        let task = {
            let mut s = conn.state();
            s.closes.retain(|c| now - c.at <= CLOSE_KEPT_MS);
            s.closes.push(TabClose {
                at: now,
                by: by.to_owned(),
                url: url.to_owned(),
                text,
                tab,
            });
            if s.current.is_some() {
                s.current = None;
            }
            s.surface.task
        };
        if by == wire::BY_ASSISTANT {
            return;
        }
        self.kit()
            .log(format!(
                "The assistant's tab in the owner's {label} Chrome was closed: {why}"
            ))
            .await;
        if let Some(task) = task {
            self.kit()
                .note(
                    task,
                    &format!("The assistant's tab was closed in your {label} Chrome ({by})"),
                    None,
                    url,
                )
                .await;
        }
    }

    /// Unused for a while: let go of its tabs, so the owner's screen can sleep and lock.
    async fn idle(&self, conn: &Arc<Conn>) {
        let now = self.now_ms();
        let release = {
            let s = conn.state();
            s.started && s.control != Control::Owner && now - s.used >= LET_GO_MS
        };
        self.keep_current(conn, false);
        if release {
            conn.let_go();
            self.kit()
                .log(format!(
                    "Let go of the assistant's tabs in the owner's {} Chrome for now (they stay \
                     open): unused for 5 minutes, so the screen can sleep and lock",
                    conn.label()
                ))
                .await;
        }
    }

    async fn gone(&self, conn: &Arc<Conn>) {
        {
            let mut book = self.book();
            if book
                .conns
                .get(&conn.id)
                .is_some_and(|c| Arc::ptr_eq(c, conn))
            {
                book.conns.remove(&conn.id);
            } else {
                return;
            }
        }
        drop(self.store().seen(conn.id, self.kit().clock.now()).await);
        for (_, waiting) in conn
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain()
        {
            drop(waiting.send(Err(format!(
                "The owner's {} Chrome went away",
                conn.label()
            ))));
        }
        let (driving, label) = {
            let s = conn.state();
            (
                s.surface.driver.is_some() && s.paused.is_none(),
                s.label.clone(),
            )
        };
        self.kit()
            .log(format!("The link to the owner's {label} Chrome closed"))
            .await;
        if driving {
            let service = self.clone();
            let id = conn.id;
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(30)).await;
                if service.online(id).is_none() {
                    service
                        .kit()
                        .desk
                        .wake(
                            "browser",
                            format!(
                                "The owner's {label} Chrome disconnected (closed, or the computer \
                                 asleep) while you worked in it. Don't retry; tell them if it matters."
                            ),
                        )
                        .await;
                }
            });
        }
    }
}

fn expired() -> ChromePairStatus {
    ChromePairStatus {
        state: "expired".to_owned(),
        label: None,
        name: None,
        email: None,
        machine: None,
    }
}

/// `chrome:<profile>` without its `chrome:`.
fn strip_chrome(what: &str) -> &str {
    let w = what.trim();
    if w.len() >= 7 && w[..6].eq_ignore_ascii_case("chrome") {
        let rest = w[6..].trim_start();
        if let Some(rest) = rest.strip_prefix(':') {
            return rest.trim_start();
        }
    }
    w
}

/// Whether `said` names a profile: `chrome:<something>`.
fn is_chrome_profile(said: &str) -> bool {
    let stripped = strip_chrome(said);
    stripped != said.trim() && !stripped.is_empty()
}

/// Whether `conn` is a profile on the Mac that runs an older version than `latest`.
fn behind(conn: &Conn, latest: &str) -> bool {
    let s = conn.state();
    s.os.as_deref() == Some("mac") && older(s.version.as_deref(), latest)
}

/// The `manifest.json` in an extension's zip (at its top, or in its one folder).
fn zip_manifest(zip: &[u8]) -> Option<Value> {
    let u16_at = |at: usize| {
        zip.get(at..at + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
    };
    let u32_at = |at: usize| {
        zip.get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let from = zip.len().saturating_sub(66_000);
    let end = (from..zip.len().saturating_sub(21))
        .rev()
        .find(|&at| u32_at(at) == Some(0x0605_4b50))?;
    let count = usize::from(u16_at(end + 10)?);
    let mut at = usize::try_from(u32_at(end + 16)?).ok()?;
    let mut best: Option<(usize, u16, usize, usize)> = None;
    for _ in 0..count {
        if u32_at(at)? != 0x0201_4b50 {
            return None;
        }
        let method = u16_at(at + 10)?;
        let size = usize::try_from(u32_at(at + 20)?).ok()?;
        let name_len = usize::from(u16_at(at + 28)?);
        let extra = usize::from(u16_at(at + 30)?);
        let comment = usize::from(u16_at(at + 32)?);
        let local = usize::try_from(u32_at(at + 42)?).ok()?;
        let name = std::str::from_utf8(zip.get(at + 46..at + 46 + name_len)?).ok()?;
        let depth = name.matches('/').count();
        if (name == "manifest.json" || name.ends_with("/manifest.json"))
            && best.is_none_or(|(d, ..)| depth < d)
        {
            best = Some((depth, method, size, local));
        }
        at += 46 + name_len + extra + comment;
    }
    let (_, method, size, local) = best?;
    let name_len = usize::from(u16_at(local + 26)?);
    let extra = usize::from(u16_at(local + 28)?);
    let data = zip.get(local + 30 + name_len + extra..local + 30 + name_len + extra + size)?;
    let text = match method {
        0 => data.to_vec(),
        8 => {
            let mut out = Vec::new();
            flate2::read::DeflateDecoder::new(data)
                .take(1_000_000)
                .read_to_end(&mut out)
                .ok()?;
            out
        }
        _ => return None,
    };
    serde_json::from_slice(&text).ok()
}

/// One of the owner's Chrome profiles as a surface.
#[derive(Debug)]
struct ChromeSurface {
    conn: Arc<Conn>,
    service: ChromeService,
}

impl Surface for ChromeSurface {
    fn place(&self) -> String {
        format!(" in your {} Chrome", self.conn.label())
    }

    fn engine(&self) -> Arc<dyn BrowserEngine> {
        self.conn.engine()
    }

    fn state(&self) -> SurfaceState {
        self.conn.state().surface.clone()
    }

    fn update(&self, change: &mut dyn FnMut(&mut SurfaceState)) {
        change(&mut self.conn.state().surface);
    }

    fn current(&self) -> Option<TabId> {
        self.conn.state().current
    }

    fn use_tab(&self, tab: Option<TabId>) {
        self.conn.state().current = tab;
    }

    fn busy(&self) -> Option<String> {
        let s = self.conn.state();
        (s.control == Control::Owner).then(|| {
            let why = s.handoff.as_ref().map_or_default(|h| format!(" ({h})"));
            format!(
                "The owner has their {} Chrome{why}. Wait for their hand-back (an event \
                 \"browser\"), then carry on; if it can't wait, browser_takeback with browser \
                 \"chrome:{}\".",
                s.label, s.name
            )
        })
    }

    fn handoff(
        &self,
        reason: String,
        task: Option<i64>,
        shot: Option<Vec<u8>>,
    ) -> BoxFuture<'_, String> {
        Box::pin(async move {
            let (label, url) = {
                let mut s = self.conn.state();
                s.control = Control::Owner;
                s.handoff = Some(reason.clone());
                (s.label.clone(), s.surface.url.clone())
            };
            self.conn
                .tell(Message::Handoff, json!({ "reason": reason, "task": task }));
            let kit = self.service.kit();
            if let Some(task) = task {
                kit.note(
                    task,
                    &format!("Handed over to the owner in their {label} Chrome: {reason}"),
                    shot,
                    &url,
                )
                .await;
                kit.desk
                    .set_action(
                        task,
                        Some(
                            format!("In your {label} Chrome: {reason}")
                                .chars()
                                .take(90)
                                .collect(),
                        ),
                    )
                    .await;
            }
            kit.push(
                &format!("The assistant needs you in your {label} Chrome"),
                &format!(
                    "{reason}. Hand back from the extension's icon in Chrome when you're done."
                ),
            )
            .await;
            kit.desk
                .wake(
                    "ask",
                    format!(
                        "You handed their {label} Chrome to the owner ({reason}). Tell them in one \
                         line; they hand back from the extension's icon in that Chrome."
                    ),
                )
                .await;
            format!(
                "The owner has their {label} Chrome now. You'll get an event \"browser\" when they \
                 hand it back. Stop for now."
            )
        })
    }

    fn takeback(&self, reason: String, task: Option<i64>) -> BoxFuture<'_, String> {
        Box::pin(async move {
            let (label, name, held, url, turn) = {
                let mut s = self.conn.state();
                if s.control != Control::Owner {
                    return format!(
                        "The owner isn't driving their {} Chrome: it's yours already. Carry on.",
                        s.label
                    );
                }
                s.control = Control::Agent;
                s.handoff = None;
                (
                    s.label.clone(),
                    s.name.clone(),
                    s.surface.task,
                    s.surface.url.clone(),
                    s.paused.as_ref().is_some_and(|p| p.turn),
                )
            };
            self.conn.tell(
                Message::Handback,
                json!({ "by": wire::BY_ASSISTANT, "reason": reason }),
            );
            if turn {
                self.service.unpause(&self.conn, "their turn ended").await;
            }
            self.service
                .kit()
                .took_back(
                    &format!("your {label} Chrome"),
                    &reason,
                    [task, held],
                    &format!("In your {label} Chrome"),
                    &url,
                )
                .await;
            format!(
                "Took their {label} Chrome back (they were told: {reason}). Take a \
                 browser_snapshot with browser \"chrome:{name}\" and carry on."
            )
        })
    }

    fn autofill(&self) -> bool {
        false
    }

    fn step(&self, line: &str) {
        self.conn.tell(Message::Step, json!({ "text": mask(line) }));
    }

    fn closes(&self) -> Option<Vec<TabClose>> {
        let now = self.service.now_ms();
        Some(
            self.conn
                .state()
                .closes
                .iter()
                .filter(|c| now - c.at <= CLOSE_KEPT_MS)
                .cloned()
                .collect(),
        )
    }
}

/// Who a note on a task is from, by its driver.
#[must_use]
pub fn note_by(who: &Who) -> NoteBy {
    who.note_by()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_update_is_tried_three_times_ten_minutes_apart() {
        let mut entry = UpdateTry::default();
        assert_eq!(entry.next(true, 0), UpdateStep::Try);
        assert_eq!(entry.next(false, 1_000), UpdateStep::Wait);
        assert_eq!(entry.next(true, UPDATE_EVERY_MS - 1), UpdateStep::Wait);
        assert_eq!(entry.next(false, UPDATE_EVERY_MS), UpdateStep::Try);
        assert_eq!(entry.next(true, 2 * UPDATE_EVERY_MS), UpdateStep::Try);
        assert_eq!(entry.next(true, 10 * UPDATE_EVERY_MS), UpdateStep::Wait);
    }

    #[test]
    fn a_told_profile_is_warned_about_only_from_a_later_hello_and_once() {
        let mut entry = UpdateTry::default();
        assert_eq!(entry.next(true, 0), UpdateStep::Try);
        entry.told = true;
        // The socket's tick: the profile may hold a tab and not have reloaded yet.
        assert_eq!(entry.next(false, UPDATE_EVERY_MS), UpdateStep::Wait);
        assert_eq!(entry.next(false, 5 * UPDATE_EVERY_MS), UpdateStep::Wait);
        assert!(!entry.warned);
        // A hello after the update that still reports the older version.
        assert_eq!(entry.next(true, 6 * UPDATE_EVERY_MS), UpdateStep::Warn);
        assert_eq!(entry.next(true, 7 * UPDATE_EVERY_MS), UpdateStep::Wait);
        assert_eq!(entry.tries.count, 1);
    }

    #[test]
    fn profiles_are_named_with_or_without_chrome() {
        assert_eq!(strip_chrome("chrome:Work"), "Work");
        assert_eq!(strip_chrome("Chrome : work"), "work");
        assert_eq!(strip_chrome("work"), "work");
        assert!(is_chrome_profile("chrome:work"));
        assert!(!is_chrome_profile("chrome"));
        assert!(!is_chrome_profile("assistant"));
    }

    #[test]
    fn a_stored_zip_s_manifest_is_read() {
        let manifest = br#"{"version":"1.2.3"}"#;
        let name = b"ext/manifest.json";
        let mut zip = Vec::new();
        let size = u32::try_from(manifest.len()).unwrap_or_default();
        let name_len = u16::try_from(name.len()).unwrap_or_default();
        zip.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        zip.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        zip.extend_from_slice(&size.to_le_bytes());
        zip.extend_from_slice(&size.to_le_bytes());
        zip.extend_from_slice(&name_len.to_le_bytes());
        zip.extend_from_slice(&0_u16.to_le_bytes());
        zip.extend_from_slice(name);
        zip.extend_from_slice(manifest);
        let central = u32::try_from(zip.len()).unwrap_or_default();
        zip.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        zip.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        zip.extend_from_slice(&size.to_le_bytes());
        zip.extend_from_slice(&size.to_le_bytes());
        zip.extend_from_slice(&name_len.to_le_bytes());
        zip.extend_from_slice(&[0; 12]);
        zip.extend_from_slice(&0_u32.to_le_bytes());
        zip.extend_from_slice(name);
        let central_len = u32::try_from(zip.len()).unwrap_or_default() - central;
        zip.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
        zip.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
        zip.extend_from_slice(&central_len.to_le_bytes());
        zip.extend_from_slice(&central.to_le_bytes());
        zip.extend_from_slice(&[0, 0]);
        assert_eq!(zip_manifest(&zip), Some(json!({ "version": "1.2.3" })));
    }
}
