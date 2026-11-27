//! `threads`, `thread_send` and `follow`: the assistant's view of the coding threads on the
//! user's machines, sending one a turn, and following one it handed work to.
//!
//! `threads` also lists the runners' accounts with their use (`runnerAccounts`) and one line
//! per runner machine on what it has left (`machines`); `thread_send` to a runner session (an
//! `r-<8 hex>` id, or a runner's name as the machine) goes to that session's runner as its next
//! turn, from the assistant.
//!
//! A thread is not tied to a task (no `task_id`, no `owner_ok` for a plan's approval); an
//! argument the type doesn't know is ignored, so an agent that sends one still gets its call
//! done.

use serde::Deserialize;

use super::{Tool, ToolContext, ToolError, ToolOutput};
use crate::services::runner_api::by;
use crate::services::runners::{SessionId, is_session_id, truthy};
use crate::services::threads::ThreadError;

/// Every coding thread the server knows about, and the machines it could not reach.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Threads;

/// `threads`' arguments.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct ThreadsArgs {
    /// Only this status, e.g. waiting.
    #[garde(length(max = 32))]
    status: Option<String>,
    /// Helper agents' threads too.
    #[serde(default)]
    #[garde(skip)]
    helpers: bool,
}

impl Tool for Threads {
    type Args = ThreadsArgs;

    fn name(&self) -> &'static str {
        "threads"
    }

    fn description(&self) -> &'static str {
        "Every coding thread on the user's machines with its status (running, waiting, \
         completed, failed...), project and last update, as of the server's last look at the \
         coding-thread app (every 30 s; free, no call to the app) and the sessions it was told \
         of; plus the machines whose coding-thread app could not be reached, with why, each \
         runner's accounts with their use (runnerAccounts: where a new session has room) and one \
         line per runner machine on what it has left (machines; the machines tool has it all). \
         helpers: helper agents' threads too; status: only threads in that status."
    }

    async fn call(&self, ctx: &ToolContext, args: ThreadsArgs) -> Result<ToolOutput, ToolError> {
        let list = ctx
            .app
            .threads()
            .list(args.status.as_deref(), args.helpers)
            .await
            .map_err(ToolError::failed)?;
        let accounts = ctx.app.accounts().all().await.map_err(ToolError::failed)?;
        let machines = ctx
            .app
            .machines()
            .summaries(None)
            .await
            .map_err(ToolError::failed)?;
        ToolOutput::json(&serde_json::json!({
            "threads": list.threads,
            "unreachable": list.unreachable,
            "runnerAccounts": accounts,
            "machines": machines,
        }))
    }
}

/// Sends a coding thread a message, as its next turn.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ThreadSend;

/// `thread_send`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct ThreadSendArgs {
    /// The machine, by the name the threads tool lists it under.
    #[garde(length(min = 1, max = 64))]
    machine: String,
    /// The thread's id.
    #[garde(length(min = 1, max = 256))]
    thread: String,
    /// What to send: at most 100 000 UTF-16 units, as a message to a room may be.
    #[garde(length(utf16, min = 1, max = 100_000))]
    message: String,
}

impl Tool for ThreadSend {
    type Args = ThreadSendArgs;

    fn name(&self) -> &'static str {
        "thread_send"
    }

    fn description(&self) -> &'static str {
        "Send a message to a thread of the coding-thread app, or to a session on one of your \
         runners (thread id r-…, or a runner as the machine), as its next turn. It queues \
         behind a turn already running. An id given without the prefix the app puts in front \
         of it is completed when one known thread on that machine matches."
    }

    async fn call(&self, ctx: &ToolContext, args: ThreadSendArgs) -> Result<ToolOutput, ToolError> {
        let runners = ctx.app.runners();
        // A runner session (its id is r-…, whatever machine was written): the runner takes the
        // turn.
        if runners.is_runner(&args.machine) || is_session_id(&args.thread) {
            let sent = async {
                let id = SessionId::parse(&args.thread)?;
                let machine = if runners.is_runner(&args.machine) {
                    args.machine.clone()
                } else {
                    runners.machine_of(&id).await?
                };
                runners
                    .send_turn(&machine, &id, &args.message, by::ASSISTANT, &[])
                    .await
            }
            .await;
            return Ok(ToolOutput::text(match sent {
                Ok(answer) => {
                    let queued = if truthy(answer.get("queued")) {
                        " (queued behind the running one)"
                    } else {
                        ""
                    };
                    format!(
                        "Sent to runner session {} as its next turn{queued}.",
                        args.thread
                    )
                }
                Err(error) => {
                    format!("Not sent: {error}. Check the session's id with the threads tool.")
                }
            }));
        }
        match ctx
            .app
            .threads()
            .send(&args.machine, &args.thread, &args.message)
            .await
        {
            Ok(sent) => {
                let full = if sent.thread == args.thread {
                    String::new()
                } else {
                    format!(" (that's the thread's full id; you wrote {})", args.thread)
                };
                let queued = if sent.sent.queued {
                    " (queued behind the running one)"
                } else {
                    ""
                };
                Ok(ToolOutput::text(format!(
                    "Sent to {}{full} as its next turn{queued}.",
                    sent.thread
                )))
            }
            // A plain answer the agent can act on.
            Err(ThreadError::App(error)) => Ok(ToolOutput::text(format!(
                "Not sent: {}. Check the thread's id and machine with the threads tool.",
                error.to_string().trim_end_matches('.')
            ))),
            Err(error) => Err(ToolError::failed(error)),
        }
    }
}

/// Follows a thread the assistant handed work to: its end is news at once.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Follow;

/// `follow`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct FollowArgs {
    /// The machine, by the name the threads tool lists it under.
    #[garde(length(min = 1, max = 64))]
    machine: String,
    /// The thread's id.
    #[garde(length(min = 1, max = 256))]
    thread: String,
    /// A few words on what it does, for the assistant and the user's app.
    #[garde(length(utf16, max = 80))]
    #[schema(max_length = 80)]
    why: String,
}

impl Tool for Follow {
    type Args = FollowArgs;

    fn name(&self) -> &'static str {
        "follow"
    }

    fn description(&self) -> &'static str {
        "After you start a coding thread for the user, follow it: when it finishes, fails or \
         waits for an answer, you are woken at once to read the result and tell them. why: a \
         few words on what it does, for you and for the user's app. Also for a session the \
         user opened on a runner machine: followed the same way."
    }

    async fn call(&self, ctx: &ToolContext, args: FollowArgs) -> Result<ToolOutput, ToolError> {
        ctx.app
            .threads()
            .follow(&args.machine, &args.thread, &args.why)
            .await
            .map_err(ToolError::failed)?;
        tracing::info!(why = %args.why, "handed off");
        Ok(ToolOutput::text("Following it."))
    }
}
