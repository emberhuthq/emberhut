//! What the guard rails' tests share: an app whose judge and git remotes the test gives, the
//! owner's and the assistant's lines in a room, and tasks with the owner's request.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure here fails the test that called them"
)]

use std::sync::{Arc, Mutex};

use emberhut_core::message::{Author, NewMessage};
use emberhut_core::task::{Owner, TaskId, Who};
use emberhut_core::{MessageId, RoomId};
use emberhut_db::{Database, NewTask};
use emberhut_server::app::{Dependencies, GuardDeps};
use emberhut_server::config::AuthMode;
use emberhut_server::services::guard::{GuardConfig, Judge, RepoRemotes};
use futures_util::future::BoxFuture;

use crate::common::TestApp;

/// A judge that answers what the test set, and keeps the prompts it was given.
#[derive(Debug, Default)]
pub(crate) struct FakeJudge {
    pub(crate) answer: Mutex<Option<String>>,
    pub(crate) prompts: Mutex<Vec<String>>,
}

impl Judge for FakeJudge {
    fn judge<'a>(&'a self, _: &'a str, prompt: &'a str) -> BoxFuture<'a, Option<String>> {
        self.prompts.lock().unwrap().push(prompt.to_owned());
        let answer = self.answer.lock().unwrap().clone();
        Box::pin(async move { answer })
    }
}

/// Folders and their remotes, as the test sets them.
#[derive(Debug, Default)]
pub(crate) struct FakeRemotes(pub(crate) Vec<(String, String)>);

impl RepoRemotes for FakeRemotes {
    fn remotes(&self, dir: &str) -> Option<Vec<String>> {
        let found: Vec<String> = self
            .0
            .iter()
            .filter(|(d, _)| d == dir)
            .map(|(_, r)| r.clone())
            .collect();
        (!found.is_empty()).then_some(found)
    }
}

/// An app with `judge`, `remotes` and the guard rails' `config`.
pub(crate) async fn guard_app(
    judge: Arc<dyn Judge>,
    remotes: Arc<dyn RepoRemotes>,
    config: GuardConfig,
) -> TestApp {
    TestApp::with_dependencies(AuthMode::Enforce, move |deps, _| Dependencies {
        guard: GuardDeps {
            config,
            judge,
            remotes,
        },
        ..deps
    })
    .await
}

/// A second connection to the app's database.
pub(crate) async fn db(app: &TestApp) -> Database {
    Database::connect(&app.url).await.unwrap()
}

/// Says `text` in `room`: the owner's (`mine`) or the assistant's.
pub(crate) async fn say(app: &TestApp, room: RoomId, mine: bool, text: &str) -> MessageId {
    let message = NewMessage {
        room,
        author: if mine {
            Author::User
        } else {
            Author::Assistant
        },
        text: text.to_owned(),
        reply_to: None,
    };
    db(app)
        .await
        .messages()
        .append(message, app.clock_now())
        .await
        .unwrap()
        .id
}

/// An assistant task; with `request`, the owner's message it was made from.
pub(crate) async fn task(app: &TestApp, title: &str, request: Option<MessageId>) -> TaskId {
    let new = NewTask {
        title: title.to_owned(),
        body: Some(format!("Do {title}.")),
        owner: Some(Owner::Assistant),
        ..NewTask::default()
    };
    let id = app
        .state
        .tasks()
        .create(new, Who::Assistant)
        .await
        .unwrap()
        .id;
    if let Some(msg) = request {
        let at = app
            .clock_now()
            .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string();
        let json = serde_json::json!({ "at": at, "msg": msg.get(), "room": 1 }).to_string();
        db(app)
            .await
            .workers()
            .set_requested(id, &json)
            .await
            .unwrap();
    }
    id
}
