//! `facts`, `fact_set` and `fact_drop` for a configured agent: its own facts (category
//! `agent-<id>`), which it keeps and forgets, and, read-only, the facts of the project it is
//! scoped to. None of the owner's other facts.

use emberhut_core::agents::Agent;
use emberhut_core::memory::facts::{CATEGORIES, Fact};
use serde::{Deserialize, Serialize};

use super::agent_set::calling_agent;
use super::{NoArgs, Tool, ToolContext, ToolError, ToolOutput};
use crate::services::facts::{FactError, ProjectChange, SetFact};

/// A fact refused (its text for the agent) or failed.
fn fact_error(error: FactError) -> ToolError {
    match error {
        FactError::Refused(text) => ToolError::refused(text),
        other => ToolError::failed(other),
    }
}

/// The start of every id of `agent`'s own facts: `agent-<id>/`.
fn own_prefix(agent: &Agent) -> String {
    format!("{}/", agent.id.facts_category())
}

/// A fact as the agent reads it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentFact {
    id: String,
    title: String,
    text: String,
    source: String,
    updated: String,
    /// One of its project's facts: read, never changed.
    read_only: bool,
}

impl AgentFact {
    fn of(fact: Fact, read_only: bool) -> Self {
        Self {
            id: fact.id,
            title: fact.title,
            text: fact.text,
            source: fact.source,
            updated: fact.updated,
            read_only,
        }
    }
}

/// The agent's facts.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AgentFacts;

impl Tool for AgentFacts {
    type Args = NoArgs;

    fn name(&self) -> &'static str {
        "facts"
    }

    fn description(&self) -> &'static str {
        "Your facts: what you keep about the people you work with, their work, decisions and \
         standing rules (fact_set keeps more). Your project's facts come too, read-only."
    }

    async fn call(&self, ctx: &ToolContext, _: NoArgs) -> Result<ToolOutput, ToolError> {
        let agent = calling_agent(ctx).await?;
        let facts = ctx.app.facts();
        let mut shown: Vec<AgentFact> = facts
            .facts(Some(&agent.id.facts_category()))
            .await
            .map_err(fact_error)?
            .into_iter()
            .map(|fact| AgentFact::of(fact, false))
            .collect();
        let project = match agent.project {
            Some(id) => ctx.app.tasks().project_name(id).await?,
            None => None,
        };
        if let Some(project) = project {
            shown.extend(
                facts
                    .facts(None)
                    .await
                    .map_err(fact_error)?
                    .into_iter()
                    .filter(|fact| {
                        CATEGORIES.contains(&fact.category.as_str())
                            && fact.project.as_deref() == Some(project.as_str())
                    })
                    .map(|fact| AgentFact::of(fact, true)),
            );
        }
        ToolOutput::json(&shown)
    }
}

/// Keeps one of the agent's facts.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AgentFactSet;

/// The agent's `fact_set`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct AgentFactSetArgs {
    /// The fact's id, to change it (one of yours); left out: a new fact (or the one with this
    /// title).
    #[garde(length(max = 200))]
    id: Option<String>,
    /// A few words ("Example person", "Rule: decks").
    #[garde(length(min = 1, max = 200))]
    title: String,
    /// One or two plain sentences.
    #[garde(length(max = 20_000))]
    text: String,
    /// Where it came from ("they said in #launch, 14 Oct").
    #[garde(length(max = 500))]
    source: Option<String>,
}

impl Tool for AgentFactSet {
    type Args = AgentFactSetArgs;

    fn name(&self) -> &'static str {
        "fact_set"
    }

    fn description(&self) -> &'static str {
        "Keep one fact of yours (new: no id; change: its id). title: a few words; text: one or \
         two plain sentences; source: where it came from. One fact per thing: update, don't \
         duplicate."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: AgentFactSetArgs,
    ) -> Result<ToolOutput, ToolError> {
        let agent = calling_agent(ctx).await?;
        let prefix = own_prefix(&agent);
        let id = args.id.filter(|id| !id.is_empty());
        if id.as_deref().is_some_and(|id| !id.starts_with(&prefix)) {
            return Err(ToolError::refused(format!(
                "Not yours: your facts' ids start with {prefix}"
            )));
        }
        let id = ctx
            .app
            .facts()
            .set_fact(SetFact {
                id,
                category: agent.id.facts_category(),
                title: args.title,
                text: args.text,
                source: args.source,
                project: ProjectChange::Keep,
            })
            .await
            .map_err(fact_error)?;
        Ok(ToolOutput::text(format!("Kept as {id}.")))
    }
}

/// Forgets one of the agent's facts.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AgentFactDrop;

/// The agent's `fact_drop`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct AgentFactDropArgs {
    /// The fact's id (one of yours).
    #[garde(length(min = 1, max = 200))]
    id: String,
}

impl Tool for AgentFactDrop {
    type Args = AgentFactDropArgs;

    fn name(&self) -> &'static str {
        "fact_drop"
    }

    fn description(&self) -> &'static str {
        "Forget one of your facts that is wrong or no longer true."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: AgentFactDropArgs,
    ) -> Result<ToolOutput, ToolError> {
        let agent = calling_agent(ctx).await?;
        let prefix = own_prefix(&agent);
        if !args.id.starts_with(&prefix) {
            return Err(ToolError::refused(format!(
                "Not yours: your facts' ids start with {prefix}"
            )));
        }
        ctx.app
            .facts()
            .drop_fact(&args.id)
            .await
            .map_err(fact_error)?;
        Ok(ToolOutput::text("Forgotten."))
    }
}
