//! The runner sessions working on tasks: a session the assistant started for a task carries
//! the task in its meta, and each of its runner's events says where its turn is. The session
//! is linked to its task (a `thread` link, which the task's page lists and the plan tools scope
//! a session's calls by), and each of its turns is a run of the task, open while it is queued
//! or running and closed when it ends, so the task shows running while its session works.
//! All of it is on a server that keeps plans (`--task-plans`) only: without, the runners'
//! events leave tasks alone.

use std::collections::BTreeMap;

use emberhut_core::task::TaskId;
use emberhut_core::task::live::{NewRun, RunKind};
use emberhut_db::DbError;

use super::{TaskError, TaskService};

/// A task link's kind for a coding thread or a runner session.
pub(crate) const THREAD_LINK: &str = "thread";

/// Where a runner session's turn is, as its latest event says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionTurn<'a> {
    /// Sent, waiting to start.
    Queued {
        /// What it was asked, when known.
        asked: Option<&'a str>,
    },
    /// Running now.
    Running {
        /// What it was asked, when known.
        asked: Option<&'a str>,
    },
    /// The turn just finished.
    Finished {
        /// How it ended (`completed`, `failed`, `cancelled`).
        outcome: &'a str,
        /// Its last answer.
        last: Option<&'a str>,
    },
    /// Not running: between turns, waiting for an answer, or over.
    Stopped {
        /// The session's status word.
        status: &'a str,
        /// Its last answer.
        last: Option<&'a str>,
    },
    /// A status the server doesn't know: nothing changes.
    Unknown,
}

impl TaskService {
    /// A runner's event about the session `session` on `machine`, which works on the task
    /// `task`: its turn opens, starts or closes the task's run (a finished turn is a turn of
    /// work on it, [`Self::turn_ended`]). The session is linked to the task when `link` (a turn
    /// queued or started, a session seen for the first time) or when its run opens now, not on
    /// every event. Nothing for a task that isn't there. Taking the same event twice changes
    /// nothing.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn session_turn(
        &self,
        task: TaskId,
        machine: &str,
        session: &str,
        turn: SessionTurn<'_>,
        link: bool,
    ) -> Result<(), TaskError> {
        if self.tasks.get(task).await?.is_none() {
            tracing::debug!(task = %task.key(), session, "a session names a task that isn't there");
            return Ok(());
        }
        if link {
            self.links
                .link(task, THREAD_LINK, session, Some(machine))
                .await?;
        }
        let now = self.now();
        match turn {
            SessionTurn::Queued { asked } | SessionTurn::Running { asked } => {
                let started = matches!(turn, SessionTurn::Running { .. });
                let (run, opened) = self
                    .runs
                    .open_thread_once(
                        NewRun {
                            task,
                            machine: machine.to_owned(),
                            thread: session.to_owned(),
                            kind: RunKind::Thread,
                            asked: asked.map(str::to_owned),
                            started,
                        },
                        now,
                    )
                    .await?;
                if opened && !link {
                    // Its queued and started events were missed.
                    self.links
                        .link(task, THREAD_LINK, session, Some(machine))
                        .await?;
                }
                if started && !opened {
                    self.runs.start(run.id, now).await?;
                }
            }
            SessionTurn::Finished { outcome, last } => {
                if let Some(run) = self.runs.open_thread(machine, session).await? {
                    self.runs.close(run.id, outcome, last, now).await?;
                }
                self.turn_ended(task).await?;
            }
            SessionTurn::Stopped { status, last } => {
                if let Some(run) = self.runs.open_thread(machine, session).await? {
                    self.runs.close(run.id, status, last, now).await?;
                }
            }
            SessionTurn::Unknown => {}
        }
        Ok(())
    }

    /// The tasks the session `session` is linked to, the oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn session_tasks(&self, session: &str) -> Result<Vec<TaskId>, DbError> {
        self.links.tasks_of(THREAD_LINK, session).await
    }

    /// The tasks each of `sessions` is linked to, for those linked to any, in one query.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn sessions_tasks(
        &self,
        sessions: &[String],
    ) -> Result<BTreeMap<String, Vec<TaskId>>, DbError> {
        self.links.tasks_of_each(THREAD_LINK, sessions).await
    }

    /// The sessions and threads linked to the task `task`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn linked_sessions(&self, task: TaskId) -> Result<Vec<String>, DbError> {
        Ok(self
            .links
            .of(task)
            .await?
            .into_iter()
            .filter(|link| link.kind == THREAD_LINK)
            .map(|link| link.reference)
            .collect())
    }
}
