//! An ultra run's lead's tools on the worker endpoint: its plan (`ultra_plan`), a part run as
//! its own session on a runner (`ultra_lane`), and waiting for those (`ultra_wait`).

use emberhut_core::task::Who;
use serde::Deserialize;

use super::{Tool, ToolContext, ToolError, ToolOutput};
use crate::services::workers::WorkerError;
use crate::services::workers::ultra::{PlanInput, PlanPart};

fn mine(ctx: &ToolContext) -> Result<emberhut_core::task::TaskId, ToolError> {
    ctx.worker.ok_or_else(|| {
        ToolError::refused("This tool is an ultra run's lead's: it acts on its own task.")
    })
}

fn refused(error: WorkerError) -> ToolError {
    match error {
        WorkerError::Refused(why) => ToolError::refused(why),
        other => ToolError::failed(other),
    }
}

/// A part's size.
#[derive(Clone, Copy, Debug, Deserialize, utoipa::ToSchema)]
pub(crate) enum SizeArg {
    /// About 80k tokens (its sub-agents' counts).
    S,
    /// About 200k.
    M,
    /// About 500k.
    L,
}

/// One part of the plan.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct PartArg {
    /// Its number.
    #[garde(range(min = 1))]
    n: u32,
    /// A short name.
    #[garde(length(min = 1, max = 80))]
    name: String,
    /// S ~80k tokens, M ~200k, L ~500k (sub-agent counts).
    #[garde(skip)]
    size: SizeArg,
    /// false: this part needs no verify agent.
    #[garde(skip)]
    verify: Option<bool>,
}

/// `ultra_plan`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct PlanArgs {
    /// The parts.
    #[garde(length(min = 1, max = 40), dive)]
    parts: Vec<PartArg>,
    /// The sub-agents (default: one per part plus its verify agent).
    #[garde(range(min = 1))]
    agents: Option<u32>,
    /// The tokens (default: the sizes' estimate).
    #[garde(range(min = 1))]
    tokens: Option<u64>,
    /// Account ids the parts run on (yours first; lanes on others).
    #[garde(skip)]
    accounts: Option<Vec<String>>,
    /// A note.
    #[garde(length(max = 300))]
    note: Option<String>,
}

/// The ultra run's plan.
#[derive(Clone, Copy, Debug)]
pub(crate) struct UltraPlan;

impl Tool for UltraPlan {
    type Args = PlanArgs;

    fn name(&self) -> &'static str {
        "ultra_plan"
    }

    fn description(&self) -> &'static str {
        "Post your ultra run's plan before any sub-agent starts: the parts with sizes, the \
         sub-agents (defaults to one per part plus its verify agent), the tokens (defaults to the \
         sizes' estimate) and the accounts. It goes on the task. A big plan waits for the owner's \
         OK (then stop); a smaller one is cleared at once. Call it again to change the plan."
    }

    async fn call(&self, ctx: &ToolContext, a: PlanArgs) -> Result<ToolOutput, ToolError> {
        let task = mine(ctx)?;
        let input = PlanInput {
            parts: a
                .parts
                .into_iter()
                .map(|p| PlanPart {
                    n: f64::from(p.n),
                    name: p.name,
                    size: match p.size {
                        SizeArg::S => "S",
                        SizeArg::M => "M",
                        SizeArg::L => "L",
                    }
                    .to_owned(),
                    verify: p.verify,
                })
                .collect(),
            agents: a.agents.map(f64::from),
            #[expect(clippy::cast_precision_loss, reason = "a token estimate")]
            tokens: a.tokens.map(|t| t as f64),
            accounts: a.accounts,
            note: a.note,
        };
        let workers = ctx.app.workers();
        let result = match workers.set_plan(task, input, Who::Worker).await {
            Ok(result) => result,
            Err(WorkerError::Refused(why)) => {
                return Ok(ToolOutput::text(format!("Not saved: {why}")));
            }
            Err(other) => return Err(ToolError::failed(other)),
        };
        let run = workers.run_of(task).await.map_err(ToolError::failed)?;
        if result.ok {
            let (parallel, budget) = run.map_or((0, 0.0), |r| (r.parallel, r.budget));
            Ok(ToolOutput::text(format!(
                "Cleared: {} sub-agents, about {} tokens, {parallel} at a time, cap {}. Start the parts now.",
                result.plan.agents,
                emberhut_core::ultra::tokens(result.plan.tokens),
                emberhut_core::ultra::tokens(budget)
            )))
        } else {
            Ok(ToolOutput::text(format!(
                "Waiting for the owner's OK ({}). Stop now, without task_done; you're resumed with their answer.",
                if result.why == Some("big") {
                    "a big run"
                } else {
                    "over its spending cap"
                }
            )))
        }
    }
}

/// `ultra_lane`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct LaneArgs {
    /// The part it runs.
    #[garde(range(min = 1))]
    part: u32,
    /// The part's name.
    #[garde(length(max = 80))]
    name: String,
    /// The part's brief (include the rules: nothing outward, report back).
    #[garde(length(min = 20))]
    brief: String,
    /// The folder it works in, on the runner.
    #[garde(length(min = 1))]
    cwd: String,
    /// An account id with room; default: the runner picks one that isn't yours.
    #[garde(skip)]
    account: Option<String>,
}

/// One part as its own session on a runner.
#[derive(Clone, Copy, Debug)]
pub(crate) struct UltraLane;

impl Tool for UltraLane {
    type Args = LaneArgs;

    fn name(&self) -> &'static str {
        "ultra_lane"
    }

    fn description(&self) -> &'static str {
        "Run one part of your ultra run as its own session on another account with room (a \
         lane), when your own account's room isn't enough for the plan. It starts on the \
         server's runner, in cwd, with your brief for the part (include the rules: nothing \
         outward, report back). It counts as a sub-agent; ultra_wait waits for it."
    }

    async fn call(&self, ctx: &ToolContext, a: LaneArgs) -> Result<ToolOutput, ToolError> {
        let task = mine(ctx)?;
        let said = ctx
            .app
            .workers()
            .lane(task, a.part, &a.name, &a.brief, &a.cwd, a.account)
            .await
            .map_err(refused)?;
        Ok(ToolOutput::text(said))
    }
}

/// `ultra_wait`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct WaitArgs {
    /// The lanes' session ids (from `ultra_lane`).
    #[garde(length(min = 1))]
    sessions: Vec<String>,
    /// How long to wait, in minutes (default 5, at most 9).
    #[garde(range(min = 0.1, max = 9.0))]
    minutes: Option<f64>,
}

/// Waits for the run's lanes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct UltraWait;

impl Tool for UltraWait {
    type Args = WaitArgs;

    fn name(&self) -> &'static str {
        "ultra_wait"
    }

    fn description(&self) -> &'static str {
        "Wait for your ultra run's lanes (session ids from ultra_lane) to finish, up to minutes \
         (default 5, at most 9; call again to keep waiting). Returns each one's state and its \
         answer."
    }

    async fn call(&self, ctx: &ToolContext, a: WaitArgs) -> Result<ToolOutput, ToolError> {
        let task = mine(ctx)?;
        let said = ctx
            .app
            .workers()
            .wait(task, &a.sessions, a.minutes.unwrap_or(5.0))
            .await
            .map_err(refused)?;
        Ok(ToolOutput::text(said))
    }
}
