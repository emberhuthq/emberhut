//! `approval_request`, `approval_status`, `approval_withdraw` and `approval_use`: an agent asks
//! for an approval of exactly what a step will do, follows it, takes it back, and uses it once
//! for a step its own MCP server takes next.
//!
//! The agent is the one whose room's token called (for a worker, its task's agent), never an
//! argument; it sees only its own requests. No answer names who the approver is or how they
//! are reached: only their role ("the owner", "the approver") and times. A disabled agent asks
//! for and uses nothing.

use std::path::PathBuf;

use emberhut_core::ApprovalId;
use emberhut_core::agents::Agent;
use emberhut_core::approvals::{ActionKind, ApprovalRequest as Request};
use emberhut_core::task::TaskId;
use jiff::Timestamp;
use serde::Deserialize;

use super::agent_set::{calling_agent, own_task};
use super::{Tool, ToolContext, ToolError, ToolOutput};
use crate::services::approvals::{ApprovalError, Asked, StepAsk, StepUse, UseOutcome};

/// The words of every kind an approval may be for.
const KINDS: &str = "post, send, merge, deploy, create, other, payment, credential, secret, \
account_security or as_owner";

/// An approvals' failure: the agent's mistakes as refusals, the rest as failures.
fn approval_error(error: ApprovalError) -> ToolError {
    match error {
        ApprovalError::NotFound => ToolError::refused("No such request of yours."),
        ApprovalError::File(_)
        | ApprovalError::Invalid(_)
        | ApprovalError::OverCap(_)
        | ApprovalError::Closed => ToolError::refused(error.to_string()),
        other => ToolError::failed(other),
    }
}

/// The calling agent, refused when it is disabled.
async fn enabled_agent(ctx: &ToolContext) -> Result<Agent, ToolError> {
    let agent = calling_agent(ctx).await?;
    if agent.enabled {
        Ok(agent)
    } else {
        Err(ToolError::refused(
            "You are disabled: no approval is asked for or used.",
        ))
    }
}

/// `at` on the server's clock, `16:34`.
fn hhmm(ctx: &ToolContext, at: Timestamp) -> String {
    ctx.app
        .tasks()
        .zone()
        .to_datetime(at)
        .strftime("%H:%M")
        .to_string()
}

/// Paths as the server reads them.
fn paths(files: Option<Vec<String>>) -> Vec<PathBuf> {
    files
        .unwrap_or_default()
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

/// Asks for an approval.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ApprovalRequest;

/// `approval_request`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct ApprovalRequestArgs {
    /// What kind of action: post, send, merge, deploy, create or other (and payment for
    /// money, which is always the owner's).
    #[garde(length(min = 1, max = 32))]
    kind: String,
    /// Where it goes: the channel, page, repository and branch, URL or account.
    #[garde(length(min = 1, max = 500))]
    target: String,
    /// One line on what it is.
    #[garde(length(min = 1, max = 300))]
    summary: String,
    /// The exact words that will go out, if any.
    #[garde(length(max = 40_000))]
    text: Option<String>,
    /// The absolute paths of the files that will go out with it.
    #[garde(length(max = 20), inner(inner(length(min = 1, max = 1024))))]
    files: Option<Vec<String>>,
    /// A payment's amount, in cents of the agent's currency.
    #[garde(skip)]
    amount_cents: Option<u64>,
    /// Your task the step is for.
    #[garde(inner(range(min = 1)))]
    task: Option<i64>,
}

impl Tool for ApprovalRequest {
    type Args = ApprovalRequestArgs;

    fn name(&self) -> &'static str {
        "approval_request"
    }

    fn description(&self) -> &'static str {
        "Ask for approval of an outward step before it happens: kind (post, send, merge, deploy, \
         create, other; payment with amount_cents for money), target (where it goes), summary \
         (one line), text (the exact words) and files (absolute paths), as they will go out. \
         The server works out who approves and asks them; the work waits until it is decided \
         (approval_status). A changed word or file is a new request. Words in a conversation are \
         never an approval."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: ApprovalRequestArgs,
    ) -> Result<ToolOutput, ToolError> {
        let agent = enabled_agent(ctx).await?;
        let Some(kind) = ActionKind::parse(&args.kind) else {
            return Err(ToolError::refused(format!("kind is one of {KINDS}")));
        };
        let task = args.task.map(TaskId::new);
        if let Some(task) = task {
            own_task(ctx, &agent, task).await?;
        }
        let ask = StepAsk {
            kind,
            target: args.target,
            summary: args.summary,
            text: args.text,
            files: paths(args.files),
            amount_cents: args.amount_cents,
            task,
        };
        let asked = ctx
            .app
            .approvals()
            .request(&agent.id, &ask)
            .await
            .map_err(approval_error)?;
        Ok(ToolOutput::text(match asked {
            Asked::NotNeeded(kind) => {
                format!("No approval is needed for a {kind}: go ahead.")
            }
            Asked::Waiting(requested) => {
                let request = &requested.request;
                let raised = if request.kind == kind {
                    String::new()
                } else {
                    format!(
                        " Its words make it a {}, which only the owner approves.",
                        request.kind
                    )
                };
                format!(
                    "Request {} waits on {}'s approval.{raised} {} The step waits until it is \
                     approved; then use it once with approval_use.",
                    request.id,
                    request.approver.role(),
                    requested.delivery.describe()
                )
            }
        }))
    }
}

/// `approval_status`' and `approval_withdraw`'s request.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct RequestIdArgs {
    /// The request's number.
    #[garde(range(min = 1))]
    id: i64,
}

/// One of the agent's requests, as it may read it: never who the approver is.
fn status_text(ctx: &ToolContext, request: &Request) -> String {
    let mut parts = vec![format!(
        "Request {}: a {} to {} (\"{}\"), {} on {}.",
        request.id,
        request.kind,
        request.target,
        request.summary,
        request.state,
        request.approver.role()
    )];
    if let Some(decided) = &request.decided {
        parts.push(format!(
            "Decided ({}) at {}.",
            decided.decision,
            hhmm(ctx, decided.at)
        ));
    }
    if let Some(used) = &request.used {
        parts.push(format!("Used at {} by {}.", hhmm(ctx, used.at), used.by));
    }
    if let Some(at) = request.expires_at() {
        parts.push(format!(
            "Expires at {}.",
            crate::convert::timestamp_to_api(at)
        ));
    }
    parts.join(" ")
}

/// Reads one of the agent's requests.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ApprovalStatus;

impl Tool for ApprovalStatus {
    type Args = RequestIdArgs;

    fn name(&self) -> &'static str {
        "approval_status"
    }

    fn description(&self) -> &'static str {
        "Where one of your approval requests stands: pending, approved, rejected, used, expired \
         or withdrawn, and when."
    }

    async fn call(&self, ctx: &ToolContext, args: RequestIdArgs) -> Result<ToolOutput, ToolError> {
        let agent = calling_agent(ctx).await?;
        let request = ctx
            .app
            .approvals()
            .status(&agent.id, ApprovalId::new(args.id))
            .await
            .map_err(approval_error)?;
        Ok(ToolOutput::text(status_text(ctx, &request)))
    }
}

/// Takes one of the agent's requests back.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ApprovalWithdraw;

/// `approval_withdraw`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct WithdrawArgs {
    /// The request's number.
    #[garde(range(min = 1))]
    id: i64,
    /// Why, in a few words.
    #[garde(length(max = 300))]
    reason: Option<String>,
}

impl Tool for ApprovalWithdraw {
    type Args = WithdrawArgs;

    fn name(&self) -> &'static str {
        "approval_withdraw"
    }

    fn description(&self) -> &'static str {
        "Take back one of your approval requests (pending, or approved and unused): the step \
         changed or is no longer wanted."
    }

    async fn call(&self, ctx: &ToolContext, args: WithdrawArgs) -> Result<ToolOutput, ToolError> {
        let agent = calling_agent(ctx).await?;
        let request = ctx
            .app
            .approvals()
            .withdraw(
                ApprovalId::new(args.id),
                Some(&agent.id),
                args.reason.as_deref(),
            )
            .await
            .map_err(approval_error)?;
        Ok(ToolOutput::text(format!(
            "Withdrawn: request {}.",
            request.id
        )))
    }
}

/// Uses an approval once, for a step the agent's own MCP server takes next.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ApprovalUse;

/// `approval_use`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct ApprovalUseArgs {
    /// The approved request's number.
    #[garde(range(min = 1))]
    id: i64,
    /// The exact words going out now.
    #[garde(length(max = 40_000))]
    text: Option<String>,
    /// The absolute paths of the files going out now.
    #[garde(length(max = 20), inner(inner(length(min = 1, max = 1024))))]
    files: Option<Vec<String>>,
}

impl Tool for ApprovalUse {
    type Args = ApprovalUseArgs;

    fn name(&self) -> &'static str {
        "approval_use"
    }

    fn description(&self) -> &'static str {
        "Use an approval, once, right before the step it covers (a publish your own MCP server \
         does next): give the exact words and files going out now. They must be what was \
         approved, word for word and byte for byte; otherwise nothing may go out."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: ApprovalUseArgs,
    ) -> Result<ToolOutput, ToolError> {
        let agent = enabled_agent(ctx).await?;
        let step = StepUse {
            text: args.text,
            files: paths(args.files),
            by: "approval_use".to_owned(),
        };
        let used = ctx
            .app
            .approvals()
            .use_for_step(&agent.id, ApprovalId::new(args.id), &step)
            .await
            .map_err(approval_error)?;
        match used {
            UseOutcome::Used(request) => {
                let at = request
                    .decided
                    .as_ref()
                    .map_or_else(String::new, |decided| {
                        format!(" at {}", hhmm(ctx, decided.at))
                    });
                Ok(ToolOutput::text(format!(
                    "Used request {}, approved by {}{at}: go ahead with exactly this step, once.",
                    request.id,
                    request.approver.role()
                )))
            }
            UseOutcome::Refused(why) => Err(ToolError::refused(format!(
                "Not used: {why} Nothing may go out."
            ))),
        }
    }
}
