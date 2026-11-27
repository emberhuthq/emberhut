//! A scripted browser and a scripted tasks desk, for the browser's and the Mac tools' tests: no
//! browser is started and nothing touches the network.
#![expect(
    clippy::unwrap_used,
    reason = "the fakes' locks: a poisoned one fails the test that holds it"
)]

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine as _;
use emberhut_server::services::browser::desk::{Card, NoteBy, TaskDesk};
use emberhut_server::services::browser::engine::{
    BrowserEngine, ElementQuery, EngineError, EngineFuture, FileAsk, FilePayload, FileVia, Nav,
    PageQuery, Tab, TabId,
};
use futures_util::future::BoxFuture;
use jiff::Timestamp;
use serde_json::{Value, json};

/// One element of a scripted page.
#[derive(Clone, Debug, Default)]
pub(crate) struct FakeElement {
    /// What it says.
    pub(crate) label: String,
    /// What a field says about itself.
    pub(crate) field: String,
    /// The text written around it.
    pub(crate) written: Vec<String>,
    /// What a code editor answers (`set`, `keys`), or none.
    pub(crate) editor: Option<String>,
    /// What `browser_save_secret` reads there.
    pub(crate) key_at: Value,
    /// What it is for a file: `field`, `inner`, `button`, `other`, `disabled`.
    pub(crate) file_kind: String,
    /// The address it links to.
    pub(crate) link: String,
    /// Clicking it (a picker button) opens no picker.
    pub(crate) no_picker: bool,
    /// Clicking it takes the tab to this address.
    pub(crate) goes_to: String,
}

/// A scripted page.
#[derive(Clone, Debug, Default)]
pub(crate) struct FakePage {
    /// Its address.
    pub(crate) url: String,
    /// Its title.
    pub(crate) title: String,
    /// Its tree, as the engine writes it.
    pub(crate) tree: String,
    /// Its elements, by ref.
    pub(crate) elements: HashMap<String, FakeElement>,
    /// Its password fields' values.
    pub(crate) secrets: Vec<String>,
    /// The keys on it.
    pub(crate) keys: Vec<String>,
    /// The focused form's submit label.
    pub(crate) form_submit: String,
    /// What has the focus (`composer`, `search`, `control`…); none: a form's field when there
    /// is a form, else nothing.
    pub(crate) focus: String,
}

impl FakePage {
    /// A page at `url` titled `title`, with `tree`.
    pub(crate) fn new(url: &str, title: &str, tree: &str) -> Self {
        Self {
            url: url.to_owned(),
            title: title.to_owned(),
            tree: tree.to_owned(),
            ..Self::default()
        }
    }

    /// With an element.
    pub(crate) fn with(mut self, at: &str, element: FakeElement) -> Self {
        self.elements.insert(at.to_owned(), element);
        self
    }
}

#[derive(Debug, Default)]
struct State {
    tabs: Vec<(TabId, FakePage)>,
    front: Option<TabId>,
    next: TabId,
    sites: HashMap<String, FakePage>,
    steps: Vec<String>,
    files: Vec<(String, usize)>,
    file_ask: Option<bool>,
    owner: bool,
    fetch: HashMap<String, (String, String, Vec<u8>)>,
    connected: bool,
    /// WhatsApp Web: linked, and its composer's text.
    wa_linked: bool,
    composer: String,
}

/// The scripted browser: its sites, and every step taken in it.
#[derive(Debug, Default)]
pub(crate) struct FakeBrowser {
    state: Mutex<State>,
}

impl FakeBrowser {
    /// The page served at `page.url` from now on.
    pub(crate) fn site(&self, page: FakePage) {
        let mut state = self.state.lock().unwrap();
        state.connected = true;
        state.sites.insert(page.url.clone(), page);
    }

    /// WhatsApp Web shows its chat list (linked) or its login page.
    pub(crate) fn whatsapp_linked(&self, linked: bool) {
        self.state.lock().unwrap().wa_linked = linked;
    }

    /// The steps taken so far (`click e3`, `fill e2 text`, `press Enter`…).
    pub(crate) fn steps(&self) -> Vec<String> {
        self.state.lock().unwrap().steps.clone()
    }

    /// The files put into pages: name and size.
    pub(crate) fn files(&self) -> Vec<(String, usize)> {
        self.state.lock().unwrap().files.clone()
    }

    /// The page asks for a file.
    pub(crate) fn ask_for_file(&self, multiple: bool) {
        self.state.lock().unwrap().file_ask = Some(multiple);
    }

    /// The owner drives, as the engine was told.
    pub(crate) fn owner_driving(&self) -> bool {
        self.state.lock().unwrap().owner
    }

    /// A file at `href`, fetched with the page's login.
    pub(crate) fn file(&self, href: &str, mime: &str, disposition: &str, bytes: &[u8]) {
        self.state.lock().unwrap().fetch.insert(
            href.to_owned(),
            (mime.to_owned(), disposition.to_owned(), bytes.to_vec()),
        );
    }

    fn step(&self, step: String) {
        self.state.lock().unwrap().steps.push(step);
    }

    fn page_of(&self, tab: TabId) -> Result<FakePage, EngineError> {
        let state = self.state.lock().unwrap();
        state
            .tabs
            .iter()
            .find(|(id, _)| *id == tab)
            .map(|(_, p)| p.clone())
            .ok_or_else(|| EngineError::TabClosed("The tab has been closed.".to_owned()))
    }

    fn element_of(&self, tab: TabId, at: &str) -> Result<FakeElement, EngineError> {
        self.page_of(tab)?
            .elements
            .get(at)
            .cloned()
            .ok_or_else(|| EngineError::NoRef(at.to_owned()))
    }
}

fn ready<T: Send + 'static>(value: Result<T, EngineError>) -> EngineFuture<'static, T> {
    Box::pin(async move { value })
}

impl BrowserEngine for FakeBrowser {
    fn connected(&self) -> bool {
        self.state.lock().unwrap().connected
    }

    fn tabs(&self) -> EngineFuture<'_, Vec<Tab>> {
        let state = self.state.lock().unwrap();
        let front = state.front.or(state.tabs.first().map(|(id, _)| *id));
        let tabs = state
            .tabs
            .iter()
            .map(|(id, p)| Tab {
                id: *id,
                url: p.url.clone(),
                title: p.title.clone(),
                front: Some(*id) == front,
            })
            .collect();
        ready(Ok(tabs))
    }

    fn new_tab(&self, url: Option<String>) -> EngineFuture<'_, TabId> {
        let mut state = self.state.lock().unwrap();
        state.next += 1;
        let id = state.next;
        let page = url
            .and_then(|u| state.sites.get(&u).cloned())
            .unwrap_or_else(|| FakePage::new("about:blank", "", ""));
        state.tabs.push((id, page));
        state.steps.push(format!("new tab {id}"));
        ready(Ok(id))
    }

    fn close_tab(&self, tab: TabId) -> EngineFuture<'_, ()> {
        let mut state = self.state.lock().unwrap();
        state.tabs.retain(|(id, _)| *id != tab);
        state.steps.push(format!("close tab {tab}"));
        ready(Ok(()))
    }

    fn select_tab(&self, tab: TabId) -> EngineFuture<'_, ()> {
        let mut state = self.state.lock().unwrap();
        state.front = Some(tab);
        state.steps.push(format!("select tab {tab}"));
        ready(Ok(()))
    }

    fn goto(&self, tab: TabId, url: String, _wait: Duration) -> EngineFuture<'_, ()> {
        let mut state = self.state.lock().unwrap();
        let page = state
            .sites
            .get(&url)
            .cloned()
            .unwrap_or_else(|| FakePage::new(&url, "Untitled", "- text: nothing here"));
        if let Some((_, p)) = state.tabs.iter_mut().find(|(id, _)| *id == tab) {
            *p = page;
        }
        state.steps.push(format!("goto {url}"));
        ready(Ok(()))
    }

    fn settle(&self, _tab: TabId, _wait: Duration) -> EngineFuture<'_, ()> {
        ready(Ok(()))
    }

    fn navigate(&self, _tab: TabId, nav: Nav) -> EngineFuture<'_, ()> {
        self.step(format!("{nav:?}"));
        ready(Ok(()))
    }

    fn info(&self, tab: TabId) -> EngineFuture<'_, Tab> {
        ready(self.page_of(tab).map(|p| Tab {
            id: tab,
            url: p.url,
            title: p.title,
            front: false,
        }))
    }

    fn snapshot(&self, tab: TabId, scope: Option<String>) -> EngineFuture<'_, Option<String>> {
        let page = self.page_of(tab);
        ready(page.map(|p| match scope.as_deref() {
            None => Some(p.tree),
            Some("main") => p.tree.contains("- main").then_some(p.tree),
            Some(at) => p.elements.contains_key(at).then_some(p.tree),
        }))
    }

    fn page(&self, tab: TabId, query: PageQuery) -> EngineFuture<'_, Value> {
        let page = match self.page_of(tab) {
            Ok(page) => page,
            Err(error) => return ready(Err(error)),
        };
        let state = self.state.lock().unwrap();
        let value = match query {
            PageQuery::IsPdf => json!(
                std::path::Path::new(&page.url)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
            ),
            PageQuery::SecretValues => json!(page.secrets),
            PageQuery::Keys => json!(page.keys),
            PageQuery::FocusedField => {
                let kind = match (page.focus.as_str(), page.form_submit.is_empty()) {
                    ("", true) => "none",
                    ("", false) => "field",
                    (kind, _) => kind,
                };
                json!({ "kind": kind, "form": page.form_submit, "label": "" })
            }
            PageQuery::FocusedWritten => json!([]),
            PageQuery::IsMac => json!(false),
            PageQuery::ScreenSize => json!([1440, 900]),
            PageQuery::FetchStart { href, max } => match state.fetch.get(&href) {
                Some((mime, cd, bytes)) if (bytes.len() as u64) <= max => json!({
                    "key": href, "size": bytes.len(), "type": mime, "cd": cd, "url": href,
                }),
                Some((_, _, bytes)) => json!({ "error": "big", "size": bytes.len() }),
                None => json!({ "error": "the site answered 404 Not Found" }),
            },
            PageQuery::Run { function, arg } => {
                use emberhut_server::services::browser::whatsapp as wa;
                if function == wa::STATE {
                    json!(if state.wa_linked { "linked" } else { "login" })
                } else if function == wa::TYPED {
                    json!(state.composer)
                } else if function == wa::HEADER {
                    json!("Book club")
                } else if function == wa::PICK {
                    json!(u64::from(arg == json!("Book club")))
                } else if function == wa::TICK {
                    json!("msg-dblcheck")
                } else {
                    json!(true)
                }
            }
            PageQuery::FetchPiece { key, at, n } => {
                let bytes = state.fetch.get(&key).map_or_default(|f| f.2.clone());
                let from = usize::try_from(at).unwrap().min(bytes.len());
                let to = usize::try_from(at + n).unwrap().min(bytes.len());
                json!(base64::engine::general_purpose::STANDARD.encode(&bytes[from..to]))
            }
            _ => Value::Null,
        };
        ready(Ok(value))
    }

    fn element(&self, tab: TabId, at: String, query: ElementQuery) -> EngineFuture<'_, Value> {
        let element = match self.element_of(tab, &at) {
            Ok(element) => element,
            Err(error) => return ready(Err(error)),
        };
        let value = match query {
            ElementQuery::Label => json!(element.label),
            ElementQuery::Field => json!(element.field),
            ElementQuery::Written => json!(element.written),
            ElementQuery::CodeEditor(_) => json!(element.editor),
            ElementQuery::KeyAt => element.key_at.clone(),
            ElementQuery::FileKind => json!([element.file_kind, ""]),
            ElementQuery::Link => json!(element.link),
            ElementQuery::Disabled => json!(false),
            _ => Value::Null,
        };
        ready(Ok(value))
    }

    fn click(&self, tab: TabId, at: String, _force: bool) -> EngineFuture<'_, ()> {
        let found = self.element_of(tab, &at);
        if let Ok(element) = &found {
            self.step(format!("click {at}"));
            if !element.goes_to.is_empty() {
                let mut state = self.state.lock().unwrap();
                let page = state
                    .sites
                    .get(&element.goes_to)
                    .cloned()
                    .unwrap_or_else(|| FakePage::new(&element.goes_to, "Untitled", ""));
                if let Some((_, p)) = state.tabs.iter_mut().find(|(id, _)| *id == tab) {
                    *p = page;
                }
            }
        }
        ready(found.map(drop))
    }

    fn fill(&self, tab: TabId, at: String, text: String) -> EngineFuture<'_, ()> {
        let found = self.element_of(tab, &at).map(drop);
        if found.is_ok() {
            self.step(format!("fill {at} {text}"));
        }
        ready(found)
    }

    fn press(&self, _tab: TabId, key: String) -> EngineFuture<'_, ()> {
        {
            let mut state = self.state.lock().unwrap();
            match key.as_str() {
                "Shift+Enter" => state.composer.push('\n'),
                "Backspace" => state.composer.clear(),
                _ => {}
            }
        }
        self.step(format!("press {key}"));
        ready(Ok(()))
    }

    fn insert_text(&self, _tab: TabId, text: String) -> EngineFuture<'_, ()> {
        self.state.lock().unwrap().composer.push_str(&text);
        self.step(format!("insert {text}"));
        ready(Ok(()))
    }

    fn wheel(&self, _tab: TabId, dy: f64) -> EngineFuture<'_, ()> {
        self.step(format!("wheel {dy}"));
        ready(Ok(()))
    }

    fn wait_for_text(&self, tab: TabId, text: String, _wait: Duration) -> EngineFuture<'_, ()> {
        ready(self.page_of(tab).and_then(|p| {
            if p.tree.contains(&text) {
                Ok(())
            } else {
                Err(EngineError::Timeout(format!(
                    "\"{text}\" didn't show within 20 s"
                )))
            }
        }))
    }

    fn screenshot(&self, tab: TabId, _quality: u8) -> EngineFuture<'_, Option<Vec<u8>>> {
        ready(self.page_of(tab).map(|_| Some(b"a picture".to_vec())))
    }

    fn set_file(
        &self,
        tab: TabId,
        at: String,
        via: FileVia,
        file: FilePayload,
    ) -> EngineFuture<'_, Option<String>> {
        let element = match self.element_of(tab, &at) {
            Ok(element) => element,
            Err(error) => return ready(Err(error)),
        };
        if via == FileVia::Picker && element.no_picker {
            return ready(Ok(None));
        }
        let mut state = self.state.lock().unwrap();
        state.files.push((file.name.clone(), file.bytes.len()));
        state.steps.push(format!("file {at} {via:?} {}", file.name));
        ready(Ok(Some(file.name)))
    }

    fn file_ask(&self) -> Option<FileAsk> {
        self.state
            .lock()
            .unwrap()
            .file_ask
            .map(|multiple| FileAsk { multiple })
    }

    fn answer_file_ask(&self, file: FilePayload) -> EngineFuture<'_, ()> {
        let mut state = self.state.lock().unwrap();
        if state.file_ask.take().is_none() {
            return ready(Err(EngineError::Failed(
                "The page isn't asking for a file".to_owned(),
            )));
        }
        state.files.push((file.name, file.bytes.len()));
        ready(Ok(()))
    }

    fn owner_drives(&self, owner: bool) {
        self.state.lock().unwrap().owner = owner;
    }
}

/// What the scripted desk was told.
#[derive(Debug, Default)]
struct Desk {
    notes: Vec<(i64, String)>,
    actions: HashMap<i64, Option<String>>,
    asks: Vec<(i64, Card)>,
    wakes: Vec<(String, String)>,
    attached: Vec<(i64, String)>,
    chose: HashMap<i64, String>,
}

/// The tasks desk of the tests: every task exists; the owner's choice is the test's.
#[derive(Debug, Default)]
pub(crate) struct FakeDesk {
    desk: Mutex<Desk>,
}

impl FakeDesk {
    /// The owner chooses `answer` on task `task`.
    pub(crate) fn choose(&self, task: i64, answer: &str) {
        self.desk
            .lock()
            .unwrap()
            .chose
            .insert(task, answer.to_owned());
    }

    /// The cards asked so far: their task and question.
    pub(crate) fn asks(&self) -> Vec<(i64, String)> {
        self.desk
            .lock()
            .unwrap()
            .asks
            .iter()
            .map(|(t, c)| (*t, c.question.clone()))
            .collect()
    }

    /// The notes on tasks.
    pub(crate) fn notes(&self) -> Vec<(i64, String)> {
        self.desk.lock().unwrap().notes.clone()
    }

    /// Task `task`'s "Needs you" line.
    pub(crate) fn action(&self, task: i64) -> Option<String> {
        self.desk
            .lock()
            .unwrap()
            .actions
            .get(&task)
            .cloned()
            .flatten()
    }

    /// The assistant's events: kind and text.
    pub(crate) fn wakes(&self) -> Vec<(String, String)> {
        self.desk.lock().unwrap().wakes.clone()
    }

    /// The files attached to tasks.
    pub(crate) fn attached(&self) -> Vec<(i64, String)> {
        self.desk.lock().unwrap().attached.clone()
    }
}

impl TaskDesk for FakeDesk {
    fn exists(&self, _task: i64) -> BoxFuture<'_, bool> {
        Box::pin(async { true })
    }

    fn note(&self, task: i64, _by: NoteBy, line: String) -> BoxFuture<'_, ()> {
        self.desk.lock().unwrap().notes.push((task, line));
        Box::pin(async {})
    }

    fn set_action(&self, task: i64, action: Option<String>) -> BoxFuture<'_, ()> {
        self.desk.lock().unwrap().actions.insert(task, action);
        Box::pin(async {})
    }

    fn clear_action(&self, task: i64, prefix: String) -> BoxFuture<'_, ()> {
        let mut desk = self.desk.lock().unwrap();
        if desk
            .actions
            .get(&task)
            .cloned()
            .flatten()
            .is_some_and(|a| a.starts_with(&prefix))
        {
            desk.actions.insert(task, None);
        }
        Box::pin(async {})
    }

    fn ask(&self, task: i64, card: Card) -> BoxFuture<'_, ()> {
        let mut desk = self.desk.lock().unwrap();
        desk.chose.remove(&task);
        desk.asks.push((task, card));
        Box::pin(async {})
    }

    fn chose_since(&self, task: i64, _at: Timestamp) -> BoxFuture<'_, Option<String>> {
        let chose = self.desk.lock().unwrap().chose.get(&task).cloned();
        Box::pin(async move { chose })
    }

    fn attach(&self, task: i64, path: String, _title: String) -> BoxFuture<'_, ()> {
        self.desk.lock().unwrap().attached.push((task, path));
        Box::pin(async {})
    }

    fn wake(&self, kind: &'static str, text: String) -> BoxFuture<'_, ()> {
        self.desk
            .lock()
            .unwrap()
            .wakes
            .push((kind.to_owned(), text));
        Box::pin(async {})
    }
}
