//! The browser the assistant drives: the server's own (a real Chrome over the `DevTools`
//! protocol, watched live in the owner's app and theirs to take over at any time), or one of the
//! owner's own Chrome profiles through the browser extension ([`crate::services::chrome`]).
//!
//! The rules:
//!
//! - Agents never type a secret (a password, a code): such a field is refused and the agent
//!   hands over to the owner (`browser_handoff`), who takes the browser, does that part and
//!   hands it back; agents wait meanwhile. The assistant may take it back early
//!   (`browser_takeback`) when the owner forgot, and the owner is told.
//! - A final step (submit, send, buy, install, delete, …) waits for the owner's OK
//!   ([`desk::StepGate`]); `browser_plan` names a job's steps once, up front.
//! - API keys and tokens on a page are masked in everything the assistant, a log or a picture
//!   gets; `browser_save_secret` moves one from the page into the secrets folder, unread.
//!   Password fields' values are blanked in every snapshot.
//! - Files go into a page (`browser_upload`) only from the owner's home or `/tmp`, never hidden
//!   folders, the secrets or key files; a page's file is fetched with the page's own login and
//!   saved there too (`browser_download`), never over another.
//! - Each task's browsing is logged on the task, with a few pictures.
//!
//! The live view itself (the whole screen, streamed) is served by the machine the browser runs
//! on; `GET /api/browser/session` gives the app its address and the password for what it may do.

pub mod cdp;
mod checks;
pub mod desk;
pub mod engine;
pub mod files;
pub mod surface;
pub mod whatsapp;

use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use emberhut_api::browser::{
    BrowserControl, BrowserFileRequest, BrowserInfo, BrowserScreen, BrowserSession,
    BrowserSessionReply, BrowserTab, StreamMode,
};
use emberhut_api::ids::{BrowserDeviceId, BrowserTabId};
use emberhut_api::time::Timestamp as WireTime;
use emberhut_core::browser::{
    DOWNLOAD_MAX, OWN_SECRETS, SNAPSHOT_LIMIT, address, blocked_site, cut, file_name, fold,
    host_of, is_enter_key, is_final, mask, mask_tree, open_url, page_key, secret_name_ok,
    size_text, url_host, wants_secret,
};
use emberhut_db::{SettingKey, SettingStore};
use futures_util::future::BoxFuture;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt as _;

use self::desk::{Gated, Step};
use self::engine::{
    BrowserEngine, ElementQuery, EngineError, FilePayload, FileVia, Nav, PageQuery, TabId,
};
use self::files::Places;
use self::surface::{CLOSE_KEPT_MS, Kit, Surface, SurfaceState, TabClose, Who, write_private};
use crate::services::calls;
use crate::services::chrome::wire::BY_ASSISTANT;
use crate::services::chrome::{ChromeService, OWN_BROWSER, Pick};

/// Who drives a browser now: an agent, or the owner (in the live view or their own Chrome).
/// Kept and sent as the contract's [`emberhut_api::chrome::ChromeControl`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    into = "emberhut_api::chrome::ChromeControl",
    from = "emberhut_api::chrome::ChromeControl"
)]
pub enum Control {
    /// The assistant or one of its workers.
    #[default]
    Agent,
    /// The owner: the assistant handed it over and waits.
    Owner,
}

impl From<emberhut_api::chrome::ChromeControl> for Control {
    fn from(control: emberhut_api::chrome::ChromeControl) -> Self {
        if control == emberhut_api::chrome::ChromeControl::Person {
            Self::Owner
        } else {
            Self::Agent
        }
    }
}

impl From<Control> for BrowserControl {
    fn from(control: Control) -> Self {
        match control {
            Control::Agent => Self::Agent,
            Control::Owner => Self::Person,
        }
    }
}

impl From<Control> for emberhut_api::chrome::ChromeControl {
    fn from(control: Control) -> Self {
        match control {
            Control::Agent => Self::Agent,
            Control::Owner => Self::Person,
        }
    }
}

/// How long a big page's snapshot may take.
const SNAP_WAIT: Duration = Duration::from_secs(25);
/// How long a page may take to open.
const OPEN_WAIT: Duration = Duration::from_secs(45);
/// How long a page gets to finish loading after it opened or a click.
const LOAD_WAIT: Duration = Duration::from_secs(10);
/// How long a card's picture of the tab is reused.
const FRAME_FRESH: Duration = Duration::from_secs(5);
/// The password manager's own fill shortcut (its extension in the browser types the login).
const AUTOFILL_KEY: &str = "Control+Backslash";
/// The biggest piece of a download read from the page at once.
const PIECE: u64 = 4_000_000;
/// The user name the live view takes, unless its address names another
/// (`live_view_address`).
pub const LIVE_VIEW_USER: &str = "owner";

/// The live view's address as the app loads it, and the user its sign-in takes: the plain name
/// the address names (`https://name@host/`, which the stream's own setup may have fixed), else
/// [`LIVE_VIEW_USER`]. The name leaves the address: it goes with the password, as Basic auth.
fn live_view_address(address: &str) -> (String, String) {
    let Ok(mut url) = url::Url::parse(address) else {
        return (address.to_owned(), LIVE_VIEW_USER.to_owned());
    };
    let named = url.username().to_owned();
    if named.is_empty() {
        return (address.to_owned(), LIVE_VIEW_USER.to_owned());
    }
    // A URL with a host always takes an empty user and no password; one that wouldn't keeps its
    // address as given.
    if url.set_username("").and(url.set_password(None)).is_err() {
        return (address.to_owned(), named);
    }
    (url.into(), named)
}

/// `POST /api/browser/input`: `kind` and what goes with it. `takeover` (`device`), `handback`;
/// while the owner drives: `back`, `forward`, `reload`, `go` (`url`), `tab` (`id`), `newtab`,
/// `closetab` (`id`, or the tab on screen), `chrome` (`show`: Chrome's own controls).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BrowserInput {
    /// What to do.
    #[serde(default)]
    pub kind: Option<String>,
    /// `takeover`: the device taking over.
    #[serde(default)]
    pub device: Option<String>,
    /// `go`: the address or words typed.
    #[serde(default)]
    pub url: Option<String>,
    /// `tab`, `closetab`: the tab.
    #[serde(default)]
    pub id: Option<f64>,
    /// `chrome`: show Chrome's own controls.
    #[serde(default)]
    pub show: Option<bool>,
}

/// The owner's hold on the browser outlives a restart: kept while they have it.
const HOLD: SettingKey<Hold> = SettingKey::new("browserHold");

/// The owner's hold, as kept.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Hold {
    control: Control,
    driver: Option<String>,
    task: Option<i64>,
    handoff: Option<String>,
    /// In ms since the epoch.
    at: Option<i64>,
    device: Option<String>,
}

/// A browser tool's answer: text for the agent, or a picture (a JPEG) with it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Act {
    /// What the agent reads (keys masked).
    pub text: String,
    /// A picture, when the tool gives one.
    pub image: Option<Vec<u8>>,
}

impl Act {
    fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            image: None,
        }
    }
}

/// What a browser tool is given, every tool's fields together; each reads its own.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BrowserArgs {
    /// Where (`assistant`, `chrome`, `chrome:<profile>`).
    pub browser: Option<String>,
    /// The older form of `browser: "chrome:<profile>"`.
    pub chrome: Option<String>,
    /// An address.
    pub url: Option<String>,
    /// Open in a new tab.
    pub new_tab: bool,
    /// A picture too.
    pub screenshot: bool,
    /// Part of the page.
    pub scope: Option<String>,
    /// An element's ref.
    pub at: Option<String>,
    /// What the element is, in a few words.
    pub element: Option<String>,
    /// Text to type, or to wait for.
    pub text: Option<String>,
    /// Press Enter after typing.
    pub submit: bool,
    /// A key to press.
    pub key: Option<String>,
    /// Pixels to scroll.
    pub dy: Option<f64>,
    /// Seconds to wait.
    pub seconds: Option<f64>,
    /// A tabs action.
    pub action: Option<String>,
    /// A tab's index.
    pub index: Option<f64>,
    /// Why (a hand-off, a take-back).
    pub reason: Option<String>,
    /// A secret's name.
    pub name: Option<String>,
    /// What the secret must start with.
    pub expect_prefix: Option<String>,
    /// A file on this machine.
    pub path: Option<String>,
    /// Where a download goes.
    pub dest: Option<String>,
}

/// Why an app's request to the browser was refused (409 with its words, as the reference
/// server answers).
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct BrowserRefused(pub String);

/// The live view isn't set up (503).
#[derive(Debug, thiserror::Error)]
#[error(
    "The live view isn't set up on this server (EMBERHUT_BROWSER_VIEW, and the browser-view and \
     browser-control secrets, make it)"
)]
pub struct NoLiveView;

/// What the browser service is built from.
#[derive(Clone, Debug)]
pub struct BrowserParts {
    /// The server's own browser.
    pub engine: Arc<dyn BrowserEngine>,
    /// What both kinds of browser share.
    pub kit: Kit,
    /// The settings (the owner's hold).
    pub settings: SettingStore,
    /// Where files may come from and go to.
    pub places: Places,
    /// The live view's address.
    pub live_view: Option<String>,
    /// The owner's Chrome profiles.
    pub chrome: ChromeService,
}

/// The server's own browser's state: its surface's, and the live view's.
#[derive(Debug, Default)]
struct Own {
    restored: bool,
    surface: SurfaceState,
    control: Control,
    handoff: Option<String>,
    device: Option<String>,
    tabs: Vec<BrowserTab>,
    chrome_ui: bool,
    current: Option<TabId>,
    front: Option<TabId>,
    file_at: Option<Timestamp>,
    frame: Option<(std::time::Instant, Vec<u8>)>,
}

/// The browser service. Cheap to clone.
#[derive(Clone)]
pub struct BrowserService {
    inner: Arc<Inner>,
}

struct Inner {
    parts: BrowserParts,
    own: Mutex<Own>,
    /// Per browser (by its state's place): a page's key → (when, its tab), opened a moment ago.
    opened: Mutex<Vec<(String, String, i64, TabId)>>,
    /// Per browser: the last close an agent was told of.
    told: Mutex<std::collections::HashMap<String, i64>>,
}

impl fmt::Debug for BrowserService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrowserService").finish_non_exhaustive()
    }
}

fn wire(at: Timestamp) -> WireTime {
    WireTime::new(
        at.round(jiff::TimestampRound::new().smallest(jiff::Unit::Millisecond))
            .unwrap_or(at)
            .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string(),
    )
}

fn first_line(text: &str) -> &str {
    text.split('\n').next().unwrap_or_default()
}

fn slice(text: &str, n: usize) -> String {
    text.trim().chars().take(n).collect()
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn strings(value: &Value) -> Vec<String> {
    value.as_array().map_or_default(|a| {
        a.iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect()
    })
}

impl BrowserService {
    /// The service over `parts`.
    #[must_use]
    pub fn new(parts: BrowserParts) -> Self {
        Self {
            inner: Arc::new(Inner {
                parts,
                own: Mutex::new(Own::default()),
                opened: Mutex::new(Vec::new()),
                told: Mutex::new(std::collections::HashMap::new()),
            }),
        }
    }

    fn own(&self) -> std::sync::MutexGuard<'_, Own> {
        self.inner
            .own
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn kit(&self) -> &Kit {
        &self.inner.parts.kit
    }

    fn engine(&self) -> &Arc<dyn BrowserEngine> {
        &self.inner.parts.engine
    }

    fn now(&self) -> Timestamp {
        self.kit().clock.now()
    }

    /// The owner's Chrome profiles.
    #[must_use]
    pub fn chrome(&self) -> &ChromeService {
        &self.inner.parts.chrome
    }

    /// The owner's hold, read back once after a restart.
    async fn restore(&self) {
        if self.own().restored {
            return;
        }
        let hold = self.inner.parts.settings.get(&HOLD).await.ok().flatten();
        let mut own = self.own();
        if own.restored {
            return;
        }
        own.restored = true;
        if let Some(hold) = hold.filter(|h| h.control == Control::Owner) {
            own.control = hold.control;
            own.surface.driver = hold.driver;
            own.surface.task = hold.task;
            own.handoff = hold.handoff;
            own.surface.at = hold.at.and_then(|ms| Timestamp::from_millisecond(ms).ok());
            own.device = hold.device;
            drop(own);
            self.engine().owner_drives(true);
        }
    }

    /// Keeps the owner's hold (or forgets it, once it's an agent's again).
    async fn keep(&self) {
        let hold = {
            let own = self.own();
            (own.control == Control::Owner).then(|| Hold {
                control: own.control,
                driver: own.surface.driver.clone(),
                task: own.surface.task,
                handoff: own.handoff.clone(),
                at: own.surface.at.map(Timestamp::as_millisecond),
                device: own.device.clone(),
            })
        };
        let settings = &self.inner.parts.settings;
        let kept = match hold {
            Some(hold) => settings.set(&HOLD, &hold).await,
            None => settings.remove(&HOLD).await.map(drop),
        };
        if let Err(error) = kept {
            tracing::warn!(%error, "the owner's hold on the browser was not kept");
        }
    }

    // ── What the owner's app shows ──

    /// The browser as the state shows it (`GET /api/state`'s `browser`).
    pub async fn summary(&self) -> BrowserInfo {
        self.restore().await;
        if self.engine().connected() {
            let _looked = tokio::time::timeout(Duration::from_millis(1_500), self.look()).await;
        }
        let ask = self.engine().file_ask();
        let now = self.now();
        let mut own = self.own();
        own.file_at = match (ask, own.file_at) {
            (Some(_), Some(at)) => Some(at),
            (Some(_), None) => Some(now),
            (None, _) => None,
        };
        BrowserInfo {
            connected: self.engine().connected(),
            url: own.surface.url.clone(),
            title: own.surface.title.clone(),
            driver: own.surface.driver.clone(),
            task: own.surface.task.map(emberhut_api::ids::TaskId::new),
            control: own.control.into(),
            handoff: own.handoff.clone(),
            at: own.surface.at.map(wire),
            tabs: own.tabs.clone(),
            chrome_ui: own.chrome_ui,
            device: own.device.clone().map(BrowserDeviceId::new),
            file_request: ask.zip(own.file_at).map(|(ask, at)| BrowserFileRequest {
                multiple: ask.multiple,
                at: wire(at),
            }),
        }
    }

    /// The tabs, and which one is on screen.
    async fn look(&self) {
        if let Ok(tabs) = self.engine().tabs().await {
            let front = tabs.iter().find(|t| t.front).or(tabs.first()).map(|t| t.id);
            let mut own = self.own();
            own.front = front;
            own.tabs = tabs
                .iter()
                .map(|t| BrowserTab {
                    id: BrowserTabId::new(i64::try_from(t.id).unwrap_or(i64::MAX)),
                    url: t.url.clone(),
                    title: t.title.clone(),
                    active: Some(t.id) == front,
                })
                .collect();
        }
    }

    /// The live view's address and the password for what `device` may do: control once it
    /// took over, else watch.
    ///
    /// # Errors
    ///
    /// [`NoLiveView`] when it isn't set up.
    pub async fn session(&self, device: Option<&str>) -> Result<BrowserSessionReply, NoLiveView> {
        self.restore().await;
        let mine = {
            let own = self.own();
            own.control == Control::Owner && own.device.as_deref().is_none_or(|d| Some(d) == device)
        };
        let secret = if mine {
            "browser-control"
        } else {
            "browser-view"
        };
        let password = tokio::fs::read_to_string(self.inner.parts.places.secrets_dir.join(secret))
            .await
            .map_or_default(|p| p.trim().to_owned());
        let Some((url, user)) = self.inner.parts.live_view.as_deref().map(live_view_address) else {
            return Err(NoLiveView);
        };
        if password.is_empty() {
            return Err(NoLiveView);
        }
        Ok(BrowserSessionReply {
            ok: true,
            session: BrowserSession {
                url: Some(url),
                user,
                password,
                mode: if mine {
                    StreamMode::Control
                } else {
                    StreamMode::View
                },
                screen: self.screen().await,
            },
        })
    }

    /// The screen's size now, as the page sees it.
    async fn screen(&self) -> Option<BrowserScreen> {
        let tab = self.shown_or_current()?;
        let size = tokio::time::timeout(
            Duration::from_secs(2),
            self.engine().page(tab, PageQuery::ScreenSize),
        )
        .await
        .ok()?
        .ok()?;
        let n = |i: usize| {
            size.get(i)
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
        };
        Some(BrowserScreen {
            width: n(0)?,
            height: n(1)?,
        })
    }

    fn shown_or_current(&self) -> Option<TabId> {
        let own = self.own();
        own.front.or(own.current)
    }

    /// The latest picture of the tab, for a card, fresh within a few seconds; none while the
    /// owner drives (a password may be on screen).
    pub async fn frame(&self) -> Option<Vec<u8>> {
        self.restore().await;
        {
            let own = self.own();
            if own.control == Control::Owner {
                return None;
            }
            if let Some((at, frame)) = &own.frame
                && at.elapsed() < FRAME_FRESH
            {
                return Some(frame.clone());
            }
        }
        let tab = self.own_page().await.ok()?;
        let shot = self.engine().screenshot(tab, 50).await.ok().flatten();
        let mut own = self.own();
        match shot {
            Some(shot) => {
                own.frame = Some((std::time::Instant::now(), shot.clone()));
                Some(shot)
            }
            None => own.frame.as_ref().map(|(_, f)| f.clone()),
        }
    }

    /// The owner's input from the live view's app: take over, hand back, and while they drive,
    /// the toolbar.
    ///
    /// # Errors
    ///
    /// [`BrowserRefused`]: not taken over, or an unknown kind.
    pub async fn input(&self, input: &BrowserInput) -> Result<BrowserInfo, BrowserRefused> {
        self.restore().await;
        let kind = input.kind.clone().unwrap_or_else(|| "undefined".to_owned());
        if kind == "takeover" {
            self.take("You").await;
            self.own().device = input
                .device
                .as_deref()
                .filter(|d| !d.is_empty())
                .map(|d| d.chars().take(64).collect());
            self.keep().await;
            return Ok(self.summary().await);
        }
        if kind == "handback" {
            if self.own().control == Control::Owner {
                self.hand_back().await;
            }
            return Ok(self.summary().await);
        }
        if self.own().control != Control::Owner {
            return Err(BrowserRefused("Take over first".to_owned()));
        }
        let engine = Arc::clone(self.engine());
        let tab = match self.shown_or_current() {
            Some(tab) => tab,
            None => self
                .own_page()
                .await
                .map_err(|e| BrowserRefused(first_line(&e.to_string()).to_owned()))?,
        };
        let id = input.id.filter(|n| n.fract() == 0.0 && *n >= 0.0).map(|n| {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a whole, non-negative tab id"
            )]
            let id = n as u64;
            id
        });
        let known = |id: Option<u64>| {
            let own = self.own();
            id.filter(|id| {
                own.tabs
                    .iter()
                    .any(|t| u64::try_from(t.id.get()).ok() == Some(*id))
            })
        };
        match kind.as_str() {
            "back" => drop(engine.navigate(tab, Nav::Back).await),
            "forward" => drop(engine.navigate(tab, Nav::Forward).await),
            "reload" => drop(engine.navigate(tab, Nav::Reload).await),
            "go" => {
                let url = address(input.url.as_deref().unwrap_or_default());
                drop(engine.goto(tab, url, Duration::from_secs(20)).await);
            }
            "tab" => {
                if let Some(t) = known(id) {
                    drop(engine.select_tab(t).await);
                    self.own().front = Some(t);
                }
            }
            "newtab" => {
                if let Ok(t) = engine.new_tab(None).await {
                    drop(engine.select_tab(t).await);
                    self.own().front = Some(t);
                }
            }
            "closetab" => drop(engine.close_tab(known(id).unwrap_or(tab)).await),
            "chrome" => {
                let show = input.show.unwrap_or(false);
                self.own().chrome_ui = show;
                drop(engine.show_controls(show).await);
            }
            _ => return Err(BrowserRefused(format!("unknown input {kind}"))),
        }
        self.look().await;
        Ok(self.summary().await)
    }

    /// The file the page asked for, from the owner's app: `body` streamed into the data
    /// folder's `browser/uploads` (at most `limit` bytes), then put into the page.
    ///
    /// # Errors
    ///
    /// [`BrowserRefused`]: the page isn't asking for a file, the file is over `limit` or didn't
    /// arrive whole, or the page didn't take it.
    pub async fn upload<S, E>(
        &self,
        name: &str,
        body: S,
        limit: u64,
    ) -> Result<BrowserInfo, BrowserRefused>
    where
        S: futures_util::Stream<Item = Result<axum::body::Bytes, E>> + Unpin,
        E: std::error::Error + Send + Sync + 'static,
    {
        self.restore().await;
        if self.engine().file_ask().is_none() {
            return Err(BrowserRefused(
                "The page isn't asking for a file".to_owned(),
            ));
        }
        let safe: String = {
            let flat = name.replace(['/', '\\'], "-");
            let flat = flat.trim_start_matches('.');
            let count = flat.chars().count();
            flat.chars().skip(count.saturating_sub(120)).collect()
        };
        let safe = if safe.is_empty() {
            "file".to_owned()
        } else {
            safe
        };
        let dir = self
            .inner
            .parts
            .places
            .data_dir
            .join("browser")
            .join("uploads");
        let path = dir.join(&safe);
        calls::make_private_dir(&dir)
            .await
            .map_err(|e| BrowserRefused(format!("The file wasn't kept: {e}")))?;
        calls::write_private(&path, body, limit)
            .await
            .map_err(|e| {
                BrowserRefused(if e.kind() == std::io::ErrorKind::FileTooLarge {
                    format!("The file is over {} MB", limit / 1_000_000)
                } else {
                    format!("The file didn't arrive whole: {e}")
                })
            })?;
        // The page takes the file in one message, so it is read back once for it.
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|e| BrowserRefused(format!("The file wasn't kept: {e}")))?;
        let ext = safe
            .rsplit_once('.')
            .map_or_default(|(_, e)| e.to_lowercase());
        let mime = emberhut_core::browser::upload_type(&ext).unwrap_or("application/octet-stream");
        self.engine()
            .answer_file_ask(FilePayload {
                name: safe.clone(),
                mime: mime.to_owned(),
                bytes,
            })
            .await
            .map_err(|e| BrowserRefused(first_line(&e.to_string()).to_owned()))?;
        let task = self.own().surface.task;
        if let Some(task) = task {
            let url = self.own().surface.url.clone();
            self.kit()
                .note(
                    task,
                    &format!("The owner attached {safe} to the page"),
                    None,
                    &url,
                )
                .await;
        }
        Ok(self.summary().await)
    }

    async fn take(&self, who: &str) {
        let handoff = {
            let now = self.now();
            let mut own = self.own();
            own.control = Control::Owner;
            own.surface.driver = Some(who.to_owned());
            own.surface.at = Some(now);
            own.handoff.clone()
        };
        self.engine().owner_drives(true);
        let why = handoff.map_or_default(|h| format!(" ({h})"));
        self.kit()
            .log(format!("The owner took over the browser{why}"))
            .await;
    }

    async fn hand_back(&self) {
        let (why, task, url) = {
            let now = self.now();
            let mut own = self.own();
            let why = own.handoff.take();
            own.control = Control::Agent;
            own.surface.driver = None;
            own.device = None;
            own.surface.at = Some(now);
            (why, own.surface.task, own.surface.url.clone())
        };
        self.keep().await;
        self.engine().owner_drives(false);
        let asked = why.map_or_default(|w| format!(" (you asked: {w})"));
        let for_task = task.map_or_default(|t| format!(" for task {t}"));
        let on = if url.is_empty() {
            "a blank tab".to_owned()
        } else {
            url.clone()
        };
        self.kit()
            .desk
            .wake(
                "browser",
                mask(&format!(
                    "The owner handed the browser back{asked}{for_task}. It's on {on}. Take a \
                     browser_snapshot and carry on."
                )),
            )
            .await;
        if let Some(task) = task {
            self.kit()
                .note(task, "The owner handed the browser back", None, &url)
                .await;
            self.kit()
                .desk
                .clear_action(task, OWN_ACTION.to_owned())
                .await;
        }
    }

    /// The tab the server's own browser acts in: the current one if it's still open, else the
    /// first, else a new one.
    async fn own_page(&self) -> Result<TabId, EngineError> {
        let current = self.own().current;
        let tab = page_of(self.engine().as_ref(), current, |_| {}).await?;
        self.own().current = Some(tab);
        Ok(tab)
    }

    fn own_surface(&self) -> Arc<dyn Surface> {
        Arc::new(OwnSurface {
            service: self.clone(),
        })
    }

    // ── The agents' side ──

    /// Runs browser tool `name` for `who`. Every failure is said in the answer (masked).
    pub async fn act(&self, name: &str, args: &BrowserArgs, who: &Who) -> Act {
        self.restore().await;
        let surface = match self.pick(args, who).await {
            Ok(surface) => surface,
            Err(said) => return Act::text(mask(&said)),
        };
        let result = self.act_on(name, args, who, surface.as_ref()).await;
        match result {
            Ok(act) => Act {
                text: mask(&act.text),
                image: act.image,
            },
            Err(EngineError::TabClosed(said)) => Act::text(mask(&said)),
            Err(error) => {
                let said = mask(first_line(&error.to_string()));
                let again = if said.contains("has been closed") || said.contains("went away") {
                    " If this was in the owner's Chrome, its link dropped for a moment and comes \
                     back by itself: call the same tool again."
                } else {
                    ""
                };
                Act::text(format!("Browser: {said}{again}"))
            }
        }
    }

    /// Where a tool acts: what it says, else its task's own browser, else the server's own.
    async fn pick(&self, args: &BrowserArgs, who: &Who) -> Result<Arc<dyn Surface>, String> {
        let said = args
            .browser
            .as_deref()
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                args.chrome
                    .as_deref()
                    .filter(|c| !c.is_empty())
                    .map(|c| format!("chrome:{c}"))
            })
            .unwrap_or_default();
        match self.chrome().pick(&said, who, self.kit()).await {
            Pick::Own => Ok(self.own_surface()),
            Pick::Chrome(surface) => Ok(surface),
            Pick::Said(said) => Err(said),
        }
    }

    #[expect(clippy::too_many_lines, reason = "one arm per tool")]
    async fn act_on(
        &self,
        name: &str,
        args: &BrowserArgs,
        who: &Who,
        on: &dyn Surface,
    ) -> Result<Act, EngineError> {
        if name == "browser_takeback" {
            let reason = slice(args.reason.as_deref().unwrap_or_default(), 160);
            let reason = if reason.is_empty() {
                "it was needed".to_owned()
            } else {
                reason
            };
            let task = who.task.or(on.state().task);
            return Ok(Act::text(on.takeback(reason, task).await));
        }
        if let Some(wait) = on.busy() {
            return Ok(Act::text(wait));
        }
        if let Some(closed) = self.closed_since_told(name, on) {
            return Ok(Act::text(closed));
        }
        let url_asked = open_url(args.url.as_deref().unwrap_or("undefined"));
        if name == "browser_open" && blocked_site(&url_asked) {
            return Ok(Act::text(checks::not_opened(&url_asked)));
        }
        if name == "browser_open"
            && let Some(no) = refused(on, &url_asked, self.now().as_millisecond())
        {
            return Ok(Act::text(no));
        }
        let now = self.now();
        on.update(&mut |s| {
            s.driver = Some(who.driver.clone());
            s.task = who.task.or(s.task);
            s.at = Some(now);
        });
        let engine = on.engine();
        let tab = surface_page(on).await?;
        let task = who.task;
        let place = on.place();
        let at = args.at.clone().unwrap_or_else(|| "undefined".to_owned());
        // A page that went to a blocked site on its own since the last tool isn't acted on.
        if !matches!(name, "browser_open" | "browser_tabs" | "browser_handoff")
            && let Some(no) = self.let_go_if_blocked(on, tab, task).await
        {
            return Ok(Act::text(no));
        }
        let output = match name {
            "browser_open" => {
                let reused = self.reuse(on, &url_asked);
                let target = match (reused, args.new_tab) {
                    (Some(t), _) => t,
                    (None, true) => engine.new_tab(None).await?,
                    (None, false) => tab,
                };
                on.use_tab(Some(target));
                let since = now.as_millisecond();
                engine.goto(target, url_asked.clone(), OPEN_WAIT).await?;
                if let Some(shut) = closed_since(on, since) {
                    return Err(EngineError::TabClosed(format!("{}.", shut.text)));
                }
                self.opened(on, &url_asked, target);
                drop(engine.settle(target, LOAD_WAIT).await);
                remember(on, target).await;
                let shot = engine.screenshot(target, 60).await.ok().flatten();
                self.log1(on, task, &format!("Opened {url_asked}"), shot)
                    .await;
                let s = on.state();
                let mut said = reopened(on, &url_asked, self.now().as_millisecond());
                if reused.is_some() && args.new_tab {
                    add(
                        &mut said,
                        format_args!(
                            "(In the tab the assistant already has on {} for this task, not a new \
                         one.)",
                            host_of(&url_asked)
                        ),
                    );
                }
                let said = said.trim();
                let said = if said.is_empty() {
                    String::new()
                } else {
                    format!("{said}\n")
                };
                format!(
                    "{} · {}\n{said}\n{}",
                    s.title,
                    s.url,
                    snap(engine.as_ref(), target, None).await?
                )
            }
            "browser_snapshot" => {
                remember(on, tab).await;
                let s = on.state();
                let scope = args.scope.clone().filter(|s| !s.is_empty());
                let tree = format!(
                    "{} · {}\n\n{}",
                    s.title,
                    s.url,
                    snap(engine.as_ref(), tab, scope).await?
                );
                if args.screenshot {
                    let image = engine.screenshot(tab, 60).await.ok().flatten();
                    return Ok(match image {
                        Some(image) => Act {
                            text: tree,
                            image: Some(image),
                        },
                        None => Act::text(format!(
                            "{tree}\n\n(No picture: the page changed while it was checked for \
                             keys.)"
                        )),
                    });
                }
                tree
            }
            "browser_click" => {
                let what = label(engine.as_ref(), tab, &at).await;
                let element = args.element.clone().unwrap_or_default();
                let mut okd = String::new();
                if is_final(&what) || is_final(&element) {
                    let Some(t) = task else {
                        return Ok(Act::text(format!(
                            "\"{what}\" looks like a final step. Browse for a task (task_id) so \
                             the owner can OK it."
                        )));
                    };
                    let s = on.state();
                    let page = if s.url.is_empty() {
                        engine.info(tab).await?.url
                    } else {
                        s.url.clone()
                    };
                    let composed = engine
                        .element(tab, at.clone(), ElementQuery::Written)
                        .await
                        .ok()
                        .map(|v| strings(&v));
                    let shown = if s.title.is_empty() {
                        page.clone()
                    } else {
                        s.title.clone()
                    };
                    let gated = self
                        .kit()
                        .gate
                        .step(Step {
                            task: t,
                            what: what.clone(),
                            page: format!("{} {page}", s.title),
                            question: format!(
                                "The browser is about to click \"{what}\" on {}{place}. Go ahead?",
                                url_host(&page).unwrap_or_default()
                            ),
                            action: format!("OK the browser's \"{what}\" step"),
                            brief: vec![format!("Page: {shown}"), format!("Step: \"{what}\"")],
                            composed,
                        })
                        .await;
                    match gated {
                        Gated::Wait(text) => {
                            let shot = engine.screenshot(tab, 60).await.ok().flatten();
                            self.log1(on, task, &format!("Paused before \"{what}\""), shot)
                                .await;
                            return Ok(Act::text(text));
                        }
                        Gated::Through(by) => okd = format!(" (OK'd: {by})"),
                    }
                }
                let force = !place.is_empty();
                if force
                    && engine
                        .element(tab, at.clone(), ElementQuery::Disabled)
                        .await
                        .ok()
                        .and_then(|v| v.as_bool())
                        == Some(true)
                {
                    return Ok(Act::text(format!(
                        "\"{what}\" is disabled on the page: nothing was clicked."
                    )));
                }
                engine.click(tab, at.clone(), force).await?;
                drop(engine.settle(tab, LOAD_WAIT).await);
                remember(on, tab).await;
                self.log1(on, task, &format!("Clicked \"{what}\"{okd}"), None)
                    .await;
                let s = on.state();
                format!("Clicked \"{what}\"{okd}. {} · {}", s.title, s.url)
            }
            "browser_type" => {
                let text = args.text.clone().unwrap_or_else(|| "undefined".to_owned());
                let field = engine
                    .element(tab, at.clone(), ElementQuery::Field)
                    .await
                    .map_or_default(|v| value_text(&v));
                if wants_secret(&field) {
                    return Ok(Act::text(
                        "That field wants a secret (a password or a code). Never type those: \
                         call browser_handoff so the owner types it themselves.",
                    ));
                }
                let editor = engine
                    .element(tab, at.clone(), ElementQuery::CodeEditor(text.clone()))
                    .await
                    .ok()
                    .map(|v| value_text(&v))
                    .filter(|e| !e.is_empty());
                match editor.as_deref() {
                    Some("keys") => {
                        let mac = engine
                            .page(tab, PageQuery::IsMac)
                            .await
                            .ok()
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        engine
                            .press(tab, if mac { "Meta+a" } else { "Control+a" }.to_owned())
                            .await?;
                        engine.insert_text(tab, text.clone()).await?;
                    }
                    Some(_) => {}
                    None => engine.fill(tab, at.clone(), text.clone()).await?,
                }
                if args.submit {
                    let press = BrowserArgs {
                        key: Some("Enter".to_owned()),
                        ..args.clone()
                    };
                    return Box::pin(self.act_on("browser_press", &press, who, on)).await;
                }
                let into = label(engine.as_ref(), tab, &at).await;
                self.log1(on, task, &format!("Typed into {into}"), None)
                    .await;
                if editor.is_some() {
                    "Typed: the editor's whole text replaced with it. Check it with a \
                     browser_snapshot before saving."
                        .to_owned()
                } else {
                    "Typed.".to_owned()
                }
            }
            "browser_press" => {
                let key = args.key.clone().unwrap_or_else(|| "undefined".to_owned());
                // Enter sends what has the focus: a final form, a final button, or what a text
                // box holds (a chat composer is in no form), each with the owner's OK.
                if is_enter_key(&key)
                    && let Some(wait) = self.before_enter(on, tab, &key, task).await?
                {
                    return Ok(Act::text(wait));
                }
                engine.press(tab, key.clone()).await?;
                remember(on, tab).await;
                format!("Pressed {key}.")
            }
            "browser_autofill" => {
                if !on.autofill() {
                    return Ok(Act::text(format!(
                        "The password manager can't be asked from here{place}: browser_handoff \
                         so the owner fills the login themselves."
                    )));
                }
                engine.press(tab, AUTOFILL_KEY.to_owned()).await?;
                tokio::time::sleep(Duration::from_millis(1_500)).await;
                self.log1(
                    on,
                    task,
                    "Asked the password manager to fill the login",
                    None,
                )
                .await;
                "Asked the password manager to fill this login. Take a browser_snapshot: filled \
                 password fields show as ••••••. If nothing filled, it is locked or has no login \
                 for this site: browser_handoff so the owner unlocks it or picks the login."
                    .to_owned()
            }
            "browser_scroll" => {
                let dy = args
                    .dy
                    .filter(|d| *d != 0.0 && d.is_finite())
                    .unwrap_or(600.0);
                engine.wheel(tab, dy).await?;
                "Scrolled.".to_owned()
            }
            "browser_wait_for" => {
                let seconds = args.seconds.filter(|s| *s != 0.0 && s.is_finite());
                if let Some(text) = args.text.clone().filter(|t| !t.is_empty()) {
                    let wait = seconds.unwrap_or(20.0).clamp(0.0, 60.0);
                    engine
                        .wait_for_text(tab, text, Duration::from_secs_f64(wait))
                        .await?;
                } else {
                    let wait = seconds.unwrap_or(2.0).clamp(0.0, 30.0);
                    tokio::time::sleep(Duration::from_secs_f64(wait)).await;
                }
                remember(on, tab).await;
                let s = on.state();
                format!("Ready. {} · {}", s.title, s.url)
            }
            "browser_tabs" => {
                let tabs = engine.tabs().await?;
                let action = args.action.clone().unwrap_or_default();
                let index = args
                    .index
                    .filter(|i| i.fract() == 0.0 && *i >= 0.0)
                    .and_then(|i| {
                        #[expect(
                            clippy::cast_possible_truncation,
                            clippy::cast_sign_loss,
                            reason = "a whole, non-negative index"
                        )]
                        let i = i as usize;
                        tabs.get(i).map(|t| t.id)
                    });
                match (action.as_str(), index) {
                    ("new", _) => {
                        // Its address as browser_open takes one: a page the owner keeps
                        // closing isn't opened again.
                        let url = args.url.as_deref().filter(|u| !u.is_empty()).map(open_url);
                        if let Some(url) = url.as_deref().filter(|url| blocked_site(url)) {
                            return Ok(Act::text(checks::not_opened(url)));
                        }
                        if let Some(url) = &url
                            && let Some(no) = refused(on, url, self.now().as_millisecond())
                        {
                            return Ok(Act::text(no));
                        }
                        let t = engine.new_tab(None).await?;
                        on.use_tab(Some(t));
                        if let Some(url) = url {
                            engine.goto(t, url.clone(), OPEN_WAIT).await?;
                            self.opened(on, &url, t);
                            self.log1(on, task, &format!("Opened {url}"), None).await;
                        }
                    }
                    ("select", Some(t)) => {
                        on.use_tab(Some(t));
                        engine.select_tab(t).await?;
                    }
                    ("close", Some(t)) => {
                        engine.close_tab(t).await?;
                        on.use_tab(None);
                    }
                    _ => {}
                }
                let front = surface_page(on).await?;
                remember(on, front).await;
                engine
                    .tabs()
                    .await?
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        let shown = if Some(t.id) == on.current() {
                            " (in front)"
                        } else {
                            ""
                        };
                        format!("{i}{shown}: {}", t.url)
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            "browser_close" => {
                engine.close_tab(tab).await?;
                on.use_tab(None);
                let next = surface_page(on).await?;
                remember(on, next).await;
                "Closed the tab.".to_owned()
            }
            "browser_handoff" => {
                let reason = slice(args.reason.as_deref().unwrap_or("undefined"), 160);
                let shot = engine.screenshot(tab, 60).await.ok().flatten();
                on.handoff(reason, task, shot).await
            }
            "browser_save_secret" => self.save_secret(args, who, on, tab).await?,
            "browser_download" => self.download(args, who, on, tab).await?,
            "browser_upload" => self.upload_file(args, who, on, tab).await?,
            other => return Ok(Act::text(format!("No tool {other}"))),
        };
        // Where the step went (a click, a key, a redirect): a blocked site is let go of. A tab
        // handed to the owner is theirs.
        if name != "browser_handoff"
            && let Some(now) = on.current()
            && let Some(no) = self.let_go_if_blocked(on, now, task).await
        {
            return Ok(Act::text(no));
        }
        Ok(Act::text(output))
    }

    /// A line in the task's browser log (keys masked), told to that browser too.
    async fn log1(&self, on: &dyn Surface, task: Option<i64>, line: &str, shot: Option<Vec<u8>>) {
        if let Some(t) = task {
            let url = on.state().url;
            self.kit()
                .note(t, &format!("{line}{}", on.place()), shot, &url)
                .await;
        }
        on.step(&mask(line));
    }

    /// Its tab closed after the last tool was done with it: said now, rather than this tool
    /// going on in another tab. (`browser_open` goes where it's told.)
    fn closed_since_told(&self, name: &str, on: &dyn Surface) -> Option<String> {
        on.closes()?;
        if matches!(name, "browser_open" | "browser_takeback" | "browser_tabs")
            || on.current().is_some()
        {
            return None;
        }
        let key = on.place();
        let told = self
            .inner
            .told
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&key)
            .copied()
            .unwrap_or(0);
        let since = closed_since(on, told + 1).filter(|c| c.by != "relinked")?;
        self.inner
            .told
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key, since.at);
        Some(said(&since, false, self.now().as_millisecond()))
    }

    /// The tab to open `url` in instead of a new one: the one opened for that page a moment
    /// ago, if it's still open.
    fn reuse(&self, on: &dyn Surface, url: &str) -> Option<TabId> {
        let key = page_key(url);
        let now = self.now().as_millisecond();
        let place = on.place();
        let opened = self
            .inner
            .opened
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        opened
            .iter()
            .rev()
            .find(|(p, k, at, _)| *p == place && *k == key && now - at <= CLOSE_KEPT_MS)
            .map(|(_, _, _, tab)| *tab)
    }

    fn opened(&self, on: &dyn Surface, url: &str, tab: TabId) {
        let now = self.now().as_millisecond();
        let place = on.place();
        let mut opened = self
            .inner
            .opened
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        opened.retain(|(_, _, at, _)| now - at <= CLOSE_KEPT_MS);
        opened.push((place, page_key(url), now, tab));
    }

    /// `browser_save_secret`: the key at a ref goes from the page into the secrets folder
    /// (owner-only), read inside this server; the agent gets its length and first characters.
    #[expect(clippy::too_many_lines, reason = "one flow, check by check")]
    async fn save_secret(
        &self,
        args: &BrowserArgs,
        who: &Who,
        on: &dyn Surface,
        tab: TabId,
    ) -> Result<String, EngineError> {
        let engine = on.engine();
        let Some(task) = who.task else {
            return Ok(
                "browser_save_secret needs task_id: the save is logged on that task.".to_owned(),
            );
        };
        let name = args.name.clone().unwrap_or_default().trim().to_owned();
        let want = args.expect_prefix.clone().unwrap_or_default();
        let own = OWN_SECRETS.contains(&name.as_str());
        if !secret_name_ok(&name) || own {
            return Ok(format!(
                "\"{name}\" can't be a secret's name: lowercase letters, digits and dashes, like \
                 secret_link's (slack-acme-user){}.",
                if own {
                    ", and not one of the server's own keys"
                } else {
                    ""
                }
            ));
        }
        let url = engine.info(tab).await?.url;
        let host = url_host(&url).unwrap_or_else(|| url.chars().take(40).collect());
        if blocked_site(&url) {
            let host = if host.is_empty() {
                "this page".to_owned()
            } else {
                host
            };
            return Ok(format!(
                "Nothing is saved from {host}: it's a blocked site (the owner's passwords and \
                 accounts). If the task needs it, browser_handoff."
            ));
        }
        let at = args.at.clone().unwrap_or_else(|| "undefined".to_owned());
        let Ok(Value::Object(got)) = engine.element(tab, at.clone(), ElementQuery::KeyAt).await
        else {
            return Ok(format!(
                "No element {at} on this page now: take a browser_snapshot and use the ref it \
                 shows."
            ));
        };
        let why = got.get("why").and_then(Value::as_str).unwrap_or_default();
        if why == "password" {
            return Ok(
                "That's a password field: passwords are never saved or read. \
                       browser_save_secret is only for an API key or token a service's \
                       developer page shows (if the page hides a key behind dots, click its Show \
                       button first)."
                    .to_owned(),
            );
        }
        if why == "several" {
            return Ok(
                "That element holds more than one value: take a browser_snapshot and pass \
                       the ref in the key's own [secret: …] mark, or the field itself."
                    .to_owned(),
            );
        }
        let value = got
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let length = value.chars().count();
        let head: String = value.chars().take((length / 4).min(5)).collect();
        if why == "hidden"
            || value.contains("•••")
            || value.contains("●●●")
            || value.contains("***")
            || value.contains('…')
        {
            return Ok(
                "The page shows that key hidden (dots): click its Show / Reveal button, \
                       then save again."
                    .to_owned(),
            );
        }
        if value.is_empty() {
            return Ok(
                "There's no key at that ref (nothing, or only words). Take a \
                       browser_snapshot: a key shows as [secret: …, use browser_save_secret ref \
                       eN]."
                    .to_owned(),
            );
        }
        let spaced = value.chars().any(char::is_whitespace);
        if spaced || !(16..=8192).contains(&length) {
            return Ok(format!(
                "That isn't an API key or token ({length} characters{}). Pick the field or text \
                 that holds only the key.",
                if spaced { ", with spaces" } else { "" }
            ));
        }
        if !want.is_empty() && !value.starts_with(&want) {
            return Ok(format!(
                "Not saved: it doesn't start with \"{want}\" (it starts with \"{head}\"). Check \
                 it's the right key on the page."
            ));
        }
        let dir = &self.inner.parts.places.secrets_dir;
        let path = dir.join(&name);
        let before = std::fs::read_to_string(&path)
            .ok()
            .map(|h| h.trim().to_owned());
        let said = format!("{length} characters, starts with \"{head}\"");
        if before.as_deref() == Some(value.as_str()) {
            return Ok(format!(
                "Already saved: {name} holds this same key ({said}). Nothing changed."
            ));
        }
        let replaced = before.as_deref().is_some_and(|h| !h.is_empty());
        let written = surface::make_private_dir(dir).and_then(|()| {
            if replaced {
                std::fs::copy(&path, dir.join(format!("{name}.prev")))?;
                write_private(
                    &dir.join(format!("{name}.prev")),
                    before.as_deref().unwrap_or_default().as_bytes(),
                )?;
            }
            write_private(&path, value.as_bytes())
        });
        if let Err(error) = written {
            return Ok(format!(
                "Not saved: the secrets folder can't be written ({error})."
            ));
        }
        let kept = if replaced {
            format!(", replacing the old one (kept as {name}.prev)")
        } else {
            String::new()
        };
        let line =
            format!("Saved a key from {host} into the server's secrets as {name} ({said}){kept}");
        self.log1(on, Some(task), &line, None).await;
        self.kit()
            .desk
            .note(task, who.note_by(), line.clone())
            .await;
        self.kit()
            .log(format!(
                "{} {} for task {task}",
                who.driver,
                lower_first(&line)
            ))
            .await;
        Ok(format!(
            "Saved {name}: {said}{}. Never read it back or repeat it; tell the owner in one line \
             that it's saved.",
            if replaced {
                format!(" (it replaced the old one, kept as {name}.prev)")
            } else {
                String::new()
            }
        ))
    }

    /// `browser_upload`: a file on this machine goes into a page's file field, with no file
    /// dialog anywhere; setting it sends nothing.
    #[expect(clippy::too_many_lines, reason = "one flow, check by check")]
    async fn upload_file(
        &self,
        args: &BrowserArgs,
        who: &Who,
        on: &dyn Surface,
        tab: TabId,
    ) -> Result<String, EngineError> {
        let engine = on.engine();
        let Some(task) = who.task else {
            return Ok("browser_upload needs task_id: the file is logged on that task.".to_owned());
        };
        // The checks read the file system (its path resolved, its size, its first bytes).
        let places = self.inner.parts.places.clone();
        let given = args.path.clone().unwrap_or_else(|| "undefined".to_owned());
        let file = match tokio::task::spawn_blocking(move || places.uploadable(&given)).await {
            Ok(Ok(file)) => file,
            Ok(Err(why)) => return Ok(why),
            Err(_) => return Ok("The file couldn't be checked: try again.".to_owned()),
        };
        let url = engine.info(tab).await?.url;
        let host = url_host(&url).unwrap_or_default();
        if blocked_site(&url) {
            return Ok(format!(
                "No file is put into {}: {}.",
                if host.is_empty() { "this page" } else { &host },
                if host.is_empty() {
                    "open the site's page first"
                } else {
                    "it's a blocked site (the owner's passwords and accounts)"
                }
            ));
        }
        let at = args.at.clone().unwrap_or_else(|| "undefined".to_owned());
        let kind = engine
            .element(tab, at.clone(), ElementQuery::FileKind)
            .await;
        let (kind, field_label) = match kind {
            Ok(v) => (
                v.get(0)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                v.get(1)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            ),
            Err(_) => {
                return Ok(format!(
                    "No element {at} on this page now: take a browser_snapshot and use the ref \
                     it shows."
                ));
            }
        };
        let mut what = label(engine.as_ref(), tab, &at).await;
        if !field_label.is_empty() && what == at {
            what = mask(&field_label);
        }
        let via = match kind.as_str() {
            "other" => {
                return Ok(format!(
                    "\"{what}\" is a text field, not a file field: pass the file field's ref, or \
                     the button that opens the page's file picker."
                ));
            }
            "disabled" => {
                return Ok(format!(
                    "The file field \"{what}\" is disabled on the page now."
                ));
            }
            "inner" => FileVia::Inner,
            "field" => FileVia::Field,
            _ => {
                if is_final(&what) {
                    return Ok(format!(
                        "\"{what}\" looks like a final step, not a file picker: pass the file \
                         field's ref or the button that opens the picker (Save or Submit \
                         afterwards is a browser_click)."
                    ));
                }
                FileVia::Picker
            }
        };
        let bytes = tokio::fs::read(&file.path)
            .await
            .map_err(|e| EngineError::Failed(format!("{} couldn't be read: {e}", file.name)))?;
        let taken = engine
            .set_file(
                tab,
                at.clone(),
                via,
                FilePayload {
                    name: file.name.clone(),
                    mime: file.mime.to_owned(),
                    bytes,
                },
            )
            .await?;
        match taken {
            None => {
                return Ok(format!(
                    "Clicking \"{what}\" didn't open a file picker. Take a browser_snapshot and \
                     pass the file field's ref (or the button that opens the picker)."
                ));
            }
            Some(name) if name != file.name => {
                return Ok(format!(
                    "The {} didn't take {}. Take a browser_snapshot.",
                    if via == FileVia::Picker {
                        "page's picker"
                    } else {
                        "file field"
                    },
                    file.name
                ));
            }
            Some(_) => {}
        }
        drop(engine.settle(tab, Duration::from_secs(5)).await);
        remember(on, tab).await;
        let through = if what.is_empty() || what == at {
            String::new()
        } else if via == FileVia::Picker {
            format!(" through \"{what}\"")
        } else {
            format!(" (\"{what}\")")
        };
        let line = format!(
            "Put {} ({}) into the page's file field{through} on {host}",
            file.name,
            size_text(file.size)
        );
        self.log1(on, Some(task), &line, None).await;
        let place = on.place();
        self.kit()
            .desk
            .note(task, who.note_by(), format!("{line}{place}"))
            .await;
        Ok(format!(
            "{line}{place}. The page has it now; nothing is sent until the page's own Save or \
             Submit (browser_click, which asks the owner). Take a browser_snapshot to see what \
             the page did with it (a preview, a crop step)."
        ))
    }

    /// `browser_download`: a file a page links to, fetched inside the tab with the page's own
    /// login (no cookie or token is read), in pieces, into a file on this machine.
    #[expect(clippy::too_many_lines, reason = "one flow, check by check")]
    async fn download(
        &self,
        args: &BrowserArgs,
        who: &Who,
        on: &dyn Surface,
        tab: TabId,
    ) -> Result<String, EngineError> {
        let engine = on.engine();
        let Some(task) = who.task else {
            return Ok(
                "browser_download needs task_id: the file is logged on that task.".to_owned(),
            );
        };
        let Some(dest) = args.dest.clone().filter(|d| !d.is_empty()) else {
            return Ok(
                "browser_download needs dest: a folder (or a file name) on this machine, \
                       like ~/Documents/."
                    .to_owned(),
            );
        };
        let mut link = args.url.clone().unwrap_or_default();
        let at = args.at.clone().unwrap_or_default();
        if link.is_empty() && !at.is_empty() {
            link = engine
                .element(tab, at.clone(), ElementQuery::Link)
                .await
                .map_or_default(|v| value_text(&v));
            if link.is_empty() {
                return Ok(format!(
                    "{at} isn't a link to a file (no address on it): take a browser_snapshot and \
                     pass the link's ref, or the file's url."
                ));
            }
        }
        if link.is_empty() {
            return Ok(
                "browser_download needs url (the file's address) or ref (its link on the \
                       page)."
                    .to_owned(),
            );
        }
        let page_url = engine.info(tab).await?.url;
        let base = url::Url::parse(&page_url).ok();
        let Ok(file_url) = url::Url::options().base_url(base.as_ref()).parse(&link) else {
            return Ok(format!("\"{link}\" isn't an address."));
        };
        if !matches!(file_url.scheme(), "http" | "https") {
            return Ok(format!("Only http(s) files: not {}:.", file_url.scheme()));
        }
        let host = url_host(&page_url).unwrap_or_default();
        let file_host = url_host(file_url.as_str()).unwrap_or_default();
        let href = file_url.to_string();
        if blocked_site(&page_url) || blocked_site(&href) {
            let from = if !host.is_empty() && !blocked_site(&href) {
                host.clone()
            } else {
                file_host.clone()
            };
            let why = if page_url.starts_with("http") {
                "it's a blocked site (the owner's passwords and accounts)"
            } else {
                "open the site's page in this tab first (the file is fetched with that page's login)"
            };
            return Ok(format!("Nothing is downloaded from {from}: {why}."));
        }
        let got = engine
            .page(
                tab,
                PageQuery::FetchStart {
                    href: href.clone(),
                    max: DOWNLOAD_MAX,
                },
            )
            .await?;
        let error = got.get("error").map_or_default(value_text);
        let size = got.get("size").and_then(Value::as_u64);
        if error == "big" {
            return Ok(format!(
                "That file is {}: downloads stop at 500 MB.",
                size_text(size.unwrap_or(0))
            ));
        }
        if !error.is_empty() {
            let elsewhere = if error.to_lowercase().contains("fetch") && file_host != host {
                format!(
                    " (it's on {file_host}, not this page's site {host}, and that site doesn't \
                     let pages fetch it: open a page of {file_host} first, then download)"
                )
            } else {
                String::new()
            };
            return Ok(format!(
                "Couldn't fetch {href} from this page: {error}{elsewhere}."
            ));
        }
        let key = got.get("key").map_or_default(value_text);
        let size = size.unwrap_or(0);
        let mime = got.get("type").map_or_default(value_text);
        let cd = got.get("cd").map_or_default(value_text);
        let from = got
            .get("url")
            .map(value_text)
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| href.clone());
        let saved = self
            .save_pieces(engine.as_ref(), tab, &key, size, &mime, &cd, &from, &dest)
            .await;
        drop(engine.page(tab, PageQuery::FetchDrop { key }).await);
        let path = match saved {
            Ok(path) => path,
            Err(said) => return Ok(said),
        };
        let stored = path
            .file_name()
            .map_or_default(|n| n.to_string_lossy().into_owned());
        let report = format!(
            "Saved {stored} ({}) from {file_host} to {}",
            size_text(size),
            path.display()
        );
        self.log1(on, Some(task), &report, None).await;
        let place = on.place();
        let fetched = if place.is_empty() {
            String::new()
        } else {
            format!(" (fetched{place})")
        };
        self.kit()
            .desk
            .note(task, who.note_by(), format!("{report}{fetched}"))
            .await;
        let shown = mime.split(';').next().unwrap_or_default();
        let shown = if shown.is_empty() { "unknown" } else { shown };
        let with = if place.is_empty() {
            String::new()
        } else {
            format!(", fetched{place} with its own login")
        };
        Ok(format!(
            "{report}{with}. Type: {shown}. The assistant's tab stayed where it was."
        ))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the fetched file's parts, read from the page"
    )]
    async fn save_pieces(
        &self,
        engine: &dyn BrowserEngine,
        tab: TabId,
        key: &str,
        size: u64,
        mime: &str,
        cd: &str,
        from: &str,
        dest: &str,
    ) -> Result<PathBuf, String> {
        let name = file_name(cd, from, mime);
        let html = |s: &str| {
            std::path::Path::new(s)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"))
        };
        if mime.to_lowercase().starts_with("text/html") && !html(&name) && !html(dest) {
            return Err(
                "That address gave a web page, not a file (likely a login page or the \
                        site's viewer page around the file). Take a browser_snapshot: find the \
                        file's own link (a PDF's is often an iframe or a \"download\" link) and \
                        download that."
                    .to_owned(),
            );
        }
        let places = self.inner.parts.places.clone();
        let (given, named) = (dest.to_owned(), name.clone());
        let path = tokio::task::spawn_blocking(move || places.dest_for(&given, &named))
            .await
            .map_err(|_| "Couldn't pick where to save it: try again.".to_owned())??;
        let part = PathBuf::from(format!("{}.part", path.display()));
        let mut file = files::create_part(&part)
            .await
            .map_err(|e| format!("Couldn't save it: {e}"))?;
        // Piece by piece into the `.part` file: a big download is never held whole.
        let written: Result<(), String> = async {
            let mut got = 0_u64;
            let mut at = 0;
            while at < size {
                let piece = engine
                    .page(
                        tab,
                        PageQuery::FetchPiece {
                            key: key.to_owned(),
                            at,
                            n: PIECE,
                        },
                    )
                    .await
                    .map_err(|e| format!("Couldn't download it: {}", first_line(&e.to_string())))?;
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(value_text(&piece))
                    .map_err(|_| "The download came out unreadable: try again.".to_owned())?;
                got += u64::try_from(decoded.len()).unwrap_or(u64::MAX);
                if got > size {
                    break;
                }
                file.write_all(&decoded)
                    .await
                    .map_err(|e| format!("Couldn't save it: {e}"))?;
                at += PIECE;
            }
            if got != size {
                return Err("The download came out short: try again.".to_owned());
            }
            file.flush()
                .await
                .map_err(|e| format!("Couldn't save it: {e}"))?;
            tokio::fs::rename(&part, &path)
                .await
                .map_err(|e| format!("Couldn't save it: {e}"))
        }
        .await;
        drop(file);
        if let Err(said) = written {
            if let Err(error) = tokio::fs::remove_file(&part).await
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(%error, "a download's part file was not removed");
            }
            return Err(said);
        }
        Ok(path)
    }

    /// `browser_plan`: a job's final steps named once, up front, on task `task`.
    pub async fn plan(&self, task: i64, job: &str, steps: Vec<String>) -> String {
        if !self.kit().desk.exists(task).await {
            return format!("Not planned: no task {task}");
        }
        mask(
            &self
                .kit()
                .gate
                .plan(task, job.trim().to_owned(), steps)
                .await,
        )
    }

    /// The owner's turn in the server's own browser, as a surface's `busy` says it.
    fn own_busy(&self) -> Option<String> {
        let own = self.own();
        (own.control == Control::Owner).then(|| {
            let why = own.handoff.as_ref().map_or_default(|h| format!(" ({h})"));
            format!(
                "The owner has the browser{why}. Wait for their hand-back (an event \"browser\"), \
                 then carry on; if it can't wait, browser_takeback."
            )
        })
    }
}

/// The "Needs you" line of a task handed over in the server's own browser starts with this.
const OWN_ACTION: &str = "In the assistant's browser";

fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map_or_default(|c| c.to_lowercase().collect::<String>() + chars.as_str())
}

/// The tab to act in: `current` if it's still open, else the first, else a new one.
async fn page_of(
    engine: &dyn BrowserEngine,
    current: Option<TabId>,
    keep: impl FnOnce(TabId),
) -> Result<TabId, EngineError> {
    let tabs = engine.tabs().await?;
    let tab = match current.filter(|c| tabs.iter().any(|t| t.id == *c)) {
        Some(tab) => tab,
        None => match tabs.first() {
            Some(first) => first.id,
            None => engine.new_tab(None).await?,
        },
    };
    keep(tab);
    Ok(tab)
}

async fn surface_page(on: &dyn Surface) -> Result<TabId, EngineError> {
    let engine = on.engine();
    let current = on.current();
    let mut picked = None;
    let tab = page_of(engine.as_ref(), current, |t| picked = Some(t)).await?;
    on.use_tab(picked);
    Ok(tab)
}

/// The surface's state takes the tab's address and title.
async fn remember(on: &dyn Surface, tab: TabId) {
    let info = on.engine().info(tab).await.ok();
    on.update(&mut |s| match &info {
        Some(info) => {
            s.url.clone_from(&info.url);
            s.title.clone_from(&info.title);
        }
        None => s.title.clear(),
    });
}

/// An element's words: its text, value, label or title; the ref itself when it says nothing.
async fn label(engine: &dyn BrowserEngine, tab: TabId, at: &str) -> String {
    let text = engine
        .element(tab, at.to_owned(), ElementQuery::Label)
        .await
        .map_or_default(|v| value_text(&v).trim().chars().take(80).collect::<String>());
    mask(if text.is_empty() { at } else { &text })
}

/// The tree the agent reads: every password field's value blanked, every key masked with the
/// ref to save it from (masked before it's folded or cut, so no key is cut in two), and a big
/// page folded to fit.
async fn snap(
    engine: &dyn BrowserEngine,
    tab: TabId,
    scope: Option<String>,
) -> Result<String, EngineError> {
    let pdf = engine
        .page(tab, PageQuery::IsPdf)
        .await
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if pdf {
        let url = engine.info(tab).await.map_or_default(|t| t.url);
        return Ok(format!(
            "(A PDF, shown in Chrome's PDF viewer: its text isn't readable here. browser_download \
             with url \"{url}\" and a dest saves it on this machine, where it can be read.)"
        ));
    }
    let tree = match tokio::time::timeout(SNAP_WAIT, engine.snapshot(tab, scope.clone())).await {
        Ok(Ok(Some(tree))) => tree,
        Ok(Ok(None)) => {
            let scope = scope.unwrap_or_default();
            let what = if scope == "main" {
                "main landmark"
            } else {
                "such ref"
            };
            return Ok(format!(
                "(Nothing for scope \"{scope}\" on this page: no {what}. Snapshot without scope.)"
            ));
        }
        Ok(Err(EngineError::Timeout(_))) | Err(_) => {
            let part = if scope.is_none() {
                "page"
            } else {
                "part of the page"
            };
            return Ok(format!(
                "(This {part} is too big or too deeply nested to read in one snapshot: nothing \
                 came back within {} s. Filter or page it down on the site, or snapshot a part \
                 with scope (\"main\", or a ref). browser_wait_for and browser_click still work.)",
                SNAP_WAIT.as_secs()
            ));
        }
        Ok(Err(error)) => return Err(error),
    };
    let mut secrets = engine
        .page(tab, PageQuery::SecretValues)
        .await
        .map_or_default(|v| strings(&v));
    secrets.retain(|s| !s.is_empty());
    secrets.sort_by_key(|s| std::cmp::Reverse(s.chars().count()));
    let mut tree = tree;
    for secret in &secrets {
        tree = tree.replace(secret.as_str(), "••••••");
    }
    let keys = engine
        .page(tab, PageQuery::Keys)
        .await
        .map_or_default(|v| strings(&v));
    let tree = mask_tree(&tree, &keys);
    Ok(if scope.is_some() {
        cut(&tree)
    } else {
        fold(&tree, SNAPSHOT_LIMIT)
    })
}

/// The last close since `since` (ms) that the assistant didn't make itself.
fn closed_since(on: &dyn Surface, since: i64) -> Option<TabClose> {
    on.closes()?
        .into_iter()
        .rev()
        .find(|c| c.at >= since && c.by != BY_ASSISTANT)
}

/// What the agent is told about a close: who did it, and what to do.
#[expect(clippy::cast_precision_loss, reason = "seconds ago, said rounded")]
fn said(gone: &TabClose, again: bool, now: i64) -> String {
    let ago = ((now - gone.at) as f64 / 1000.0).round().max(1.0);
    let head = if again {
        format!("Not opened again: {} {ago} s ago.", lower_first(&gone.text))
    } else {
        format!("{}.", gone.text)
    };
    match gone.by.as_str() {
        "page" => format!(
            "{head} That site shuts its own tab (some do when a browser is driven), so opening it \
             again here only closes again. Don't retry it in this browser: use the server's own \
             browser (browser \"{OWN_BROWSER}\"), or browser_handoff so the owner does it."
        ),
        "crash" => {
            format!("{head} You may open it once more; if it crashes again, tell the owner.")
        }
        "frame" => format!(
            "{head} Don't open it again in another tab (that one would be cut the same way): \
             browser_handoff so the owner does this step there, or use the server's own browser \
             (browser \"{OWN_BROWSER}\")."
        ),
        _ if again => format!(
            "{head} They closed it twice: don't open it again. If the task still needs that page, \
             ask them (task_ask) or use the server's own browser (browser \"{OWN_BROWSER}\")."
        ),
        _ => format!(
            "{head} They may have closed it on purpose: open it again only if the task needs it \
             (once; if they close it again, it isn't reopened), else ask them."
        ),
    }
}

/// A page opened a moment ago that closed itself, or whose link was cut, isn't opened again;
/// one the owner closed twice isn't either.
fn refused(on: &dyn Surface, url: &str, now: i64) -> Option<String> {
    let key = page_key(url);
    let closes: Vec<TabClose> = on
        .closes()?
        .into_iter()
        .filter(|c| page_key(&c.url) == key)
        .collect();
    let gone = closes.last()?;
    if gone.by == BY_ASSISTANT || matches!(gone.by.as_str(), "crash" | "relinked") {
        return None;
    }
    let by_owner = closes
        .iter()
        .filter(|c| matches!(c.by.as_str(), "chrome" | "window"))
        .count();
    if matches!(gone.by.as_str(), "page" | "frame") || by_owner >= 2 {
        return Some(said(gone, true, now));
    }
    None
}

/// Said with a `browser_open` that goes back to a page the owner closed a moment ago (once).
#[expect(clippy::cast_precision_loss, reason = "seconds ago, said rounded")]
fn reopened(on: &dyn Surface, url: &str, now: i64) -> String {
    let key = page_key(url);
    match on
        .closes()
        .and_then(|c| c.into_iter().rev().find(|c| page_key(&c.url) == key))
    {
        Some(last) if matches!(last.by.as_str(), "chrome" | "window") => {
            let ago = ((now - last.at) as f64 / 1000.0).round().max(1.0);
            format!(
                "(The owner closed the assistant's tab on this page {ago} s ago; opened again \
                 once. If they close it again, it isn't reopened: ask them.) "
            )
        }
        _ => String::new(),
    }
}

/// The server's own browser as a surface: its hand-off gives the owner the live view.
#[derive(Debug)]
struct OwnSurface {
    service: BrowserService,
}

impl Surface for OwnSurface {
    fn place(&self) -> String {
        String::new()
    }

    fn engine(&self) -> Arc<dyn BrowserEngine> {
        Arc::clone(self.service.engine())
    }

    fn state(&self) -> SurfaceState {
        self.service.own().surface.clone()
    }

    fn update(&self, change: &mut dyn FnMut(&mut SurfaceState)) {
        change(&mut self.service.own().surface);
    }

    fn current(&self) -> Option<TabId> {
        self.service.own().current
    }

    fn use_tab(&self, tab: Option<TabId>) {
        self.service.own().current = tab;
    }

    fn busy(&self) -> Option<String> {
        self.service.own_busy()
    }

    fn handoff(
        &self,
        reason: String,
        task: Option<i64>,
        shot: Option<Vec<u8>>,
    ) -> BoxFuture<'_, String> {
        Box::pin(async move {
            let service = &self.service;
            service.own().handoff = Some(reason.clone());
            service.take("the owner (asked by the assistant)").await;
            service.own().device = Some(String::new());
            service.keep().await;
            let kit = service.kit();
            if let Some(task) = task {
                let url = service.own().surface.url.clone();
                kit.note(
                    task,
                    &format!("Handed over to the owner: {reason}"),
                    shot,
                    &url,
                )
                .await;
                kit.desk
                    .set_action(task, Some(slice(&format!("{OWN_ACTION}: {reason}"), 90)))
                    .await;
            }
            kit.push(
                "The assistant needs you in the browser",
                &format!("{reason}. Open the app, Browser, to take over."),
            )
            .await;
            kit.desk
                .wake(
                    "ask",
                    format!(
                        "You handed the browser to the owner ({reason}). Tell them in one line; \
                         it's on their screen under Browser."
                    ),
                )
                .await;
            "The owner has the browser now. You'll get an event \"browser\" when they hand it \
             back. Stop for now."
                .to_owned()
        })
    }

    fn takeback(&self, reason: String, task: Option<i64>) -> BoxFuture<'_, String> {
        Box::pin(async move {
            let service = &self.service;
            let (held, url) = {
                let now = service.now();
                let mut own = service.own();
                if own.control != Control::Owner {
                    return "The owner isn't driving: the browser is yours already. Carry on."
                        .to_owned();
                }
                own.control = Control::Agent;
                own.handoff = None;
                own.surface.driver = None;
                own.device = None;
                own.surface.at = Some(now);
                (own.surface.task, own.surface.url.clone())
            };
            service.keep().await;
            service.engine().owner_drives(false);
            service
                .kit()
                .took_back("the browser", &reason, [task, held], OWN_ACTION, &url)
                .await;
            let on = if url.is_empty() {
                "a blank tab".to_owned()
            } else {
                url
            };
            format!(
                "Took the browser back from the owner (they were told: {reason}). It's on {on}: \
                 take a browser_snapshot and carry on."
            )
        })
    }

    fn autofill(&self) -> bool {
        true
    }
}

/// Appends `part` to `line` (writing to a `String` never fails).
fn add(line: &mut String, part: fmt::Arguments<'_>) {
    use std::fmt::Write as _;
    let _written = line.write_fmt(part);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_close_is_said_with_what_to_do() {
        let gone = TabClose {
            at: 1_000,
            by: "page".to_owned(),
            url: "https://a.com/x".to_owned(),
            text: "The tab on a.com closed: the page closed it itself".to_owned(),
            tab: 3,
        };
        let text = said(&gone, true, 4_000);
        assert!(text.starts_with(
            "Not opened again: the tab on a.com closed: the page closed it itself 3 s ago."
        ));
        assert!(text.contains("Don't retry it in this browser"));
        assert_eq!(lower_first("Saved"), "saved");
    }
}
