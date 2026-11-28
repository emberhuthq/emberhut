//! The guard rails around what the assistant and its workers do, over `emberhut_core::guard`'s
//! rules: where the owner's OKs are kept (the settings), who is asked, and what a step that
//! is held back is told.
//!
//! - [`grants`]: one job, one OK: the job's grant, the OK of an exact text for any channel,
//!   a post's OK, the cards about one step only, the standing OK for a 1:1 chat.
//! - [`chat_ok`]: the owner's go in the chat on an exact text, and their yes about a task.
//! - [`outward`]: a push, merge or deploy held until the owner's words cover it.
//! - [`go_ahead`]: the Go ahead cards of a job's final steps (the browser's and the Mac's).
//! - [`ask_guard`]: an ask the owner's words already cover comes back "do it".
//! - [`agents`]: a task made in a configured agent's room, decided by the agent's policies.
//! - [`send_tap`]: the owner's Send it on a message's card sends it there and then, and the
//!   [`SendOks`](crate::services::messaging::hooks::SendOks) messaging asks.
//!
//! Outside services are traits: the judge model ([`Judge`], none by default) and a folder's
//! git remotes ([`RepoRemotes`], `git remote -v`).

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use emberhut_core::task::{Task, TaskId};
use emberhut_core::{Clock, RoomId};
use emberhut_db::{
    Database, DbError, EventStore, MessageStore, RoomStore, SettingStore, TaskStore, WorkerStore,
};
use futures_util::future::BoxFuture;
use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::convert::timestamp_to_api;
use crate::services::messaging::Messaging;
use crate::services::tasks::{TaskError, TaskService};

mod agent_files;
pub mod agents;
pub mod ask_guard;
pub mod chat_ok;
pub mod go_ahead;
pub mod grants;
pub mod outward;
pub mod send_tap;

pub use ask_guard::{Judge, NoJudge};
pub use outward::{GitRemotes, RepoRemotes};

/// Why a guard rail could not answer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GuardError {
    /// The database failed.
    #[error("the guard rails could not read or write their OKs")]
    Db(#[from] DbError),
    /// The tasks refused or failed.
    #[error(transparent)]
    Task(#[from] TaskError),
    /// An agent's approval could not be asked for or used.
    #[error(transparent)]
    Approvals(#[from] crate::services::approvals::ApprovalError),
}

/// How the guard rails are set up.
#[derive(Clone, Debug, Default)]
pub struct GuardConfig {
    /// The repositories whose push and merge always go through (`owner/name`, or `*/name` for
    /// a name under any owner); none by default.
    pub always_merge: Vec<String>,
    /// A JSON file of the people the assistant may message without an OK per text
    /// (`{ "Name": ["slack:team:id", "wa:123"] }`), read at every send; none: nobody.
    pub send_without_ok: Option<PathBuf>,
    /// The server user's home (`~` and `$HOME` in a command line).
    pub home: Option<PathBuf>,
}

/// The guard rails. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Guard {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    settings: SettingStore,
    messages: MessageStore,
    rooms: RoomStore,
    store: TaskStore,
    workers: WorkerStore,
    events: EventStore,
    tasks: TaskService,
    clock: Arc<dyn Clock>,
    zone: TimeZone,
    config: GuardConfig,
    judge: Arc<dyn Judge>,
    remotes: Arc<dyn RepoRemotes>,
    /// Messaging, for a card's Send it, once it is built (it is built after the guard).
    messaging: OnceLock<Messaging>,
    /// The owner's standing rules, for the ask guard, once set.
    learning: OnceLock<crate::services::learning::LearningService>,
    /// The facts the owner set, for the ask guard, once set.
    facts: OnceLock<crate::services::facts::FactService>,
    /// The approvals and agents, for the tasks made in an agent's room, once set.
    agents: OnceLock<agents::AgentHook>,
    /// The cards whose send answer the owner just tapped: `(task, asked)`, read once by the
    /// sending tool's own guard.
    tapped: Mutex<Vec<(i64, Timestamp)>>,
    /// The assistant's last verdicts on commands the outward rules couldn't be sure of:
    /// (folder and command, whether it only reads).
    read_only: Mutex<Vec<(String, bool)>>,
}

/// What the guard rails are built from.
#[derive(Clone, Debug)]
pub struct GuardParts {
    /// The database.
    pub db: Database,
    /// The tasks.
    pub tasks: TaskService,
    /// What time it is.
    pub clock: Arc<dyn Clock>,
    /// The server's time zone (the times of day an OK names).
    pub zone: TimeZone,
    /// The setup.
    pub config: GuardConfig,
    /// The ask guard's judge.
    pub judge: Arc<dyn Judge>,
    /// A folder's git remotes.
    pub remotes: Arc<dyn RepoRemotes>,
}

impl Guard {
    /// The guard rails over `parts`.
    #[must_use]
    pub fn new(parts: GuardParts) -> Self {
        let GuardParts {
            db,
            tasks,
            clock,
            zone,
            config,
            judge,
            remotes,
        } = parts;
        Self {
            inner: Arc::new(Inner {
                settings: db.settings(),
                messages: db.messages(),
                rooms: db.rooms(),
                store: db.tasks(),
                workers: db.workers(),
                events: db.events(),
                tasks,
                clock,
                zone,
                config,
                judge,
                remotes,
                messaging: OnceLock::new(),
                learning: OnceLock::new(),
                facts: OnceLock::new(),
                agents: OnceLock::new(),
                tapped: Mutex::new(Vec::new()),
                read_only: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Hands the guard rails messaging, for the cards' Send it.
    pub fn use_messaging(&self, messaging: Messaging) {
        if self.inner.messaging.set(messaging).is_err() {
            tracing::warn!("the guard rails have their messaging already");
        }
    }

    /// Hands the ask guard the owner's standing rules and facts, which the judge reads.
    pub fn use_memory(
        &self,
        learning: crate::services::learning::LearningService,
        facts: crate::services::facts::FactService,
    ) {
        if self.inner.learning.set(learning).is_err() || self.inner.facts.set(facts).is_err() {
            tracing::warn!("the guard rails have their memory already");
        }
    }

    fn now(&self) -> Timestamp {
        self.inner.clock.now()
    }

    /// `at` as the settings write an instant (the wire's ISO form).
    fn iso(at: Timestamp) -> String {
        timestamp_to_api(at).as_str().to_owned()
    }

    /// An ISO instant read back.
    fn parse(at: &str) -> Option<Timestamp> {
        at.parse().ok()
    }

    /// `at`'s time of day on the server's clock, `16:34`.
    fn hhmm(&self, at: Timestamp) -> String {
        at.to_zoned_datetime(self.inner.zone.clone())
            .strftime("%H:%M")
            .to_string()
    }

    /// The home folder, as a command line's `~` reads it.
    fn home(&self) -> String {
        self.inner
            .config
            .home
            .as_ref()
            .map_or_else(|| "/".to_owned(), |home| home.display().to_string())
    }

    async fn task(&self, id: TaskId) -> Result<Option<Task>, DbError> {
        self.inner.store.get(id).await
    }

    /// The room a task's asks are told in: Main until projects have rooms of their own.
    fn room_of(_task: TaskId) -> RoomId {
        RoomId::MAIN
    }

    /// A room's name, as a line names it.
    async fn room_name(&self, room: RoomId) -> String {
        if room == RoomId::MAIN {
            return "Main".to_owned();
        }
        match self.inner.rooms.get(room).await {
            Ok(Some(room)) => room.name,
            _ => format!("room {room}"),
        }
    }

    fn take_tap(&self, task: i64, asked: Timestamp) -> bool {
        let mut tapped = self
            .inner
            .tapped
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let before = tapped.len();
        tapped.retain(|(t, at)| !(*t == task && *at == asked));
        tapped.len() != before
    }

    fn note_tap(&self, task: i64, asked: Timestamp) {
        self.inner
            .tapped
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((task, asked));
    }
}

/// A boxed future, for the traits here.
pub type Answer<'a, T> = BoxFuture<'a, T>;
