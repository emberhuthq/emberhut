//! The seam to a real browser: [`BrowserEngine`] is one browser (the server's own, or one of the
//! owner's Chrome profiles through the extension) as the browser service drives it, tab by tab.
//!
//! An engine reads and acts; the rules (what is a final step, what is masked, who may drive)
//! stay in the service. Elements are named by the refs the engine's last snapshot of that tab
//! gave (`e12`). Every picture an engine takes has the page's keys and tokens blacked out.
//!
//! [`super::cdp::CdpEngine`] is the real one (the `DevTools` protocol over a WebSocket); the
//! tests script their own.

use std::fmt;
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::Value;

/// A tab, by the engine's own id for it.
pub type TabId = u64;

/// What an engine answers.
pub type EngineResult<T> = Result<T, EngineError>;

/// A future an engine returns.
pub type EngineFuture<'a, T> = BoxFuture<'a, EngineResult<T>>;

/// Why an engine did not do what it was asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EngineError {
    /// The browser can't be reached (not running, not set up, gone).
    #[error("{0}")]
    Unavailable(String),
    /// The tab closed (or its link was cut) while the step worked in it.
    #[error("{0}")]
    TabClosed(String),
    /// No element by that ref on the page now.
    #[error("No element {0} on this page now: take a browser_snapshot and use the ref it shows.")]
    NoRef(String),
    /// The step didn't finish in time.
    #[error("{0}")]
    Timeout(String),
    /// The browser refused or failed the step; its words.
    #[error("{0}")]
    Failed(String),
}

/// One tab as the engine sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    /// Its id.
    pub id: TabId,
    /// Its address.
    pub url: String,
    /// Its title.
    pub title: String,
    /// It is the one on screen.
    pub front: bool,
}

/// A move in a tab's history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    /// Back.
    Back,
    /// Forward.
    Forward,
    /// Reload.
    Reload,
}

/// A question about the whole page, answered in it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PageQuery {
    /// Whether the page is a PDF in Chrome's viewer: a bool.
    IsPdf,
    /// The values of its password and one-time-code fields: strings.
    SecretValues,
    /// The keys and tokens on it (field values, text): strings.
    Keys,
    /// What has the focus, for a key that may send it: `{ kind, form, label }`. `kind` is
    /// `search` (a search field), `composer` (a text box, a text area or an editor: what a
    /// message or a post is written in), `field` (a one-line field of a form), `control` (a
    /// button, a link, a box to tick), `none` (nothing focused) or `unknown` (in a frame the
    /// page can't read); `form` is the label of the submit button of the form it is in
    /// (`submit`, or `""` in no form); `label` is what it says.
    FocusedField,
    /// The text written in the boxes around the focused element (what a send would send):
    /// strings.
    FocusedWritten,
    /// Whether the page's platform is a Mac (its select-all key): a bool.
    IsMac,
    /// The screen's size: `[width, height]`.
    ScreenSize,
    /// Fetches `href` with the page's own login, keeping the file in the page: `{ key, size,
    /// type, cd, url }`, or `{ error }` (`"big"` with `size` past `max`).
    FetchStart {
        /// The file's address.
        href: String,
        /// The most it may be, in bytes.
        max: u64,
    },
    /// `n` bytes from `at` of the file kept under `key`: base64.
    FetchPiece {
        /// The file's key.
        key: String,
        /// Where the piece starts.
        at: u64,
        /// Its length.
        n: u64,
    },
    /// Forgets the file kept under `key`.
    FetchDrop {
        /// The file's key.
        key: String,
    },
    /// One of the server's own page scripts (a function of one argument), with its argument:
    /// its value. The scripts are constants (WhatsApp Web's, [`super::whatsapp`]).
    Run {
        /// The script.
        function: &'static str,
        /// Its argument.
        arg: Value,
    },
}

/// A question about one element, answered in the page.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ElementQuery {
    /// What it says (its text, value, label or title, 80 characters): a string.
    Label,
    /// What a field says about itself (type, autocomplete, name, id, label, placeholder): a
    /// string.
    Field,
    /// The text written in the boxes around it, nearest first: strings.
    Written,
    /// Puts `text` into the code editor it is part of: `"set"` (the editor took it as one
    /// edit), `"keys"` (its input has the focus: select all, then type), or null (no editor).
    CodeEditor(String),
    /// The key it holds, for saving: `{ value }`, or `{ why }` (`password`, `several`,
    /// `hidden`, `none`).
    KeyAt,
    /// What it is for a file: `[kind, label]`, kind `field`, `disabled`, `other`, `inner` (a box
    /// holding one file field) or `button`.
    FileKind,
    /// The address it links to: a string, `""` for none.
    Link,
    /// Whether it is disabled: a bool.
    Disabled,
}

/// Where a file goes for [`BrowserEngine::set_file`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileVia {
    /// The element is the file field (or its label).
    Field,
    /// The element holds exactly one file field.
    Inner,
    /// The element is a button that opens the page's picker: clicked, the picker caught.
    Picker,
}

/// A file for a page.
#[derive(Clone, PartialEq, Eq)]
pub struct FilePayload {
    /// Its name.
    pub name: String,
    /// Its type.
    pub mime: String,
    /// Its bytes.
    pub bytes: Vec<u8>,
}

impl fmt::Debug for FilePayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilePayload")
            .field("name", &self.name)
            .field("mime", &self.mime)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

/// The page asks for a file (its own picker held back).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileAsk {
    /// It takes several.
    pub multiple: bool,
}

/// One browser, driven tab by tab.
pub trait BrowserEngine: fmt::Debug + Send + Sync {
    /// Connected now (without connecting).
    fn connected(&self) -> bool;

    /// Its tabs, in order (connecting first, when it isn't).
    fn tabs(&self) -> EngineFuture<'_, Vec<Tab>>;

    /// A new tab, on `url` when given.
    fn new_tab(&self, url: Option<String>) -> EngineFuture<'_, TabId>;

    /// Closes a tab.
    fn close_tab(&self, tab: TabId) -> EngineFuture<'_, ()>;

    /// Brings a tab to the front.
    fn select_tab(&self, tab: TabId) -> EngineFuture<'_, ()>;

    /// Goes to `url`, until its document is loaded (not its pictures), within `wait`.
    fn goto(&self, tab: TabId, url: String, wait: Duration) -> EngineFuture<'_, ()>;

    /// Waits until the page has loaded, at most `wait` (never an error for the wait).
    fn settle(&self, tab: TabId, wait: Duration) -> EngineFuture<'_, ()>;

    /// Back, forward or reload.
    fn navigate(&self, tab: TabId, nav: Nav) -> EngineFuture<'_, ()>;

    /// The tab's address and title now.
    fn info(&self, tab: TabId) -> EngineFuture<'_, Tab>;

    /// The page (or the part `scope`: `"main"` or a ref) as an accessibility tree, one element a
    /// line (`- button "Save" [ref=e12]`), children indented; refs are kept for the steps that
    /// follow. None: nothing on the page fits `scope`.
    fn snapshot(&self, tab: TabId, scope: Option<String>) -> EngineFuture<'_, Option<String>>;

    /// Asks the page.
    fn page(&self, tab: TabId, query: PageQuery) -> EngineFuture<'_, Value>;

    /// Asks an element by its ref.
    fn element(&self, tab: TabId, at: String, query: ElementQuery) -> EngineFuture<'_, Value>;

    /// Clicks an element. `force`: straight to it, without waiting for it to hold still.
    fn click(&self, tab: TabId, at: String, force: bool) -> EngineFuture<'_, ()>;

    /// Fills a field with `text` (its old text replaced).
    fn fill(&self, tab: TabId, at: String, text: String) -> EngineFuture<'_, ()>;

    /// Presses a key or a chord (`Enter`, `Meta+a`, `Control+Backslash`).
    fn press(&self, tab: TabId, key: String) -> EngineFuture<'_, ()>;

    /// Types `text` where the focus is, as one insertion.
    fn insert_text(&self, tab: TabId, text: String) -> EngineFuture<'_, ()>;

    /// Scrolls by `dy` pixels.
    fn wheel(&self, tab: TabId, dy: f64) -> EngineFuture<'_, ()>;

    /// Waits for `text` to show on the page, at most `wait` ([`EngineError::Timeout`] then).
    fn wait_for_text(&self, tab: TabId, text: String, wait: Duration) -> EngineFuture<'_, ()>;

    /// A JPEG of the tab with every key on it blacked out; none when the page couldn't be
    /// checked for keys.
    fn screenshot(&self, tab: TabId, quality: u8) -> EngineFuture<'_, Option<Vec<u8>>>;

    /// Puts `file` into a file field, by the element at `at`: the name the field took, or none
    /// when clicking a [`FileVia::Picker`] opened no picker.
    fn set_file(
        &self,
        tab: TabId,
        at: String,
        via: FileVia,
        file: FilePayload,
    ) -> EngineFuture<'_, Option<String>>;

    /// The page asks for a file and its picker is held back for the owner's app.
    fn file_ask(&self) -> Option<FileAsk> {
        None
    }

    /// The owner's file for the page that asked ([`Self::file_ask`]).
    fn answer_file_ask(&self, file: FilePayload) -> EngineFuture<'_, ()> {
        let _ = file;
        Box::pin(async {
            Err(EngineError::Failed(
                "The page isn't asking for a file".into(),
            ))
        })
    }

    /// The owner drives (a page's own dialog stays for them to answer) or an agent does (it is
    /// dismissed, and any they left open goes).
    fn owner_drives(&self, owner: bool) {
        let _ = owner;
    }

    /// Shows Chrome's own controls (tab strip, address bar) or only the page, full screen.
    fn show_controls(&self, show: bool) -> EngineFuture<'_, ()> {
        let _ = show;
        Box::pin(async { Ok(()) })
    }
}

/// No browser set up on this server: every step says so.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoBrowser;

/// What a step is told without a browser.
const NONE: &str = "No browser is set up on this server (EMBERHUT_BROWSER_CDP gives its DevTools \
                    address).";

fn none<T: Send + 'static>() -> EngineFuture<'static, T> {
    Box::pin(async { Err(EngineError::Unavailable(NONE.to_owned())) })
}

impl BrowserEngine for NoBrowser {
    fn connected(&self) -> bool {
        false
    }

    fn tabs(&self) -> EngineFuture<'_, Vec<Tab>> {
        none()
    }

    fn new_tab(&self, _url: Option<String>) -> EngineFuture<'_, TabId> {
        none()
    }

    fn close_tab(&self, _tab: TabId) -> EngineFuture<'_, ()> {
        none()
    }

    fn select_tab(&self, _tab: TabId) -> EngineFuture<'_, ()> {
        none()
    }

    fn goto(&self, _tab: TabId, _url: String, _wait: Duration) -> EngineFuture<'_, ()> {
        none()
    }

    fn settle(&self, _tab: TabId, _wait: Duration) -> EngineFuture<'_, ()> {
        none()
    }

    fn navigate(&self, _tab: TabId, _nav: Nav) -> EngineFuture<'_, ()> {
        none()
    }

    fn info(&self, _tab: TabId) -> EngineFuture<'_, Tab> {
        none()
    }

    fn snapshot(&self, _tab: TabId, _scope: Option<String>) -> EngineFuture<'_, Option<String>> {
        none()
    }

    fn page(&self, _tab: TabId, _query: PageQuery) -> EngineFuture<'_, Value> {
        none()
    }

    fn element(&self, _tab: TabId, _at: String, _query: ElementQuery) -> EngineFuture<'_, Value> {
        none()
    }

    fn click(&self, _tab: TabId, _at: String, _force: bool) -> EngineFuture<'_, ()> {
        none()
    }

    fn fill(&self, _tab: TabId, _at: String, _text: String) -> EngineFuture<'_, ()> {
        none()
    }

    fn press(&self, _tab: TabId, _key: String) -> EngineFuture<'_, ()> {
        none()
    }

    fn insert_text(&self, _tab: TabId, _text: String) -> EngineFuture<'_, ()> {
        none()
    }

    fn wheel(&self, _tab: TabId, _dy: f64) -> EngineFuture<'_, ()> {
        none()
    }

    fn wait_for_text(&self, _tab: TabId, _text: String, _wait: Duration) -> EngineFuture<'_, ()> {
        none()
    }

    fn screenshot(&self, _tab: TabId, _quality: u8) -> EngineFuture<'_, Option<Vec<u8>>> {
        none()
    }

    fn set_file(
        &self,
        _tab: TabId,
        _at: String,
        _via: FileVia,
        _file: FilePayload,
    ) -> EngineFuture<'_, Option<String>> {
        none()
    }
}
