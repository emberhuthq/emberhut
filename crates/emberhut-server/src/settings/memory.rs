//! What the owner changes on the app's "About you" page: `factSet` (a corrected fact),
//! `personSet` (whether a person's messages wake the assistant), `factDrop` and `factForget`
//! (forget a fact everywhere), `factConfirm` (confirm one that waits), `factProfile` (pin or
//! hide one in the profile), `memoryNightly` (the nightly memory review on or off),
//! `trialDrop` (drop a rule on trial) and `trialKeep` (keep one now). The assistant hears of
//! each fact change as a `fact` event in Main; a dropped trial rule is gone from its rules, and
//! a kept one is among its standing rules, the next time it reads them.

use emberhut_core::memory::facts::origin;
use emberhut_core::memory::learning::TrialJudge;
use serde_json::Value;

use super::{Action, ActionContext, ActionError, Body};
use crate::services::facts::{FactError, ProjectChange, SetFact, SetPerson};

/// A fact change refused (a category that isn't one, an id that isn't a fact) or failed.
fn fact_error(error: FactError) -> ActionError {
    match error {
        FactError::Refused(text) => ActionError::refused(text),
        other => ActionError::failed(other),
    }
}

/// The text of `value`: a string as it is,
/// anything else as its JSON.
fn as_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// `{ factSet: { id?, category, title, text, project? } }`: the owner corrected (or added) a
/// fact; its source becomes "you corrected it" with the day. A `factSet` that is not an
/// object asks for nothing.
#[derive(Clone, Copy, Debug)]
pub(super) struct FactSet;

impl Action for FactSet {
    fn key(&self) -> &'static str {
        "factSet"
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(Value::Object(change)) = body.get("factSet") else {
            return Ok(());
        };
        let text = |key: &str| change.get(key).and_then(Value::as_str).map(str::to_owned);
        let facts = ctx.app.facts();
        let day =
            emberhut_core::memory::cut(&emberhut_core::memory::iso(facts.now()), 10).to_owned();
        let set = SetFact {
            id: text("id"),
            category: text("category").unwrap_or_default(),
            title: text("title").unwrap_or_default(),
            text: text("text").unwrap_or_default(),
            source: Some(format!("you corrected it, {day}")),
            project: change
                .get("project")
                .map_or(ProjectChange::Keep, |project| {
                    ProjectChange::given(project.as_str())
                }),
            origin: Some(origin::OWNER.to_owned()),
            from: None,
            pending: false,
        };
        let shown = set.text.clone();
        let id = facts.set_fact(set).await.map_err(fact_error)?;
        facts
            .tell(format!("The user corrected what you know: {id}: {shown}"))
            .await
            .map_err(ActionError::failed)
    }
}

/// `{ personSet: { id, important } }`: the owner marks whether a person's messages wake the
/// assistant at once. A `personSet` that is not an object asks for nothing.
#[derive(Clone, Copy, Debug)]
pub(super) struct PersonSet;

impl Action for PersonSet {
    fn key(&self) -> &'static str {
        "personSet"
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(Value::Object(change)) = body.get("personSet") else {
            return Ok(());
        };
        let important = match change.get("important") {
            Some(Value::Bool(flag)) => *flag,
            Some(Value::Null) | None => false,
            Some(Value::Number(number)) => number.as_f64().is_some_and(|n| n != 0.0),
            Some(Value::String(text)) => !text.is_empty(),
            Some(_) => true,
        };
        let facts = ctx.app.facts();
        let id = facts
            .set_person(SetPerson {
                id: Some(as_text(change.get("id"))),
                important: Some(important),
                ..SetPerson::default()
            })
            .await
            .map_err(fact_error)?;
        let told = if important {
            format!("The user marked {id} as important: their messages wake you at once.")
        } else {
            format!("The user unmarked {id} as important.")
        };
        facts.tell(told).await.map_err(ActionError::failed)
    }
}

/// `{ factDrop: "<id>" }`: the owner told the assistant to forget a fact: it is forgotten
/// everywhere, with every piece of the record that says it. A value that is not text asks for
/// nothing.
#[derive(Clone, Copy, Debug)]
pub(super) struct FactDrop;

impl Action for FactDrop {
    fn key(&self) -> &'static str {
        "factDrop"
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(Value::String(id)) = body.get("factDrop") else {
            return Ok(());
        };
        forget_everywhere(ctx, id, None).await
    }
}

/// `{ factForget: { id, pieces? } }`: the owner forgets a fact everywhere, with the pieces of
/// the record they picked (none: every one that says it). A `factForget` that is not an object
/// with an id asks for nothing.
#[derive(Clone, Copy, Debug)]
pub(super) struct FactForget;

impl Action for FactForget {
    fn key(&self) -> &'static str {
        "factForget"
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(Value::Object(forget)) = body.get("factForget") else {
            return Ok(());
        };
        let Some(id) = forget.get("id").and_then(Value::as_str) else {
            return Ok(());
        };
        let pieces = forget
            .get("pieces")
            .and_then(Value::as_array)
            .map(|pieces| pieces.iter().filter_map(Value::as_i64).collect());
        forget_everywhere(ctx, id, pieces).await
    }
}

/// Forgets fact `id` everywhere and tells the assistant, without repeating what it said.
async fn forget_everywhere(
    ctx: &ActionContext,
    id: &str,
    pieces: Option<Vec<i64>>,
) -> Result<(), ActionError> {
    let forgotten = match ctx.app.recall().forget_everywhere(id, pieces).await {
        Ok(forgotten) => forgotten,
        Err(crate::services::recall::RecallError::Facts(error)) => return Err(fact_error(error)),
        Err(error) => return Err(ActionError::failed(error)),
    };
    ctx.app
        .facts()
        .tell(format!(
            "The user told you to forget {id} everywhere: it and {} pieces of what was said are \
             gone from your memory. Don't keep it again, and don't bring it up.",
            forgotten.pieces
        ))
        .await
        .map_err(ActionError::failed)
}

/// `{ factConfirm: "<id>" }`: the owner confirms a fact that waited for them. A value that is
/// not text asks for nothing.
#[derive(Clone, Copy, Debug)]
pub(super) struct FactConfirm;

impl Action for FactConfirm {
    fn key(&self) -> &'static str {
        "factConfirm"
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(Value::String(id)) = body.get("factConfirm") else {
            return Ok(());
        };
        let facts = ctx.app.facts();
        facts.confirm(id).await.map_err(fact_error)?;
        facts
            .tell(format!("The user confirmed {id}."))
            .await
            .map_err(ActionError::failed)
    }
}

/// `{ factProfile: { id, profile } }`: the owner pins a fact in the profile (`pin`), hides it
/// from it (`hide`), or leaves it to come as it comes (none). A `factProfile` that is not an
/// object with an id asks for nothing.
#[derive(Clone, Copy, Debug)]
pub(super) struct FactProfile;

impl Action for FactProfile {
    fn key(&self) -> &'static str {
        "factProfile"
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(Value::Object(say)) = body.get("factProfile") else {
            return Ok(());
        };
        let Some(id) = say.get("id").and_then(Value::as_str) else {
            return Ok(());
        };
        let profile = say.get("profile").and_then(Value::as_str);
        ctx.app
            .facts()
            .set_profile(id, profile)
            .await
            .map_err(fact_error)
    }
}

/// `{ memoryNightly: true | false }`: the nightly memory review on or off. A value that is not
/// a boolean asks for nothing.
#[derive(Clone, Copy, Debug)]
pub(super) struct MemoryNightly;

impl Action for MemoryNightly {
    fn key(&self) -> &'static str {
        "memoryNightly"
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(on) = body.get("memoryNightly").and_then(Value::as_bool) else {
            return Ok(());
        };
        ctx.app
            .nightly()
            .set_on(on)
            .await
            .map_err(ActionError::failed)
    }
}

/// `{ trialDrop: <correction id> }`: the owner drops a rule on trial. A value that is not a
/// whole number asks for nothing; one that isn't on trial changes nothing.
#[derive(Clone, Copy, Debug)]
pub(super) struct TrialDrop;

impl Action for TrialDrop {
    fn key(&self) -> &'static str {
        "trialDrop"
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(id) = body.get("trialDrop").and_then(Value::as_i64) else {
            return Ok(());
        };
        ctx.app
            .learning()
            .drop_trial(id, TrialJudge::Owner)
            .await
            .map(|_dropped| ())
            .map_err(ActionError::failed)
    }
}

/// `{ trialKeep: <correction id> }`: the owner keeps a rule on trial now, as a standing rule,
/// instead of leaving it to the weekly review. A value that is not a whole number asks for
/// nothing; one that isn't on trial changes nothing.
#[derive(Clone, Copy, Debug)]
pub(super) struct TrialKeep;

impl Action for TrialKeep {
    fn key(&self) -> &'static str {
        "trialKeep"
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(id) = body.get("trialKeep").and_then(Value::as_i64) else {
            return Ok(());
        };
        ctx.app
            .learning()
            .keep_trial(id, TrialJudge::Owner)
            .await
            .map(|_kept| ())
            .map_err(ActionError::failed)
    }
}
