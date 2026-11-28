//! Runner sessions: what the runners report of their sessions, the sessions list, a session's
//! transcript and live stream, the person's replies, stops and settings, their own new sessions
//! and the folder and branch pickers, and each runner's accounts and health.
//!
//! A runner's session is a coding thread (`threads`, ADR 0013) on the runner's machine: every
//! event the runner sends (`POST /api/runner/events`) carries the session and its turn as they
//! are now, so taking one twice changes nothing, and [`RunnerService::event`] records the
//! thread's new look through [`ThreadService::record`] (a followed session's end is news for the
//! assistant). The transcript, the live stream and the pickers are the runner's own answers,
//! passed through with what the server knows added. See [`crate::services::runner_api`] for how
//! runners are reached.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use emberhut_api as api;
use emberhut_api::chat::ChatFile;
use emberhut_core::account::AccountUse;
use emberhut_core::runner::{LAST_LENGTH, QUESTION_LENGTH, asked, clip, status_line};
use emberhut_core::thread::{CodingThread, ThreadStatus};
use emberhut_core::{Clock, RoomId};
use emberhut_db::{DbError, ThreadStore, UsageStore};
use jiff::Timestamp;
use serde_json::{Map, Value, json};
use subtle::ConstantTimeEq as _;
use tokio_util::sync::CancellationToken;

use crate::convert::{room_to_api, timestamp_to_api};
use crate::services::activity::ActivityLog;
use crate::services::runner_api::{ByteStream, RunnerApi, RunnerError, Verb, by};
use crate::services::tasks::{SessionTurn, TaskService};
use crate::services::threads::ThreadService;
use crate::services::transfer::{TransferError, TransferService};

/// The runner a request names when it names none.
pub const DEFAULT_RUNNER: &str = "runner";

/// How many sessions the list has at most.
pub const LIST_LENGTH: usize = 50;

/// The fewest sessions the state lists (more when more are running).
pub const STATE_SESSIONS: usize = 30;

/// How many of the person's own sessions the state lists.
pub const STATE_MINE: usize = 40;

/// How often each runner's health and accounts are looked at.
pub const TICK_EVERY: Duration = Duration::from_secs(60);

/// The text a turn carries when the person sent only files.
pub const SEE_THE_FILES: &str = "(see the files)";

/// Why a runner session operation failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SessionError {
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
    /// The runner failed, or there is no such runner.
    #[error(transparent)]
    Runner(#[from] RunnerError),
    /// A file couldn't be sent with the turn.
    #[error(transparent)]
    Transfer(#[from] TransferError),
    /// The request can't be done as asked (nothing to send, nothing to start with).
    #[error("{0}")]
    Refused(String),
    /// The runner's answer isn't what it should be.
    #[error("{0}")]
    Malformed(String),
}

/// A runner session's id, checked: `r-` and 1 to [`SessionId::MAX_HEX`] lowercase hex digits,
/// the form the session routes take. Only such an id is put into a path on a runner: one with
/// `/`, `.`, `?`, `#` or `%` in it would reach another of the runner's endpoints, with the
/// runner's token, so every operation that calls a runner about a session takes this type.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionId(String);

impl SessionId {
    /// The most hex digits an id has after `r-`.
    pub const MAX_HEX: usize = 32;

    /// `id` as a session's id.
    ///
    /// # Errors
    ///
    /// [`SessionError::Refused`] when it isn't `r-` and 1 to [`SessionId::MAX_HEX`] lowercase
    /// hex digits.
    pub fn parse(id: &str) -> Result<Self, SessionError> {
        let fits = id.strip_prefix("r-").is_some_and(|hex| {
            (1..=Self::MAX_HEX).contains(&hex.len()) && hex.bytes().all(is_lower_hex)
        });
        if fits {
            Ok(Self(id.to_owned()))
        } else {
            Err(SessionError::Refused(
                "not a runner session's id (r- and its hex digits)".to_owned(),
            ))
        }
    }

    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The person's own new session ([`RunnerService::start_direct`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DirectSession {
    /// The runner machine.
    pub machine: String,
    /// The folder there.
    pub cwd: String,
    /// The first message.
    pub message: String,
    /// Files sent with it.
    pub files: Vec<ChatFile>,
    /// The account to run on.
    pub account: Option<String>,
    /// The model.
    pub model: Option<String>,
    /// The reasoning effort.
    pub effort: Option<String>,
    /// A worktree of its own, passed to the runner as is.
    pub worktree: Option<Value>,
    /// Its title.
    pub title: Option<String>,
}

/// A session the assistant starts for a piece of work (the `thread_start` tool).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkSession {
    /// The runner machine.
    pub machine: String,
    /// The folder there.
    pub cwd: String,
    /// The project it is for, by name.
    pub project: Option<String>,
    /// Its title.
    pub title: String,
    /// The brief.
    pub message: String,
    /// The account to run on (the runner picks when none).
    pub account: Option<String>,
    /// The model.
    pub model: Option<String>,
    /// The reasoning effort (default `high`).
    pub effort: Option<String>,
    /// A worktree of its own.
    pub worktree: Option<Value>,
    /// What the session's meta carries (the task it is for).
    pub meta: Map<String, Value>,
}

/// What a runner answered when it opened a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opened {
    /// The session's id.
    pub id: String,
    /// Its first turn's id.
    pub turn: Option<String>,
    /// The account it runs on.
    pub account: Option<String>,
    /// The folder it works in.
    pub cwd: Option<String>,
    /// Its branch.
    pub branch: Option<String>,
    /// Its model.
    pub model: Option<String>,
}

/// The sessions list's filter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    /// Only the sessions linked to this task.
    pub task: Option<i64>,
    /// Only this room's (Main: every room's).
    pub room: Option<RoomId>,
    /// Only the person's own.
    pub direct: bool,
}

/// What the server last saw of a runner: when it last answered or reported, and its error at
/// the last look.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunnerLook {
    /// When it last answered or reported.
    pub at: Option<Timestamp>,
    /// Its error at the last look (`None`: it answered).
    pub error: Option<String>,
}

/// The parts [`RunnerService`] is built from.
#[derive(Clone, Debug)]
pub struct RunnerParts {
    /// The runners.
    pub api: Arc<dyn RunnerApi>,
    /// Where threads are recorded.
    pub threads: ThreadService,
    /// The threads, to read.
    pub store: ThreadStore,
    /// Each runner's accounts.
    pub usage: UsageStore,
    /// The activity log.
    pub activity: ActivityLog,
    /// Files to and from runners.
    pub transfer: TransferService,
    /// What time it is.
    pub clock: Arc<dyn Clock>,
    /// The tasks the sessions work on: their links and runs.
    pub tasks: TaskService,
}

/// The runners' sessions, as the server sees and drives them.
#[derive(Clone, Debug)]
pub struct RunnerService {
    parts: RunnerParts,
    looks: Arc<Mutex<BTreeMap<String, RunnerLook>>>,
}

impl RunnerService {
    /// The service over `parts`.
    #[must_use]
    pub fn new(parts: RunnerParts) -> Self {
        Self {
            parts,
            looks: Arc::default(),
        }
    }

    /// The runners.
    #[must_use]
    pub fn api(&self) -> &dyn RunnerApi {
        self.parts.api.as_ref()
    }

    /// The configured runners' names.
    #[must_use]
    pub fn machines(&self) -> Vec<String> {
        self.parts.api.machines()
    }

    /// Whether `machine` is a configured runner.
    #[must_use]
    pub fn is_runner(&self, machine: &str) -> bool {
        self.machines().iter().any(|known| known == machine)
    }

    /// `machine` in words.
    #[must_use]
    pub fn place(&self, machine: &str) -> String {
        self.parts.api.place(machine)
    }

    /// The runner whose token is `token`, if one is: the tokens are read once, and each is
    /// compared in constant time.
    pub async fn runner_of_token(&self, token: &str) -> Option<String> {
        let tokens = self.parts.api.tokens().await;
        self.machines().into_iter().find(|machine| {
            tokens
                .get(machine)
                .is_some_and(|known| bool::from(known.as_bytes().ct_eq(token.as_bytes())))
        })
    }

    /// What the server last saw of each runner.
    #[must_use]
    pub fn looks(&self) -> BTreeMap<String, RunnerLook> {
        self.looks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Notes that `machine` was heard from now.
    pub fn heard(&self, machine: &str) {
        let now = self.parts.clock.now();
        self.looks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(machine.to_owned())
            .or_default()
            .at = Some(now);
    }

    /// Each runner's error at its last look (`None`: it answered), for the state's `machines`.
    #[must_use]
    pub fn errors(&self) -> BTreeMap<String, Option<String>> {
        let looks = self.looks();
        self.machines()
            .into_iter()
            .filter_map(|machine| {
                let look = looks.get(&machine)?;
                Some((machine, look.error.clone()))
            })
            .collect()
    }

    /// Every thread on the runners' machines.
    async fn runner_threads(&self) -> Result<Vec<CodingThread>, DbError> {
        let mut all = Vec::new();
        for machine in self.machines() {
            all.extend(self.parts.store.on_machine(&machine).await?);
        }
        Ok(all)
    }

    /// The session `id`'s thread, on whichever runner it is.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn session(&self, id: &str) -> Result<Option<CodingThread>, DbError> {
        for machine in self.machines() {
            if let Some(thread) = self.parts.store.get(&machine, id).await? {
                return Ok(Some(thread));
            }
        }
        Ok(None)
    }

    /// The runner the session `id` is on: the one that reported it, else the first runner.
    ///
    /// # Errors
    ///
    /// [`SessionError::Runner`] with [`RunnerError::Unknown`] when no runner is configured.
    pub async fn machine_of(&self, id: &SessionId) -> Result<String, SessionError> {
        if let Some(thread) = self.session(id.as_str()).await? {
            return Ok(thread.machine);
        }
        self.machines()
            .into_iter()
            .next()
            .ok_or_else(|| RunnerError::Unknown(DEFAULT_RUNNER.to_owned()).into())
    }

    /// The name of the account `id` on `machine`: its runner's name for it, else another
    /// runner's, else the id.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn account_name(
        &self,
        machine: &str,
        id: Option<&str>,
    ) -> Result<Option<String>, DbError> {
        let Some(id) = id.filter(|id| !id.is_empty()) else {
            return Ok(None);
        };
        let accounts = self.parts.usage.all().await?;
        Ok(Some(name_in(&accounts, machine, id)))
    }

    /// The sessions list: the runners' sessions `filter` keeps, newest first, [`LIST_LENGTH`] at
    /// most.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn sessions(&self, filter: Filter) -> Result<Vec<api::work::RunnerSession>, DbError> {
        let plans = self.parts.tasks.keeps_plans();
        let linked: Option<Vec<String>> = match filter.task.filter(|task| *task != 0) {
            // Sessions are linked to tasks only on a server that keeps plans.
            Some(_) if !plans => return Ok(Vec::new()),
            Some(task) => Some(
                self.parts
                    .tasks
                    .linked_sessions(emberhut_core::task::TaskId::new(task))
                    .await?,
            ),
            None => None,
        };
        let mut threads: Vec<CodingThread> = self
            .runner_threads()
            .await?
            .into_iter()
            .filter(|thread| {
                filter
                    .room
                    .is_none_or(|room| room == RoomId::MAIN || room_of(thread) == room)
                    && (!filter.direct || thread.direct)
                    && linked.as_ref().is_none_or(|ids| ids.contains(&thread.id))
            })
            .collect();
        threads.sort_by_key(|thread| std::cmp::Reverse(thread.updated_at));
        threads.truncate(LIST_LENGTH);
        let accounts = self.parts.usage.all().await?;
        let tasks = if plans {
            let ids: Vec<String> = threads.iter().map(|thread| thread.id.clone()).collect();
            self.parts.tasks.sessions_tasks(&ids).await?
        } else {
            BTreeMap::new()
        };
        Ok(threads
            .iter()
            .map(|thread| self.row(thread, &accounts, tasks.get(&thread.id)))
            .collect())
    }

    /// The state's `sessions`: every room's, the running and queued ones first, then the
    /// newest others, [`STATE_SESSIONS`] at least.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn state_sessions(&self) -> Result<Vec<api::work::RunnerSession>, DbError> {
        let all = self.sessions(Filter::default()).await?;
        let busy =
            |row: &api::work::RunnerSession| matches!(row.status.as_str(), "running" | "queued");
        let running = all.iter().filter(|row| busy(row)).count();
        let (first, rest): (Vec<_>, Vec<_>) = all.into_iter().partition(busy);
        Ok(first
            .into_iter()
            .chain(rest)
            .take(STATE_SESSIONS.max(running))
            .collect())
    }

    /// The state's `mySessions`: the person's own, every room's, [`STATE_MINE`] at most.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn my_sessions(&self) -> Result<Vec<api::work::RunnerSession>, DbError> {
        let mut mine = self
            .sessions(Filter {
                direct: true,
                ..Filter::default()
            })
            .await?;
        mine.truncate(STATE_MINE);
        Ok(mine)
    }

    fn row(
        &self,
        thread: &CodingThread,
        accounts: &[AccountUse],
        tasks: Option<&Vec<emberhut_core::task::TaskId>>,
    ) -> api::work::RunnerSession {
        let running = thread.status == ThreadStatus::Running;
        api::work::RunnerSession {
            id: thread.id.clone().into(),
            machine: thread.machine.clone().into(),
            title: thread.title.clone(),
            project: thread.project.clone(),
            status: thread.status.as_str().to_owned(),
            updated_at: timestamp_to_api(thread.updated_at),
            place: self.place(&thread.machine),
            doing: thread.doing.clone(),
            account: thread.account.clone(),
            account_name: thread
                .account
                .as_deref()
                .filter(|id| !id.is_empty())
                .map(|id| name_in(accounts, &thread.machine, id)),
            ask: thread.ask.clone(),
            last: thread.last.clone(),
            direct: thread.direct,
            model: thread.model.clone(),
            cwd: thread.cwd.clone(),
            branch: thread.branch.clone(),
            status_line: thread.status_line.clone(),
            since: thread.since.filter(|_| running).map(timestamp_to_api),
            effort: thread.effort.clone(),
            question: thread.question.clone(),
            tasks: tasks
                .into_iter()
                .flatten()
                .map(|task| api::ids::TaskId::new(task.get()))
                .collect(),
            room: room_to_api(room_of(thread)),
        }
    }

    /// Takes `machine`'s event: the session's thread gets its new look (what it is doing, what
    /// it was asked, its latest answer, its question), and a note or an account move goes to
    /// the activity log.
    ///
    /// # Errors
    ///
    /// [`SessionError::Refused`] when it isn't a runner event of `machine`'s (no session id, or
    /// another runner's name); [`SessionError::Database`].
    #[expect(clippy::too_many_lines, reason = "one case per kind of event")]
    pub async fn event(&self, machine: &str, event: &Value) -> Result<(), SessionError> {
        let named = text(event, "machine").unwrap_or_default();
        let session = event.get("session").unwrap_or(&Value::Null);
        let Some(id) = text(session, "id").filter(|id| !id.is_empty()) else {
            return Err(SessionError::Refused(format!(
                "not a runner event ({named})"
            )));
        };
        if named != machine || !self.is_runner(machine) {
            return Err(SessionError::Refused(format!(
                "not a runner event ({named})"
            )));
        }
        self.heard(machine);
        let now = self.parts.clock.now();
        let title = text(session, "title").unwrap_or_default();
        let kind = text(event, "kind").unwrap_or_default();
        let was = self.parts.store.get(machine, &id).await?;
        let at = text(event, "at")
            .and_then(|at| at.parse::<Timestamp>().ok())
            .unwrap_or(now);
        let mut thread = was.clone().unwrap_or_else(|| {
            CodingThread::new(machine, id.clone(), "", ThreadStatus::Queued, at)
        });
        thread.title.clone_from(&title);
        thread.project = text(session, "project").unwrap_or_default();
        thread.status = ThreadStatus::from(text(session, "status").unwrap_or_default().as_str());
        thread.updated_at = at;
        thread.parent = None;
        thread.account = text(session, "account");
        thread.cwd = text(session, "cwd");
        thread.branch = text(session, "branch");
        if truthy(session.pointer("/meta/direct")) {
            thread.direct = true;
            thread.model = text(session, "model");
            thread.effort = text(session, "effort");
        }
        let turn = event.get("turn").filter(|turn| turn.is_object());
        match (kind.as_str(), turn) {
            ("turn_queued", Some(turn)) => {
                let from = text(turn, "by").unwrap_or_default();
                if let Some(prompt) = text(turn, "prompt").filter(|prompt| !prompt.is_empty())
                    && !matches!(from.as_str(), by::SELF | by::RUNNER)
                {
                    let ask = asked(&prompt);
                    thread.ask.get_or_insert_with(|| ask.clone());
                    thread.last_ask = Some(ask);
                }
            }
            ("turn_started", _) => {
                thread.doing = Some("Starting".to_owned());
                thread.since = turn
                    .and_then(|turn| text(turn, "started"))
                    .or_else(|| text(event, "at"))
                    .and_then(|at| at.parse().ok());
                thread.question = None;
            }
            ("activity", _) => {
                if let Some(doing) = text(event, "doing") {
                    thread.doing = Some(doing);
                }
            }
            ("turn_finished", Some(turn)) => {
                thread.doing = None;
                thread.since = None;
                if text(turn, "outcome").as_deref() != Some("cancelled") {
                    thread.last = Some(clip(&text(turn, "last").unwrap_or_default(), LAST_LENGTH));
                }
            }
            ("question", _) => {
                if let Some(question) = text(event, "text").filter(|question| !question.is_empty())
                {
                    thread.question = Some(clip(&question, QUESTION_LENGTH));
                }
            }
            _ => {}
        }
        if let Some(line) = thread.last.as_deref().and_then(status_line) {
            thread.status_line = Some(line);
        }
        if let Some(end) =
            self.parts
                .threads
                .record(&thread)
                .await
                .map_err(|error| match error {
                    crate::services::threads::ThreadError::Database(error) => {
                        SessionError::Database(error)
                    }
                    other => SessionError::Malformed(other.to_string()),
                })?
        {
            tracing::info!(
                machine = %end.thread.machine,
                thread = %end.thread.id,
                status = %end.thread.status,
                why = %end.followed.why,
                "a followed session came to an end"
            );
        }
        // A session's task is followed only on a server that keeps plans.
        if self.parts.tasks.keeps_plans()
            && let Some(task) = task_in(session)
        {
            let turn = match (kind.as_str(), &thread.status) {
                ("turn_finished", _) => SessionTurn::Finished {
                    outcome: &turn
                        .and_then(|turn| text(turn, "outcome"))
                        .unwrap_or_else(|| "completed".to_owned()),
                    last: thread.last.as_deref(),
                },
                (_, ThreadStatus::Running) => SessionTurn::Running {
                    asked: thread.last_ask.as_deref(),
                },
                (_, ThreadStatus::Queued) => SessionTurn::Queued {
                    asked: thread.last_ask.as_deref(),
                },
                (_, ThreadStatus::Other(_)) => SessionTurn::Unknown,
                (_, status) => SessionTurn::Stopped {
                    status: status.as_str(),
                    last: thread.last.as_deref(),
                },
            };
            // The link is written when a turn is sent or starts, or the session is new, not
            // on every event (an `activity` comes every few seconds).
            let link = was.is_none() || matches!(kind.as_str(), "turn_queued" | "turn_started");
            // The task's bookkeeping never fails the event: its note and account lines go on.
            if let Err(error) = self
                .parts
                .tasks
                .session_turn(task, machine, &id, turn, link)
                .await
            {
                tracing::warn!(
                    task = %task.key(),
                    %machine,
                    session = %id,
                    %error,
                    "a runner session's turn could not be kept on its task"
                );
            }
        }
        match kind.as_str() {
            "note" => {
                if let Some(note) = text(event, "text").filter(|note| !note.is_empty()) {
                    self.note(format!("Runner session \"{title}\": {note}"))
                        .await;
                }
            }
            "account" => {
                let line = self.moved_line(machine, event).await?;
                self.note(format!("Runner session \"{title}\": {line}"))
                    .await;
            }
            _ => {}
        }
        Ok(())
    }

    /// An account move's line: to which account and why, or that none was left.
    async fn moved_line(&self, machine: &str, event: &Value) -> Result<String, DbError> {
        let accounts = self.parts.usage.all().await?;
        let name = |id: Option<String>| {
            id.filter(|id| !id.is_empty())
                .map_or_else(|| "?".to_owned(), |id| name_in(&accounts, machine, &id))
        };
        let why = text(event, "why").unwrap_or_default();
        let from = name(text(event, "from"));
        Ok(
            if let Some(to) = text(event, "to").filter(|to| !to.is_empty()) {
                format!(
                    "Moved to another account ({}): {from} {why}",
                    name(Some(to))
                )
            } else {
                let when = text(event, "resumeAt")
                    .filter(|at| !at.is_empty())
                    .map_or_else(|| "in an hour".to_owned(), |at| format!("at {at}"));
                format!(
                    "{from} {why}, and every other account is out too: it carries on by itself {when}"
                )
            },
        )
    }

    async fn note(&self, text: String) {
        if let Err(error) = self.parts.activity.note(text).await {
            tracing::warn!(%error, "an activity line could not be written");
        }
    }

    /// The transcript of the session `id`: the runner's, `after` the step it names and at most
    /// `limit` of them, its head with what the server knows of it added (`place`,
    /// `accountName`, `tasks`, `doing`, `project`).
    ///
    /// # Errors
    ///
    /// [`SessionError::Runner`]; [`SessionError::Malformed`] when the runner's answer has no
    /// session.
    pub async fn transcript(
        &self,
        id: &SessionId,
        after: &str,
        limit: &str,
    ) -> Result<Map<String, Value>, SessionError> {
        let machine = self.machine_of(id).await?;
        let query = form(&[("after", after), ("limit", limit)]);
        let answer = self
            .parts
            .api
            .call(
                &machine,
                Verb::Get,
                &format!("/sessions/{id}/transcript?{query}"),
                None,
            )
            .await?;
        let Value::Object(mut answer) = answer else {
            return Err(SessionError::Malformed(
                "the runner's transcript is not an object".to_owned(),
            ));
        };
        let Some(Value::Object(mut head)) = answer.remove("session") else {
            return Err(SessionError::Malformed(
                "the runner's transcript has no session".to_owned(),
            ));
        };
        head.insert("place".to_owned(), Value::String(self.place(&machine)));
        let account = head
            .get("account")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let name = self.account_name(&machine, account.as_deref()).await?;
        head.insert(
            "accountName".to_owned(),
            name.map_or(Value::Null, Value::String),
        );
        if let Some(mine) = self.parts.store.get(&machine, id.as_str()).await? {
            let accounts = self.parts.usage.all().await?;
            let tasks = if self.parts.tasks.keeps_plans() {
                self.parts.tasks.session_tasks(id.as_str()).await?
            } else {
                Vec::new()
            };
            let row = self.row(&mine, &accounts, Some(&tasks));
            head.insert("accountName".to_owned(), json!(row.account_name));
            head.insert("tasks".to_owned(), json!(row.tasks));
            head.insert("doing".to_owned(), json!(row.doing));
            head.insert("project".to_owned(), json!(row.project));
        }
        answer.insert("session".to_owned(), Value::Object(head));
        if !answer.get("events").is_some_and(Value::is_array) {
            answer.insert("events".to_owned(), Value::Array(Vec::new()));
        }
        Ok(answer)
    }

    /// The session `id`'s live stream from its runner, from the step after `after`.
    ///
    /// # Errors
    ///
    /// [`SessionError::Runner`].
    pub async fn live(&self, id: &SessionId, after: &str) -> Result<ByteStream, SessionError> {
        let machine = self.machine_of(id).await?;
        let after = after
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .unwrap_or(0.0);
        let path = format!(
            "/sessions/{id}/live?after={}",
            emberhut_core::machine::num(after.trunc())
        );
        Ok(self.parts.api.stream(&machine, &path).await?)
    }

    /// The person's reply in the session `id`: `text` (or, with files alone, a line saying so)
    /// and `files`, as its next turn. The runner's answer (`{ turn, queued }`).
    ///
    /// # Errors
    ///
    /// [`SessionError::Refused`] with nothing to send; [`SessionError::Runner`];
    /// [`SessionError::Transfer`].
    pub async fn reply(
        &self,
        id: &SessionId,
        text: &str,
        files: &[ChatFile],
    ) -> Result<Map<String, Value>, SessionError> {
        let text = text.trim();
        if text.is_empty() && files.is_empty() {
            return Err(SessionError::Refused("nothing to send".to_owned()));
        }
        let machine = self.machine_of(id).await?;
        let message = if text.is_empty() { SEE_THE_FILES } else { text };
        let answer = self
            .send_turn(&machine, id, message, by::PERSON, files)
            .await?;
        let title = self
            .parts
            .store
            .get(&machine, id.as_str())
            .await?
            .map_or_else(|| id.to_string(), |thread| thread.title);
        self.note(format!("Replied in runner session {title} from the app"))
            .await;
        Ok(answer)
    }

    /// The next turn of the session `id` on `machine`, sent `by` someone, with `files` put in
    /// its inbox first: the runner's answer (`{ turn, queued }`).
    ///
    /// # Errors
    ///
    /// [`SessionError::Runner`]; [`SessionError::Transfer`].
    pub async fn send_turn(
        &self,
        machine: &str,
        id: &SessionId,
        message: &str,
        from: &str,
        files: &[ChatFile],
    ) -> Result<Map<String, Value>, SessionError> {
        for file in files {
            if let Some(path) = &file.path {
                self.parts.transfer.push(machine, id, path, from).await?;
            }
        }
        let mut body = json!({ "prompt": message, "by": from });
        if !files.is_empty() {
            body["files"] = Value::Array(
                files
                    .iter()
                    .map(|file| json!({ "name": file.name, "url": file.url, "kind": file.kind }))
                    .collect(),
            );
        }
        let answer = self
            .parts
            .api
            .call(
                machine,
                Verb::Post,
                &format!("/sessions/{id}/send"),
                Some(body),
            )
            .await?;
        Ok(object(answer))
    }

    /// Stops the session `id`'s running turn (`interrupt`), or it and its queued turns
    /// (`stop`): the runner's answer (`{ running }`).
    ///
    /// # Errors
    ///
    /// [`SessionError::Runner`].
    pub async fn halt(
        &self,
        id: &SessionId,
        stop: bool,
    ) -> Result<Map<String, Value>, SessionError> {
        let machine = self.machine_of(id).await?;
        let what = if stop { "stop" } else { "interrupt" };
        let answer = self
            .parts
            .api
            .call(
                &machine,
                Verb::Post,
                &format!("/sessions/{id}/{what}"),
                None,
            )
            .await?;
        let done = if stop { "Stopped" } else { "Interrupted" };
        self.note(format!("{done} runner session {id}")).await;
        Ok(object(answer))
    }

    /// Changes the session `id`'s settings (`model`, `effort`, `title`) as `body` asks: its new
    /// head, as the runner answered.
    ///
    /// # Errors
    ///
    /// [`SessionError::Runner`].
    pub async fn settings(&self, id: &SessionId, body: Value) -> Result<Value, SessionError> {
        let machine = self.machine_of(id).await?;
        Ok(self
            .parts
            .api
            .call(
                &machine,
                Verb::Post,
                &format!("/sessions/{id}/settings"),
                Some(body),
            )
            .await?)
    }

    /// The person's own session, from the app: on the machine and in the folder they picked,
    /// their first message (and files) its first turn. Not followed, and no task: the assistant
    /// sees it in its threads at once. The runner's session answer, with the first turn's id.
    ///
    /// # Errors
    ///
    /// [`SessionError::Runner`] for a machine that isn't a runner, or the runner's refusal;
    /// [`SessionError::Refused`] with nothing to start it with; [`SessionError::Transfer`].
    pub async fn start_direct(
        &self,
        ask: DirectSession,
    ) -> Result<Map<String, Value>, SessionError> {
        if !self.is_runner(&ask.machine) {
            return Err(RunnerError::Unknown(ask.machine).into());
        }
        let message = ask.message.trim().to_owned();
        if message.is_empty() && ask.files.is_empty() {
            return Err(SessionError::Refused(
                "say something to start it with".to_owned(),
            ));
        }
        let mut body = Map::new();
        body.insert("cwd".to_owned(), Value::String(ask.cwd.clone()));
        // Projects don't carry runner folders yet: a folder belongs to no project.
        body.insert("project".to_owned(), Value::Null);
        for (key, value) in [
            ("account", &ask.account),
            ("model", &ask.model),
            ("effort", &ask.effort),
        ] {
            if let Some(value) = value.as_ref().filter(|value| !value.is_empty()) {
                body.insert(key.to_owned(), Value::String(value.clone()));
            }
        }
        if ask
            .worktree
            .as_ref()
            .is_some_and(|worktree| truthy(Some(worktree)))
        {
            body.insert(
                "worktree".to_owned(),
                ask.worktree.clone().unwrap_or(Value::Null),
            );
        }
        let title = ask
            .title
            .clone()
            .filter(|title| !title.is_empty())
            .or_else(|| {
                message
                    .is_empty()
                    .then(|| ask.files.first().map(|file| file.name.clone()))
                    .flatten()
            });
        if let Some(title) = title {
            body.insert("title".to_owned(), Value::String(title));
        }
        body.insert("meta".to_owned(), json!({ "direct": true }));
        body.insert("by".to_owned(), Value::String(by::PERSON.to_owned()));
        if ask.files.is_empty() {
            body.insert("prompt".to_owned(), Value::String(message));
            let opened = self.open(&ask.machine, body).await?;
            return Ok(opened);
        }
        // Opened empty: the files go in first, then the message.
        let mut opened = self.open(&ask.machine, body).await?;
        let id = opened
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| SessionId::parse(id).ok())
            .ok_or_else(|| {
                SessionError::Malformed("the runner opened no session it can be sent".to_owned())
            })?;
        let message = if message.is_empty() {
            SEE_THE_FILES
        } else {
            &message
        };
        let sent = self
            .send_turn(&ask.machine, &id, message, by::PERSON, &ask.files)
            .await?;
        opened.insert(
            "turn".to_owned(),
            sent.get("turn").cloned().unwrap_or(Value::Null),
        );
        Ok(opened)
    }

    /// A session the assistant starts for a piece of work, followed for `why`.
    ///
    /// # Errors
    ///
    /// [`SessionError::Runner`]; [`SessionError::Database`].
    pub async fn start_work(&self, ask: WorkSession, why: &str) -> Result<Opened, SessionError> {
        let mut body = Map::new();
        body.insert("title".to_owned(), Value::String(ask.title.clone()));
        body.insert("cwd".to_owned(), Value::String(ask.cwd.clone()));
        body.insert("prompt".to_owned(), Value::String(ask.message.clone()));
        body.insert(
            "project".to_owned(),
            ask.project.clone().map_or(Value::Null, Value::String),
        );
        if let Some(account) = &ask.account {
            body.insert("account".to_owned(), Value::String(account.clone()));
        }
        if let Some(model) = &ask.model {
            body.insert("model".to_owned(), Value::String(model.clone()));
        }
        body.insert(
            "effort".to_owned(),
            Value::String(ask.effort.clone().unwrap_or_else(|| "high".to_owned())),
        );
        if let Some(worktree) = &ask.worktree {
            body.insert("worktree".to_owned(), worktree.clone());
        }
        body.insert("meta".to_owned(), Value::Object(ask.meta.clone()));
        let opened = self.open(&ask.machine, body).await?;
        let id = opened
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if id.is_empty() {
            return Err(SessionError::Malformed(
                "the runner opened no session".to_owned(),
            ));
        }
        self.parts
            .threads
            .follow(&ask.machine, &id, why)
            .await
            .map_err(|error| SessionError::Malformed(error.to_string()))?;
        let field = |key: &str| opened.get(key).and_then(Value::as_str).map(str::to_owned);
        Ok(Opened {
            turn: field("turn"),
            account: field("account"),
            cwd: field("cwd"),
            branch: field("branch"),
            model: field("model"),
            id,
        })
    }

    async fn open(
        &self,
        machine: &str,
        body: Map<String, Value>,
    ) -> Result<Map<String, Value>, SessionError> {
        let answer = self
            .parts
            .api
            .call(machine, Verb::Post, "/sessions", Some(Value::Object(body)))
            .await?;
        Ok(object(answer))
    }

    /// The folders on `machine` for the new-session sheet: where to start (no `path`, with the
    /// projects' folders there), or `path`'s folders.
    ///
    /// # Errors
    ///
    /// [`SessionError::Runner`].
    pub async fn folders(
        &self,
        machine: &str,
        path: Option<&str>,
    ) -> Result<Map<String, Value>, SessionError> {
        let at = path.filter(|path| !path.is_empty());
        let query = at.map_or_default(|path| format!("?{}", form(&[("path", path)])));
        let mut answer = object(
            self.parts
                .api
                .call(machine, Verb::Get, &format!("/folders{query}"), None)
                .await?,
        );
        if at.is_none() {
            // Projects don't carry runner folders yet: none to put first.
            answer.insert("projects".to_owned(), Value::Array(Vec::new()));
        }
        Ok(answer)
    }

    /// The branches of the repository `cwd` is in, on `machine`.
    ///
    /// # Errors
    ///
    /// [`SessionError::Runner`].
    pub async fn branches(
        &self,
        machine: &str,
        cwd: &str,
    ) -> Result<Map<String, Value>, SessionError> {
        let query = form(&[("cwd", cwd)]);
        Ok(object(
            self.parts
                .api
                .call(machine, Verb::Get, &format!("/branches?{query}"), None)
                .await?,
        ))
    }

    /// `machine`'s accounts as its runner lists them now, kept (logins left out).
    ///
    /// # Errors
    ///
    /// [`SessionError::Runner`]; [`SessionError::Database`].
    pub async fn refresh_accounts(&self, machine: &str) -> Result<(), SessionError> {
        let answer = self
            .parts
            .api
            .call(machine, Verb::Get, "/accounts", None)
            .await?;
        let accounts: Vec<AccountUse> = answer
            .get("accounts")
            .and_then(Value::as_array)
            .map_or_default(|list| {
                list.iter()
                    .filter_map(|item| account_of(machine, item))
                    .collect()
            });
        self.parts
            .usage
            .replace(machine, &accounts, self.parts.clock.now())
            .await?;
        Ok(())
    }

    /// Looks at every runner once: its health, and when it answers, its accounts.
    pub async fn tick(&self) {
        for machine in self.machines() {
            self.tick_one(&machine).await;
        }
    }

    /// Looks at `machine`'s runner once: its health, and when it answers, its accounts.
    pub async fn tick_one(&self, machine: &str) {
        let error = self.parts.api.health(machine).await.err();
        {
            let mut looks = self.looks.lock().unwrap_or_else(PoisonError::into_inner);
            let look = looks.entry(machine.to_owned()).or_default();
            look.error.clone_from(&error);
            if error.is_none() {
                look.at = Some(self.parts.clock.now());
            }
        }
        if error.is_none()
            && let Err(error) = self.refresh_accounts(machine).await
        {
            tracing::debug!(%machine, %error, "a runner's accounts could not be read");
        }
    }

    /// Looks at every runner every `every` until `stop`, when any is configured.
    pub async fn keep_looking(self, every: Duration, stop: CancellationToken) {
        if self.machines().is_empty() {
            return;
        }
        let mut ticks = tokio::time::interval(every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = stop.cancelled() => return,
                _ = ticks.tick() => {}
            }
            self.tick().await;
        }
    }

    /// The `session_read` tool: the session `id`'s last `limit` steps, plainly, after a line
    /// about it.
    ///
    /// # Errors
    ///
    /// [`SessionError::Runner`]; [`SessionError::Malformed`] when the transcript has no steps.
    pub async fn read(&self, id: &SessionId, limit: usize) -> Result<String, SessionError> {
        let machine = self.machine_of(id).await?;
        let answer = self
            .parts
            .api
            .call(
                &machine,
                Verb::Get,
                &format!("/sessions/{id}/transcript?limit={}", limit.min(400)),
                None,
            )
            .await?;
        let Some(events) = answer.get("events").and_then(Value::as_array) else {
            return Err(SessionError::Malformed(
                "the runner's transcript has no steps".to_owned(),
            ));
        };
        let lines: Vec<String> = events.iter().filter_map(step_line).collect();
        let head = answer.get("session").unwrap_or(&Value::Null);
        let account = self
            .account_name(&machine, head.get("account").and_then(Value::as_str))
            .await?
            .unwrap_or_else(|| "null".to_owned());
        let running = if truthy(head.get("running")) {
            ", running"
        } else {
            ""
        };
        let direct = if truthy(head.pointer("/meta/direct")) {
            " (the person's own session)"
        } else {
            ""
        };
        let branch = text(head, "branch")
            .filter(|branch| !branch.is_empty())
            .map_or_default(|branch| format!(" (branch {branch})"));
        Ok(format!(
            "{} [{}{running}]{direct} on the {}'s runner, account {account}, in {}{branch}\n{}",
            js(head.get("title")),
            js(head.get("status")),
            self.place(&machine),
            js(head.get("cwd")),
            lines.join("\n")
        ))
    }
}

/// A transcript step as one plain line (`None` for one that says nothing).
fn step_line(step: &Value) -> Option<String> {
    let field = |key: &str| text(step, key).unwrap_or_default();
    Some(match field("kind").as_str() {
        "user" => format!(
            "[{}] {}",
            text(step, "by").unwrap_or_else(|| by::ASSISTANT.to_owned()),
            clip(&field("text"), 1500)
        ),
        "text" => format!("[agent] {}", clip(&field("text"), 2000)),
        "tool" => {
            let summary = text(step, "summary")
                .filter(|summary| !summary.is_empty())
                .map_or_default(|summary| format!(": {}", clip(&summary, 160)));
            format!("  · {}{summary}", js(step.get("doing")))
        }
        "result" => {
            if !truthy(step.get("error")) {
                return None;
            }
            format!("    ! {}", clip(&field("text"), 300))
        }
        "end" => format!("— turn {} {}", js(step.get("n")), js(step.get("outcome"))),
        "question" => {
            let questions = step
                .get("questions")
                .and_then(Value::as_array)
                .map_or_default(|questions| {
                    questions
                        .iter()
                        .map(|question| {
                            let options = question
                                .get("options")
                                .and_then(Value::as_array)
                                .map_or_default(|options| {
                                    options
                                        .iter()
                                        .map(|option| text(option, "label").unwrap_or_default())
                                        .collect::<Vec<_>>()
                                        .join(" / ")
                                });
                            format!("{} ({options})", js(question.get("question")))
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                });
            format!("[the agent asks the person] {questions}")
        }
        _ => format!("({})", clip(&field("text"), 200)),
    })
}

/// A value as a JavaScript template prints it: `undefined` when it isn't there.
fn js(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_owned(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    }
}

/// A runner's account as it listed it (`{ id, name, scope, order, enabled, loggedIn, email,
/// plan, used5h, used7d, resets5h, resets7d, limitedUntil, limitedWhy, checked, error }`); its
/// login is never kept.
fn account_of(machine: &str, item: &Value) -> Option<AccountUse> {
    let id = text(item, "id").filter(|id| !id.is_empty())?;
    let mut account = AccountUse::new(machine, id.clone(), text(item, "name").unwrap_or(id));
    if let Some(scope) = text(item, "scope") {
        account.scope = scope;
    }
    account.enabled = item.get("enabled").and_then(Value::as_bool) != Some(false);
    account.logged_in = item.get("loggedIn").and_then(Value::as_bool) == Some(true);
    account.rank = item.get("order").and_then(Value::as_i64);
    account.email = text(item, "email");
    account.plan = text(item, "plan");
    account.used5h = item.get("used5h").and_then(Value::as_f64);
    account.used7d = item.get("used7d").and_then(Value::as_f64);
    let when = |key: &str| text(item, key).and_then(|at| at.parse::<Timestamp>().ok());
    account.resets5h = when("resets5h");
    account.resets7d = when("resets7d");
    account.limited_until = when("limitedUntil");
    account.limited_why = text(item, "limitedWhy");
    account.checked = when("checked");
    account.error = text(item, "error");
    Some(account)
}

/// The account `id`'s name: `machine`'s runner's, else another runner's, else the id.
fn name_in(accounts: &[AccountUse], machine: &str, id: &str) -> String {
    accounts
        .iter()
        .find(|account| account.machine == machine && account.id == id)
        .or_else(|| accounts.iter().find(|account| account.id == id))
        .map_or_else(|| id.to_owned(), |account| account.name.clone())
}

/// The room a session belongs to. Rooms don't carry projects yet, so every session is Main's.
fn room_of(_thread: &CodingThread) -> RoomId {
    RoomId::MAIN
}

/// `key`'s text in `value` (a string, or another value as JSON writes it; `None` for null or
/// missing).
pub(crate) fn text(value: &Value, key: &str) -> Option<String> {
    match value.get(key)? {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        other => Some(other.to_string()),
    }
}

/// The task a session's meta names (`meta.task`: its number, as a number or as text, or its
/// key), if any.
fn task_in(session: &Value) -> Option<emberhut_core::task::TaskId> {
    let named = session.pointer("/meta/task")?;
    let number = match named {
        Value::Number(n) => n.as_i64(),
        Value::String(text) => {
            let text = text.trim();
            text.strip_prefix(emberhut_core::task::wire::KEY_PREFIX)
                .unwrap_or(text)
                .parse()
                .ok()
        }
        _ => None,
    }?;
    (number > 0).then(|| emberhut_core::task::TaskId::new(number))
}

/// Whether a JSON value is truthy, as JavaScript reads it.
pub(crate) fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(on)) => *on,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Some(Value::String(text)) => !text.is_empty(),
        Some(_) => true,
    }
}

/// An answer as an object (anything else is `{}`).
fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// `pairs` as a form-encoded query.
pub(crate) fn form(pairs: &[(&str, &str)]) -> String {
    form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

/// Whether `id` is a runner session's id as the runners make them: `r-` and 8 hex digits.
#[must_use]
pub fn is_session_id(id: &str) -> bool {
    id.strip_prefix("r-")
        .is_some_and(|hex| hex.len() == 8 && hex.bytes().all(is_lower_hex))
}

/// Whether `b` is a hex digit, lowercase.
fn is_lower_hex(b: u8) -> bool {
    b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_reads_as_one_plain_line() {
        assert_eq!(
            step_line(&json!({ "kind": "user", "by": "owner", "text": "Fix  it" })).as_deref(),
            Some("[owner] Fix it")
        );
        assert_eq!(
            step_line(&json!({ "kind": "tool", "doing": "Reading", "summary": "a.rs" })).as_deref(),
            Some("  · Reading: a.rs")
        );
        assert_eq!(step_line(&json!({ "kind": "result", "text": "ok" })), None);
        assert_eq!(
            step_line(&json!({ "kind": "end", "n": 2, "outcome": "completed" })).as_deref(),
            Some("— turn 2 completed")
        );
        assert_eq!(
            step_line(&json!({ "kind": "question", "questions": [
                { "question": "Which?", "options": [{ "label": "A" }, { "label": "B" }] }
            ] }))
            .as_deref(),
            Some("[the agent asks the person] Which? (A / B)")
        );
    }

    #[test]
    fn an_account_drops_its_login() {
        let account = account_of(
            "build",
            &json!({ "id": "a", "name": "A", "loggedIn": true, "login": "secret", "used5h": 12.5,
                     "resets5h": "2026-11-25T10:00:00Z", "enabled": false }),
        )
        .unwrap();
        assert_eq!(account.machine, "build");
        assert!(account.logged_in);
        assert!(!account.enabled);
        assert_eq!(account.used5h, Some(12.5));
        assert!(account.resets5h.is_some());
        assert_eq!(account_of("build", &json!({ "name": "no id" })), None);
    }

    #[test]
    fn a_session_id_is_r_and_8_hex_digits() {
        assert!(is_session_id("r-1a2b3c4d"));
        assert!(!is_session_id("r-1a2b"));
        assert!(!is_session_id("r-1A2B3C4D"));
        assert!(!is_session_id("t-1a2b3c4d"));
    }

    #[test]
    fn only_r_and_lowercase_hex_digits_go_into_a_runner_s_path() {
        for id in ["r-1", "r-1a2b3c4d", "r-0123456789abcdef0123456789abcdef"] {
            assert_eq!(SessionId::parse(id).unwrap().as_str(), id);
        }
        for id in [
            "",
            "r-",
            "1a2b",
            "r-1A2B",
            "r-../jobs/stop?",
            "r-1a/../off#",
            "../files?path=x&",
            "r-1a%2f",
            "r-0123456789abcdef0123456789abcdef0",
        ] {
            assert!(
                matches!(SessionId::parse(id), Err(SessionError::Refused(_))),
                "{id}"
            );
        }
    }
}
