//! A task's plan, kept by the agent doing its work: `plan_set` writes the whole plan,
//! `plan_step` changes or adds one step, `plan_read` reads it back as a checklist with each
//! step's id. Served only by a server that keeps plans (`--task-plans`).
//!
//! The task is the caller's own when it has one, and then the only one whose plan it may
//! write (reading any is fine): a worker's is its task; a runner session's (the session its
//! request names, [`emberhut_api::wire::mcp::SESSION_HEADER`]) the task it is linked to. Any
//! other session of the assistant names the task, and may write any task's plan, as may a call
//! with the runner sessions' token that names no session linked to a task (it is told the
//! session header is missing when it leaves the task out).

use emberhut_api::wire::mcp::{SESSION_HEADER, SESSION_HEADER_VALUE};
use emberhut_core::task::plan::{AddStep, Edit, NewStep, StepRef, StepState};
use emberhut_core::task::wire::KEY_PREFIX;
use emberhut_core::task::{TaskId, Who};
use serde::Deserialize;

use super::{Tool, ToolContext, ToolError, ToolOutput};

/// The plan tools' names: absent from every tool list while the server keeps no plans.
pub(crate) const PLAN_TOOLS: [&str; 3] = ["plan_set", "plan_step", "plan_read"];

/// A task, by its number or its key (`T-12`).
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
#[serde(untagged)]
pub(crate) enum TaskArg {
    /// Its number.
    Number(i64),
    /// Its key, or its number as text.
    Key(String),
}

impl TaskArg {
    /// The task it names, if it names one.
    fn id(&self) -> Option<TaskId> {
        let number = match self {
            Self::Number(n) => Some(*n),
            Self::Key(key) => {
                let key = key.trim();
                key.strip_prefix(KEY_PREFIX)
                    .or_else(|| key.strip_prefix(&KEY_PREFIX.to_lowercase()))
                    .unwrap_or(key)
                    .parse()
                    .ok()
            }
        }?;
        (number > 0).then(|| TaskId::new(number))
    }
}

/// A step, by its id or its exact title.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
#[serde(untagged)]
pub(crate) enum StepArg {
    /// Its id, as `plan_read` lists it.
    Id(u32),
    /// Its title (or its id as text).
    Title(String),
}

impl From<StepArg> for StepRef {
    fn from(arg: StepArg) -> Self {
        match arg {
            StepArg::Id(id) => Self::Id(id),
            StepArg::Title(title) => match title.trim().parse() {
                Ok(id) => Self::Id(id),
                Err(_) => Self::Title(title),
            },
        }
    }
}

/// Where a step is.
#[derive(Clone, Copy, Debug, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum StateArg {
    /// Not started.
    Todo,
    /// Being worked on now.
    Doing,
    /// Done.
    Done,
    /// Stuck: say on what in its note.
    Blocked,
    /// Not needed after all.
    Skipped,
}

impl From<StateArg> for StepState {
    fn from(arg: StateArg) -> Self {
        match arg {
            StateArg::Todo => Self::Todo,
            StateArg::Doing => Self::Doing,
            StateArg::Done => Self::Done,
            StateArg::Blocked => Self::Blocked,
            StateArg::Skipped => Self::Skipped,
        }
    }
}

/// A sub-step of a plan's step (sub-steps have none of their own).
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub(crate) struct SubStepArg {
    /// One short line.
    #[garde(length(chars, min = 1, max = 200))]
    title: String,
    /// Its state (default todo; a step already in the plan keeps its own).
    #[garde(skip)]
    state: Option<StateArg>,
    /// What it came to, or what blocks it.
    #[garde(length(chars, max = 2000))]
    note: Option<String>,
    /// A link to what it produced (a pull request, a page, a file).
    #[garde(length(chars, max = 2000))]
    link: Option<String>,
}

/// A step of a whole plan, with its sub-steps.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct PlanStepArg {
    /// One short line.
    #[garde(length(chars, min = 1, max = 200))]
    title: String,
    /// Its state (default todo; a step already in the plan keeps its own).
    #[garde(skip)]
    state: Option<StateArg>,
    /// What it came to, or what blocks it.
    #[garde(length(chars, max = 2000))]
    note: Option<String>,
    /// A link to what it produced (a pull request, a page, a file).
    #[garde(length(chars, max = 2000))]
    link: Option<String>,
    /// Its sub-steps, one level deep.
    #[serde(default)]
    #[garde(dive)]
    steps: Vec<SubStepArg>,
}

impl From<SubStepArg> for NewStep {
    fn from(arg: SubStepArg) -> Self {
        Self {
            title: arg.title,
            state: arg.state.map(StepState::from),
            note: arg.note,
            link: arg.link,
            steps: Vec::new(),
        }
    }
}

impl From<PlanStepArg> for NewStep {
    fn from(arg: PlanStepArg) -> Self {
        Self {
            title: arg.title,
            state: arg.state.map(StepState::from),
            note: arg.note,
            link: arg.link,
            steps: arg.steps.into_iter().map(Self::from).collect(),
        }
    }
}

/// An estimate as given: empty clears it.
#[expect(
    clippy::option_option,
    reason = "an estimate's change: left out is unchanged, none clears it"
)]
fn eta(eta: Option<String>) -> Option<Option<String>> {
    eta.map(|eta| Some(eta).filter(|eta| !eta.trim().is_empty()))
}

/// Whose a call is: a worker's or a runner session's task, or any (the assistant's own sessions,
/// and the runner sessions' token without a session linked to a task).
#[derive(Clone, Debug, PartialEq, Eq)]
enum Scope {
    /// The caller's own tasks: it writes only theirs.
    Own(Vec<TaskId>),
    /// Any task.
    Any,
    /// Any task, by the runner sessions' token with no session linked to a task: as [`Self::Any`],
    /// but a call that leaves the task out is told why there is no task of its own.
    Unlinked,
}

/// Whose `ctx`'s call is: a worker's task; else the tasks its runner session is linked to
/// (a session the server doesn't know, or linked to none, is as no session); else, with the
/// runner sessions' token, any but unlinked; else any.
async fn scope(ctx: &ToolContext) -> Result<Scope, ToolError> {
    if let Some(task) = ctx.worker {
        return Ok(Scope::Own(vec![task]));
    }
    if let Some(session) = &ctx.session {
        let tasks = ctx
            .app
            .tasks()
            .session_tasks(session)
            .await
            .map_err(ToolError::failed)?;
        if !tasks.is_empty() {
            return Ok(Scope::Own(tasks));
        }
    }
    Ok(if ctx.runner {
        Scope::Unlinked
    } else {
        Scope::Any
    })
}

/// The task a call is about: the one it names, else the caller's own (when it has exactly
/// one). Writing, a task that isn't the caller's own is refused.
async fn task_of(
    ctx: &ToolContext,
    named: Option<&TaskArg>,
    writes: bool,
) -> Result<TaskId, ToolError> {
    let scope = scope(ctx).await?;
    let named = match named {
        Some(arg) => Some(
            arg.id()
                .ok_or_else(|| ToolError::refused("task: a task's number or its key (T-12)"))?,
        ),
        None => None,
    };
    match (named, &scope) {
        (Some(task), Scope::Own(own)) if writes && !own.contains(&task) => {
            Err(ToolError::refused(format!(
                "You work on {}: you may read {}'s plan but not write it.",
                keys(own),
                task.key()
            )))
        }
        (Some(task), _) => Ok(task),
        (None, Scope::Own(own)) if own.len() == 1 => Ok(own[0]),
        (None, Scope::Own(own)) => Err(ToolError::refused(format!(
            "Say which task (task): you work on {}.",
            keys(own)
        ))),
        (None, Scope::Any) => Err(ToolError::refused(
            "Say which task (task: its number or key, T-12).",
        )),
        (None, Scope::Unlinked) => Err(ToolError::refused(format!(
            "Say which task (task: its number or key, T-12): this call's {SESSION_HEADER} \
             header names no session the server knows linked to a task, so it has no task of \
             its own (the runner's sessions send {SESSION_HEADER_VALUE})."
        ))),
    }
}

fn keys(tasks: &[TaskId]) -> String {
    tasks
        .iter()
        .map(|task| task.key())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Who writes: a worker, or the assistant (a runner session too, whichever token it calls
/// with: the history names the assistant, as it does for the session's other changes).
const fn who(ctx: &ToolContext) -> Who {
    if ctx.worker.is_some() {
        Who::Worker
    } else {
        Who::Assistant
    }
}

/// The task's plan as the tools answer it: its key and title, then the checklist.
async fn answer(ctx: &ToolContext, task: TaskId) -> Result<ToolOutput, ToolError> {
    let tasks = ctx.app.tasks();
    let Some(issue) = tasks.issue(task).await? else {
        return Err(ToolError::refused(format!("No task {}.", task.key())));
    };
    let stored = tasks.stored_plan(task).await?;
    Ok(ToolOutput::text(format!(
        "Plan of {} \"{}\":\n{}",
        issue.key,
        issue.title,
        emberhut_core::task::plan::checklist(&stored.steps, stored.eta.as_deref())
    )))
}

/// Writes a task's whole plan.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlanSet;

/// `plan_set`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct PlanSetArgs {
    /// The task: its number or key (T-12). Left out: the task you work on.
    #[garde(skip)]
    task: Option<TaskArg>,
    /// Every step of the work, in order; sub-steps one level deep.
    #[garde(length(max = 100), dive)]
    steps: Vec<PlanStepArg>,
    /// What is left, in words (about 2 hours); empty clears it.
    #[garde(length(chars, max = 120))]
    eta: Option<String>,
}

impl Tool for PlanSet {
    type Args = PlanSetArgs;

    fn name(&self) -> &'static str {
        "plan_set"
    }

    fn description(&self) -> &'static str {
        "Write a task's whole plan: every step of the work, in order, each one short line, with \
         sub-steps one level deep. When you start work on a task, your first act is plan_set \
         with the whole plan; then mark each step doing (plan_step) when you start it and done \
         the moment it's finished, and blocked with a note when you're stuck. The plan is how \
         the user follows the task, live in their app: it replaces HTML progress pages and \
         progress reports. Calling it again replaces the plan but keeps every step whose title \
         matches one already there (its id, state and times), so ticks are never lost: call it \
         again when the work changes shape. task: its number or key (T-12), left out for the \
         task you work on. state: todo, doing, done, blocked or skipped. eta: what is left, in \
         words (about 2 hours)."
    }

    async fn call(&self, ctx: &ToolContext, args: PlanSetArgs) -> Result<ToolOutput, ToolError> {
        let task = task_of(ctx, args.task.as_ref(), true).await?;
        let steps = args.steps.into_iter().map(NewStep::from).collect();
        ctx.app
            .tasks()
            .set_plan(task, steps, eta(args.eta), who(ctx))
            .await?;
        answer(ctx, task).await
    }
}

/// Changes or adds one step of a task's plan.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlanStepTool;

/// A step to add with `plan_step`.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct AddArg {
    /// One short line.
    #[garde(length(chars, min = 1, max = 200))]
    title: String,
    /// The top-level step it is a sub-step of (its id or title).
    #[garde(skip)]
    parent: Option<StepArg>,
    /// The step it comes right after (its id or title); left out: last.
    #[garde(skip)]
    after: Option<StepArg>,
}

/// `plan_step`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct PlanStepArgs {
    /// The task: its number or key (T-12). Left out: the task you work on.
    #[garde(skip)]
    task: Option<TaskArg>,
    /// The step to change: its id (`plan_read` lists them) or its exact title.
    #[garde(skip)]
    step: Option<StepArg>,
    /// A step to add; state, note and link then apply to it.
    #[garde(dive)]
    add: Option<AddArg>,
    /// The step's new title.
    #[garde(length(chars, min = 1, max = 200))]
    title: Option<String>,
    /// Its new state: doing when you start it, done the moment it's finished, blocked (with a
    /// note on what) when stuck, skipped when not needed after all, todo to reopen it.
    #[garde(skip)]
    state: Option<StateArg>,
    /// What it came to, or what blocks it; empty clears it.
    #[garde(length(chars, max = 2000))]
    note: Option<String>,
    /// A link to what it produced (a pull request, a page, a file); empty clears it.
    #[garde(length(chars, max = 2000))]
    link: Option<String>,
    /// What is left of the whole plan, in words (about 2 hours); empty clears it.
    #[garde(length(chars, max = 120))]
    eta: Option<String>,
}

impl Tool for PlanStepTool {
    type Args = PlanStepArgs;

    fn name(&self) -> &'static str {
        "plan_step"
    }

    fn description(&self) -> &'static str {
        "Change one step of a task's plan, or add one: state doing when you start a step, done \
         the moment it's finished, blocked with a note saying on what when you're stuck, \
         skipped when it isn't needed after all. Keep the plan true as the work changes: the \
         user follows the task by it. step: its id (plan_read lists them) or exact title. add: \
         {title, parent?, after?} adds a step (a sub-step of parent, after a step, or last), \
         with the state and note given. title renames the step; note and link (a pull request, \
         a page, a file it produced) show with it; eta: what is left of the whole plan. task: \
         its number or key, left out for the task you work on."
    }

    async fn call(&self, ctx: &ToolContext, args: PlanStepArgs) -> Result<ToolOutput, ToolError> {
        let task = task_of(ctx, args.task.as_ref(), true).await?;
        let edit = Edit {
            step: args.step.map(StepRef::from),
            add: args.add.map(|add| AddStep {
                title: add.title,
                parent: add.parent.map(StepRef::from),
                after: add.after.map(StepRef::from),
            }),
            title: args.title,
            state: args.state.map(StepState::from),
            note: args.note,
            link: args.link,
        };
        ctx.app
            .tasks()
            .edit_plan(task, edit, eta(args.eta), who(ctx))
            .await?;
        answer(ctx, task).await
    }
}

/// Reads a task's plan back.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlanRead;

/// `plan_read`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct PlanReadArgs {
    /// The task: its number or key (T-12). Left out: the task you work on.
    #[garde(skip)]
    task: Option<TaskArg>,
}

impl Tool for PlanRead {
    type Args = PlanReadArgs;

    fn name(&self) -> &'static str {
        "plan_read"
    }

    fn description(&self) -> &'static str {
        "A task's plan as a checklist with each step's id ([x] done, [>] doing, [!] blocked, \
         [ ] to do, [-] skipped; sub-steps indented) and how far it is. task: its number or \
         key (T-12), left out for the task you work on."
    }

    async fn call(&self, ctx: &ToolContext, args: PlanReadArgs) -> Result<ToolOutput, ToolError> {
        let task = task_of(ctx, args.task.as_ref(), false).await?;
        answer(ctx, task).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_is_named_by_its_number_or_key() {
        assert_eq!(TaskArg::Number(12).id(), Some(TaskId::new(12)));
        assert_eq!(TaskArg::Key("T-12".to_owned()).id(), Some(TaskId::new(12)));
        assert_eq!(TaskArg::Key(" t-7 ".to_owned()).id(), Some(TaskId::new(7)));
        assert_eq!(TaskArg::Key("12".to_owned()).id(), Some(TaskId::new(12)));
        assert_eq!(TaskArg::Key("the build".to_owned()).id(), None);
        assert_eq!(TaskArg::Number(0).id(), None);
    }

    #[test]
    fn a_step_is_named_by_its_id_or_title() {
        assert_eq!(StepRef::from(StepArg::Id(3)), StepRef::Id(3));
        assert_eq!(
            StepRef::from(StepArg::Title("3".to_owned())),
            StepRef::Id(3)
        );
        assert_eq!(
            StepRef::from(StepArg::Title("Run the tests".to_owned())),
            StepRef::Title("Run the tests".to_owned())
        );
    }
}
