//! The guard rails for a task made in a configured agent's room: every guarded step of it is
//! decided by the agent's policies, never by the owner's job grant or cards.
//!
//! - A final step (a post, a comment, another final click) and an outward command (a merge, a
//!   deploy) become an approval request of the step's kind to the approver the policy names (the
//!   owner, in the apps, when it names the owner), its target the page or the repositories and
//!   its text the words written or the command line; a matching approval is used once and the
//!   step goes through, naming only the approver's role and when. A policy naming nobody lets
//!   it through.
//! - A hard step (money, passwords, account security) is always the owner's own card, which no
//!   grant covers; a message sent as the owner goes through messaging's own guard and no
//!   agent's tool.
//! - The owner's job grant, their OKs on a task and a trusted requester's request never apply to
//!   an agent's task (`Guard::job_grant` and `Guard::covered_by` have none for it), so the gate is
//!   always on for its workers.
//! - An agent's worker's file tools reach only the agent's roots (its own folder and its tasks'
//!   work folders), as real paths: every field of theirs that names a path or a glob is
//!   checked, and a call the gate can't be sure of is refused (`agent_files`); any other
//!   built-in tool is refused.
//! - A disabled agent's steps, and every tool of its workers, wait; a task made in a room no
//!   agent owns now is no one's, and every step of it is held.
//!
//! The default agent's tasks (every task not made in a configured agent's room) are not
//! touched: the owner's rules decide, as before. A trusted requester's request
//! (`Guard::trusted_request`) counts for a default agent's task only.

use emberhut_core::agents::{Agent, AgentId};
use emberhut_core::approvals::{ActionKind, steps};
use emberhut_core::guard::grants as grant_rules;
use emberhut_core::guard::outward as outward_rules;
use emberhut_core::task::{Task, TaskId};
use emberhut_db::{AgentStore, DbError};
use serde_json::Value;

use super::agent_files::{self, FILE_TOOLS};
use super::outward::Reader;
use super::{Guard, GuardError};
use crate::services::agents::scope::{self, AgentRoots, TaskOwner};
use crate::services::approvals::{ApprovalService, Gate, StepAsk};

/// A held step's line when its agent is disabled.
const DISABLED: &str = "Held: the agent is disabled, so its steps wait. Stop for now.";

/// A held step's line on a task no agent owns now.
const ORPHANED: &str = "Held: this task's agent is gone, so it is no one's and nothing of it \
runs. Stop now.";

/// What an agent's policy makes of a step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentCheck {
    /// The owner's rules decide (a default agent's task, a hard step, or a command with no
    /// outward step).
    Owner,
    /// It goes through; the line says why.
    Through(String),
    /// It waits; the line says on whom, for the worker.
    Held(String),
}

/// What the guard rails consult an agent's policies with, handed to them once both are built.
#[derive(Clone, Debug)]
pub(crate) struct AgentHook {
    approvals: ApprovalService,
    agents: AgentStore,
    roots: AgentRoots,
}

/// Whose a task is, with what the guard rails decide an agent's with.
enum Scope<'a> {
    Owner,
    Agent(Box<Agent>, &'a AgentHook),
    Orphaned,
}

impl Guard {
    /// Hands the guard rails the approvals, the agents and their roots, for the tasks made in
    /// an agent's room.
    pub fn use_agents(&self, approvals: ApprovalService, agents: AgentStore, roots: AgentRoots) {
        let hook = AgentHook {
            approvals,
            agents,
            roots,
        };
        if self.inner.agents.set(hook).is_err() {
            tracing::warn!("the guard rails have their agents already");
        }
    }

    /// Whose the task `task` is: the owner's until the agents are handed over.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn task_owner(&self, task: TaskId) -> Result<TaskOwner, DbError> {
        match self.inner.agents.get() {
            Some(hook) => scope::task_owner(&self.inner.store, &hook.agents, task).await,
            None => Ok(TaskOwner::Owner),
        }
    }

    /// Whether the task `task` is not the owner's: a configured agent's, or no one's.
    pub(crate) async fn not_the_owners(&self, task: TaskId) -> Result<bool, DbError> {
        Ok(self.task_owner(task).await? != TaskOwner::Owner)
    }

    async fn scope(&self, task: TaskId) -> Result<Scope<'_>, DbError> {
        let Some(hook) = self.inner.agents.get() else {
            return Ok(Scope::Owner);
        };
        Ok(
            match scope::task_owner(&self.inner.store, &hook.agents, task).await? {
                TaskOwner::Owner => Scope::Owner,
                TaskOwner::Agent(agent) => Scope::Agent(agent, hook),
                TaskOwner::Orphaned => Scope::Orphaned,
            },
        )
    }

    /// A final step of `task` about to be clicked (`what` on `page`, `composed` the words
    /// written for it), as its agent's policy decides it.
    ///
    /// # Errors
    ///
    /// [`GuardError`].
    pub async fn agent_step(
        &self,
        task: TaskId,
        what: &str,
        page: &str,
        composed: &[String],
    ) -> Result<AgentCheck, GuardError> {
        let (agent, hook) = match self.scope(task).await? {
            Scope::Owner => return Ok(AgentCheck::Owner),
            Scope::Orphaned => return Ok(AgentCheck::Held(ORPHANED.to_owned())),
            Scope::Agent(agent, hook) => (agent, hook),
        };
        if !agent.enabled {
            return Ok(AgentCheck::Held(DISABLED.to_owned()));
        }
        let Some(kind) = steps::final_step(grant_rules::kind(what, page)) else {
            // A hard step: the owner's own card, every time.
            return Ok(AgentCheck::Owner);
        };
        let text = composed
            .iter()
            .map(|said| emberhut_core::messaging::text::js_trim(said))
            .find(|said| !said.is_empty())
            .map(str::to_owned);
        let target = if page.trim().is_empty() { what } else { page };
        let ask = StepAsk {
            kind,
            target: target.to_owned(),
            summary: format!("\"{what}\" on {target}"),
            text,
            files: Vec::new(),
            amount_cents: None,
            task: Some(task),
        };
        self.decide(task, &agent, hook, &ask).await
    }

    /// An outward command of `task`'s worker (`command`, run in `cwd`), as its agent's policy
    /// decides it. A command with no outward step is the owner's rules' (which let it run),
    /// except on a disabled agent's task or no one's, where nothing runs.
    ///
    /// # Errors
    ///
    /// [`GuardError`].
    pub async fn agent_outward(
        &self,
        task: TaskId,
        command: &str,
        cwd: &str,
    ) -> Result<AgentCheck, GuardError> {
        let (agent, hook) = match self.scope(task).await? {
            Scope::Owner => return Ok(AgentCheck::Owner),
            Scope::Orphaned => return Ok(AgentCheck::Held(ORPHANED.to_owned())),
            Scope::Agent(agent, hook) => (agent, hook),
        };
        if !agent.enabled {
            return Ok(AgentCheck::Held(DISABLED.to_owned()));
        }
        if !self.is_outward(command) {
            return Ok(AgentCheck::Owner);
        }
        let (kind, target) = self.outward_target(command, cwd).await;
        let ask = StepAsk {
            kind,
            target,
            summary: format!("a {kind} from {}", task.key()),
            text: Some(command.to_owned()),
            files: Vec::new(),
            amount_cents: None,
            task: Some(task),
        };
        self.decide(task, &agent, hook, &ask).await
    }

    /// A built-in tool of `task`'s worker, before it runs (`input` its input, `cwd` where it
    /// runs): on an agent's task, a file tool goes through only on paths in the agent's roots
    /// and any other tool but the shell is held; the shell is the outward rule's
    /// ([`Self::agent_outward`]). A disabled agent's worker, or no one's, runs nothing.
    ///
    /// # Errors
    ///
    /// [`GuardError`].
    pub async fn agent_tool(
        &self,
        task: TaskId,
        tool: &str,
        input: &Value,
        cwd: &str,
    ) -> Result<AgentCheck, GuardError> {
        let (agent, hook) = match self.scope(task).await? {
            Scope::Owner => return Ok(AgentCheck::Owner),
            Scope::Orphaned => return Ok(AgentCheck::Held(ORPHANED.to_owned())),
            Scope::Agent(agent, hook) => (agent, hook),
        };
        if !agent.enabled {
            return Ok(AgentCheck::Held(DISABLED.to_owned()));
        }
        if tool == "Bash" {
            return Ok(AgentCheck::Owner);
        }
        if !FILE_TOOLS.contains(&tool) {
            return Ok(AgentCheck::Held(format!(
                "{tool} is not a tool an agent's worker runs."
            )));
        }
        let roots = hook.roots.of(&agent).await?;
        let outside = agent_files::first_outside(tool, input, std::path::Path::new(cwd), &roots);
        Ok(match outside {
            Some(path) => AgentCheck::Held(format!(
                "Not yours to open: {path}. You work only in your own folder and your tasks' \
                 folders."
            )),
            None => AgentCheck::Through(format!("{tool} in the agent's own folders")),
        })
    }

    /// The kind of an outward command and where it goes: the repositories its steps touch, or
    /// the folder it runs in. Reads the folders' git remotes, off the async threads.
    async fn outward_target(&self, command: &str, cwd: &str) -> (ActionKind, String) {
        let guard = self.clone();
        let (command, cwd) = (command.to_owned(), cwd.to_owned());
        let fallback = cwd.clone();
        tokio::task::spawn_blocking(move || {
            let reader = Reader {
                home: guard.home(),
                remotes: guard.inner.remotes.as_ref(),
            };
            let found = outward_rules::steps_or_none(&command, Some(&cwd), &reader);
            let words: Option<Vec<&str>> = found
                .as_ref()
                .map(|found| found.iter().map(|step| step.what.as_str()).collect());
            let kind = steps::outward(words.as_deref());
            let mut repos: Vec<String> = found
                .iter()
                .flatten()
                .filter_map(|step| step.repos.clone())
                .flatten()
                .collect();
            repos.sort_unstable();
            repos.dedup();
            let target = if repos.is_empty() {
                cwd
            } else {
                repos.join(", ")
            };
            (kind, target)
        })
        .await
        .unwrap_or_else(|error| {
            // Fails closed: the strictest kind, and where it runs.
            tracing::error!(%error, "an outward command's repositories couldn't be read");
            (ActionKind::Deploy, fallback)
        })
    }

    /// `ask` of `agent`'s task as its policy decides it, always through an approval: the
    /// request goes to the approver the policy names (the owner, in the apps, for a kind it
    /// gives the owner), and the step goes through once on a matching approval; a kind nobody
    /// approves goes through.
    async fn decide(
        &self,
        task: TaskId,
        agent: &Agent,
        hook: &AgentHook,
        ask: &StepAsk,
    ) -> Result<AgentCheck, GuardError> {
        let by = format!("task {}", task.key());
        Ok(match hook.approvals.gate(&agent.id, ask, &by).await? {
            Gate::Through(line) => AgentCheck::Through(line),
            Gate::Held(line) => AgentCheck::Held(format!(
                "{line} Stop for now; the step goes through when it is approved."
            )),
        })
    }

    /// The request of someone other than the owner that the default agent's `task` was made
    /// for (its `requested_by`, as an import keeps it), when they are a trusted requester: when,
    /// and the line the job's grant says. Never for a configured agent's task, whoever it names.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub(crate) async fn trusted_request(
        &self,
        task: &Task,
    ) -> Result<Option<(String, String)>, DbError> {
        let Some(hook) = self.inner.agents.get() else {
            return Ok(None);
        };
        if self.not_the_owners(task.id).await? {
            return Ok(None);
        }
        let Some(asked) = task
            .requested_by
            .as_deref()
            .and_then(|text| serde_json::from_str::<RequestedBy>(text).ok())
        else {
            return Ok(None);
        };
        let trusted = hook
            .agents
            .trusted_requesters()
            .await?
            .iter()
            .any(|requester| requester.covers(&asked.who, &AgentId::MAIN));
        if !trusted {
            return Ok(None);
        }
        let when =
            Self::parse(&asked.at).map_or_else(|| "Invalid Date".to_owned(), |at| self.hhmm(at));
        Ok(Some((
            asked.at,
            format!(
                "approved by {}'s request at {when}, a trusted requester",
                asked.who
            ),
        )))
    }
}

/// Who a task was made for, when not the owner (`requested_by`, the reference server's shape).
#[derive(Clone, Debug, serde::Deserialize)]
struct RequestedBy {
    /// Who asked.
    who: String,
    /// When (an ISO instant).
    at: String,
}
