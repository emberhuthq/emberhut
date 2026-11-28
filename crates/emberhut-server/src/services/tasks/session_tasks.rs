//! Every runner session works for a task, on a server that keeps plans (`--task-plans`): a
//! session started without one gets a task made for it (the person's own sessions, and the
//! assistant's started without a task named), titled from what it was first asked
//! ([`emberhut_core::task::session`]), and marked as made for that session; a session moves to
//! another task when the person (or the assistant) says it belongs there, and a task made for
//! it that is left with nothing of its own is put away.
//!
//! Sessions the server saw before it made tasks for sessions are left as they are: a session
//! gets a task made for it only when it was opened at or after [`SINCE`], the time this server
//! first ran with this rule, which it writes once (its first tick, at start-up, or the first
//! time the rule is asked).

use emberhut_core::task::session as rules;
use emberhut_core::task::{Owner, Status, Task, TaskChange, TaskId, Who};
use emberhut_db::{NewTask, SettingKey};
use jiff::Timestamp;

use super::sessions::THREAD_LINK;
use super::{TaskError, TaskService};

/// When this server first made tasks for sessions (RFC 3339): sessions opened before are left
/// as they are.
const SINCE: SettingKey<String> = SettingKey::new("sessionTasksSince");

/// A task to make for a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionTask {
    /// Its title ([`rules::session_title`]).
    pub title: String,
    /// Whose it is: the person's for their own session, the assistant's for one it started.
    pub owner: Owner,
    /// The history line saying why it was made ([`rules::opened_from`],
    /// [`rules::MADE_FOR_SESSION`]).
    pub why: String,
    /// The session it is made for, when it is open already (one made before its session
    /// opens is marked once it has, [`TaskService::made_for_session`]).
    pub session: Option<String>,
}

/// A session to move to another task ([`TaskService::move_session`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionMove {
    /// The session's id.
    pub session: String,
    /// The machine it runs on, as its runner names itself.
    pub machine: String,
    /// Its title, for the history lines.
    pub title: String,
    /// The task it works for from now on.
    pub to: TaskId,
    /// Who moved it.
    pub by: Who,
}

/// Why a session couldn't be moved.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SessionMoveError {
    /// Sessions are not linked to tasks on this server (it keeps no plans).
    #[error("sessions work for tasks only where tasks carry plans")]
    NotKept,
    /// The server doesn't know the session.
    #[error("no session {0}")]
    NoSession(String),
    /// There is no such task.
    #[error("no task {0}")]
    NoTask(String),
    /// The task is done or archived: no session works for it any more.
    #[error("{0} is {1}: a session can't move to it")]
    Closed(String, &'static str),
    /// The tasks could not be read or written.
    #[error(transparent)]
    Task(#[from] TaskError),
}

impl From<emberhut_db::DbError> for SessionMoveError {
    fn from(error: emberhut_db::DbError) -> Self {
        Self::Task(TaskError::Db(error))
    }
}

impl TaskService {
    /// When this server first made tasks for sessions, written now the first time it is asked
    /// (only on a server that keeps plans: none otherwise).
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn sessions_tasked_since(&self) -> Result<Option<Timestamp>, TaskError> {
        if !self.task_plans {
            return Ok(None);
        }
        if let Some(at) = self
            .settings
            .get(&SINCE)
            .await?
            .and_then(|at| at.parse::<Timestamp>().ok())
        {
            return Ok(Some(at));
        }
        let now = self.now();
        self.settings.set(&SINCE, &now.to_string()).await?;
        Ok(Some(now))
    }

    /// Whether a session the runner says was opened at `created` (when it says) is new enough
    /// to get a task made for it: opened at or after [`SINCE`]; with no time from the runner,
    /// when the server had no record of it before (`first_seen`).
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn is_new_session(
        &self,
        created: Option<Timestamp>,
        first_seen: bool,
    ) -> Result<bool, TaskError> {
        let Some(since) = self.sessions_tasked_since().await? else {
            return Ok(false);
        };
        Ok(created.map_or(first_seen, |created| created >= since))
    }

    /// Makes `new`: a task working now, owned as it says, with no brief (an assistant's task
    /// with a brief would get a worker of its own; the session is its work), its history saying
    /// why it was made.
    ///
    /// # Errors
    ///
    /// [`TaskError`].
    pub async fn make_session_task(&self, new: SessionTask) -> Result<Task, TaskError> {
        let by = match new.owner {
            Owner::Person => Who::Person,
            Owner::Assistant => Who::Assistant,
        };
        let made = self
            .create(
                NewTask {
                    title: new.title,
                    owner: Some(new.owner),
                    ..NewTask::default()
                },
                by,
            )
            .await?;
        let made = self
            .update(
                made.id,
                TaskChange::status(Status::Working),
                by,
                Some(&new.why),
            )
            .await?;
        if let Some(session) = &new.session {
            self.made_for_session(made.id, session).await?;
        }
        Ok(made)
    }

    /// Marks the task `id` as made for the session `session`.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn made_for_session(&self, id: TaskId, session: &str) -> Result<(), TaskError> {
        self.tasks.set_made_for_session(id, session).await?;
        Ok(())
    }

    /// Puts away the task `id`, made for a session that then could not be started, saying why.
    ///
    /// # Errors
    ///
    /// [`TaskError`].
    pub async fn session_not_started(&self, id: TaskId, why: &str) -> Result<(), TaskError> {
        let line = rules::not_started(why);
        self.update(
            id,
            TaskChange::status(Status::Canceled),
            Who::Assistant,
            Some(&line),
        )
        .await?;
        Ok(())
    }

    /// Links the session `session` on `machine` to each of `tasks` there is (a session with no
    /// task of its own works for its parent's): the tasks it was linked to.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn link_session(
        &self,
        tasks: &[TaskId],
        session: &str,
        machine: &str,
    ) -> Result<Vec<TaskId>, TaskError> {
        let mut linked = Vec::new();
        for task in tasks {
            if self.tasks.get(*task).await?.is_some() {
                self.links
                    .link(*task, THREAD_LINK, session, Some(machine))
                    .await?;
                linked.push(*task);
            }
        }
        Ok(linked)
    }

    /// Moves `it` to the task `it.to`: the session is linked to it alone, its open run goes
    /// with it, both tasks' histories say so, and a task made for this session that is left
    /// with nothing of its own (no other session, no plan steps, no subtasks or files, nothing
    /// in its history but the server's own lines) is put away (`Merged into T-n`). The tasks it
    /// moved from.
    ///
    /// # Errors
    ///
    /// [`SessionMoveError::NoTask`] for a task that isn't there, [`SessionMoveError::Closed`]
    /// for one that is done or archived, [`SessionMoveError::Task`].
    pub async fn move_session(&self, it: SessionMove) -> Result<Vec<TaskId>, SessionMoveError> {
        let to = it.to;
        let target = self
            .tasks
            .get(to)
            .await?
            .ok_or_else(|| SessionMoveError::NoTask(to.key()))?;
        match target.status {
            Status::Done => return Err(SessionMoveError::Closed(to.key(), "done")),
            Status::Canceled => return Err(SessionMoveError::Closed(to.key(), "archived")),
            _ => {}
        }
        let (already, from): (Vec<TaskId>, Vec<TaskId>) = self
            .session_tasks(&it.session)
            .await?
            .into_iter()
            .partition(|task| *task == to);
        if from.is_empty() && !already.is_empty() {
            return Ok(Vec::new());
        }
        let label = rules::session_label(&it.session, &it.title);
        self.links
            .link(to, THREAD_LINK, &it.session, Some(&it.machine))
            .await?;
        for old in &from {
            self.links.unlink(*old, &it.session).await?;
        }
        if let Some(run) = self.runs.open_thread(&it.machine, &it.session).await? {
            self.runs.move_to(run.id, to).await?;
        }
        self.line(to, it.by, &rules::moved_here(&label, &from))
            .await?;
        for old in &from {
            self.line(*old, it.by, &rules::moved_to(&label, to)).await?;
            if self.left_empty(*old, &it.session).await? {
                self.update(
                    *old,
                    TaskChange {
                        status: Some(Status::Canceled),
                        today: Some(false),
                        ..TaskChange::default()
                    },
                    it.by,
                    Some(&rules::merged_into(to)),
                )
                .await
                .map_err(SessionMoveError::Task)?;
            }
        }
        Ok(from)
    }

    /// Writes `text` (its keys masked) to the task `id`'s history, as `who`; unlike
    /// [`Self::note`], a link in it (a session's title may have one) is not attached.
    async fn line(&self, id: TaskId, who: Who, text: &str) -> Result<(), TaskError> {
        let text = emberhut_core::guard::key_mask::mask(text, &[], None);
        self.tasks.note(id, who, &text, self.now()).await?;
        Ok(())
    }

    /// Whether `id`, which `session` just left, was made for it and is left with nothing of
    /// its own: open, no session linked, no plan steps, no subtasks, no files, and nothing in
    /// its history but the server's own lines.
    async fn left_empty(&self, id: TaskId, session: &str) -> Result<bool, TaskError> {
        if self.tasks.made_for_session(id).await?.as_deref() != Some(session) {
            return Ok(false);
        }
        let Some(task) = self.tasks.get(id).await? else {
            return Ok(false);
        };
        if !task.status.is_open() || !self.linked_sessions(id).await?.is_empty() {
            return Ok(false);
        }
        let planned = self
            .tasks
            .plan(id)
            .await?
            .is_some_and(|plan| !plan.steps.is_empty());
        let files = self
            .artifacts
            .of(&[id])
            .await?
            .get(&id)
            .is_some_and(|files| !files.is_empty());
        if planned || files || !self.tasks.subtasks(id).await?.is_empty() {
            return Ok(false);
        }
        Ok(self
            .tasks
            .history(id)
            .await?
            .iter()
            .all(|line| rules::is_bookkeeping(&line.text)))
    }
}
