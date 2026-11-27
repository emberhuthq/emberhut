//! The messaging tests' outside world: fakes of Slack, Google, the Mac helper's message routes,
//! speech to text, the WhatsApp daemon, WhatsApp Web and the tasks, each scripted by the test
//! and recording what it was asked; and [`Kit`], an app wired to all of them. No test here
//! reaches the network. Every name, workspace and number in these tests is made up.
#![expect(
    clippy::unwrap_used,
    reason = "the fakes' locks: a poisoned one fails the test that holds it"
)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use emberhut_core::RoomId;
use emberhut_server::config::AuthMode;
use emberhut_server::services::mac::MacError;
use emberhut_server::services::messaging::MessagingDeps;
use emberhut_server::services::messaging::gmail::{GoogleAnswer, GoogleApi};
use emberhut_server::services::messaging::hooks::{
    Answers, Ask, AskId, ChatOk, Chosen, HookError, NoTasks, SendOks, TaskId, Tasks, TextOk,
};
use emberhut_server::services::messaging::mac::{MacAnswer, MacMessages};
use emberhut_server::services::messaging::slack::{Download, SlackAnswer, SlackApi, SlackError};
use emberhut_server::services::messaging::voice_notes::Transcriber;
use emberhut_server::services::messaging::whatsapp::{DaemonError, WaDaemon};
use emberhut_server::services::messaging::whatsapp_web::{
    Contact, Tick, WebSendError, WhatsAppWeb,
};
use futures_util::future::BoxFuture;
use jiff::{SignedDuration, Timestamp};
use serde_json::{Value, json};

use crate::common::TestApp;

/// A call: its method and arguments.
type SlackCall = (String, Vec<(String, String)>);

/// Slack's Web API, scripted per method (and per its first argument's value).
#[derive(Debug, Default)]
pub(crate) struct FakeSlack {
    answers: Mutex<HashMap<(String, Option<String>), Value>>,
    scopes: Mutex<Option<String>>,
    files: Mutex<HashMap<String, Download>>,
    calls: Mutex<Vec<SlackCall>>,
}

impl FakeSlack {
    /// `method` answers `json` (with `ok: true`), for any arguments.
    pub(crate) fn answers(&self, method: &str, json: Value) {
        self.answers
            .lock()
            .unwrap()
            .insert((method.to_owned(), None), json);
    }

    /// `method` answers `json` when its first argument is `value`.
    pub(crate) fn answers_for(&self, method: &str, value: &str, json: Value) {
        self.answers
            .lock()
            .unwrap()
            .insert((method.to_owned(), Some(value.to_owned())), json);
    }

    /// Every answer says the token has these scopes.
    pub(crate) fn scopes(&self, scopes: &str) {
        *self.scopes.lock().unwrap() = Some(scopes.to_owned());
    }

    /// `url` downloads as `body` of `kind`.
    pub(crate) fn file(&self, url: &str, status: u16, kind: &str, body: &[u8]) {
        self.files.lock().unwrap().insert(
            url.to_owned(),
            Download {
                status,
                content_type: kind.to_owned(),
                body: Bytes::copy_from_slice(body),
            },
        );
    }

    /// The calls of `method`, their arguments.
    pub(crate) fn calls(&self, method: &str) -> Vec<Vec<(String, String)>> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == method)
            .map(|(_, args)| args.clone())
            .collect()
    }
}

impl SlackApi for FakeSlack {
    fn call<'a>(
        &'a self,
        _: &'a str,
        method: &'a str,
        args: &'a [(&'a str, String)],
    ) -> BoxFuture<'a, Result<SlackAnswer, SlackError>> {
        let recorded = args
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect();
        self.calls
            .lock()
            .unwrap()
            .push((method.to_owned(), recorded));
        let first = args.first().map(|(_, v)| v.clone());
        let answers = self.answers.lock().unwrap();
        let answer = answers
            .get(&(method.to_owned(), first))
            .or_else(|| answers.get(&(method.to_owned(), None)))
            .cloned();
        let scopes = self.scopes.lock().unwrap().clone();
        Box::pin(async move {
            match answer {
                Some(json) if json["ok"] == false => Err(SlackError::Refused {
                    method: method.to_owned(),
                    error: json["error"].as_str().unwrap_or("error").to_owned(),
                }),
                Some(json) => Ok(SlackAnswer { json, scopes }),
                None => Err(SlackError::Refused {
                    method: method.to_owned(),
                    error: "not_scripted".to_owned(),
                }),
            }
        })
    }

    fn download<'a>(
        &'a self,
        url: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<Download, SlackError>> {
        let file = self.files.lock().unwrap().get(url).cloned();
        Box::pin(
            async move { file.ok_or_else(|| SlackError::Unreachable("fetch failed".to_owned())) },
        )
    }
}

/// The Mac helper's message routes, scripted per path (exact, or without its query).
#[derive(Debug, Default)]
pub(crate) struct FakeMacMessages {
    answers: Mutex<HashMap<String, MacAnswer>>,
    paths: Mutex<Vec<String>>,
}

impl FakeMacMessages {
    /// `path` answers `json`.
    pub(crate) fn answers(&self, path: &str, json: &Value) {
        self.raw(
            path,
            200,
            "application/json",
            json.to_string().as_bytes(),
            None,
        );
    }

    /// `path` answers this.
    pub(crate) fn raw(
        &self,
        path: &str,
        status: u16,
        kind: &str,
        body: &[u8],
        seconds: Option<&str>,
    ) {
        self.answers.lock().unwrap().insert(
            path.to_owned(),
            MacAnswer {
                status,
                content_type: kind.to_owned(),
                seconds: seconds.map(str::to_owned),
                body: Bytes::copy_from_slice(body),
            },
        );
    }

    /// The paths asked, in order.
    pub(crate) fn paths(&self) -> Vec<String> {
        self.paths.lock().unwrap().clone()
    }
}

impl MacMessages for FakeMacMessages {
    fn get<'a>(&'a self, path: &'a str, _: Duration) -> BoxFuture<'a, Result<MacAnswer, MacError>> {
        self.paths.lock().unwrap().push(path.to_owned());
        let answers = self.answers.lock().unwrap();
        let bare = path.split('?').next().unwrap_or(path);
        let answer = answers.get(path).or_else(|| answers.get(bare)).cloned();
        Box::pin(async move { answer.ok_or(MacError::NotSetUp) })
    }
}

/// Google's token endpoint and Gmail, scripted.
#[derive(Debug, Default)]
pub(crate) struct FakeGoogle {
    tokens: Mutex<Vec<GoogleAnswer>>,
    gets: Mutex<HashMap<String, GoogleAnswer>>,
    forms: Mutex<Vec<Vec<(String, String)>>>,
    paths: Mutex<Vec<String>>,
}

impl FakeGoogle {
    /// The token endpoint's next answers, in order (the last one repeats).
    pub(crate) fn token(&self, status: u16, json: Value) {
        self.tokens
            .lock()
            .unwrap()
            .push(GoogleAnswer { status, json });
    }

    /// `GET users/me/<path>` (exact, or by what comes before its `?`) answers this.
    pub(crate) fn get(&self, path: &str, status: u16, json: Value) {
        self.gets
            .lock()
            .unwrap()
            .insert(path.to_owned(), GoogleAnswer { status, json });
    }

    /// The forms sent to the token endpoint.
    pub(crate) fn forms(&self) -> Vec<Vec<(String, String)>> {
        self.forms.lock().unwrap().clone()
    }

    /// The paths asked.
    pub(crate) fn paths(&self) -> Vec<String> {
        self.paths.lock().unwrap().clone()
    }
}

impl GoogleApi for FakeGoogle {
    fn token<'a>(
        &'a self,
        form: &'a [(&'a str, String)],
    ) -> BoxFuture<'a, Result<GoogleAnswer, String>> {
        self.forms.lock().unwrap().push(
            form.iter()
                .map(|(k, v)| ((*k).to_owned(), v.clone()))
                .collect(),
        );
        let mut tokens = self.tokens.lock().unwrap();
        let answer = if tokens.len() > 1 {
            Some(tokens.remove(0))
        } else {
            tokens.first().cloned()
        };
        Box::pin(async move { answer.ok_or_else(|| "fetch failed".to_owned()) })
    }

    fn get<'a>(&'a self, _: &'a str, path: &'a str) -> BoxFuture<'a, Result<GoogleAnswer, String>> {
        self.paths.lock().unwrap().push(path.to_owned());
        let gets = self.gets.lock().unwrap();
        let bare = path.split('?').next().unwrap_or(path);
        let answer = gets.get(path).or_else(|| gets.get(bare)).cloned();
        Box::pin(async move {
            Ok(answer.unwrap_or(GoogleAnswer {
                status: 404,
                json: json!({ "error": { "message": "not found" } }),
            }))
        })
    }

    fn consent_url(&self) -> String {
        "https://consent.example/auth".to_owned()
    }
}

/// Speech to text: what it hears, or why it can't.
#[derive(Debug)]
pub(crate) struct FakeTranscriber {
    said: Mutex<Result<String, String>>,
    heard: Mutex<Vec<(String, usize, String)>>,
}

impl Default for FakeTranscriber {
    fn default() -> Self {
        Self {
            said: Mutex::new(Err("nothing scripted".to_owned())),
            heard: Mutex::default(),
        }
    }
}

impl FakeTranscriber {
    /// It hears `text` from now on.
    pub(crate) fn hears(&self, text: &str) {
        *self.said.lock().unwrap() = Ok(text.to_owned());
    }

    /// It fails with `why` from now on.
    pub(crate) fn fails(&self, why: &str) {
        *self.said.lock().unwrap() = Err(why.to_owned());
    }

    /// What it was given: file names, sizes and prompts.
    pub(crate) fn heard(&self) -> Vec<(String, usize, String)> {
        self.heard.lock().unwrap().clone()
    }
}

impl Transcriber for FakeTranscriber {
    fn transcribe<'a>(
        &'a self,
        audio: Bytes,
        name: &'a str,
        prompt: &'a str,
    ) -> BoxFuture<'a, Result<String, String>> {
        self.heard
            .lock()
            .unwrap()
            .push((name.to_owned(), audio.len(), prompt.to_owned()));
        let said = self.said.lock().unwrap().clone();
        Box::pin(async move { said })
    }
}

/// The WhatsApp daemon, scripted per path.
#[derive(Debug, Default)]
pub(crate) struct FakeWaDaemon {
    answers: Mutex<HashMap<String, Result<Value, DaemonError>>>,
    calls: Mutex<Vec<(String, Option<Value>)>>,
}

impl FakeWaDaemon {
    /// `path` answers this.
    pub(crate) fn answers(&self, path: &str, answer: Result<Value, DaemonError>) {
        self.answers.lock().unwrap().insert(path.to_owned(), answer);
    }

    /// The calls of `path`, their bodies.
    pub(crate) fn calls(&self, path: &str) -> Vec<Option<Value>> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p == path)
            .map(|(_, body)| body.clone())
            .collect()
    }
}

impl WaDaemon for FakeWaDaemon {
    fn call<'a>(
        &'a self,
        path: &'a str,
        body: Option<Value>,
        _: Duration,
    ) -> BoxFuture<'a, Result<Value, DaemonError>> {
        self.calls.lock().unwrap().push((path.to_owned(), body));
        let answer = self
            .answers
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .unwrap_or_else(|| Err(DaemonError::NoAnswer("fetch failed".to_owned())));
        Box::pin(async move { answer })
    }
}

/// WhatsApp Web: types what it is given, and sends with the tick it is told.
#[derive(Debug, Default)]
pub(crate) struct FakeWaWeb {
    refuse: Mutex<Option<WebSendError>>,
    unsent: Mutex<Option<WebSendError>>,
    drafts: Mutex<Vec<(Contact, String)>>,
    sent: Mutex<Vec<String>>,
}

impl FakeWaWeb {
    /// Typing fails with `error` from now on.
    pub(crate) fn refuses(&self, error: WebSendError) {
        *self.refuse.lock().unwrap() = Some(error);
    }

    /// Pressing Send fails with `error` from now on (none: it goes through again).
    pub(crate) fn fails_to_send(&self, error: Option<WebSendError>) {
        *self.unsent.lock().unwrap() = error;
    }

    /// The drafts typed: to whom, what.
    pub(crate) fn drafts(&self) -> Vec<(Contact, String)> {
        self.drafts.lock().unwrap().clone()
    }

    /// What was sent.
    pub(crate) fn sent(&self) -> Vec<String> {
        self.sent.lock().unwrap().clone()
    }
}

impl WhatsAppWeb for FakeWaWeb {
    fn type_draft<'a>(
        &'a self,
        contact: &'a Contact,
        text: &'a str,
    ) -> BoxFuture<'a, Result<String, WebSendError>> {
        let refused = self.refuse.lock().unwrap().clone();
        if refused.is_none() {
            self.drafts
                .lock()
                .unwrap()
                .push((contact.clone(), text.to_owned()));
        }
        let header = contact.name.clone();
        Box::pin(async move { refused.map_or(Ok(header), Err) })
    }

    fn send<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<Tick, WebSendError>> {
        let unsent = self.unsent.lock().unwrap().clone();
        if unsent.is_none() {
            self.sent.lock().unwrap().push(text.to_owned());
        }
        Box::pin(async move { unsent.map_or(Ok(Tick::Delivered), Err) })
    }
}

/// The tasks: each made one gets the next id; what is asked (each ask with an id of its own)
/// and noted is kept; the test chooses answers, each to the task's latest ask.
#[derive(Debug, Default)]
pub(crate) struct FakeTasks {
    made: Mutex<Vec<(TaskId, String)>>,
    asks: Mutex<Vec<(TaskId, AskId, Ask)>>,
    notes: Mutex<Vec<(TaskId, String)>>,
    chosen: Mutex<Vec<(TaskId, Option<AskId>, Chosen)>>,
    done: Mutex<Vec<TaskId>>,
}

impl FakeTasks {
    /// The asks on `task`.
    pub(crate) fn asks(&self, task: TaskId) -> Vec<Ask> {
        self.asks
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, _, _)| *t == task)
            .map(|(_, _, ask)| ask.clone())
            .collect()
    }

    /// The latest ask on `task`.
    pub(crate) fn latest(&self, task: TaskId) -> Option<AskId> {
        self.asks
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(t, _, _)| *t == task)
            .map(|(_, id, _)| *id)
    }

    /// Asks `question` on `task` (Send it / Cancel), as the asker of a send card does; the
    /// ask's id.
    pub(crate) fn ask_card(&self, task: TaskId, question: &str) -> AskId {
        let mut asks = self.asks.lock().unwrap();
        let id = AskId::new(1 + i64::try_from(asks.len()).unwrap());
        let ask = Ask {
            question: question.to_owned(),
            options: vec!["Send it".to_owned(), "Cancel".to_owned()],
            title: question.to_owned(),
            brief: Vec::new(),
            approve: true,
        };
        asks.push((task, id, ask));
        id
    }

    /// The notes on `task`.
    pub(crate) fn notes(&self, task: TaskId) -> Vec<String> {
        self.notes
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, _)| *t == task)
            .map(|(_, note)| note.clone())
            .collect()
    }

    /// The tasks made: ids and titles.
    pub(crate) fn made(&self) -> Vec<(TaskId, String)> {
        self.made.lock().unwrap().clone()
    }

    /// The tasks marked done.
    pub(crate) fn done(&self) -> Vec<TaskId> {
        self.done.lock().unwrap().clone()
    }

    /// The owner chooses `choice` on `task` at `at`, answering the latest ask there.
    pub(crate) fn choose(&self, task: TaskId, choice: &str, at: Timestamp) {
        let ask = self.latest(task);
        self.chosen.lock().unwrap().push((
            task,
            ask,
            Chosen {
                at,
                choice: choice.to_owned(),
            },
        ));
    }
}

impl Tasks for FakeTasks {
    fn create<'a>(&'a self, title: &'a str, _: RoomId) -> BoxFuture<'a, Result<TaskId, HookError>> {
        let mut made = self.made.lock().unwrap();
        let id = 100 + TaskId::try_from(made.len()).unwrap();
        made.push((id, title.to_owned()));
        Box::pin(async move { Ok(id) })
    }

    fn is_open(&self, task: TaskId) -> BoxFuture<'_, Result<Option<bool>, HookError>> {
        let known = self.made.lock().unwrap().iter().any(|(id, _)| *id == task);
        let open = !self.done.lock().unwrap().contains(&task);
        Box::pin(async move { Ok(known.then_some(open)) })
    }

    fn ask(&self, task: TaskId, ask: Ask) -> BoxFuture<'_, Result<AskId, HookError>> {
        let id = self.ask_card(task, &ask.question);
        let mut asks = self.asks.lock().unwrap();
        if let Some(last) = asks.last_mut() {
            last.2 = ask;
        }
        Box::pin(async move { Ok(id) })
    }

    fn note(&self, task: TaskId, text: String) -> BoxFuture<'_, Result<(), HookError>> {
        self.notes.lock().unwrap().push((task, text));
        Box::pin(async { Ok(()) })
    }

    fn answers(&self, task: TaskId, ask: AskId) -> BoxFuture<'_, Result<Answers, HookError>> {
        let chosen = self
            .chosen
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, a, _)| *t == task && *a == Some(ask))
            .map(|(_, _, c)| c.clone())
            .collect();
        let latest = self.latest(task);
        Box::pin(async move { Ok(Answers { chosen, latest }) })
    }

    fn done(&self, task: TaskId) -> BoxFuture<'_, Result<(), HookError>> {
        self.done.lock().unwrap().push(task);
        Box::pin(async { Ok(()) })
    }

    fn project_room<'a>(
        &'a self,
        name: &'a str,
        _: &'a str,
        make: bool,
    ) -> BoxFuture<'a, Result<Option<RoomId>, HookError>> {
        let known = make || name == "Garden";
        Box::pin(async move { Ok(known.then_some(RoomId::MAIN)) })
    }
}

/// The guard rails' other OKs, as the test gives them: a tap on one card of a task (by the
/// time it was asked), a go in the chat for an exact text, an OK of an exact text on another
/// card, and a standing OK for a chat. It records what went out, which OKs were spent and
/// which cards were closed.
#[derive(Debug, Default)]
pub(crate) struct FakeOks {
    tapped: Mutex<Vec<(TaskId, Timestamp)>>,
    chat_oks: Mutex<Vec<(String, ChatOk)>>,
    text_oks: Mutex<Vec<(TaskId, String, TextOk)>>,
    spent: Mutex<Vec<String>>,
    standing: Mutex<Vec<String>>,
    sent: Mutex<Vec<(TaskId, String, String)>>,
    closed: Mutex<Vec<(TaskId, String)>>,
}

impl FakeOks {
    /// The owner taps the send answer on `task`'s card asked at `asked` (that card only).
    pub(crate) fn tap(&self, task: TaskId, asked: Timestamp) {
        self.tapped.lock().unwrap().push((task, asked));
    }

    /// The owner says go in the chat for exactly `text`; the OK is called `id`.
    pub(crate) fn go_in_chat(&self, text: &str, id: &str) {
        self.chat_oks.lock().unwrap().push((
            text.to_owned(),
            ChatOk {
                id: id.to_owned(),
                line: format!("The owner said send it in chat ({id})"),
            },
        ));
    }

    /// The owner OK'd exactly `text` on another card of `task`.
    pub(crate) fn ok_text(&self, task: TaskId, text: &str, how: &str) {
        self.text_oks.lock().unwrap().push((
            task,
            text.to_owned(),
            TextOk {
                how: how.to_owned(),
            },
        ));
    }

    /// The OK `id` was spent already, on another message.
    pub(crate) fn spend_elsewhere(&self, id: &str) {
        self.spent.lock().unwrap().push(id.to_owned());
    }

    /// The OKs spent, in order.
    pub(crate) fn spent(&self) -> Vec<String> {
        self.spent.lock().unwrap().clone()
    }

    /// `chat` is on the owner's standing list.
    pub(crate) fn stand(&self, chat: &str) {
        self.standing.lock().unwrap().push(chat.to_owned());
    }

    /// What went out (or was tried): task, text, channel.
    pub(crate) fn sent(&self) -> Vec<(TaskId, String, String)> {
        self.sent.lock().unwrap().clone()
    }

    /// The cards closed by an OK given another way: task, question.
    pub(crate) fn closed(&self) -> Vec<(TaskId, String)> {
        self.closed.lock().unwrap().clone()
    }
}

impl SendOks for FakeOks {
    fn tapped(&self, task: TaskId, asked: Timestamp) -> BoxFuture<'_, bool> {
        let tapped = self.tapped.lock().unwrap().contains(&(task, asked));
        Box::pin(async move { tapped })
    }

    fn chat_ok<'a>(
        &'a self,
        _: Option<RoomId>,
        text: &'a str,
        _: &'a str,
        _: bool,
        _: Option<Timestamp>,
    ) -> BoxFuture<'a, Option<ChatOk>> {
        let ok = self
            .chat_oks
            .lock()
            .unwrap()
            .iter()
            .find(|(said, _)| said == text)
            .map(|(_, ok)| ok.clone());
        Box::pin(async move { ok })
    }

    fn spend<'a>(&'a self, ok: &'a ChatOk) -> BoxFuture<'a, bool> {
        let mut spent = self.spent.lock().unwrap();
        let fresh = !spent.contains(&ok.id);
        if fresh {
            spent.push(ok.id.clone());
        }
        Box::pin(async move { fresh })
    }

    fn text_ok<'a>(&'a self, task: TaskId, text: &'a str) -> BoxFuture<'a, Option<TextOk>> {
        let ok = self
            .text_oks
            .lock()
            .unwrap()
            .iter()
            .find(|(t, said, _)| *t == task && said == text)
            .map(|(_, _, ok)| ok.clone());
        Box::pin(async move { ok })
    }

    fn standing_ok<'a>(
        &'a self,
        chats: &'a [String],
        one_to_one: bool,
    ) -> BoxFuture<'a, Option<String>> {
        let standing = self.standing.lock().unwrap();
        let free = (one_to_one && chats.iter().any(|c| standing.contains(c)))
            .then(|| "the owner's standing OK for this chat".to_owned());
        Box::pin(async move { free })
    }

    fn sent<'a>(&'a self, task: TaskId, text: &'a str, via: &'a str) -> BoxFuture<'a, ()> {
        self.sent
            .lock()
            .unwrap()
            .push((task, text.to_owned(), via.to_owned()));
        Box::pin(async {})
    }

    fn close_card<'a>(&'a self, task: TaskId, question: &'a str) -> BoxFuture<'a, ()> {
        self.closed
            .lock()
            .unwrap()
            .push((task, question.to_owned()));
        Box::pin(async {})
    }
}

/// An app wired to every messaging fake.
pub(crate) struct Kit {
    /// The app.
    pub(crate) app: TestApp,
    /// Slack.
    pub(crate) slack: Arc<FakeSlack>,
    /// Google.
    pub(crate) google: Arc<FakeGoogle>,
    /// The Mac helper's message routes.
    pub(crate) mac: Arc<FakeMacMessages>,
    /// Speech to text.
    pub(crate) transcriber: Arc<FakeTranscriber>,
    /// The WhatsApp daemon.
    pub(crate) daemon: Arc<FakeWaDaemon>,
    /// WhatsApp Web.
    pub(crate) web: Arc<FakeWaWeb>,
    /// The tasks.
    pub(crate) tasks: Arc<FakeTasks>,
    /// The guard rails' other OKs.
    pub(crate) oks: Arc<FakeOks>,
    /// The secrets directory.
    pub(crate) secrets: PathBuf,
}

impl Kit {
    /// A new app over fresh fakes.
    pub(crate) async fn new() -> Self {
        Self::with(|_| {}).await
    }

    /// A new app over fresh fakes, with no tasks at all ([`NoTasks`]) and the other OKs and
    /// WhatsApp Web the server's defaults ([`MessagingDeps::default`]: only the card's OK, no
    /// browser). The kit's fakes of those three are then not wired in.
    pub(crate) async fn with_default_guard() -> Self {
        Self::with(|deps| {
            let defaults = MessagingDeps::default();
            deps.tasks = Some(Arc::new(NoTasks));
            deps.oks = defaults.oks;
            deps.wa_web = defaults.wa_web;
        })
        .await
    }

    /// A new app over fresh fakes, its messaging dependencies then changed by `adjust`.
    pub(crate) async fn with(adjust: impl FnOnce(&mut MessagingDeps)) -> Self {
        Self::with_auth(AuthMode::Enforce, adjust).await
    }

    /// [`Self::with`], in auth mode `mode`.
    pub(crate) async fn with_auth(mode: AuthMode, adjust: impl FnOnce(&mut MessagingDeps)) -> Self {
        let slack = Arc::new(FakeSlack::default());
        let google = Arc::new(FakeGoogle::default());
        let mac = Arc::new(FakeMacMessages::default());
        let transcriber = Arc::new(FakeTranscriber::default());
        let daemon = Arc::new(FakeWaDaemon::default());
        let web = Arc::new(FakeWaWeb::default());
        let tasks = Arc::new(FakeTasks::default());
        let oks = Arc::new(FakeOks::default());
        let mut secrets = PathBuf::new();
        let app = TestApp::with_dependencies(mode, |mut deps, _| {
            secrets.clone_from(&deps.secrets_dir);
            "https://box.example".clone_into(&mut deps.public_url);
            deps.messaging = MessagingDeps {
                slack: Arc::clone(&slack) as _,
                google: Arc::clone(&google) as _,
                transcriber: Arc::clone(&transcriber) as _,
                mac: Arc::clone(&mac) as _,
                wa_daemon: Arc::clone(&daemon) as _,
                wa_web: Arc::clone(&web) as _,
                tasks: Some(Arc::clone(&tasks) as _),
                oks: Arc::clone(&oks) as _,
                wake_name: "nova".to_owned(),
                owner_phone: Some("+1 555 010 0100".to_owned()),
                assistant_phone: None,
                voice_retry_pause: Duration::ZERO,
                ..MessagingDeps::default()
            };
            adjust(&mut deps.messaging);
            deps
        })
        .await;
        Self {
            app,
            slack,
            google,
            mac,
            transcriber,
            daemon,
            web,
            tasks,
            oks,
            secrets,
        }
    }

    /// Saves the secret `name`.
    pub(crate) fn secret(&self, name: &str, value: &str) {
        std::fs::create_dir_all(&self.secrets).unwrap();
        std::fs::write(self.secrets.join(name), value).unwrap();
    }

    /// Whether the secret `name` is saved, and what it holds.
    pub(crate) fn read_secret(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.secrets.join(name)).ok()
    }

    /// What the app's clock reads.
    pub(crate) fn now(&self) -> Timestamp {
        self.app.clock_now()
    }

    /// Moves the app's clock on by `seconds`.
    pub(crate) fn later(&self, seconds: i64) {
        self.app.clock.advance(SignedDuration::from_secs(seconds));
    }
}
