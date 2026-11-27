//! The tasks, as the personal services read and change them: the calendar's items (what the
//! owner has to work on, each on its day) and the walk rules' health project.

use emberhut_api as api;
use emberhut_api::calendar::CalendarItem;
use emberhut_api::tasks::{Issue, ProjectStatus, TaskStatus};
use emberhut_core::task::{Owner, Status, TaskChange, TaskId, Who};
use futures_util::future::BoxFuture;
use jiff::Timestamp;
use jiff::civil::Date;

use crate::services::calendar::{CalendarItems, ItemsError};
use crate::services::health_data::{HealthProject, WalkTaskError, WalkTasks};
use crate::services::tasks::{PlanState, TaskService};

/// The server's tasks, for the calendar and the walk rules.
#[derive(Clone, Debug)]
pub struct PersonalTasks {
    tasks: TaskService,
}

impl PersonalTasks {
    /// Over `tasks`.
    #[must_use]
    pub fn new(tasks: TaskService) -> Self {
        Self { tasks }
    }
}

/// Still to be finished: to do, working or waiting.
fn open(issue: &Issue) -> bool {
    matches!(
        issue.status,
        TaskStatus::Todo | TaskStatus::Working | TaskStatus::Waiting
    )
}

/// A top-level task, not a subtask.
fn top(issue: &Issue) -> bool {
    issue.parent.is_none()
}

/// The day an ask has to be decided by, and whether it is a hard day: its decide-by day when
/// that comes before its deadline, else its deadline (its own or its project's).
pub(crate) fn decide_by_of(issue: &Issue) -> Option<(&str, bool)> {
    let deadline = issue.deadline.as_ref().map(api::time::Day::as_str);
    match issue.decide_by.as_ref().map(api::time::Day::as_str) {
        Some(by) if deadline.is_none_or(|d| by < d) => Some((by, false)),
        _ => deadline.map(|d| (d, issue.hard_deadline)),
    }
}

/// More than 30 days out: folded in the app.
pub(crate) fn later(day: &str, today: Date) -> bool {
    today
        .checked_add(jiff::Span::new().days(30))
        .is_ok_and(|out| day > out.to_string().as_str())
}

/// An item of `issue`'s on `day`.
fn item_of(issue: &Issue, kind: &str, day: &str, hard: bool) -> CalendarItem {
    CalendarItem {
        kind: kind.into(),
        day: api::time::Day::new(day),
        task: Some(issue.id),
        key: Some(issue.key.clone()),
        project: issue.pid,
        title: issue.title.clone(),
        hard,
        status: Some(issue.status.clone()),
        count: None,
        action: None,
        later: None,
    }
}

/// What the owner has to work on in `from..=to`, each on its day, as the calendar has it: an
/// ask on the day it has to be decided by; a task's own deadline (not
/// again when its ask is on that day); a project's deadline once, with how many of its open
/// tasks take it; and, on today, the work ranked on Today that isn't an ask or waiting on one.
#[must_use]
pub fn calendar_items(board: &PlanState, from: Date, to: Date, today: Date) -> Vec<CalendarItem> {
    let today_date = today;
    let (from, to, today) = (from.to_string(), to.to_string(), today.to_string());
    let in_range = |day: &str| from.as_str() <= day && day <= to.as_str();
    let open: Vec<&Issue> = board.issues.iter().filter(|i| open(i)).collect();
    let mut items = Vec::new();
    let mut asked = Vec::new();
    for issue in open
        .iter()
        .filter(|i| i.status == TaskStatus::Waiting && top(i))
    {
        asked.push(issue.id);
        let Some((day, hard)) = decide_by_of(issue) else {
            continue;
        };
        if in_range(day) {
            items.push(CalendarItem {
                action: Some(issue.action.clone()),
                later: Some(later(day, today_date)),
                ..item_of(issue, "decide", day, hard)
            });
        }
    }
    for issue in open.iter().filter(|i| top(i) && i.due.is_some()) {
        let Some(deadline) = issue.deadline.as_ref().map(api::time::Day::as_str) else {
            continue;
        };
        let on_its_ask = asked.contains(&issue.id)
            && decide_by_of(issue).is_some_and(|(day, _)| day == deadline);
        if !on_its_ask && in_range(deadline) {
            items.push(item_of(issue, "deadline", deadline, issue.hard_deadline));
        }
    }
    for project in board
        .projects
        .iter()
        .filter(|p| p.status == ProjectStatus::Active)
    {
        let Some(deadline) = project.deadline.as_ref().map(api::time::Day::as_str) else {
            continue;
        };
        if !in_range(deadline) {
            continue;
        }
        let count = open
            .iter()
            .filter(|i| i.pid == Some(project.id) && i.due.is_none() && top(i))
            .count();
        items.push(CalendarItem {
            kind: "deadline".into(),
            day: api::time::Day::new(deadline),
            task: None,
            key: None,
            project: Some(project.id),
            title: project.name.clone(),
            hard: project.hard,
            status: None,
            count: Some(u32::try_from(count).unwrap_or(u32::MAX)),
            action: None,
            later: None,
        });
    }
    if in_range(&today) {
        for id in &board.today {
            let Some(issue) = open.iter().find(|i| i.id == *id) else {
                continue;
            };
            let waits = issue.sub.as_ref().is_some_and(|s| !s.waiting.is_empty());
            if asked.contains(&issue.id) || waits {
                continue;
            }
            let hard = issue.hard_deadline
                && issue.deadline.as_ref().map(api::time::Day::as_str) == Some(today.as_str());
            items.push(item_of(issue, "today", &today, hard));
        }
    }
    items
}

impl CalendarItems for PersonalTasks {
    fn items(
        &self,
        from: Date,
        to: Date,
        today: Date,
    ) -> BoxFuture<'_, Result<Vec<CalendarItem>, ItemsError>> {
        Box::pin(async move {
            let board = self
                .tasks
                .plan_state()
                .await
                .map_err(|error| ItemsError(Box::new(error)))?;
            Ok(calendar_items(&board, from, to, today))
        })
    }
}

/// A failure of the tasks, for the walk rules.
fn walk_error(error: impl std::error::Error + Send + Sync + 'static) -> WalkTaskError {
    WalkTaskError(Box::new(error))
}

impl PersonalTasks {
    /// Closes `id` as done `done_at`, with `evidence` as the assistant's note.
    async fn finish(
        &self,
        id: TaskId,
        evidence: &str,
        done_at: Timestamp,
    ) -> Result<(), WalkTaskError> {
        self.tasks
            .note(id, Who::Assistant, evidence, false)
            .await
            .map_err(walk_error)?;
        self.tasks
            .update(id, TaskChange::status(Status::Done), Who::Assistant, None)
            .await
            .map_err(walk_error)?;
        self.tasks.done_on(id, done_at).await.map_err(walk_error)?;
        Ok(())
    }
}

impl WalkTasks for PersonalTasks {
    fn project(&self) -> BoxFuture<'_, Result<Option<HealthProject>, WalkTaskError>> {
        Box::pin(async move {
            let board = self.tasks.plan_state().await.map_err(walk_error)?;
            Ok(board
                .projects
                .iter()
                .find(|p| p.status == ProjectStatus::Active && p.name.to_lowercase() == "health")
                .map(|p| HealthProject {
                    id: p.id.get(),
                    goal: p.goal.map(api::ids::GoalId::get),
                }))
        })
    }

    fn open_tasks(
        &self,
        project: HealthProject,
    ) -> BoxFuture<'_, Result<Vec<(i64, String)>, WalkTaskError>> {
        Box::pin(async move {
            let board = self.tasks.plan_state().await.map_err(walk_error)?;
            let mut tasks: Vec<(i64, String)> = board
                .issues
                .iter()
                .filter(|i| {
                    open(i)
                        && top(i)
                        && i.kind.is_none()
                        && i.pid.map(api::ids::ProjectId::get) == Some(project.id)
                })
                .map(|i| (i.id.get(), i.title.clone()))
                .collect();
            tasks.sort_by_key(|(id, _)| *id);
            Ok(tasks)
        })
    }

    fn close(
        &self,
        task: i64,
        evidence: String,
        done_at: Timestamp,
    ) -> BoxFuture<'_, Result<(), WalkTaskError>> {
        Box::pin(async move { self.finish(TaskId::new(task), &evidence, done_at).await })
    }

    fn log(
        &self,
        project: HealthProject,
        title: String,
        evidence: String,
        done_at: Timestamp,
    ) -> BoxFuture<'_, Result<i64, WalkTaskError>> {
        Box::pin(async move {
            let new = emberhut_db::NewTask {
                title,
                owner: Some(Owner::Person),
                pid: Some(emberhut_core::task::ProjectId::new(project.id)),
                goal: project.goal.map(emberhut_core::task::GoalId::new),
                ..emberhut_db::NewTask::default()
            };
            let made = self
                .tasks
                .create(new, Who::Assistant)
                .await
                .map_err(walk_error)?;
            self.finish(made.id, &evidence, done_at).await?;
            Ok(made.id.get())
        })
    }
}
