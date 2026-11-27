//! A configured agent's task tools, each scoped to the tasks made in its room: `tasks` (its
//! tasks, or one in full), `task_create` (work handed to a worker, for the person who asked),
//! `task_send` (a message to one's worker, as the agent), `task_stop` and `task_cancel`.

use emberhut_core::task::{Status, TaskId};
use serde::Deserialize;

use super::agent_set::{calling_agent, own_task};
use super::{Tool, ToolContext, ToolError, ToolOutput};
use crate::services::tasks::RoomTask;

/// The calling agent, refused when it is disabled: a disabled agent's tasks take no work.
async fn enabled(ctx: &ToolContext) -> Result<emberhut_core::agents::Agent, ToolError> {
    let agent = calling_agent(ctx).await?;
    if agent.enabled {
        Ok(agent)
    } else {
        Err(ToolError::refused(
            "You are disabled: your tasks take no new work, message or change.",
        ))
    }
}

/// Characters of a task's last line in the list.
const LAST_LINE: usize = 200;

/// `tasks`' arguments.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct AgentTasksArgs {
    /// One of your tasks, in full: its brief, history and what it produced.
    #[garde(inner(range(min = 1)))]
    id: Option<i64>,
}

/// The agent's tasks.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AgentTasks;

impl Tool for AgentTasks {
    type Args = AgentTasksArgs;

    fn name(&self) -> &'static str {
        "tasks"
    }

    fn description(&self) -> &'static str {
        "Your tasks (made in your room): open and recent ones with their status and last line. \
         id: one of them in full, with its brief, history (what its worker did and said) and \
         what it produced."
    }

    async fn call(&self, ctx: &ToolContext, args: AgentTasksArgs) -> Result<ToolOutput, ToolError> {
        let agent = calling_agent(ctx).await?;
        if let Some(id) = args.id.map(TaskId::new) {
            own_task(ctx, &agent, id).await?;
            return ToolOutput::json(&ctx.app.tasks().task_json(id).await?);
        }
        let tasks = ctx.app.tasks().in_room(agent.room).await?;
        if tasks.is_empty() {
            return Ok(ToolOutput::text("No tasks yet."));
        }
        let lines: Vec<String> = tasks
            .iter()
            .map(|task| {
                let last = task.last.as_deref().map_or_else(String::new, |last| {
                    format!(" → {}", emberhut_core::message::preview(last, LAST_LINE))
                });
                format!(
                    "{} [{}] {}{last}",
                    task.key(),
                    task.status.as_str(),
                    task.title
                )
            })
            .collect();
        Ok(ToolOutput::text(lines.join("\n")))
    }
}

/// Hands work to a worker, as one of the agent's tasks.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AgentTaskCreate;

/// The agent's `task_create`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct AgentTaskCreateArgs {
    /// A few words.
    #[garde(length(min = 3, max = 120))]
    title: String,
    /// Everything the worker needs: who asked, their exact words, where it came from, what done
    /// looks like, where things live.
    #[garde(length(min = 20, max = 20_000))]
    brief: String,
    /// Who asked for it, as they are named in the conversation.
    #[garde(length(min = 1, max = 120))]
    requested_by: String,
}

impl Tool for AgentTaskCreate {
    type Args = AgentTaskCreateArgs;

    fn name(&self) -> &'static str {
        "task_create"
    }

    fn description(&self) -> &'static str {
        "Hand real work to a worker: title (a few words), brief (everything it needs), \
         requested_by (who asked). It starts within a minute; you hear back when it is done or \
         needs something. Its outward steps wait on approvals by your policies."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: AgentTaskCreateArgs,
    ) -> Result<ToolOutput, ToolError> {
        let agent = enabled(ctx).await?;
        let made = ctx
            .app
            .tasks()
            .create_in_room(RoomTask {
                room: agent.room,
                agent: agent.name.clone(),
                title: args.title,
                brief: args.brief,
                requested_by: args.requested_by,
                project: agent.project,
            })
            .await?;
        Ok(ToolOutput::text(format!(
            "{} created. A worker starts on it within a minute.",
            made.key()
        )))
    }
}

/// `task_send`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct AgentTaskSendArgs {
    /// Your task's number.
    #[garde(range(min = 1))]
    id: i64,
    /// What to pass on.
    #[garde(length(min = 2, max = 20_000))]
    text: String,
}

/// Passes a message to one of the agent's tasks' worker.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AgentTaskSend;

impl Tool for AgentTaskSend {
    type Args = AgentTaskSendArgs;

    fn name(&self) -> &'static str {
        "task_send"
    }

    fn description(&self) -> &'static str {
        "Pass something to one of your tasks' worker (a follow-up, an answer to its question, \
         something missing in what it handed back): the same worker carries on with it."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: AgentTaskSendArgs,
    ) -> Result<ToolOutput, ToolError> {
        let agent = enabled(ctx).await?;
        let id = TaskId::new(args.id);
        own_task(ctx, &agent, id).await?;
        let running = ctx
            .app
            .tasks()
            .pass_to_worker(id, &agent.name, &args.text)
            .await?;
        Ok(ToolOutput::text(if running {
            "Its worker gets it as soon as its current step ends."
        } else {
            "Its worker picks it up within a minute."
        }))
    }
}

/// `task_stop`'s and `task_cancel`'s task.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct AgentTaskIdArgs {
    /// Your task's number.
    #[garde(range(min = 1))]
    id: i64,
}

/// Stops the worker on one of the agent's tasks.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AgentTaskStop;

impl Tool for AgentTaskStop {
    type Args = AgentTaskIdArgs;

    fn name(&self) -> &'static str {
        "task_stop"
    }

    fn description(&self) -> &'static str {
        "Stop the worker on one of your tasks (it goes back to to do)."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: AgentTaskIdArgs,
    ) -> Result<ToolOutput, ToolError> {
        let agent = enabled(ctx).await?;
        let id = TaskId::new(args.id);
        own_task(ctx, &agent, id).await?;
        ctx.app.tasks().stop_worker(id).await?;
        Ok(ToolOutput::text("Stopped."))
    }
}

/// `task_cancel`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct AgentTaskCancelArgs {
    /// Your task's number.
    #[garde(range(min = 1))]
    id: i64,
    /// Why ("they changed their mind", "a duplicate").
    #[garde(length(min = 1, max = 500))]
    why: String,
}

/// Cancels one of the agent's tasks.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AgentTaskCancel;

impl Tool for AgentTaskCancel {
    type Args = AgentTaskCancelArgs;

    fn name(&self) -> &'static str {
        "task_cancel"
    }

    fn description(&self) -> &'static str {
        "Cancel one of your tasks (they changed their mind, or it's a duplicate): its worker \
         stops."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: AgentTaskCancelArgs,
    ) -> Result<ToolOutput, ToolError> {
        let agent = enabled(ctx).await?;
        let id = TaskId::new(args.id);
        own_task(ctx, &agent, id).await?;
        if ctx.app.tasks().status_of(id).await? == Some(Status::Canceled) {
            return Ok(ToolOutput::text("It was canceled already."));
        }
        ctx.app.tasks().cancel(id, &agent.name, &args.why).await?;
        Ok(ToolOutput::text("Canceled."))
    }
}
