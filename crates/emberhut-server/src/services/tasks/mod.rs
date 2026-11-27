//! Tasks: the person's and the assistant's work, with its projects, areas, history, asks,
//! findings, artifacts, links and runs. Every route, settings action and tool of the area goes
//! through this service; its rules are `emberhut-core`'s and its writes `emberhut-db`'s.
//!
//! The work itself is done elsewhere: a worker (a session doing one task) is the workers'
//! area, reached through [`TaskWorkers`]; this service keeps what the workers read and write
//! (a task's brief, what it carries on with next, its runs).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use emberhut_api as api;
use emberhut_core::event::NewEvent;
use emberhut_core::task::ask::AskRefusal;
use emberhut_core::task::board::{self, Board, FOCUS_HOURS, Project, week_start};
use emberhut_core::task::live::{self, live_of};
use emberhut_core::task::rollup::Family;
use emberhut_core::task::search::TaskSearch;
use emberhut_core::task::{Owner, ProjectId, Status, Task, TaskChange, TaskId, TaskRefusal, Who};
use emberhut_core::{Clock, RoomId};
use emberhut_db::{
    AgentStore, AreaStore, AreaStoreError, ArtifactStore, Database, DbError, EventStore, LinkStore,
    NewTask, ProjectStore, ProjectStoreError, RunStore, SettingStore, TaskStore, TaskStoreError,
    ThreadStore,
};
use futures_util::future::BoxFuture;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

use crate::convert::timestamp_to_api;
use crate::services::activity::ActivityLog;
use crate::services::agents::scope::{self, TaskOwner};
use crate::services::guard::send_tap::{self, SendResult};
use crate::services::plan::{PlanError, PlanService};

mod asks;
mod changes;
pub mod files;
mod outline;
pub mod rooms;
mod shape;

pub use asks::{AskInput, MessageFile};
pub use changes::{FindingsItem, ProgressItem, reply_posts};
pub use files::{ArtifactFiles, Ffmpeg, Thumbnailer};
pub use rooms::RoomTask;

/// `GET /api/tasks`'s query, read leniently: every value is
/// text, and one that isn't what it should be narrows nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TasksQuery {
    /// A task's key, its bare number, or words of its title and description.
    #[serde(default)]
    pub q: Option<String>,
    /// At most this many (the default 50, at most 200).
    #[serde(default)]
    pub limit: Option<String>,
    /// A project's id, or `none` for tasks with no project.
    #[serde(default)]
    pub project: Option<String>,
    /// `1`: only done and canceled ones.
    #[serde(default)]
    pub closed: Option<String>,
    /// `1`: only canceled ones (the Archived list).
    #[serde(default)]
    pub archived: Option<String>,
    /// An area's id.
    #[serde(default)]
    pub area: Option<String>,
    /// Statuses, comma separated.
    #[serde(default)]
    pub status: Option<String>,
    /// Whose: the person's or the assistant's id.
    #[serde(default)]
    pub owner: Option<String>,
}

/// Why a task call has no ordinary answer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TaskError {
    /// A rule says no; the text is for the person or the assistant, as is.
    #[error("{0}")]
    Refused(String),
    /// The database failed.
    #[error("the tasks could not be read or written")]
    Db(#[from] DbError),
}

impl TaskError {
    pub(crate) fn refused(text: impl Into<String>) -> Self {
        Self::Refused(text.into())
    }
}

impl From<TaskRefusal> for TaskError {
    fn from(refusal: TaskRefusal) -> Self {
        Self::Refused(refusal.to_string())
    }
}

impl From<AskRefusal> for TaskError {
    fn from(refusal: AskRefusal) -> Self {
        Self::Refused(refusal.to_string())
    }
}

impl From<TaskStoreError> for TaskError {
    fn from(error: TaskStoreError) -> Self {
        match error {
            TaskStoreError::Refused(refusal) => refusal.into(),
            TaskStoreError::Db(error) => Self::Db(error),
            other => Self::Refused(other.to_string()),
        }
    }
}

impl From<ProjectStoreError> for TaskError {
    fn from(error: ProjectStoreError) -> Self {
        match error {
            ProjectStoreError::Db(error) => Self::Db(error),
            other => Self::Refused(other.to_string()),
        }
    }
}

impl From<AreaStoreError> for TaskError {
    fn from(error: AreaStoreError) -> Self {
        match error {
            AreaStoreError::Db(error) => Self::Db(error),
            other => Self::Refused(other.to_string()),
        }
    }
}

impl From<PlanError> for TaskError {
    fn from(error: PlanError) -> Self {
        match error {
            PlanError::Db(error) => Self::Db(error),
            PlanError::Refused(text) => Self::Refused(text),
        }
    }
}

/// The workers' area, as tasks need it: a worker started on a task, or stopped.
pub trait TaskWorkers: fmt::Debug + Send + Sync {
    /// Starts a worker on the task now.
    fn start(&self, task: TaskId) -> BoxFuture<'_, Result<(), String>>;
    /// Stops the worker's session on the task; the task's side (its run, its status) is the
    /// tasks' own.
    fn stop(&self, task: TaskId) -> BoxFuture<'_, Result<(), String>>;
}

/// Another area that may take the person's answer on a task as its own (the assistant's own
/// WhatsApp number takes its linking task's `Link` and the phone number written on it): then
/// the task's own handling stops there.
pub trait AnswerTaker: fmt::Debug + Send + Sync {
    /// Whether the person's `choice` on `task` (already in its history) was this area's.
    fn decide<'a>(&'a self, task: TaskId, choice: &'a str) -> BoxFuture<'a, bool>;
    /// Whether the person's reply `text` on `task`'s page (already in its history) was this
    /// area's.
    fn message<'a>(&'a self, task: TaskId, text: &'a str) -> BoxFuture<'a, bool>;
}

/// A server with no workers: nothing can be started, and there is nothing to stop.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoWorkers;

impl TaskWorkers for NoWorkers {
    fn start(&self, _task: TaskId) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async { Err("this server runs no workers".to_owned()) })
    }

    fn stop(&self, _task: TaskId) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
}

/// Everything the state shows of the tasks and the plan, built at once.
#[derive(Clone, Debug, Default)]
pub struct PlanState {
    /// The board's tasks: open ones, this week's done ones and open parents' subtasks.
    pub issues: Vec<api::tasks::Issue>,
    /// Today's ids, ranked.
    pub today: Vec<api::ids::TaskId>,
    /// The projects, in board order.
    pub projects: Vec<api::tasks::Project>,
    /// The goals, with this week's steps.
    pub goals: Vec<api::tasks::Goal>,
    /// The tasks as steps: open ones, then this week's done ones.
    pub todos: Vec<api::plan::Todo>,
    /// Where the week starts.
    pub week_start: Option<api::time::Timestamp>,
    /// Every project's areas.
    pub areas: Vec<api::areas::Area>,
    /// The weekly report.
    pub report: Option<api::plan::WeekReport>,
}

/// What the task service is made of.
#[derive(Clone, Debug)]
pub struct TaskParts {
    /// The database.
    pub db: Database,
    /// What time it is.
    pub clock: Arc<dyn Clock>,
    /// The server's time zone: days, the week and deadlines are its.
    pub zone: TimeZone,
    /// The goals, the Focus list and the concern.
    pub plan: PlanService,
    /// The activity log.
    pub activity: ActivityLog,
    /// Where a task's files may come from.
    pub files: ArtifactFiles,
    /// The workers' area.
    pub workers: Arc<dyn TaskWorkers>,
}

/// The tasks.
#[derive(Clone, Debug)]
pub struct TaskService {
    tasks: TaskStore,
    agents: AgentStore,
    projects: ProjectStore,
    areas: AreaStore,
    artifacts: ArtifactStore,
    links: LinkStore,
    runs: RunStore,
    events: EventStore,
    threads: ThreadStore,
    settings: SettingStore,
    activity: ActivityLog,
    plan: PlanService,
    clock: Arc<dyn Clock>,
    zone: TimeZone,
    files: ArtifactFiles,
    workers: Arc<dyn TaskWorkers>,
    /// The areas that may take the person's answers first, in the order they were handed
    /// them (they are built after the tasks, over them).
    answers: Arc<std::sync::RwLock<Vec<Arc<dyn AnswerTaker>>>>,
}

/// The days of the weekly report's chart.
const REPORT_WEEKS: i64 = 8;

impl TaskService {
    /// A service over `parts`.
    #[must_use]
    pub fn new(parts: TaskParts) -> Self {
        let TaskParts {
            db,
            clock,
            zone,
            plan,
            activity,
            files,
            workers,
        } = parts;
        Self {
            tasks: db.tasks(),
            agents: db.agents(),
            projects: db.projects(),
            areas: db.areas(),
            artifacts: db.artifacts(),
            links: db.links(),
            runs: db.runs(),
            events: db.events(),
            threads: db.threads(),
            settings: db.settings(),
            activity,
            plan,
            clock,
            zone,
            files,
            workers,
            answers: Arc::default(),
        }
    }

    /// Lets `taker` take the person's answers on tasks first, after the areas handed them
    /// before it: the first that takes an answer ends its handling (each one before it heard
    /// it too).
    pub fn hand_answers_to(&self, taker: Arc<dyn AnswerTaker>) {
        self.answers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(taker);
    }

    fn takers(&self) -> Vec<Arc<dyn AnswerTaker>> {
        self.answers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Whether an area the answers go to took the person's `choice` on `id`.
    async fn answer_taken(&self, id: TaskId, choice: &str) -> bool {
        for taker in self.takers() {
            if taker.decide(id, choice).await {
                return true;
            }
        }
        false
    }

    /// Whether an area the answers go to took the person's reply `text` on `id`.
    async fn reply_taken(&self, id: TaskId, text: &str) -> bool {
        for taker in self.takers() {
            if taker.message(id, text).await {
                return true;
            }
        }
        false
    }

    /// A task's status; none when there is no such task.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn status_of(&self, id: TaskId) -> Result<Option<Status>, TaskError> {
        Ok(self.tasks.get(id).await?.map(|task| task.status))
    }

    /// Dates done task `id`'s finish `at`, the moment it really happened (a walk logged from the
    /// phone's day); false when there is no such task or it isn't done.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn done_on(&self, id: TaskId, at: Timestamp) -> Result<bool, TaskError> {
        Ok(self.tasks.done_on(id, at).await?)
    }

    /// A task's history, oldest first.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn history_of(&self, id: TaskId) -> Result<Vec<emberhut_db::HistoryLine>, TaskError> {
        Ok(self.tasks.history(id).await?)
    }

    /// The server's time zone.
    #[must_use]
    pub fn zone(&self) -> &TimeZone {
        &self.zone
    }

    /// The findings filed under a scan.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn findings_of(&self, scan: TaskId) -> Result<Vec<Task>, TaskError> {
        Ok(self.tasks.findings(scan).await?)
    }

    fn now(&self) -> Timestamp {
        self.clock.now()
    }

    fn iso(&self) -> String {
        timestamp_to_api(self.now()).as_str().to_owned()
    }

    /// Tells the owner's assistant `text` in Main, news about no one task (a project, the
    /// plan), once per `key` when one is given.
    async fn tell(&self, kind: &str, text: String, key: Option<String>) -> Result<(), TaskError> {
        self.tell_in(RoomId::MAIN, kind, text, key)
            .await
            .map(|_| ())
    }

    /// Tells whoever `task` is for about it, once per `key` when one is given: the owner's
    /// assistant in Main (`text`) for the owner's task; for a configured agent's task, that
    /// agent in its own room, a neutral line saying the task `agent_news` (naming only the
    /// agent's own tools), never Main, so nothing of an agent's task (its title, what its worker
    /// wrote) reaches the owner's session; nobody for a task no one owns. Whether it was told
    /// now (`false`: its key was used already, or nobody is told).
    async fn tell_about(
        &self,
        task: &Task,
        kind: &str,
        text: String,
        agent_news: &str,
        key: Option<String>,
    ) -> Result<bool, TaskError> {
        match scope::task_owner(&self.tasks, &self.agents, task.id).await? {
            TaskOwner::Owner => self.tell_in(RoomId::MAIN, kind, text, key).await,
            TaskOwner::Agent(agent) => {
                let news = format!(
                    "Your task {} \"{}\" {agent_news}. Read it with tasks (its id) if you need to.",
                    task.key(),
                    task.title
                );
                self.tell_in(agent.room, kind, news, key).await
            }
            TaskOwner::Orphaned => {
                tracing::info!(task = %task.key(), "a task no one owns: nobody is told");
                Ok(false)
            }
        }
    }

    /// Adds `text` as a `kind` event in `room`, once per `key` when one is given; whether it
    /// was added now.
    async fn tell_in(
        &self,
        room: RoomId,
        kind: &str,
        text: String,
        key: Option<String>,
    ) -> Result<bool, TaskError> {
        let mut event = NewEvent::new(room, kind, text);
        event.key = key;
        Ok(self.events.add(event, self.now()).await?.is_some())
    }

    async fn task(&self, id: TaskId) -> Result<Task, TaskError> {
        self.tasks
            .get(id)
            .await?
            .ok_or_else(|| TaskRefusal::NoTask(id).into())
    }

    /// Tasks as the contract writes them: with their project's name, rollup, artifacts (the
    /// files still there) and live state, and their board place when `board` has them.
    async fn shape(
        &self,
        tasks: &[Task],
        board: Option<&Board>,
    ) -> Result<Vec<api::tasks::Issue>, TaskError> {
        let family = self.tasks.family().await?;
        self.shape_with(tasks, board, &family).await
    }

    async fn shape_with(
        &self,
        tasks: &[Task],
        board: Option<&Board>,
        family: &Family,
    ) -> Result<Vec<api::tasks::Issue>, TaskError> {
        let ids: Vec<TaskId> = tasks.iter().map(|t| t.id).collect();
        let names: BTreeMap<ProjectId, String> = self
            .projects
            .list()
            .await?
            .into_iter()
            .map(|p| (p.id, p.name))
            .collect();
        let artifacts = self.artifacts.of(&ids).await?;
        // Which files are still there, looked up off the runtime's threads, all at once.
        let files: Vec<String> = artifacts
            .values()
            .flatten()
            .filter(|a| a.kind != "link" && a.kind != emberhut_db::X_REPLY)
            .map(|a| a.reference.clone())
            .collect();
        let there: BTreeSet<String> =
            futures_util::future::join_all(files.into_iter().map(|reference| async move {
                tokio::fs::try_exists(&reference)
                    .await
                    .unwrap_or(false)
                    .then_some(reference)
            }))
            .await
            .into_iter()
            .flatten()
            .collect();
        let runs = self.runs.latest(&ids).await?;
        let parent_ids: Vec<TaskId> = tasks
            .iter()
            .filter_map(|t| t.parent)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let parents: BTreeMap<TaskId, Task> = self
            .tasks
            .many(&parent_ids)
            .await?
            .into_iter()
            .map(|t| (t.id, t))
            .collect();
        let asked = self.asked_for(tasks, family).await?;
        let sends = self.send_results().await?;
        let now = self.now();
        Ok(tasks
            .iter()
            .map(|t| {
                let (open, last) = runs.get(&t.id).cloned().unwrap_or_default();
                let (requested, requested_by) = asked.get(&t.id).cloned().unwrap_or_default();
                let shown = artifacts.get(&t.id).map_or_default(|list| {
                    list.iter()
                        .filter(|a| {
                            a.kind == "link"
                                || a.kind == emberhut_db::X_REPLY
                                || there.contains(&a.reference)
                        })
                        .map(shape::artifact)
                        .collect()
                });
                shape::issue(
                    t,
                    shape::Extras {
                        project: t.pid.and_then(|pid| names.get(&pid).cloned()),
                        sub: family.rollup(t.id),
                        artifacts: shown,
                        live: Some(live_of(
                            t,
                            open.as_ref(),
                            last.as_ref(),
                            t.parent.and_then(|p| parents.get(&p)),
                            now,
                        )),
                        scored: board.and_then(|b| b.scores.get(&t.id).copied()),
                        requested,
                        requested_by,
                        send: sends.get(&t.id.get().to_string()).map(send_to_api),
                    },
                )
            })
            .collect())
    }

    /// Who asked for each of `tasks`: the person's request and someone else's, each its own or
    /// else its nearest ancestor's that has one.
    async fn asked_for(
        &self,
        tasks: &[Task],
        family: &Family,
    ) -> Result<BTreeMap<TaskId, Asked>, TaskError> {
        let ancestors: BTreeMap<TaskId, Vec<TaskId>> = tasks
            .iter()
            .filter(|t| t.requested.is_none() || t.requested_by.is_none())
            .map(|t| (t.id, family.ancestors(t.id)))
            .collect();
        let wanted: Vec<TaskId> = ancestors
            .values()
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let above: BTreeMap<TaskId, Task> = self
            .tasks
            .many(&wanted)
            .await?
            .into_iter()
            .map(|t| (t.id, t))
            .collect();
        Ok(tasks
            .iter()
            .map(|t| {
                let line = ancestors.get(&t.id).map_or(&[][..], Vec::as_slice);
                let up = || line.iter().filter_map(|id| above.get(id));
                let requested = t
                    .requested
                    .as_deref()
                    .or_else(|| up().find_map(|a| a.requested.as_deref()))
                    .and_then(|text| parse_or_log(t.id, "requested", text));
                let requested_by = t
                    .requested_by
                    .as_deref()
                    .or_else(|| up().find_map(|a| a.requested_by.as_deref()))
                    .and_then(|text| parse_or_log(t.id, "requested_by", text));
                (t.id, (requested, requested_by))
            })
            .collect())
    }

    /// The person's last tap on each task's send card, by task id as text. A value that is not
    /// of its shape is logged and read as none, so the tasks still show.
    async fn send_results(&self) -> Result<BTreeMap<String, SendResult>, TaskError> {
        match self
            .settings
            .get_or(&send_tap::RESULTS, BTreeMap::new())
            .await
        {
            Ok(results) => Ok(results),
            Err(DbError::Setting { key, source }) => {
                tracing::warn!(%key, %source, "the send taps are not of their shape");
                Ok(BTreeMap::new())
            }
            Err(error) => Err(error.into()),
        }
    }

    /// The task `id` as the contract writes it.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn issue(&self, id: TaskId) -> Result<Option<api::tasks::Issue>, TaskError> {
        let Some(task) = self.tasks.get(id).await? else {
            return Ok(None);
        };
        Ok(self.shape(&[task], None).await?.into_iter().next())
    }

    /// The board: every project's health, every listed task's score, and Today; with the
    /// listed tasks (canceled ones left out), the projects and the tree.
    async fn board(&self) -> Result<(Board, Vec<Task>, Vec<Project>, Family), TaskError> {
        let now = self.now();
        let since = week_start(now, &self.zone);
        let listed: Vec<Task> = self
            .tasks
            .listed(since, now)
            .await?
            .into_iter()
            .filter(|t| t.status != Status::Canceled)
            .collect();
        let projects = self.projects.list().await?;
        let family = self.tasks.family().await?;
        let waits_below: BTreeSet<TaskId> = listed
            .iter()
            .filter(|t| family.rollup(t.id).is_some_and(|r| !r.waiting.is_empty()))
            .map(|t| t.id)
            .collect();
        let board = board::board(
            &projects,
            &listed,
            &waits_below,
            now,
            &self.zone,
            FOCUS_HOURS,
        );
        Ok((board, listed, projects, family))
    }

    /// Everything the state shows of the tasks and the plan.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn plan_state(&self) -> Result<PlanState, TaskError> {
        let (board, listed, projects, family) = self.board().await?;
        let issues = self.shape_with(&listed, Some(&board), &family).await?;
        let lines: BTreeMap<ProjectId, &board::ProjectLine> =
            board.projects.iter().map(|l| (l.id, l)).collect();
        let api_projects: Vec<api::tasks::Project> = projects
            .iter()
            .map(|p| shape::project(p, lines.get(&p.id).copied()))
            .collect();
        let goal_of: BTreeMap<ProjectId, i64> = projects
            .iter()
            .filter_map(|p| Some((p.id, p.goal?.get())))
            .collect();
        let steps: Vec<&Task> = listed.iter().filter(|t| t.parent.is_none()).collect();
        let todo = |t: &Task| api::plan::Todo {
            id: shape::task_to_api(t.id),
            goal: t
                .goal
                .map(emberhut_core::task::GoalId::get)
                .or_else(|| t.pid.and_then(|pid| goal_of.get(&pid).copied()))
                .map(api::ids::GoalId::new),
            text: t.title.clone(),
            who: t.owner.as_str().to_owned(),
            today: t.today,
            done: (t.status == Status::Done)
                .then(|| t.done_at.map(timestamp_to_api))
                .flatten(),
            parked_until: t.parked_until.clone().map(api::time::Timestamp::from),
            at: timestamp_to_api(t.updated),
        };
        let mut open: Vec<api::plan::Todo> = steps
            .iter()
            .filter(|t| t.status != Status::Done)
            .map(|t| todo(t))
            .collect();
        let mut done: Vec<api::plan::Todo> = steps
            .iter()
            .filter(|t| t.status == Status::Done)
            .map(|t| todo(t))
            .collect();
        done.sort_by(|a, b| b.done.cmp(&a.done));
        let goals: Vec<api::tasks::Goal> = self
            .plan
            .goals()
            .await?
            .into_iter()
            .map(|mut g| {
                let count = |owner: Owner| {
                    u32::try_from(
                        done.iter()
                            .filter(|t| t.goal == Some(g.id) && t.who == owner.as_str())
                            .count(),
                    )
                    .unwrap_or(u32::MAX)
                };
                g.week = api::plan::GoalWeek(
                    [Owner::Person, Owner::Assistant]
                        .into_iter()
                        .map(|owner| (owner.as_str().to_owned(), count(owner)))
                        .collect(),
                );
                g
            })
            .collect();
        open.extend(done);
        let report = self.report(&api_projects, &goals).await?;
        Ok(PlanState {
            issues,
            today: board
                .today
                .iter()
                .map(|id| shape::task_to_api(*id))
                .collect(),
            projects: api_projects,
            goals,
            todos: open,
            week_start: Some(timestamp_to_api(week_start(self.now(), &self.zone))),
            areas: self
                .area_trees(None)
                .await?
                .iter()
                .map(shape::area)
                .collect(),
            report: Some(report),
        })
    }

    /// The weekly report: the concern, each goal with the projects that moved it this week,
    /// the week's done top-level tasks, and how many were done in each of the last 8 weeks.
    async fn report(
        &self,
        projects: &[api::tasks::Project],
        goals: &[api::tasks::Goal],
    ) -> Result<api::plan::WeekReport, TaskError> {
        let start = week_start(self.now(), &self.zone);
        let week = SignedDuration::from_hours(24 * 7);
        let mut weeks = Vec::new();
        let mut starts = Vec::new();
        for back in (0..REPORT_WEEKS).rev() {
            let from = start - week * i32::try_from(back).unwrap_or(0);
            let count = self.tasks.count_done(from, from + week).await?;
            weeks.push(u32::try_from(count).unwrap_or(u32::MAX));
            starts.push(timestamp_to_api(from));
        }
        let done = self.tasks.done_since(start).await?;
        let project_of = |t: &Task| {
            t.pid
                .and_then(|pid| projects.iter().find(|p| p.id.get() == pid.get()))
        };
        Ok(api::plan::WeekReport {
            concern: self.plan.concern().await?,
            weeks,
            week_starts: Some(starts),
            since: self.tasks.first_created().await?.map(timestamp_to_api),
            goals: goals
                .iter()
                .map(|g| api::plan::ReportGoal {
                    id: g.id,
                    title: g.title.clone(),
                    short: g.short.clone(),
                    done: u32::try_from(
                        done.iter()
                            .filter(|t| {
                                t.goal.is_some_and(|goal| goal.get() == g.id.get())
                                    || project_of(t).is_some_and(|p| p.goal == Some(g.id))
                            })
                            .count(),
                    )
                    .unwrap_or(u32::MAX),
                    projects: projects
                        .iter()
                        .filter(|p| p.goal == Some(g.id))
                        .map(|p| api::plan::ReportProject {
                            id: p.id,
                            name: p.name.clone(),
                            done_week: p.done_week,
                            open: p.open,
                            health: p.health.clone(),
                        })
                        .collect(),
                })
                .collect(),
            done: done
                .iter()
                .take(30)
                .map(|t| api::plan::ReportDone {
                    id: shape::task_to_api(t.id),
                    title: t.title.clone(),
                    owner: t.owner.as_str().to_owned(),
                    project: project_of(t).map(|p| p.name.clone()),
                })
                .collect(),
        })
    }

    /// Any task, done and canceled ones too, as the app searches and filters them.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn search(&self, query: &TasksQuery) -> Result<Vec<api::tasks::Issue>, TaskError> {
        let number = |text: &Option<String>| {
            text.as_deref()
                .and_then(|t| t.trim().parse::<f64>().ok())
                .filter(|n| n.is_finite() && *n != 0.0)
        };
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a limit or an id the app sends as a number, truncated"
        )]
        let whole = |n: f64| n.trunc() as i64;
        let mut search = TaskSearch::new(query.q.as_deref().unwrap_or_default())
            .limited(number(&query.limit).map(|n| u64::try_from(whole(n)).unwrap_or(0)));
        search.project = if query.project.as_deref() == Some("none") {
            Some(None)
        } else {
            number(&query.project).map(|n| Some(ProjectId::new(whole(n))))
        };
        search.area = number(&query.area).map(whole);
        search.statuses = query
            .status
            .as_deref()
            .unwrap_or_default()
            .split(',')
            .filter_map(|word| Status::parse(word.trim()))
            .collect();
        search.owner = query.owner.as_deref().and_then(Owner::parse);
        search.closed = query.closed.as_deref() == Some("1");
        search.archived = query.archived.as_deref() == Some("1");
        let found = self.tasks.search(&search).await?;
        self.shape(&found, None).await
    }

    /// A task with everything its page shows.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn detail(&self, id: TaskId) -> Result<Option<api::task_page::TaskPage>, TaskError> {
        let Some(task) = self.tasks.get(id).await? else {
            return Ok(None);
        };
        let family = self.tasks.family().await?;
        let subtasks = self.tasks.subtasks(id).await?;
        let findings = self.tasks.findings(id).await?;
        let mut shaped = self.shape_with(&[task], None, &family).await?.into_iter();
        let Some(task) = shaped.next() else {
            return Ok(None);
        };
        let history = self
            .tasks
            .history(id)
            .await?
            .into_iter()
            .map(|line| api::task_page::Moment {
                at: timestamp_to_api(line.at),
                who: line.who.as_str().to_owned(),
                text: line.text,
            })
            .collect();
        let mut links = Vec::new();
        for link in self.links.of(id).await? {
            let thread = match (&link.machine, link.kind.as_str()) {
                (Some(machine), "thread") => self.threads.get(machine, &link.reference).await?,
                _ => None,
            };
            links.push(api::task_page::TaskLink {
                title: thread.as_ref().map_or_else(
                    || {
                        if link.kind == "thread" {
                            "A thread".to_owned()
                        } else {
                            link.reference.clone()
                        }
                    },
                    |t| t.title.clone(),
                ),
                status: thread
                    .as_ref()
                    .map(|t| t.status.as_str().to_owned())
                    .or_else(|| (link.kind == "thread").then(|| "unknown".to_owned())),
                at: thread.as_ref().map(|t| timestamp_to_api(t.updated_at)),
                line: thread
                    .as_ref()
                    .and_then(|t| t.last.as_deref())
                    .and_then(live::plain_line),
                kind: api::task_page::LinkKind::from(link.kind),
                reference: link.reference,
                machine: link.machine.map(api::ids::MachineId::new),
            });
        }
        let runs = self
            .runs
            .of_task(id, 40)
            .await?
            .into_iter()
            .map(|run| {
                let clip = |text: Option<&str>, n: usize| {
                    text.map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
                        .map(|t| t.chars().take(n).collect::<String>())
                        .filter(|t| !t.is_empty())
                };
                let asked = run.asked.as_deref().map(live::asked_text);
                let answer = run.last.as_deref().map(live::answer_text);
                api::task_page::Run {
                    id: api::ids::RunId::new(run.id.get()),
                    kind: run.kind.as_str().to_owned(),
                    // A runner session's title is its thread's, which the runners' area keeps:
                    // it waits for that area (a worker's run has none anyway).
                    title: None,
                    asked: clip(asked.as_deref(), 240),
                    answer: clip(answer.as_deref(), 280),
                    status: run
                        .last
                        .as_deref()
                        .and_then(live::parse_status)
                        .map(|s| s.raw)
                        .or(run.status_line),
                    requested: run.requested.map(timestamp_to_api),
                    started: run.started.map(timestamp_to_api),
                    ended: run.ended.map(timestamp_to_api),
                    outcome: run.outcome,
                    machine: api::ids::MachineId::new(run.machine),
                    thread: run.thread,
                }
            })
            .collect();
        Ok(Some(api::task_page::TaskPage {
            task,
            history,
            subtasks: self.shape_with(&subtasks, None, &family).await?,
            findings: self.shape_with(&findings, None, &family).await?,
            links,
            runs: Some(runs),
            sessions: Vec::new(),
            ultra_run: None,
        }))
    }

    /// A file a task produced, opened, and its content type, when it may be served: its path
    /// is resolved and checked again now, and the file opened is the one checked.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn artifact_file(
        &self,
        id: emberhut_core::task::ArtifactId,
    ) -> Result<Option<(tokio::fs::File, &'static str)>, TaskError> {
        let Some(found) = self.artifacts.get(id).await? else {
            return Ok(None);
        };
        if found.kind == "link" || found.kind == emberhut_db::X_REPLY {
            return Ok(None);
        }
        Ok(self.files.open(&found.reference).await)
    }

    /// The thumbnail of a picture or a video a task produced, made once.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn artifact_thumb(
        &self,
        id: emberhut_core::task::ArtifactId,
    ) -> Result<Option<std::path::PathBuf>, TaskError> {
        let Some(found) = self.artifacts.get(id).await? else {
            return Ok(None);
        };
        if !matches!(found.kind.as_str(), "image" | "video") {
            return Ok(None);
        }
        let Some(source) = self.files.resolve(&found.reference).await else {
            return Ok(None);
        };
        match self
            .files
            .thumbnail(id.get(), &source, found.kind == "video")
            .await
        {
            Ok(path) => Ok(Some(path)),
            Err(error) => {
                tracing::warn!(%error, artifact = id.get(), "no thumbnail");
                Ok(None)
            }
        }
    }

    /// Makes a task, as `by` asked; the person's own add is their request.
    ///
    /// # Errors
    ///
    /// [`TaskError`].
    pub async fn create(&self, mut new: NewTask, by: Who) -> Result<Task, TaskError> {
        emberhut_core::task::check_lengths(Some(&new.title), new.body.as_deref())?;
        if by == Who::Person {
            new.requested = Some(self.requested());
        }
        let made = self.tasks.create(new, by, self.now()).await?;
        self.activity
            .note(format!("Task {}: {}", made.key(), made.title))
            .await?;
        Ok(made)
    }

    fn requested(&self) -> String {
        serde_json::json!({ "at": self.iso(), "msg": null, "room": null }).to_string()
    }

    /// Changes a task, as `by` asked.
    ///
    /// # Errors
    ///
    /// [`TaskError`].
    pub async fn update(
        &self,
        id: TaskId,
        change: TaskChange,
        by: Who,
        said: Option<&str>,
    ) -> Result<Task, TaskError> {
        emberhut_core::task::check_lengths(
            change.title.as_deref(),
            change.body.as_ref().and_then(Option::as_deref),
        )?;
        Ok(self.tasks.update(id, change, by, said, self.now()).await?)
    }

    /// Adds a line to a task's history; the files and links it names become its artifacts
    /// (`files` false: links only).
    ///
    /// # Errors
    ///
    /// [`TaskError`].
    pub async fn note(
        &self,
        id: TaskId,
        who: Who,
        text: &str,
        files: bool,
    ) -> Result<(), TaskError> {
        // A key or token in it is masked: the history is shown in the apps and read by the
        // assistant.
        let text = &emberhut_core::guard::key_mask::mask(text, &[], None);
        if !self.tasks.note(id, who, text, self.now()).await? {
            return Err(TaskRefusal::NoTask(id).into());
        }
        self.scan(id, text, files).await
    }

    async fn scan(&self, id: TaskId, text: &str, files: bool) -> Result<(), TaskError> {
        for reference in self.files.named_in(text, files).await {
            if let Some((reference, kind)) = self.files.attachable(&reference).await {
                self.artifacts
                    .attach(id, &reference, kind, None, self.now())
                    .await?;
            }
        }
        Ok(())
    }

    /// Attaches each of `refs` (a file on this machine or a link) to the task; the answer
    /// names those that couldn't be.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn attach(&self, id: TaskId, refs: &[String]) -> Result<String, TaskError> {
        let mut off = Vec::new();
        for reference in refs {
            match self.files.attachable(reference).await {
                Some((path, kind)) => {
                    self.artifacts
                        .attach(id, &path, kind, None, self.now())
                        .await?;
                }
                None => off.push(reference.clone()),
            }
        }
        Ok(if off.is_empty() {
            String::new()
        } else {
            format!(
                " Not attached (missing, private, or not something the app opens): {}.",
                off.join(", ")
            )
        })
    }

    /// The artifact a file or link became on the task, if it did.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn attached_as(
        &self,
        id: TaskId,
        reference: &str,
    ) -> Result<Option<emberhut_core::task::ArtifactId>, TaskError> {
        let Some((path, _)) = self.files.attachable(reference).await else {
            return Ok(None);
        };
        Ok(self.artifacts.find(id, &path).await?.map(|a| a.id))
    }

    /// Links work to a task: a thread (on its machine), a ticket or a chat.
    ///
    /// # Errors
    ///
    /// [`TaskError`].
    pub async fn link(
        &self,
        id: TaskId,
        kind: &str,
        reference: &str,
        machine: Option<&str>,
    ) -> Result<(), TaskError> {
        self.task(id).await?;
        self.links.link(id, kind, reference, machine).await?;
        Ok(())
    }

    /// Takes a link off a task.
    ///
    /// # Errors
    ///
    /// [`TaskError::Db`].
    pub async fn unlink(&self, id: TaskId, reference: &str) -> Result<(), TaskError> {
        self.links.unlink(id, reference).await?;
        Ok(())
    }

    /// Starts a worker on the assistant's task now.
    ///
    /// # Errors
    ///
    /// [`TaskError::Refused`] with the workers' reason.
    pub async fn start_worker(&self, id: TaskId) -> Result<(), TaskError> {
        let task = self.task(id).await?;
        if task.owner != Owner::Assistant
            || task.body.as_deref().is_none_or(|b| b.trim().is_empty())
        {
            return Err(TaskError::refused(format!(
                "{} needs a brief (a body) and to be the assistant's",
                task.key()
            )));
        }
        self.workers.start(id).await.map_err(TaskError::Refused)
    }

    /// Stops the worker on a task: its session stops (one gone already is no matter), its run
    /// closes, and a task still marked working goes back to the assistant's to do.
    ///
    /// # Errors
    ///
    /// [`TaskError`].
    pub async fn stop_worker(&self, id: TaskId) -> Result<(), TaskError> {
        let task = self.task(id).await?;
        if let Some(unit) = task.worker.as_deref() {
            if let Err(error) = self.workers.stop(id).await {
                tracing::warn!(%error, task = id.get(), "the worker didn't stop; gone already?");
            }
            self.worker_ended(unit).await?;
        }
        if task.status == Status::Working {
            self.update(
                id,
                TaskChange {
                    status: Some(Status::Todo),
                    owner: Some(Owner::Assistant),
                    ..TaskChange::default()
                },
                Who::Person,
                None,
            )
            .await?;
        }
        self.tasks.set_worker(id, None, None).await?;
        Ok(())
    }
}

/// Who asked for a task: the person's request and someone else's.
type Asked = (
    Option<api::tasks::Requested>,
    Option<api::tasks::RequestedBy>,
);

/// `text`, a task's stored JSON, read as `T`; a value that is not is logged and read as none.
fn parse_or_log<T: serde::de::DeserializeOwned>(task: TaskId, what: &str, text: &str) -> Option<T> {
    serde_json::from_str(text)
        .inspect_err(|error| tracing::warn!(task = task.get(), what, %error, "not of its shape"))
        .ok()
}

/// A send tap's result as the contract writes it.
fn send_to_api(result: &SendResult) -> api::tasks::TaskSend {
    api::tasks::TaskSend {
        state: result.state.clone(),
        at: api::time::Timestamp::new(result.at.clone()),
        why: result.why.clone(),
        via: result.via.clone(),
        to: result.to.clone(),
    }
}
