//! The coding-thread app: another coding tool on the person's machines, whose threads the
//! server follows. Each machine's app is reached over its MCP endpoint (JSON-RPC over HTTP,
//! the streamable transport); the server lists its threads, reads a thread's items, and sends a
//! thread a message as its own turn.
//!
//! [`CodingThreadApp`] is what the server needs of it, so tests use a fake and a server with no
//! machines configured uses [`NoCodingApp`]. [`McpCodingApp`] is the real one, configured with
//! `--coding-app NAME=URL` for each machine and `--coding-app-tokens`, a file of `NAME=TOKEN`
//! lines read at every call (so a new token works without a restart). It is three parts, each
//! hiding one decision:
//!
//! - `transport`: the MCP client (the session, the protocol header, JSON or event-stream
//!   answers), on the server's one HTTP client;
//! - `tokens`: the tokens file;
//! - `answers`: the app's answers, read into typed values, so a field it renames is an error
//!   rather than an empty value.
//!
//! The app's tool names carry its own name; they are its spelling, kept here only
//! ([`tool`]).

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use emberhut_core::thread::ActivityItem;
use futures_util::future::BoxFuture;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::config::CodingAppMachine;
use crate::http::ClientError;

mod answers;
mod tokens;
mod transport;

use answers::{ProjectList, ReadAnswer, SendAnswer, ThreadList};
use transport::Transport;

/// The coding-thread app's MCP tools the server calls, as the app spells them.
pub mod tool {
    /// Its projects: `{ limit }` → `{ projects: [{ id, title }] }`.
    pub const PROJECT_LIST: &str = "t3_project_list";
    /// A project's threads: `{ projectId, limit, settled }` → `{ threads: [{ threadId, title,
    /// status, updatedAt, parentThreadId }] }`.
    pub const THREAD_LIST: &str = "t3_thread_list";
    /// A thread and some of its items: `{ threadId, limit, afterPosition?, view?,
    /// maxCharsPerItem }` → `{ thread: { title, status, updatedAt, itemCount }, items }`.
    pub const THREAD_READ: &str = "t3_thread_read";
    /// A message to a thread: `{ threadId, message, mode }` → `{ runId?, queued?, mode? }`.
    pub const THREAD_SEND: &str = "t3_thread_send";
}

/// The most projects, and threads per project, one listing asks for.
const LIST_LIMIT: u64 = 100;

/// Why the coding-thread app could not be asked, or refused. Its words are the ones the app
/// shows ([`emberhut_api::threads::ACTIVITY`] answers them with 502).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CodingAppError {
    /// No coding-thread app is configured on that machine.
    #[error("No coding-thread app on that machine.")]
    UnknownMachine,
    /// The app could not be reached, or answered with something the server can't read.
    #[error("{machine}: {reason}")]
    Unreachable {
        /// The machine.
        machine: String,
        /// What went wrong.
        reason: String,
    },
    /// The app answered with an error of its own (an unknown thread, a refused send).
    #[error("{machine}: {reason}")]
    Refused {
        /// The machine.
        machine: String,
        /// The app's reason, as it gave it.
        reason: String,
    },
}

/// A thread in a project's listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedThread {
    /// Its id.
    pub id: String,
    /// Its title.
    pub title: String,
    /// Its project's title.
    pub project: String,
    /// Its status's word.
    pub status: String,
    /// When it last changed, as the app wrote it.
    pub updated_at: String,
    /// The thread that started it, for a helper agent's.
    pub parent: Option<String>,
}

/// What to read of a thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadAsk {
    /// The most items.
    pub limit: usize,
    /// Read the items after this position (the first ones when `None`).
    pub after: Option<u64>,
    /// Ask for the activity view (every step), not only the messages.
    pub activity: bool,
    /// The longest text of an item, in characters.
    pub max_chars: usize,
}

/// A thread as read: what it is, and the items asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThreadPage {
    /// Its title, if the app gave one.
    pub title: Option<String>,
    /// Its status's word, if the app gave one.
    pub status: Option<String>,
    /// When it last changed, as the app wrote it.
    pub updated_at: Option<String>,
    /// How many items it has in all.
    pub item_count: u64,
    /// The items asked for, oldest first.
    pub items: Vec<ActivityItem>,
}

/// What the app said to a message sent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sent {
    /// The run the message became, when the app said.
    pub run_id: Option<String>,
    /// The message waits behind a turn already running.
    pub queued: bool,
}

/// What the server needs of the coding-thread app.
pub trait CodingThreadApp: fmt::Debug + Send + Sync {
    /// The machines it is on, by the names the server gives them.
    fn machines(&self) -> Vec<String>;

    /// Every thread of every project on `machine` (each project's newest 100).
    fn list<'a>(
        &'a self,
        machine: &'a str,
    ) -> BoxFuture<'a, Result<Vec<ListedThread>, CodingAppError>>;

    /// The thread `thread` on `machine`, and the items `ask` names.
    fn read<'a>(
        &'a self,
        machine: &'a str,
        thread: &'a str,
        ask: ReadAsk,
    ) -> BoxFuture<'a, Result<ThreadPage, CodingAppError>>;

    /// Sends `message` to `thread` on `machine` as a turn of its own, queued behind a running
    /// one.
    fn send<'a>(
        &'a self,
        machine: &'a str,
        thread: &'a str,
        message: &'a str,
    ) -> BoxFuture<'a, Result<Sent, CodingAppError>>;
}

/// No coding-thread app: no machines, and every call [`CodingAppError::UnknownMachine`].
#[derive(Clone, Copy, Debug, Default)]
pub struct NoCodingApp;

impl CodingThreadApp for NoCodingApp {
    fn machines(&self) -> Vec<String> {
        Vec::new()
    }

    fn list<'a>(
        &'a self,
        _machine: &'a str,
    ) -> BoxFuture<'a, Result<Vec<ListedThread>, CodingAppError>> {
        Box::pin(async { Err(CodingAppError::UnknownMachine) })
    }

    fn read<'a>(
        &'a self,
        _machine: &'a str,
        _thread: &'a str,
        _ask: ReadAsk,
    ) -> BoxFuture<'a, Result<ThreadPage, CodingAppError>> {
        Box::pin(async { Err(CodingAppError::UnknownMachine) })
    }

    fn send<'a>(
        &'a self,
        _machine: &'a str,
        _thread: &'a str,
        _message: &'a str,
    ) -> BoxFuture<'a, Result<Sent, CodingAppError>> {
        Box::pin(async { Err(CodingAppError::UnknownMachine) })
    }
}

/// Why the coding-thread app's client could not be made.
#[derive(Debug, thiserror::Error)]
#[error("the coding-thread app's HTTP client could not be made")]
pub struct CodingAppSetupError(#[source] ClientError);

/// The coding-thread app on each configured machine, over its MCP endpoint.
#[derive(Clone, Debug)]
pub struct McpCodingApp {
    inner: Arc<McpInner>,
}

#[derive(Debug)]
struct McpInner {
    machines: Vec<CodingAppMachine>,
    tokens: Option<PathBuf>,
    transport: Transport,
}

impl McpCodingApp {
    /// The app on `machines`, with the bearer tokens in the file `tokens` (`NAME=TOKEN` lines;
    /// none: no token sent).
    ///
    /// # Errors
    ///
    /// [`CodingAppSetupError`] when its HTTP client cannot be made.
    pub fn new(
        machines: Vec<CodingAppMachine>,
        tokens: Option<PathBuf>,
    ) -> Result<Self, CodingAppSetupError> {
        Ok(Self {
            inner: Arc::new(McpInner {
                machines,
                tokens,
                transport: Transport::new().map_err(CodingAppSetupError)?,
            }),
        })
    }

    /// Calls the tool `name` with `arguments` on `machine`, and reads its result as a `T`.
    async fn call<T: DeserializeOwned>(
        &self,
        machine: &str,
        name: &'static str,
        arguments: Value,
    ) -> Result<T, CodingAppError> {
        let url = self
            .inner
            .machines
            .iter()
            .find(|m| m.name == machine)
            .map(|m| m.url.as_str())
            .ok_or(CodingAppError::UnknownMachine)?;
        let token = match &self.inner.tokens {
            Some(path) => tokens::read(path, machine).await?,
            None => None,
        };
        let body = self
            .inner
            .transport
            .call(machine, url, token.as_deref(), name, &arguments)
            .await?;
        answers::tool_result(machine, &body)
    }
}

impl CodingThreadApp for McpCodingApp {
    fn machines(&self) -> Vec<String> {
        self.inner.machines.iter().map(|m| m.name.clone()).collect()
    }

    fn list<'a>(
        &'a self,
        machine: &'a str,
    ) -> BoxFuture<'a, Result<Vec<ListedThread>, CodingAppError>> {
        Box::pin(async move {
            let listed: ProjectList = self
                .call(machine, tool::PROJECT_LIST, json!({ "limit": LIST_LIMIT }))
                .await?;
            let mut all = Vec::new();
            for project in listed.projects {
                let ask = json!({
                    "projectId": project.id,
                    "limit": LIST_LIMIT,
                    "settled": false,
                });
                let threads: ThreadList = self.call(machine, tool::THREAD_LIST, ask).await?;
                let title = project.title.unwrap_or_default();
                all.extend(threads.into_listed(&title));
            }
            Ok(all)
        })
    }

    fn read<'a>(
        &'a self,
        machine: &'a str,
        thread: &'a str,
        ask: ReadAsk,
    ) -> BoxFuture<'a, Result<ThreadPage, CodingAppError>> {
        Box::pin(async move {
            let mut arguments = Map::new();
            arguments.insert("threadId".to_owned(), json!(thread));
            arguments.insert("limit".to_owned(), json!(ask.limit));
            arguments.insert("maxCharsPerItem".to_owned(), json!(ask.max_chars));
            if let Some(after) = ask.after {
                arguments.insert("afterPosition".to_owned(), json!(after));
            }
            if ask.activity {
                arguments.insert("view".to_owned(), json!("activity"));
            }
            let answer: ReadAnswer = self
                .call(machine, tool::THREAD_READ, Value::Object(arguments))
                .await?;
            Ok(answer.into_page())
        })
    }

    fn send<'a>(
        &'a self,
        machine: &'a str,
        thread: &'a str,
        message: &'a str,
    ) -> BoxFuture<'a, Result<Sent, CodingAppError>> {
        Box::pin(async move {
            let ask = json!({ "threadId": thread, "message": message, "mode": "queue" });
            let answer: SendAnswer = self.call(machine, tool::THREAD_SEND, ask).await?;
            Ok(answer.into_sent())
        })
    }
}
