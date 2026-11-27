//! The app's changes to tasks, projects and the plan: Focus pins, drops and parks, ticks and
//! parks of steps, Archive and back, the answer to an ask, a posted X reply, a quick add, and a
//! project's or a task's fields. Each reads its field (and those that go with it) leniently,
//! and does nothing for a field of the wrong kind.

use emberhut_api::ids::{ProjectId, TaskId};
use emberhut_api::patch::Patch;
use emberhut_api::settings::{ProjectChange, TaskChange};
use emberhut_api::tasks::ProjectStatus;
use emberhut_api::time::Day;
use emberhut_core::task::{ArtifactId, TaskId as CoreTaskId};
use serde_json::Value;

use super::{Action, ActionContext, ActionError, Body};
use crate::services::plan::{FocusChange, PlanError};
use crate::services::tasks::TaskError;

impl From<TaskError> for ActionError {
    fn from(error: TaskError) -> Self {
        match error {
            TaskError::Refused(text) => Self::Refused(text),
            other => Self::failed(other),
        }
    }
}

impl From<PlanError> for ActionError {
    fn from(error: PlanError) -> Self {
        match error {
            PlanError::Refused(text) => Self::Refused(text),
            other => Self::failed(other),
        }
    }
}

/// A JSON number as a whole number (truncated).
fn number(value: Option<&Value>) -> Option<i64> {
    let n = value?.as_f64()?;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "an id the app sends, truncated"
    )]
    let whole = n.trunc() as i64;
    n.is_finite().then_some(whole)
}

fn task(value: Option<&Value>) -> Option<CoreTaskId> {
    number(value).map(CoreTaskId::new)
}

/// A field of an object, as given: absent is `None`, `null` is `Some(None)`.
#[expect(
    clippy::option_option,
    reason = "a partial change: left out is unchanged, null clears"
)]
fn given<'a>(object: &'a Value, key: &str) -> Option<Option<&'a Value>> {
    let object = object.as_object()?;
    object.get(key).map(|v| (!v.is_null()).then_some(v))
}

/// JavaScript's truthiness, for a flag read with it.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

fn text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// `pinFocus: id` (with `pinned: false` to unpin): the person keeps a Focus item.
#[derive(Debug)]
pub(super) struct PinFocus;

impl Action for PinFocus {
    fn key(&self) -> &'static str {
        "pinFocus"
    }

    fn needs_device(&self) -> bool {
        true
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(id) = number(body.get("pinFocus")) else {
            return Ok(());
        };
        let pinned = body.get("pinned") != Some(&Value::Bool(false));
        Ok(ctx.app.plan().pin_focus(id, pinned).await?)
    }
}

/// `dropFocus: id`: the person drops a Focus item, pinned or not.
#[derive(Debug)]
pub(super) struct DropFocus;

impl Action for DropFocus {
    fn key(&self) -> &'static str {
        "dropFocus"
    }

    fn needs_device(&self) -> bool {
        true
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(id) = number(body.get("dropFocus")) else {
            return Ok(());
        };
        ctx.app.plan().drop_focus(id, true).await?;
        ctx.app
            .activity()
            .note("The person dropped a Focus item")
            .await
            .map_err(ActionError::failed)
    }
}

/// `parkFocus: id` with `until` (a day or an instant; `null` brings it back).
#[derive(Debug)]
pub(super) struct ParkFocus;

impl Action for ParkFocus {
    fn key(&self) -> &'static str {
        "parkFocus"
    }

    fn needs_device(&self) -> bool {
        true
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(id) = number(body.get("parkFocus")) else {
            return Ok(());
        };
        let until = body.get("until").and_then(Value::as_str).map(str::to_owned);
        let parked = truthy(body.get("until"));
        let item = ctx
            .app
            .plan()
            .set_focus(FocusChange {
                id: Some(id),
                parked_until: Some(until.clone()),
                parked_why: Some(parked.then(|| "The person parked it".to_owned())),
                ..FocusChange::default()
            })
            .await?;
        let line = match until.filter(|_| parked) {
            Some(until) => format!(
                "The person parked {} until {}",
                item.title,
                until.chars().take(10).collect::<String>()
            ),
            None => format!("The person brought back {}", item.title),
        };
        ctx.app
            .activity()
            .note(line)
            .await
            .map_err(ActionError::failed)
    }
}

/// `todoDone: id` (with `done: false` to take the tick back): the person ticks a step.
#[derive(Debug)]
pub(super) struct TodoDone;

impl Action for TodoDone {
    fn key(&self) -> &'static str {
        "todoDone"
    }

    fn needs_device(&self) -> bool {
        true
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(id) = task(body.get("todoDone")) else {
            return Ok(());
        };
        let done = body.get("done") != Some(&Value::Bool(false));
        Ok(ctx.app.tasks().todo_done(id, done).await?)
    }
}

/// `todoPark: id` with `until`: the person parks a step.
#[derive(Debug)]
pub(super) struct TodoPark;

impl Action for TodoPark {
    fn key(&self) -> &'static str {
        "todoPark"
    }

    fn needs_device(&self) -> bool {
        true
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(id) = task(body.get("todoPark")) else {
            return Ok(());
        };
        let until = text(body.get("until"));
        Ok(ctx.app.tasks().todo_park(id, until).await?)
    }
}

/// `taskArchive: id`: the person's Archive, put away for good.
#[derive(Debug)]
pub(super) struct TaskArchive;

impl Action for TaskArchive {
    fn key(&self) -> &'static str {
        "taskArchive"
    }

    fn needs_device(&self) -> bool {
        true
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(id) = task(body.get("taskArchive")) else {
            return Ok(());
        };
        Ok(ctx.app.tasks().archive(id).await?)
    }
}

/// `taskUnarchive: id`: back to to do.
#[derive(Debug)]
pub(super) struct TaskUnarchive;

impl Action for TaskUnarchive {
    fn key(&self) -> &'static str {
        "taskUnarchive"
    }

    fn needs_device(&self) -> bool {
        true
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(id) = task(body.get("taskUnarchive")) else {
            return Ok(());
        };
        Ok(ctx.app.tasks().unarchive(id).await?)
    }
}

/// `taskDecide: id` with `choice`: the person's answer to an ask. An ask that approves
/// something to go out takes its answer from a signed-in device only, report mode included: a
/// request without a session can't OK a message sent as the owner.
#[derive(Debug)]
pub(super) struct TaskDecide;

impl Action for TaskDecide {
    fn key(&self) -> &'static str {
        "taskDecide"
    }

    fn needs_device(&self) -> bool {
        true
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let (Some(id), Some(choice)) = (
            task(body.get("taskDecide")),
            body.get("choice").and_then(Value::as_str),
        ) else {
            return Ok(());
        };
        if ctx.caller.is_none() && ctx.app.tasks().asks_approval(id).await? {
            return Err(ActionError::SignInRequired);
        }
        Ok(ctx.app.tasks().decide(id, choice).await?)
    }
}

/// `xPosted: artifact` with `option`: the person posted that X reply.
#[derive(Debug)]
pub(super) struct XPosted;

impl Action for XPosted {
    fn key(&self) -> &'static str {
        "xPosted"
    }

    fn needs_device(&self) -> bool {
        true
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let (Some(artifact), Some(option)) =
            (number(body.get("xPosted")), number(body.get("option")))
        else {
            return Ok(());
        };
        let Ok(option) = usize::try_from(option) else {
            return Ok(());
        };
        Ok(ctx
            .app
            .tasks()
            .x_posted(ArtifactId::new(artifact), option)
            .await?)
    }
}

/// `taskCreate: title` (with `goal`, `project`): the person's quick add.
#[derive(Debug)]
pub(super) struct TaskCreate;

impl Action for TaskCreate {
    fn key(&self) -> &'static str {
        "taskCreate"
    }

    fn needs_device(&self) -> bool {
        true
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(title) = body
            .get("taskCreate")
            .and_then(Value::as_str)
            .filter(|t| !t.trim().is_empty())
        else {
            return Ok(());
        };
        Ok(ctx
            .app
            .tasks()
            .quick_add(title, number(body.get("goal")), number(body.get("project")))
            .await?)
    }
}

/// `projectSet: { id, priority?, deadline?, hard?, status?, pausedUntil? }`: the person sets a
/// project; a field given as `null` is cleared.
#[derive(Debug)]
pub(super) struct ProjectSetAction;

impl Action for ProjectSetAction {
    fn key(&self) -> &'static str {
        "projectSet"
    }

    fn needs_device(&self) -> bool {
        true
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(set) = body.get("projectSet").filter(|v| truthy(Some(v))) else {
            return Ok(());
        };
        let Some(id) = number(set.get("id")) else {
            return Ok(());
        };
        let day = |key: &str| given(set, key).map(|v| text(v).map(Day::new));
        // `null` priority is none (0).
        let project = ProjectChange {
            id: ProjectId::new(id),
            priority: given(set, "priority")
                .map(|v| number(v).and_then(|n| u8::try_from(n).ok()).unwrap_or(0)),
            deadline: patch(day("deadline")),
            hard: given(set, "hard").map(truthy),
            status: given(set, "status")
                .and_then(text)
                .map(|s| ProjectStatus::from(s.as_str())),
            paused_until: day("pausedUntil").flatten(),
        };
        Ok(ctx.app.tasks().project_set(&project).await?)
    }
}

/// `taskSet: { id, deadline?, hard?, size?, project?, decideBy?, body?, ultra? }`: the person
/// sets a task's fields.
#[derive(Debug)]
pub(super) struct TaskSetAction;

impl Action for TaskSetAction {
    fn key(&self) -> &'static str {
        "taskSet"
    }

    fn needs_device(&self) -> bool {
        true
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(set) = body.get("taskSet").filter(|v| truthy(Some(v))) else {
            return Ok(());
        };
        let Some(id) = number(set.get("id")) else {
            return Ok(());
        };
        let task = TaskChange {
            id: TaskId::new(id),
            deadline: patch(given(set, "deadline").map(|v| text(v).map(Day::new))),
            hard: given(set, "hard").map(truthy),
            size: patch(given(set, "size").map(text)),
            project: patch(given(set, "project").map(|v| number(v).map(ProjectId::new))),
            decide_by: patch(
                given(set, "decideBy").map(|v| text(v).filter(|_| truthy(v)).map(Day::new)),
            ),
            body: set.get("body").and_then(Value::as_str).map(str::to_owned),
            ultra: set.get("ultra").and_then(Value::as_bool),
        };
        Ok(ctx.app.tasks().task_set(&task).await?)
    }
}

/// A field read with [`given`] as a change: left out keeps, `null` (or an unusable value)
/// clears, a value sets.
#[expect(
    clippy::option_option,
    reason = "a partial change as read: left out, null, or a value"
)]
fn patch<T>(field: Option<Option<T>>) -> Patch<T> {
    field.map_or(Patch::Keep, Patch::of)
}
