//! The tasks' tools: the board as trees (`tasks`), one task in full (`task`), making and
//! changing tasks (`task_create`, `task_update`, `task_progress`), their history (`task_note`),
//! the one decision a task needs (`task_ask`), a scan's findings (`task_findings`), finishing
//! one (`task_done`), its worker (`task_work`, `task_stop`), and the projects and areas they
//! are grouped by (`projects`, `project_set`, `area_set`).
//!
//! A worker's session calls the same tools with its own task's id. The owners' words in the
//! arguments are `owner` and `assistant`.

#![expect(
    clippy::option_option,
    reason = "a partial change: left out is unchanged, null clears"
)]

use emberhut_core::task::area::AreaStatus;
use emberhut_core::task::ask::{CardIn, OptionIn};
use emberhut_core::task::board::{ProjectChange, ProjectKind, ProjectStatus};
use emberhut_core::task::finding::FindingIn;
use emberhut_core::task::{
    AreaId, GoalId, Landed, Owner, ProjectId, Size, Status, TaskChange, TaskId, Who, date_only, day,
};
use emberhut_db::{AreaChange, NewTask};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::{NoArgs, Tool, ToolContext, ToolError, ToolOutput};
use crate::services::tasks::{AskInput, ProgressItem, TaskError};

impl From<TaskError> for ToolError {
    fn from(error: TaskError) -> Self {
        match error {
            TaskError::Refused(text) => Self::refused(text),
            other => Self::failed(other),
        }
    }
}

/// For `deserialize_with` on an `Option<Option<T>>` with `#[serde(default)]`: left out is
/// `None` (unchanged), `null` is `Some(None)` (cleared).
pub(crate) fn patch<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Some(Option::deserialize(deserializer)?))
}

/// Whose a task is: the person's (`owner`) or the assistant's (`assistant`).
#[derive(Clone, Copy, Debug, Deserialize, utoipa::ToSchema)]
pub(crate) enum OwnerArg {
    /// The person's own to do.
    #[serde(rename = "owner")]
    Person,
    /// The assistant's work.
    #[serde(rename = "assistant")]
    Assistant,
}

impl From<OwnerArg> for Owner {
    fn from(owner: OwnerArg) -> Self {
        match owner {
            OwnerArg::Person => Self::Person,
            OwnerArg::Assistant => Self::Assistant,
        }
    }
}

/// A task's status.
#[derive(Clone, Copy, Debug, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum StatusArg {
    /// Not started.
    Todo,
    /// Being worked on (or discussed).
    Working,
    /// Waiting on the person (needs an action).
    Waiting,
    /// Done.
    Done,
    /// Put away for good.
    Canceled,
}

impl From<StatusArg> for Status {
    fn from(status: StatusArg) -> Self {
        match status {
            StatusArg::Todo => Self::Todo,
            StatusArg::Working => Self::Working,
            StatusArg::Waiting => Self::Waiting,
            StatusArg::Done => Self::Done,
            StatusArg::Canceled => Self::Canceled,
        }
    }
}

/// A rough size of the person's time: S about an hour, M three, L a day, XL several days.
#[derive(Clone, Copy, Debug, Deserialize, utoipa::ToSchema)]
pub(crate) enum SizeArg {
    /// About an hour.
    S,
    /// About three hours.
    M,
    /// About a day.
    L,
    /// Several days.
    #[serde(rename = "XL")]
    Xl,
}

impl From<SizeArg> for Size {
    fn from(size: SizeArg) -> Self {
        match size {
            SizeArg::S => Self::S,
            SizeArg::M => Self::M,
            SizeArg::L => Self::L,
            SizeArg::Xl => Self::Xl,
        }
    }
}

/// Whether a finding landed.
#[derive(Clone, Copy, Debug, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LandedArg {
    /// It landed.
    Landed,
    /// Partly landed.
    Partly,
    /// Not landed.
    NotLanded,
    /// Made moot by later work.
    Superseded,
    /// Not checked against the work yet.
    Unchecked,
}

impl From<LandedArg> for Landed {
    fn from(landed: LandedArg) -> Self {
        match landed {
            LandedArg::Landed => Self::Landed,
            LandedArg::Partly => Self::Partly,
            LandedArg::NotLanded => Self::NotLanded,
            LandedArg::Superseded => Self::Superseded,
            LandedArg::Unchecked => Self::Unchecked,
        }
    }
}

/// A state the `tasks` filter takes.
#[derive(Clone, Copy, Debug, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StateArg {
    /// Waits on the person.
    NeedsYou,
    /// Under way.
    Working,
    /// Not started.
    NotStarted,
    /// Done.
    Done,
}

impl StateArg {
    const fn word(self) -> &'static str {
        match self {
            Self::NeedsYou => "needs_you",
            Self::Working => "working",
            Self::NotStarted => "not_started",
            Self::Done => "done",
        }
    }
}

fn whole(id: i64) -> Option<i64> {
    (id != 0).then_some(id)
}

fn days(text: Option<&str>) -> Result<Option<jiff::civil::Date>, ToolError> {
    day(text).map_err(ToolError::refused)
}

/// The person's tasks as trees.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Tasks;

/// `tasks`' arguments.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct TasksArgs {
    /// One task with its full history, subtasks, findings and links.
    #[garde(skip)]
    id: Option<i64>,
    /// An area: its id, or its name with `project_id`.
    #[garde(length(max = 200))]
    area: Option<String>,
    /// Only this project (0: tasks with no project).
    #[garde(skip)]
    project_id: Option<i64>,
    /// The flat JSON list (every field), subtasks under their parents, and Today.
    #[garde(skip)]
    full: Option<bool>,
    /// Only tasks in these states (a task shows when it matches or something under it does).
    #[garde(skip)]
    status: Option<Vec<StateArg>>,
    /// Only the person's (`owner`) or the assistant's (`assistant`).
    #[garde(skip)]
    owner: Option<OwnerArg>,
}

impl Tool for Tasks {
    type Args = TasksArgs;

    fn name(&self) -> &'static str {
        "tasks"
    }

    fn description(&self) -> &'static str {
        "The user's tasks as trees: each project with open work, its areas (project > area > tasks > \
         subtasks, any depth) and its tasks outside them, one line per task with its state rolled up \
         from its leaves (! needs the user, ~ working, ○ not started, ✓ done; done work folded into a \
         count). project_id: only that project. area: one area in full, with every task and the \
         findings not landed. id: one task with its full history, subtasks, findings and links. \
         status (needs_you, working, not_started, done) and owner (owner: the user's own, assistant: \
         yours) filter it. full: the flat JSON list with every field and Today; it's big."
    }

    async fn call(&self, ctx: &ToolContext, args: TasksArgs) -> Result<ToolOutput, ToolError> {
        let tasks = ctx.app.tasks();
        if let Some(id) = args.id.and_then(whole) {
            return ToolOutput::json(&tasks.task_json(TaskId::new(id)).await?);
        }
        let states: Vec<&str> = args.status.iter().flatten().map(|s| s.word()).collect();
        let pid = args.project_id.map(|p| whole(p).map(ProjectId::new));
        let owner = args.owner.map(Owner::from);
        let mut filter = crate::services::tasks::TaskService::filter(pid, None, &states, owner)?;
        if let Some(which) = args.area.as_deref().filter(|a| !a.trim().is_empty()) {
            let Some(area) = tasks.find_area(which, pid.flatten()).await? else {
                return Ok(ToolOutput::text(format!("No area \"{which}\".")));
            };
            if states.is_empty() && owner.is_none() {
                return Ok(ToolOutput::text(tasks.area_outline(area).await?));
            }
            filter.pid = None;
            filter.area = Some(area);
        }
        if args.full == Some(true) {
            return ToolOutput::json(&tasks.full_list().await?);
        }
        Ok(ToolOutput::text(tasks.overview(&filter).await?))
    }
}

/// One task in full.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TaskTool;

/// `task`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct TaskArgs {
    /// The task.
    #[garde(range(min = 1))]
    id: i64,
}

impl Tool for TaskTool {
    type Args = TaskArgs;

    fn name(&self) -> &'static str {
        "task"
    }

    fn description(&self) -> &'static str {
        "A task and its full history: its brief, status, ask, subtasks, findings, links and runs. \
         A worker reads its own task with it."
    }

    async fn call(&self, ctx: &ToolContext, args: TaskArgs) -> Result<ToolOutput, ToolError> {
        ToolOutput::json(&ctx.app.tasks().task_json(TaskId::new(args.id)).await?)
    }
}

/// Makes a task.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TaskCreate;

/// `task_create`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct TaskCreateArgs {
    /// A few words.
    #[garde(length(min = 1, max = 500))]
    title: String,
    /// The brief: what to do, what done looks like, what to prepare for the user.
    #[garde(skip)]
    body: Option<String>,
    /// `owner`: the user's to do; `assistant`: work you own (with a brief, a worker picks it up).
    #[garde(skip)]
    owner: OwnerArg,
    /// The project it belongs to.
    #[garde(skip)]
    project_id: Option<i64>,
    /// Its own deadline, a date (2026-10-15).
    #[garde(skip)]
    deadline: Option<String>,
    /// Someone else holds the user to that deadline.
    #[garde(skip)]
    hard: Option<bool>,
    /// S (~1 h), M (~3 h), L (~1 day), XL (several days) of the user's time.
    #[garde(skip)]
    size: Option<SizeArg>,
    /// Pin it to the user's Today (only when they ask).
    #[garde(skip)]
    today: Option<bool>,
    /// Its goal.
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    goal: Option<Option<i64>>,
    /// Where it starts (waiting needs an action).
    #[garde(skip)]
    status: Option<StatusArg>,
    /// Make it a subtask of that task, at any depth (it takes the parent's project, area, goal
    /// and owner).
    #[garde(skip)]
    parent_id: Option<i64>,
    /// Its area in the project (a top-level task's; a subtask takes its parent's).
    #[garde(skip)]
    area_id: Option<i64>,
    /// `finding`: a scan's or review's finding, under the scan (`parent_id`).
    #[garde(length(max = 16))]
    kind: Option<String>,
    /// A finding's state (default unchecked).
    #[garde(skip)]
    landed: Option<LandedArg>,
    /// A finding's: the task that fixes it.
    #[garde(skip)]
    fixed_by: Option<i64>,
    /// Done elsewhere: you only keep its status, no worker takes it. Subtasks inherit it.
    #[garde(skip)]
    tracked: Option<bool>,
    /// Its count so far ("4" of 12 episodes).
    #[garde(range(min = 0))]
    progress: Option<i64>,
    /// What its count goes to.
    #[garde(range(min = 1))]
    total: Option<i64>,
    /// What it counts.
    #[garde(length(max = 40))]
    unit: Option<String>,
    /// Only when it needs the user now: what they should do (it starts waiting).
    #[garde(length(max = 300))]
    action: Option<String>,
    /// With an action: the day the user has to decide by.
    #[garde(skip)]
    decide_by: Option<String>,
    /// Files on this machine or https links it produced.
    #[garde(skip)]
    files: Option<Vec<String>>,
    /// X posts with the replies suggested for each.
    #[garde(skip)]
    x_replies: Option<Vec<Value>>,
    /// Ultra mode: its run splits the job across sub-agents.
    #[garde(skip)]
    ultra: Option<bool>,
    /// The model its work runs on (none: the default).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    model: Option<Option<String>>,
}

impl Tool for TaskCreate {
    type Args = TaskCreateArgs;

    fn name(&self) -> &'static str {
        "task_create"
    }

    fn description(&self) -> &'static str {
        "Create a task. project_id: the project it belongs to. owner: \"owner\" for the user's to \
         do, \"assistant\" for work you own; with a body (the brief: what to do, what done looks \
         like) a worker picks it up and works it in the background. deadline: a date when it has \
         its own; hard: someone else holds the user to it. size: S, M, L, XL of the user's time. \
         today: pin it to Today (only when they ask). parent_id: a subtask of that task, at any \
         depth; progress, total, unit: a count like 4 of 12 episodes. area_id: the area of its \
         project. kind finding: a scan's finding under it (parent_id the scan), with landed and \
         fixed_by. action: only when it needs the user now (it waits, under Needs you), with \
         decide_by."
    }

    async fn call(&self, ctx: &ToolContext, a: TaskCreateArgs) -> Result<ToolOutput, ToolError> {
        let tasks = ctx.app.tasks();
        if matches!(a.status, Some(StatusArg::Waiting))
            && a.action.as_deref().is_none_or(|x| x.trim().is_empty())
        {
            return Ok(ToolOutput::text(
                "Not created: waiting needs an action (what the user should do). Work you're discussing with them is working.",
            ));
        }
        let finding = match a.kind.as_deref() {
            None | Some("") => false,
            Some("finding") => true,
            Some(other) => {
                return Err(ToolError::refused(format!(
                    "kind is \"finding\" or left out, not \"{other}\""
                )));
            }
        };
        let mut pid = a.project_id.and_then(whole).map(ProjectId::new);
        let area = a.area_id.and_then(whole).map(AreaId::new);
        let made = tasks
            .create(
                NewTask {
                    title: a.title.clone(),
                    body: a.body.clone(),
                    owner: Some(a.owner.into()),
                    goal: a.goal.flatten().map(GoalId::new),
                    due: days(a.deadline.as_deref())?,
                    today: a.today.unwrap_or(false),
                    pid: pid.take(),
                    hard: a.hard.unwrap_or(false),
                    size: a.size.map(Size::from),
                    parent: a.parent_id.and_then(whole).map(TaskId::new),
                    progress: a.progress,
                    total: a.total,
                    unit: a.unit.clone(),
                    tracked: a.tracked.unwrap_or(false),
                    area,
                    finding,
                    landed: a.landed.map(Landed::from),
                    fixed_by: a.fixed_by.and_then(whole).map(TaskId::new),
                    ..NewTask::default()
                },
                Who::Assistant,
            )
            .await?;
        let mut later = TaskChange {
            model: a.model.clone().map(|m| m.filter(|m| !m.is_empty())),
            ultra: a.ultra.filter(|u| *u),
            ..TaskChange::default()
        };
        let mut off = tasks
            .attach(made.id, a.files.as_deref().unwrap_or_default())
            .await?;
        if let Some(replies) = &a.x_replies {
            let posts: Vec<Value> = replies
                .iter()
                .flat_map(crate::services::tasks::reply_posts)
                .collect();
            let kept = tasks.attach_replies(made.id, &posts).await?;
            if kept < posts.len() {
                off.push_str(" Some X replies had no post id or no reply.");
            }
        }
        if let Some(action) = a.action.as_deref().filter(|x| !x.trim().is_empty()) {
            later.action = Some(Some(action.to_owned()));
            if let Some(by) = a.decide_by.as_deref().filter(|d| !d.trim().is_empty()) {
                later.decide_by = Some(Some(
                    date_only(by, ctx.app.tasks().zone()).map_err(ToolError::refused)?,
                ));
            }
        } else if let Some(status) = a.status.map(Status::from).filter(|s| *s != Status::Todo) {
            later.status = Some(status);
        }
        let task = if later.is_empty() {
            made
        } else {
            tasks.update(made.id, later, Who::Assistant, None).await?
        };
        let place = match task.area {
            Some(area) => format!(
                " In area {area}{}.",
                task.parent
                    .map_or_default(|p| format!(", under {}", p.key()))
            ),
            None => String::new(),
        };
        Ok(ToolOutput::text(format!(
            "{} created.{place}{off}",
            task.key()
        )))
    }
}

/// A thread to link to a task.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct LinkThread {
    /// The machine it runs on.
    machine: String,
    /// The thread's id.
    thread: String,
}

/// Changes a task.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TaskUpdate;

/// `task_update`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct TaskUpdateArgs {
    /// The task.
    #[garde(range(min = 1))]
    id: i64,
    /// Its status (done when it's done: a step on its goal's ring).
    #[garde(skip)]
    status: Option<StatusArg>,
    /// Its title.
    #[garde(length(min = 1, max = 500))]
    title: Option<String>,
    /// Its brief (giving your task one lets a worker pick it up).
    #[garde(skip)]
    body: Option<String>,
    /// Whose it is.
    #[garde(skip)]
    owner: Option<OwnerArg>,
    /// Its project (null: none).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    project_id: Option<Option<i64>>,
    /// Its own deadline (null: its project's).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    deadline: Option<Option<String>>,
    /// Someone else holds the user to it.
    #[garde(skip)]
    hard: Option<bool>,
    /// Its rough size (null: none).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    size: Option<Option<SizeArg>>,
    /// Its goal (null: none).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    goal: Option<Option<i64>>,
    /// Pinned to Today.
    #[garde(skip)]
    today: Option<bool>,
    /// A date: the user said not now; it leaves their screen until then (null: back now).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    parked_until: Option<Option<String>>,
    /// Put it under any task, at any depth (null takes it out).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    parent_id: Option<Option<i64>>,
    /// Move it and everything under it into that area (null: out of its area).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    area_id: Option<Option<i64>>,
    /// A finding's state.
    #[garde(skip)]
    landed: Option<LandedArg>,
    /// A finding's fixing task (null: none).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    fixed_by: Option<Option<i64>>,
    /// Done elsewhere; subtasks follow.
    #[garde(skip)]
    tracked: Option<bool>,
    /// Its count so far (reaching its total finishes it).
    #[garde(range(min = 0))]
    progress: Option<i64>,
    /// What its count goes to.
    #[garde(range(min = 1))]
    total: Option<i64>,
    /// What it counts.
    #[garde(length(max = 40))]
    unit: Option<String>,
    /// When you expect it done, in plain words (null clears it).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    eta: Option<Option<String>>,
    /// It needs the user: what they should do (it waits); null when they did it.
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    action: Option<Option<String>>,
    /// The day the user has to decide by (null: none).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    decide_by: Option<Option<String>>,
    /// Threads that do this task.
    #[garde(skip)]
    link_threads: Option<Vec<LinkThread>>,
    /// Tickets that move it.
    #[garde(skip)]
    link_tickets: Option<Vec<String>>,
    /// Chats it lives in (their ids).
    #[garde(skip)]
    link_chats: Option<Vec<String>>,
    /// Thread ids, tickets or chats to unlink.
    #[garde(skip)]
    unlink: Option<Vec<String>>,
    /// Files on this machine or https links it produced.
    #[garde(skip)]
    files: Option<Vec<String>>,
    /// X posts with the replies suggested for each.
    #[garde(skip)]
    x_replies: Option<Vec<Value>>,
    /// What does the work when it isn't a turn: `unit:<name>` or `watch:<id>` (null clears).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    runner: Option<Option<String>>,
    /// Ultra mode.
    #[garde(skip)]
    ultra: Option<bool>,
    /// The model its work runs on (null: the default).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    model: Option<Option<String>>,
}

impl TaskUpdateArgs {
    fn change(&self, zone: &jiff::tz::TimeZone) -> Result<TaskChange, ToolError> {
        let runner = match &self.runner {
            Some(Some(runner)) if !valid_runner(runner) => {
                return Err(ToolError::refused(
                    "runner is \"unit:<systemd user unit>\" or \"watch:<id>\"",
                ));
            }
            other => other.clone(),
        };
        Ok(TaskChange {
            status: self.status.map(Status::from),
            title: self.title.clone(),
            body: self.body.clone().map(Some),
            owner: self.owner.map(Owner::from),
            hard: self.hard,
            size: self.size.map(|s| s.map(Size::from)),
            goal: self.goal.map(|g| g.and_then(whole).map(GoalId::new)),
            today: self.today,
            tracked: self.tracked,
            progress: self.progress,
            total: self.total,
            unit: self.unit.clone(),
            eta: self.eta.clone(),
            action: self.action.clone(),
            runner,
            parked_until: self.parked_until.clone(),
            pid: self
                .project_id
                .map(|p| p.and_then(whole).map(ProjectId::new)),
            parent: self.parent_id.map(|p| p.and_then(whole).map(TaskId::new)),
            area: self.area_id.map(|a| a.and_then(whole).map(AreaId::new)),
            landed: self.landed.map(Landed::from),
            fixed_by: self.fixed_by.map(|f| f.and_then(whole).map(TaskId::new)),
            due: self
                .deadline
                .as_ref()
                .map(|d| days(d.as_deref()))
                .transpose()?,
            decide_by: match &self.decide_by {
                Some(Some(text)) if !text.trim().is_empty() => {
                    Some(Some(date_only(text, zone).map_err(ToolError::refused)?))
                }
                Some(_) => Some(None),
                None => None,
            },
            ultra: self.ultra,
            model: self.model.clone().map(|m| m.filter(|m| !m.is_empty())),
            priority: None,
        })
    }
}

fn valid_runner(runner: &str) -> bool {
    let ok = |rest: &str, extra: &str| {
        !rest.is_empty()
            && rest
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || extra.contains(c))
    };
    runner
        .strip_prefix("unit:")
        .is_some_and(|rest| ok(rest, "@.:-"))
        || runner
            .strip_prefix("watch:")
            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

impl Tool for TaskUpdate {
    type Args = TaskUpdateArgs;

    fn name(&self) -> &'static str {
        "task_update"
    }

    fn description(&self) -> &'static str {
        "Change a task: status (done when it's done), title, body (a brief lets a worker pick your \
         task up), owner, project_id, deadline (null: the project's), hard, size, today, \
         parked_until (a date: the user said not now; always give one). link_threads, \
         link_tickets, link_chats: the work that moves it; unlink takes one off. action: it needs \
         the user and this is what they should do (it waits, under Needs you), with decide_by; \
         null when they did it. Waiting needs an action; a task you're only discussing stays \
         working. parent_id: put it under any task (null takes it out); area_id: move it and \
         everything under it. A finding's landed and fixed_by. progress reaching total finishes it."
    }

    async fn call(&self, ctx: &ToolContext, a: TaskUpdateArgs) -> Result<ToolOutput, ToolError> {
        let tasks = ctx.app.tasks();
        let id = TaskId::new(a.id);
        let change = a.change(tasks.zone())?;
        if tasks.issue(id).await?.is_none() {
            return Err(ToolError::refused(format!("No task {}", id.key())));
        }
        let mut off = tasks
            .attach(id, a.files.as_deref().unwrap_or_default())
            .await?;
        if let Some(replies) = &a.x_replies {
            let posts: Vec<Value> = replies
                .iter()
                .flat_map(crate::services::tasks::reply_posts)
                .collect();
            if tasks.attach_replies(id, &posts).await? < posts.len() {
                off.push_str(" Some X replies had no post id or no reply.");
            }
        }
        for link in a.link_threads.iter().flatten() {
            tasks
                .link(id, "thread", &link.thread, Some(&link.machine))
                .await?;
        }
        for ticket in a.link_tickets.iter().flatten() {
            tasks.link(id, "ticket", ticket, None).await?;
        }
        for chat in a.link_chats.iter().flatten() {
            tasks.link(id, "chat", chat, None).await?;
        }
        for reference in a.unlink.iter().flatten() {
            tasks.unlink(id, reference).await?;
        }
        let task = tasks.update(id, change, Who::Assistant, None).await?;
        let action = task
            .action
            .as_deref()
            .map_or_default(|action| format!(" (the user: {action})"));
        let count = match (task.progress, task.total) {
            (Some(progress), Some(total)) if total != 0 => format!(
                " ({progress}/{total} {})",
                task.unit.as_deref().unwrap_or_default()
            )
            .trim_end()
            .to_owned(),
            _ => String::new(),
        };
        let mut place = if a.area_id.is_some() || a.parent_id.is_some() {
            format!(
                " In {}{}.",
                task.area
                    .map_or_else(|| "no area".to_owned(), |area| format!("area {area}")),
                task.parent
                    .map_or_default(|p| format!(", under {}", p.key()))
            )
        } else {
            String::new()
        };
        if task.finding {
            place = format!(
                "{place} Finding: {}{}.",
                task.landed.unwrap_or(Landed::Unchecked).words(),
                task.fixed_by
                    .map_or_default(|f| format!(", fixed by {}", f.key()))
            );
        }
        Ok(ToolOutput::text(format!(
            "{}: {}{action}{count}.{place}{off}",
            task.key(),
            task.status.as_str()
        )))
    }
}

/// One subtask to keep in step.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct ProgressArg {
    /// The subtask.
    #[garde(skip)]
    id: Option<i64>,
    /// Its title (made as a new subtask when none has it).
    #[garde(length(max = 500))]
    title: Option<String>,
    /// Its status.
    #[garde(skip)]
    status: Option<StatusArg>,
    /// Its count so far.
    #[garde(range(min = 0))]
    progress: Option<i64>,
    /// What its count goes to.
    #[garde(range(min = 1))]
    total: Option<i64>,
    /// What it counts.
    #[garde(length(max = 40))]
    unit: Option<String>,
}

/// Keeps many subtasks of one parent in step.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TaskProgress;

/// `task_progress`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct TaskProgressArgs {
    /// The parent.
    #[garde(range(min = 1))]
    parent_id: i64,
    /// Its subtasks, by id or by title.
    #[garde(length(min = 1, max = 100), dive)]
    items: Vec<ProgressArg>,
}

impl Tool for TaskProgress {
    type Args = TaskProgressArgs;

    fn name(&self) -> &'static str {
        "task_progress"
    }

    fn description(&self) -> &'static str {
        "Keep many subtasks of one parent in sync at once (e.g. from a queue file on each watch). \
         items: each found by id, or by title under the parent (made as a new subtask if missing). \
         status, progress, total, unit as on task_update; progress reaching total finishes it. A \
         parent is done when all its subtasks are. Returns the parent's rollup."
    }

    async fn call(&self, ctx: &ToolContext, a: TaskProgressArgs) -> Result<ToolOutput, ToolError> {
        let items = a
            .items
            .into_iter()
            .map(|item| ProgressItem {
                id: item.id.and_then(whole).map(TaskId::new),
                title: item.title,
                status: item.status.map(Status::from),
                progress: item.progress,
                total: item.total,
                unit: item.unit,
            })
            .collect();
        ToolOutput::json(
            &ctx.app
                .tasks()
                .progress(TaskId::new(a.parent_id), items)
                .await?,
        )
    }
}

/// Adds a line to a task's history.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TaskNote;

/// `task_note`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct TaskNoteArgs {
    /// The task.
    #[garde(range(min = 1))]
    id: i64,
    /// What you did or found, one plain line.
    #[garde(length(min = 1, max = 20_000))]
    text: String,
    /// Files on this machine or https links it produced.
    #[garde(skip)]
    files: Option<Vec<String>>,
}

impl Tool for TaskNote {
    type Args = TaskNoteArgs;

    fn name(&self) -> &'static str {
        "task_note"
    }

    fn description(&self) -> &'static str {
        "Add a line to a task's history (what you did, what you found). files: what it produced \
         (files on this machine or https links); the files and links the line names become its \
         results too."
    }

    async fn call(&self, ctx: &ToolContext, a: TaskNoteArgs) -> Result<ToolOutput, ToolError> {
        let tasks = ctx.app.tasks();
        let id = TaskId::new(a.id);
        tasks.note(id, Who::Assistant, &a.text, true).await?;
        let off = tasks
            .attach(id, a.files.as_deref().unwrap_or_default())
            .await?;
        Ok(ToolOutput::text(format!("Noted.{off}")))
    }
}

/// One option card of an ask.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct OptionCard {
    /// Its button: the answer it sends.
    label: String,
    /// Its name.
    #[serde(default)]
    title: Option<String>,
    /// A sentence or two.
    #[serde(default)]
    summary: Option<String>,
    /// 2-4 facts to compare, the same keys on every card.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    fields: Option<serde_json::Map<String, Value>>,
    /// A short number at a glance (a score, a price, a time).
    #[serde(default)]
    badge: Option<String>,
    /// The one you advise.
    #[serde(default)]
    pick: Option<bool>,
    /// Why, on the pick.
    #[serde(default)]
    why: Option<String>,
    /// What happens when the user taps it.
    #[serde(default)]
    then: Option<String>,
    /// The long version.
    #[serde(default)]
    more: Option<String>,
    /// Its pictures and videos (files on this machine).
    #[serde(default)]
    files: Option<Vec<String>>,
}

/// An answer: a plain string, or a card.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
#[serde(untagged)]
pub(crate) enum AskOption {
    /// A plain answer.
    Plain(String),
    /// A card to read and compare.
    Card(Box<OptionCard>),
}

/// What an ask costs the user.
#[derive(Clone, Copy, Debug, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum KindArg {
    /// A yes or no.
    Approve,
    /// They read and pick.
    Choose,
    /// Something only they can do, outside the assistant.
    Do,
}

/// Puts the one decision a task needs in front of the user.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TaskAsk;

/// `task_ask`'s arguments: the action, the question and the answers (`AskOption` cards or
/// plain strings), with what lays the decision out.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct TaskAskArgs {
    /// The task.
    #[garde(range(min = 1))]
    id: i64,
    /// What the user should do, short: the card's headline.
    #[garde(length(min = 1, max = 300))]
    action: String,
    /// Your advice and what the cards don't say.
    #[garde(length(min = 1))]
    question: String,
    /// The answers: 2-3 plain strings for a yes/no, or cards (label, title, summary, fields,
    /// badge, pick + why, then, files) to compare.
    #[garde(length(min = 1, max = 6))]
    options: Vec<AskOption>,
    /// Why now, a line.
    #[garde(length(max = 400))]
    context: Option<String>,
    /// approve: a yes or no; choose: they read and pick; do: something only they can do.
    #[garde(skip)]
    kind: Option<KindArg>,
    /// 2-4 short bullets: only what they need to decide.
    #[garde(length(max = 4), inner(length(max = 160)))]
    brief: Option<Vec<String>>,
    /// Plain answers: one line per option, what happens if they pick it.
    #[garde(inner(length(max = 140)))]
    notes: Option<Vec<String>>,
    /// Your pick: the index of the option you advise.
    #[garde(skip)]
    recommend: Option<usize>,
    /// What backs the decision (a report, a PR): folded below the answers.
    #[garde(skip)]
    files: Option<Vec<String>>,
    /// The ask the user already has, re-checked and still needed (it keeps its place).
    #[garde(skip)]
    same: Option<bool>,
    /// The day the user really has to decide by (left out: a week from now).
    #[garde(skip)]
    decide_by: Option<String>,
    /// The exact message the user would send, plain text in their voice.
    #[garde(skip)]
    draft: Option<String>,
    /// Where the draft goes (a chat's id).
    #[garde(skip)]
    chat: Option<String>,
}

impl Tool for TaskAsk {
    type Args = TaskAskArgs;

    fn name(&self) -> &'static str {
        "task_ask"
    }

    fn description(&self) -> &'static str {
        "Put the one decision a task needs in front of the user: what they should do (action, the \
         card's headline), context (why now), the question and the answers. Only for a real ask: it \
         goes under Needs you. A real choice between things to compare: each option a card (label, \
         title, summary, 2-4 fields with the same keys, badge, pick + why, then, files), so they \
         compare them on one screen; a yes/no or a short answer: 2-3 plain strings. Fill kind, \
         brief and recommend too (notes: plain answers only). A message for them to send: draft \
         and chat. decide_by: the day they really have to decide by. same: the ask they already \
         have, re-checked. A worker asks its own task's decision with it, then stops."
    }

    async fn call(&self, ctx: &ToolContext, a: TaskAskArgs) -> Result<ToolOutput, ToolError> {
        let tasks = ctx.app.tasks();
        let id = TaskId::new(a.id);
        let mut options = Vec::with_capacity(a.options.len());
        for option in a.options {
            options.push(match option {
                AskOption::Plain(text) => OptionIn::Plain(text),
                AskOption::Card(card) => {
                    let card = *card;
                    let mut media = Vec::new();
                    for file in card.files.iter().flatten() {
                        tasks.attach(id, std::slice::from_ref(file)).await?;
                        if let Some(artifact) = tasks.attached_as(id, file).await? {
                            media.push(artifact);
                        }
                    }
                    OptionIn::Card(CardIn {
                        label: card.label,
                        title: card.title,
                        summary: card.summary,
                        fields: card
                            .fields
                            .unwrap_or_default()
                            .into_iter()
                            .map(|(k, v)| {
                                let v = match v {
                                    Value::String(s) => s,
                                    other => other.to_string(),
                                };
                                (k, v)
                            })
                            .collect(),
                        badge: card.badge,
                        pick: card.pick.unwrap_or(false),
                        why: card.why,
                        then: card.then,
                        more: card.more,
                        media,
                    })
                }
            });
        }
        let off = tasks
            .attach(id, a.files.as_deref().unwrap_or_default())
            .await?;
        let asked = tasks
            .ask(
                id,
                AskInput {
                    question: a.question,
                    options,
                    action: Some(a.action),
                    context: a.context,
                    kind: a.kind.map(|k| {
                        match k {
                            KindArg::Approve => "approve",
                            KindArg::Choose => "choose",
                            KindArg::Do => "do",
                        }
                        .to_owned()
                    }),
                    brief: a.brief,
                    notes: a.notes,
                    recommend: a.recommend,
                    draft: a.draft,
                    chat: a.chat,
                    same: a.same.unwrap_or(false),
                    decide_by: a.decide_by,
                },
                Who::Assistant,
            )
            .await?;
        let by = asked
            .decide_by
            .as_ref()
            .map_or_default(|d| format!(" (decide by {d})"));
        Ok(ToolOutput::text(format!(
            "Waiting on the user: {}{by}.{off}",
            asked.action.as_deref().unwrap_or("null")
        )))
    }
}

/// One finding to file or update.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct FindingArg {
    /// The finding, when it is filed already.
    #[garde(skip)]
    id: Option<i64>,
    /// Its title.
    #[garde(length(max = 500))]
    title: Option<String>,
    /// What and where, a few lines.
    #[garde(skip)]
    body: Option<String>,
    /// Its state.
    #[garde(skip)]
    landed: Option<LandedArg>,
    /// The task that fixes it (null: none).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    fixed_by: Option<Option<i64>>,
    /// Filed twice: the finding this one repeats. It is put away: superseded, out of the
    /// area's counts.
    #[garde(skip)]
    duplicate_of: Option<i64>,
}

/// Files a scan's or a review's findings under it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TaskFindings;

/// `task_findings`' arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct TaskFindingsArgs {
    /// The scan or review task that found them.
    #[garde(range(min = 1))]
    scan_id: i64,
    /// The findings, each by id or by title under that scan.
    #[garde(length(min = 1, max = 100), dive)]
    items: Vec<FindingArg>,
}

impl Tool for TaskFindings {
    type Args = TaskFindingsArgs;

    fn name(&self) -> &'static str {
        "task_findings"
    }

    fn description(&self) -> &'static str {
        "File a scan's or review's findings as items under it, or update them: each its own task \
         (kind finding) with landed (landed, partly, not_landed, superseded, unchecked) and \
         fixed_by (the task that fixes it). Found by id, or by its title under that scan, so filing \
         again updates. Its area shows the ones not landed until each lands or is superseded; mark \
         one landed only after checking the work. Findings never go on Today or Needs you."
    }

    async fn call(&self, ctx: &ToolContext, a: TaskFindingsArgs) -> Result<ToolOutput, ToolError> {
        let tasks = ctx.app.tasks();
        let scan = TaskId::new(a.scan_id);
        let items: Vec<FindingIn> = a
            .items
            .into_iter()
            .map(|f| FindingIn {
                id: f.id.and_then(whole).map(TaskId::new),
                title: f.title,
                body: f.body,
                landed: f.landed.map(Landed::from),
                fixed_by: f.fixed_by.flatten().and_then(whole).map(TaskId::new),
                fixed_by_given: f.fixed_by.is_some(),
                duplicate_of: f.duplicate_of.and_then(whole).map(TaskId::new),
            })
            .collect();
        let said = tasks.file_findings(scan, items, Who::Assistant).await?;
        let findings = tasks.findings_of(scan).await?;
        let open = findings
            .iter()
            .filter(|f| f.landed.unwrap_or(Landed::Unchecked).is_open())
            .count();
        Ok(ToolOutput::text(format!(
            "{}. {}: {} findings, {open} not landed.",
            if said.is_empty() {
                "Nothing changed".to_owned()
            } else {
                said.join("; ")
            },
            scan.key(),
            findings.len()
        )))
    }
}

/// Finishes a task.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TaskDone;

/// `task_done`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct TaskDoneArgs {
    /// The task.
    #[garde(range(min = 1))]
    id: i64,
    /// A short plain summary of the result.
    #[garde(length(min = 1, max = 20_000))]
    summary: String,
}

impl Tool for TaskDone {
    type Args = TaskDoneArgs;

    fn name(&self) -> &'static str {
        "task_done"
    }

    fn description(&self) -> &'static str {
        "A task is finished and needs nothing from the user: a short plain summary of the result \
         goes to its history and it is done. A worker finishes its own task with it."
    }

    async fn call(&self, ctx: &ToolContext, a: TaskDoneArgs) -> Result<ToolOutput, ToolError> {
        ctx.app
            .tasks()
            .done(TaskId::new(a.id), &a.summary, Who::Worker)
            .await?;
        Ok(ToolOutput::text("Done."))
    }
}

/// A task's id, alone.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct TaskIdArgs {
    /// The task.
    #[garde(range(min = 1))]
    id: i64,
}

/// Starts a worker on a task now.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TaskWork;

impl Tool for TaskWork {
    type Args = TaskIdArgs;

    fn name(&self) -> &'static str {
        "task_work"
    }

    fn description(&self) -> &'static str {
        "Start a worker on one of your tasks now (it needs a body, the brief), instead of waiting \
         for a free slot."
    }

    async fn call(&self, ctx: &ToolContext, a: TaskIdArgs) -> Result<ToolOutput, ToolError> {
        match ctx.app.tasks().start_worker(TaskId::new(a.id)).await {
            Ok(()) => Ok(ToolOutput::text("Worker started.")),
            Err(TaskError::Refused(why)) => Ok(ToolOutput::text(format!("Not started: {why}"))),
            Err(other) => Err(other.into()),
        }
    }
}

/// Stops the worker on a task.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TaskStop;

impl Tool for TaskStop {
    type Args = TaskIdArgs;

    fn name(&self) -> &'static str {
        "task_stop"
    }

    fn description(&self) -> &'static str {
        "Stop the worker on a task (it goes back to to do)."
    }

    async fn call(&self, ctx: &ToolContext, a: TaskIdArgs) -> Result<ToolOutput, ToolError> {
        ctx.app.tasks().stop_worker(TaskId::new(a.id)).await?;
        Ok(ToolOutput::text("Stopped."))
    }
}

/// The user's projects.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Projects;

impl Tool for Projects {
    type Args = NoArgs;

    fn name(&self) -> &'static str {
        "projects"
    }

    fn description(&self) -> &'static str {
        "The user's projects: goal, priority, kind (deliverable or ongoing), deadline, hard, \
         status, open and done-this-week counts, health (on_track, at_risk: less than a day of \
         slack before a hard deadline, late) and hoursLeft (their sized work), and areas: each area \
         (id, name, status, goal, about, state, its tasks by state, needsYou, findingsOpen)."
    }

    async fn call(&self, ctx: &ToolContext, _: NoArgs) -> Result<ToolOutput, ToolError> {
        ToolOutput::json(&ctx.app.tasks().projects_json().await?)
    }
}

/// A project's kind.
#[derive(Clone, Copy, Debug, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum KindOfProject {
    /// It ends with something delivered.
    Deliverable,
    /// It goes on.
    Ongoing,
}

/// A project's status.
#[derive(Clone, Copy, Debug, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ProjectStatusArg {
    /// Worked on.
    Active,
    /// Off Today, its room quiet, until `paused_until`.
    Paused,
    /// No longer worked on: its open tasks are parked.
    Archived,
    /// Finished.
    Done,
}

/// Makes or changes a project.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ProjectSet;

/// `project_set`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct ProjectSetArgs {
    /// The project (none: a new one, with `name`).
    #[garde(skip)]
    id: Option<i64>,
    /// Its name.
    #[garde(length(max = 200))]
    name: Option<String>,
    /// The goal it serves (null: none).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    goal: Option<Option<i64>>,
    /// 0 none, 1 urgent, 2 high, 3 medium, 4 low: only as the user says.
    #[garde(range(min = 0, max = 4))]
    priority: Option<i32>,
    /// ongoing for things with no end.
    #[garde(skip)]
    kind: Option<KindOfProject>,
    /// When the whole project ends, a date (null: none).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    deadline: Option<Option<String>>,
    /// Someone else holds the user to that deadline.
    #[garde(skip)]
    hard: Option<bool>,
    /// Where it is: paused, archived, active or done, only as the user says.
    #[garde(skip)]
    status: Option<ProjectStatusArg>,
    /// With paused: the date it comes back (null: until they resume it).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    paused_until: Option<Option<String>>,
}

impl Tool for ProjectSet {
    type Args = ProjectSetArgs;

    fn name(&self) -> &'static str {
        "project_set"
    }

    fn description(&self) -> &'static str {
        "Create a project (no id: name) or change one. goal: the goal id it serves. priority: only \
         as the user says. kind: ongoing for things with no end. deadline: a date when the whole \
         project ends; hard: someone else holds them to it. status: paused (off Today; give \
         paused_until, the day it comes back), archived (no longer worked on: its open tasks are \
         parked; active brings it back) or done. Only as the user says."
    }

    async fn call(&self, ctx: &ToolContext, a: ProjectSetArgs) -> Result<ToolOutput, ToolError> {
        let given = a.id.and_then(whole).map(ProjectId::new);
        let change = ProjectChange {
            name: a.name.clone(),
            goal: a.goal.map(|g| g.and_then(whole).map(GoalId::new)),
            priority: a.priority,
            kind: a.kind.map(|k| match k {
                KindOfProject::Deliverable => ProjectKind::Deliverable,
                KindOfProject::Ongoing => ProjectKind::Ongoing,
            }),
            deadline: a
                .deadline
                .as_ref()
                .map(|d| days(d.as_deref()))
                .transpose()?,
            hard: a.hard,
            status: a.status.map(|s| match s {
                ProjectStatusArg::Active => ProjectStatus::Active,
                ProjectStatusArg::Paused => ProjectStatus::Paused,
                ProjectStatusArg::Archived => ProjectStatus::Archived,
                ProjectStatusArg::Done => ProjectStatus::Done,
            }),
            paused_until: a
                .paused_until
                .as_ref()
                .map(|d| days(d.as_deref()))
                .transpose()?,
        };
        let id = ctx.app.tasks().set_project(given, change).await?;
        ctx.app
            .activity()
            .note(format!(
                "Project {}: {}",
                if given.is_some() { "updated" } else { "added" },
                a.name.unwrap_or_else(|| id.to_string())
            ))
            .await
            .map_err(ToolError::failed)?;
        Ok(ToolOutput::text(format!("Project #{id}.")))
    }
}

/// An area's status.
#[derive(Clone, Copy, Debug, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AreaStatusArg {
    /// Being worked on.
    Active,
    /// On hold.
    Paused,
    /// Finished.
    Done,
}

/// Makes or changes an area of a project.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AreaSet;

/// `area_set`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct AreaSetArgs {
    /// The area (none: a new one, with `project_id` and `name`).
    #[garde(skip)]
    id: Option<i64>,
    /// A new area's project.
    #[garde(skip)]
    project_id: Option<i64>,
    /// Its name, unique in the project.
    #[garde(length(max = 200))]
    name: Option<String>,
    /// active, paused or done.
    #[garde(skip)]
    status: Option<AreaStatusArg>,
    /// What done looks like (null clears it).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    goal: Option<Option<String>>,
    /// One short line on what it is (null clears it).
    #[serde(default, deserialize_with = "patch")]
    #[garde(skip)]
    about: Option<Option<String>>,
}

impl Tool for AreaSet {
    type Args = AreaSetArgs;

    fn name(&self) -> &'static str {
        "area_set"
    }

    fn description(&self) -> &'static str {
        "Create an area in a project (no id: project_id and name) or change one: rename it, close \
         it (status done), pause it or bring it back (active), set its goal (what done looks like) \
         and about. An area holds a stream of related work: its tasks, their subtasks at any depth, \
         and the findings its scans file. Put tasks in it with task_create or task_update area_id \
         (a subtask sits in its parent's area). Ask the user before closing one with open work."
    }

    async fn call(&self, ctx: &ToolContext, a: AreaSetArgs) -> Result<ToolOutput, ToolError> {
        let line = ctx
            .app
            .tasks()
            .set_area(AreaChange {
                id: a.id.and_then(whole).map(AreaId::new),
                pid: a.project_id.and_then(whole).map(ProjectId::new),
                name: a.name,
                status: a.status.map(|s| match s {
                    AreaStatusArg::Active => AreaStatus::Active,
                    AreaStatusArg::Paused => AreaStatus::Paused,
                    AreaStatusArg::Done => AreaStatus::Done,
                }),
                goal: a.goal,
                about: a.about,
            })
            .await?;
        Ok(ToolOutput::text(line))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_runner_is_a_unit_or_a_watch() {
        assert!(valid_runner("unit:build@1"));
        assert!(valid_runner("unit:build.service"));
        assert!(valid_runner("watch:31"));
        assert!(!valid_runner("unit:"));
        assert!(!valid_runner("watch:x"));
        assert!(!valid_runner("cron:1"));
    }
}
