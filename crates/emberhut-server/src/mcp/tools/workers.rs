//! A worker's own task tools on the worker endpoint (`POST /mcp/worker`): its task is the one
//! its token names, so none takes the task's id (`task_note`, `task_findings` and
//! `task_runner` take another task's only when the worker's brief names it). Each does what
//! the room endpoint's tool of the same name does, on that task, written as the worker; and
//! `task_ask` goes through the ask guard first. `task_runner` sets what does the task's work
//! after the worker (a detached unit), so the task shows running while it runs.

use emberhut_core::task::{TaskChange, TaskId, Who};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{AnyTool, NoArgs, Tool, ToolContext, ToolError, ToolOutput, room_tools};
use crate::services::agents::scope::TaskOwner;
use crate::services::guard::ask_guard::TaskAsk as GuardAsk;

/// The worker's own task, or the error a call on a room's endpoint gets.
fn mine(ctx: &ToolContext) -> Result<TaskId, ToolError> {
    ctx.worker.ok_or_else(|| {
        ToolError::refused("This tool is a worker's: it acts on the worker's own task.")
    })
}

/// Whether the worker on `own` may write on `other`: its own task, or one its brief names.
async fn may_write(ctx: &ToolContext, own: TaskId, other: TaskId) -> Result<bool, ToolError> {
    if own == other {
        return Ok(true);
    }
    // A worker on a configured agent's task (whose brief the agent wrote) writes only on that
    // agent's own tasks, whatever its brief names.
    let guard = ctx.app.guard();
    let owner = guard.task_owner(own).await.map_err(ToolError::failed)?;
    if owner != TaskOwner::Owner
        && guard.task_owner(other).await.map_err(ToolError::failed)? != owner
    {
        return Ok(false);
    }
    let detail = ctx.app.tasks().detail(own).await?;
    let key = other.key();
    Ok(detail.is_some_and(|d| {
        let text = format!("{}\n{}", d.task.body.unwrap_or_default(), d.task.title);
        emberhut_core::guard::chat_ok::names_key(&text, &key)
    }))
}

/// Calls the room endpoint's tool `name` with `args`, its task's id put in as `field`.
async fn room_call(
    ctx: &ToolContext,
    name: &str,
    args: &impl Serialize,
    field: &str,
    task: TaskId,
) -> Result<ToolOutput, ToolError> {
    let mut value = serde_json::to_value(args).map_err(ToolError::failed)?;
    if let Value::Object(map) = &mut value {
        map.retain(|_, v| !v.is_null());
        map.remove("task_id");
        map.insert(field.to_owned(), Value::from(task.get()));
    }
    let tools = room_tools();
    let Some(tool) = tools.get(name) else {
        return Err(ToolError::refused(format!("no tool {name}")));
    };
    let tool: &dyn AnyTool = tool;
    tool.call(ctx, value).await
}

/// The worker's task in full.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorkerTask;

impl Tool for WorkerTask {
    type Args = NoArgs;

    fn name(&self) -> &'static str {
        "task"
    }

    fn description(&self) -> &'static str {
        "Your task and its full history."
    }

    async fn call(&self, ctx: &ToolContext, _: NoArgs) -> Result<ToolOutput, ToolError> {
        let task = mine(ctx)?;
        ToolOutput::json(&ctx.app.tasks().task_json(task).await?)
    }
}

/// `task_note`'s arguments.
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct WorkerNoteArgs {
    /// One plain line: what you did or found.
    #[garde(length(min = 1, max = 20_000))]
    text: String,
    /// What it produced: files on this machine or https links.
    #[garde(skip)]
    files: Option<Vec<String>>,
    /// Another task your brief names; left out: your own.
    #[garde(skip)]
    task_id: Option<i64>,
}

/// A line in a task's history.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorkerNote;

impl Tool for WorkerNote {
    type Args = WorkerNoteArgs;

    fn name(&self) -> &'static str {
        "task_note"
    }

    fn description(&self) -> &'static str {
        "Add one plain line to a task's history: what you did or found. files: what it produced. \
         task_id: another task your brief names; left out: your own."
    }

    async fn call(&self, ctx: &ToolContext, a: WorkerNoteArgs) -> Result<ToolOutput, ToolError> {
        let own = mine(ctx)?;
        let task = a.task_id.map_or(own, TaskId::new);
        if !may_write(ctx, own, task).await? {
            return Ok(ToolOutput::text(format!(
                "Not noted: {} isn't your task and your brief doesn't name it.",
                task.key()
            )));
        }
        let tasks = ctx.app.tasks();
        tasks.note(task, Who::Worker, &a.text, true).await?;
        let off = tasks
            .attach(task, a.files.as_deref().unwrap_or_default())
            .await?;
        Ok(ToolOutput::text(format!("Noted on {}.{off}", task.key())))
    }
}

/// `task_findings`' arguments.
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct WorkerFindingsArgs {
    /// The findings, each by id or by title (filing again updates): title, body, landed
    /// (landed, partly, `not_landed`, superseded, unchecked), `fixed_by`, `duplicate_of`.
    #[garde(length(min = 1, max = 100))]
    #[schema(value_type = Vec<Object>)]
    items: Vec<Value>,
    /// Another task your brief names; left out: your own.
    #[garde(skip)]
    task_id: Option<i64>,
}

/// A scan's findings filed under the worker's task.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorkerFindings;

impl Tool for WorkerFindings {
    type Args = WorkerFindingsArgs;

    fn name(&self) -> &'static str {
        "task_findings"
    }

    fn description(&self) -> &'static str {
        "Your scan, review or audit's findings, filed as items under your task (or a task your \
         brief names): each with landed (landed, partly, not_landed, superseded, unchecked: not \
         checked against the code yet) and fixed_by. Found by id or by title, so filing again \
         updates. File every finding before task_done."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        a: WorkerFindingsArgs,
    ) -> Result<ToolOutput, ToolError> {
        let own = mine(ctx)?;
        let task = a.task_id.map_or(own, TaskId::new);
        if !may_write(ctx, own, task).await? {
            return Ok(ToolOutput::text(format!(
                "Not filed: {} isn't your task and your brief doesn't name it.",
                task.key()
            )));
        }
        room_call(ctx, "task_findings", &a, "scan_id", task).await
    }
}

/// `task_ask`'s arguments: the room's, without the task's id, and `not_covered`.
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct WorkerAskArgs {
    /// What the owner should do, short: the card's headline.
    #[garde(length(min = 1, max = 300))]
    action: String,
    /// Your advice and what the cards don't say.
    #[garde(length(min = 1))]
    question: String,
    /// The answers: 2-3 plain strings, or cards (label, title, summary, fields, badge, pick +
    /// why, then, files) to compare.
    #[garde(length(min = 1, max = 6))]
    #[schema(value_type = Vec<Object>)]
    options: Vec<Value>,
    /// Why now, a line.
    #[garde(length(max = 400))]
    context: Option<String>,
    /// approve, choose or do.
    #[garde(skip)]
    kind: Option<String>,
    /// 2-4 short bullets: only what they need to decide.
    #[garde(length(max = 4), inner(length(max = 160)))]
    brief: Option<Vec<String>>,
    /// Plain answers: one line per option, what happens if they pick it.
    #[garde(inner(length(max = 140)))]
    notes: Option<Vec<String>>,
    /// The index of the option you advise.
    #[garde(skip)]
    recommend: Option<usize>,
    /// What backs the decision.
    #[garde(skip)]
    files: Option<Vec<String>>,
    /// The ask the owner already has, re-checked.
    #[garde(skip)]
    same: Option<bool>,
    /// The day the owner really has to decide by.
    #[garde(skip)]
    decide_by: Option<String>,
    /// The exact message the owner would send, plain text.
    #[garde(skip)]
    draft: Option<String>,
    /// Where the draft goes (a chat's id).
    #[garde(skip)]
    chat: Option<String>,
    /// Only after this ask came back "covered, do it" and the owner's quoted words really
    /// don't cover it: why. Then it reaches them, and your reason is noted on the task.
    #[garde(length(max = 300))]
    not_covered: Option<String>,
}

/// The one decision the worker's task needs.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorkerAsk;

impl Tool for WorkerAsk {
    type Args = WorkerAskArgs;

    fn name(&self) -> &'static str {
        "task_ask"
    }

    fn description(&self) -> &'static str {
        "Ask the owner the one decision your task needs; then stop and wait. Only when they \
         really have to act or decide: it puts the task under Needs you. action: what they should \
         do, short. context: why now. A real choice between things to compare: each option a \
         card; a yes/no: 2-3 plain strings. A message for them to send goes in draft, with chat. \
         The server checks the ask against the owner's words first: one they already cover comes \
         back \"Not asked: covered, do it\"; not_covered: why it truly isn't."
    }

    async fn call(&self, ctx: &ToolContext, a: WorkerAskArgs) -> Result<ToolOutput, ToolError> {
        let task = mine(ctx)?;
        let options: Vec<String> = a
            .options
            .iter()
            .filter_map(|o| match o {
                Value::String(s) => Some(s.clone()),
                Value::Object(card) => card.get("label").and_then(Value::as_str).map(str::to_owned),
                _ => None,
            })
            .collect();
        let check = GuardAsk {
            action: Some(a.action.clone()),
            context: a.context.clone(),
            question: a.question.clone(),
            brief: a.brief.clone().unwrap_or_default(),
            options,
            draft: a.draft.clone(),
            chat: a.chat.clone(),
            kind: a.kind.clone(),
            same: a.same.unwrap_or(false),
            not_covered: a.not_covered.clone(),
        };
        let back = ctx
            .app
            .guard()
            .task_ask(task, Who::Worker, &check)
            .await
            .map_err(ToolError::failed)?;
        if let Some(back) = back {
            return Ok(ToolOutput::text(back));
        }
        let asked = room_call(ctx, "task_ask", &a, "id", task).await?;
        if asked.is_error {
            return Ok(asked);
        }
        Ok(ToolOutput::text(
            "Asked. Stop now; you'll be resumed with their answer.",
        ))
    }
}

/// `task_runner`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct RunnerArgs {
    /// The unit you started (letters, digits, `@`, `.`, `:`, `_` and `-`); null clears it.
    #[garde(custom(unit_name))]
    unit: Option<String>,
    /// Another task your brief names; left out: your own.
    #[garde(skip)]
    task_id: Option<i64>,
}

#[expect(
    clippy::ref_option,
    clippy::trivially_copy_pass_by_ref,
    reason = "garde's custom rules take the field and the context by reference"
)]
fn unit_name(unit: &Option<String>, (): &()) -> garde::Result {
    match unit {
        Some(u)
            if u.is_empty()
                || !u
                    .chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '@' | '.' | ':' | '_' | '-')) =>
        {
            Err(garde::Error::new("a unit's name"))
        }
        _ => Ok(()),
    }
}

/// What does the task's work after the worker.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TaskRunner;

impl Tool for TaskRunner {
    type Args = RunnerArgs;

    fn name(&self) -> &'static str {
        "task_runner"
    }

    fn description(&self) -> &'static str {
        "Set what does your task's work after you, when you start something detached: unit: the \
         unit you started (a systemd user unit named for the task). While it runs, the task shows \
         Running in the owner's app; when it ends, the assistant hears it. null clears it. \
         task_id: another task your brief names; left out: your own."
    }

    async fn call(&self, ctx: &ToolContext, a: RunnerArgs) -> Result<ToolOutput, ToolError> {
        let own = mine(ctx)?;
        let task = a.task_id.map_or(own, TaskId::new);
        if !may_write(ctx, own, task).await? {
            return Ok(ToolOutput::text(format!(
                "Not set: {} isn't your task and your brief doesn't name it.",
                task.key()
            )));
        }
        let unit = a
            .unit
            .map(|u| {
                u.strip_suffix(".service")
                    .map_or_else(|| u.clone(), str::to_owned)
            })
            .filter(|u| !u.is_empty());
        let change = TaskChange {
            runner: Some(unit.as_ref().map(|u| format!("unit:{u}"))),
            ..TaskChange::default()
        };
        let tasks = ctx.app.tasks();
        tasks.update(task, change, Who::Worker, None).await?;
        Ok(ToolOutput::text(match unit {
            Some(unit) => {
                tasks
                    .note(
                        task,
                        Who::Worker,
                        &format!("Runs in the background as {unit}"),
                        false,
                    )
                    .await?;
                format!(
                    "{} runs on {unit}: it shows Running while the unit is active.",
                    task.key()
                )
            }
            None => format!("{} has no runner now.", task.key()),
        }))
    }
}

/// `task_done`'s arguments.
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct WorkerDoneArgs {
    /// A short plain summary of the result.
    #[garde(length(min = 1, max = 20_000))]
    summary: String,
}

/// The worker's task is finished.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorkerDone;

impl Tool for WorkerDone {
    type Args = WorkerDoneArgs;

    fn name(&self) -> &'static str {
        "task_done"
    }

    fn description(&self) -> &'static str {
        "Your task is finished and needs nothing from the owner: a short plain summary of the \
         result."
    }

    async fn call(&self, ctx: &ToolContext, a: WorkerDoneArgs) -> Result<ToolOutput, ToolError> {
        let task = mine(ctx)?;
        let run = ctx
            .app
            .workers()
            .run_of(task)
            .await
            .map_err(ToolError::failed)?;
        if run.is_some_and(|r| r.state == "asking" || r.state == "capped") {
            return Ok(ToolOutput::text(
                "Not done: this ultra run waits for the owner's answer. Note where it stands \
                 (task_note) and stop; you're resumed with their answer.",
            ));
        }
        ctx.app.tasks().done(task, &a.summary, Who::Worker).await?;
        ctx.app
            .guard()
            .outward_ended(task)
            .await
            .map_err(ToolError::failed)?;
        Ok(ToolOutput::text("Done."))
    }
}
