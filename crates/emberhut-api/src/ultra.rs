//! A task's run as a tree of agents (a task page's `ultraRun`): its lead (the task's worker, or
//! the runner session it runs in) and every sub-agent the lead started. Ultra mode adds a plan,
//! parts done and a spending cap; an ordinary worker has only its lead and any helpers.
//!
//! [`GateAnswer`] is what the ultra gate answers a lead's hook; the state's `agentsNow` is
//! [`crate::runner::agents::AgentNow`].
//!
//! Token counts and money are JSON numbers the server keeps as doubles; they are read as `f64`.

use serde::{Deserialize, Serialize};

use crate::ids::{AgentId, MachineId};
use crate::open_enum::open_enum;
use crate::time::Timestamp;

open_enum! {
    /// Where an ultra run's lead is.
    pub enum UltraState {
        /// No plan yet.
        Planning = "planning",
        /// A big plan waits for the person's OK.
        Asking = "asking",
        /// Its agents run.
        Running = "running",
        /// The spending cap stopped new agents; the person is asked.
        Capped = "capped",
    }
}

open_enum! {
    /// Where an agent is.
    pub enum AgentState {
        /// Working.
        Running = "running",
        /// Waiting to start.
        Queued = "queued",
        /// Finished.
        Done = "done",
        /// Failed.
        Failed = "failed",
        /// Stopped.
        Stopped = "stopped",
    }
}

open_enum! {
    /// What a sub-agent does in its run (the server's `other` reads as [`AgentKind::Other`]).
    pub enum AgentKind {
        /// One part of the plan.
        Part = "part",
        /// The check of a part.
        Verify = "verify",
        /// A lane of its own.
        Lane = "lane",
    }
}

/// A task's run of agents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UltraRun {
    /// Ultra mode is on for the task.
    pub on: bool,
    /// Where its lead is; none before an ultra run starts.
    pub state: Option<UltraState>,
    /// How many agents may run at a time.
    pub parallel: u32,
    /// Its token budget.
    pub budget: f64,
    /// The tokens spent so far.
    pub spent: f64,
    /// What it cost so far, in US dollars.
    #[serde(default)]
    pub cost: Option<f64>,
    /// The lead's own tokens.
    #[serde(default)]
    pub lead_tokens: Option<f64>,
    /// The account it runs on.
    #[serde(default)]
    pub account: Option<String>,
    /// Its plan, once the lead made one.
    #[serde(default)]
    pub plan: Option<UltraPlan>,
    /// Parts done.
    pub done: u32,
    /// Parts in all.
    pub total: u32,
    /// Agents running or queued now.
    pub running: u32,
    /// Its sub-agents, in the order they started.
    #[serde(default)]
    pub agents: Vec<UltraAgent>,
    /// Its lead.
    #[serde(default)]
    pub lead: Option<UltraLead>,
}

impl UltraRun {
    /// Whether something in it runs now: a client asks for the page again every few seconds
    /// while it does.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.running > 0 || self.lead.as_ref().is_some_and(|lead| lead.running)
    }

    /// Whether a page has anything of it to show.
    #[must_use]
    pub fn shows(&self) -> bool {
        self.on || !self.agents.is_empty() || self.lead.is_some()
    }
}

/// An ultra run's plan, as far as a client shows it (the server also sends its parts, accounts,
/// note and time).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UltraPlan {
    /// The agents it plans.
    pub agents: f64,
    /// The tokens it plans to spend.
    pub tokens: f64,
}

/// A run's lead: the task's worker, or the runner session it runs in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UltraLead {
    /// The session to watch it in.
    pub viewer: String,
    /// Its name.
    pub name: String,
    /// Where it runs, in words.
    #[serde(default)]
    pub place: String,
    /// Where it is.
    pub state: AgentState,
    /// It runs now.
    pub running: bool,
    /// What it is doing now.
    #[serde(default)]
    pub doing: Option<String>,
    /// When it started.
    #[serde(default)]
    pub started: Option<Timestamp>,
    /// When it ended.
    #[serde(default)]
    pub ended: Option<Timestamp>,
    /// Its tokens so far.
    #[serde(default)]
    pub tokens: Option<f64>,
    /// What it cost so far, in US dollars.
    #[serde(default)]
    pub cost: Option<f64>,
    /// The runner it is on.
    #[serde(default)]
    pub machine: Option<MachineId>,
    /// The account it runs on.
    #[serde(default)]
    pub account: Option<String>,
    /// That account's name.
    #[serde(default)]
    pub account_name: Option<String>,
    /// The person opened its session.
    #[serde(default)]
    pub direct: Option<bool>,
}

/// One sub-agent of a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UltraAgent {
    /// Its id.
    pub id: AgentId,
    /// Its name.
    pub name: Option<String>,
    /// What it does.
    pub kind: Option<AgentKind>,
    /// The part of the plan it does or checks.
    pub part: Option<u32>,
    /// Where it is.
    pub state: Option<AgentState>,
    /// Its latest step, or its result once done.
    pub last: Option<String>,
    /// Its tokens so far.
    pub tokens: Option<f64>,
    /// Its steps so far.
    pub tools: Option<f64>,
    /// The account it runs on.
    #[serde(default)]
    pub account: Option<String>,
    /// That account's name.
    #[serde(default)]
    pub account_name: Option<String>,
    /// The runner it is on.
    #[serde(default)]
    pub machine: Option<MachineId>,
    /// The session that started it.
    #[serde(default)]
    pub run: String,
    /// When it started.
    #[serde(default)]
    pub started: Option<Timestamp>,
    /// When it ended.
    #[serde(default)]
    pub ended: Option<Timestamp>,
    /// It is one of a workflow's agents.
    #[serde(default)]
    pub workflow: bool,
    /// The session to watch it in; none: nothing to watch.
    #[serde(default)]
    pub viewer: Option<String>,
    /// Its own steps in that session: the call that started it.
    #[serde(default)]
    pub parent: Option<String>,
}

/// What the ultra gate answers a lead's hook: nothing lets the tool run; a hook output with a
/// refusal holds it (the CLI's `PreToolUse` shape).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GateAnswer {
    /// The refusal, when it is held.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_specific_output: Option<HookOutput>,
}

/// A `PreToolUse` hook's refusal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HookOutput {
    /// `PreToolUse`.
    pub hook_event_name: String,
    /// `deny`.
    pub permission_decision: String,
    /// Why, for the agent.
    pub permission_decision_reason: String,
}

impl GateAnswer {
    /// Lets the tool run.
    #[must_use]
    pub fn allow() -> Self {
        Self::default()
    }

    /// Holds it, telling the agent `why`.
    #[must_use]
    pub fn deny(why: impl Into<String>) -> Self {
        Self {
            hook_specific_output: Some(HookOutput {
                hook_event_name: "PreToolUse".to_owned(),
                permission_decision: "deny".to_owned(),
                permission_decision_reason: why.into(),
            }),
        }
    }
}
