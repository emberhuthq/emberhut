//! `facts`, `fact_set`, `fact_drop`, `person_set` and `context`: what the assistant keeps about
//! the owner (the context folder, one short fact per file, by category), its cards on the
//! people the owner talks with, and its running awareness.

use emberhut_core::memory::facts::{CATEGORIES, origin};
use serde::Deserialize;

use super::{NoArgs, Tool, ToolContext, ToolError, ToolOutput};
use crate::services::facts::{FactError, ProjectChange, SetFact, SetPerson, fact_to_api};

/// A fact refused (its text for the agent) or failed.
fn fact_error(error: FactError) -> ToolError {
    match error {
        FactError::Refused(text) => ToolError::refused(text),
        other => ToolError::failed(other),
    }
}

/// Whether `category` is one of the categories.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "garde's custom rules take their context by reference"
)]
fn known_category(category: &str, (): &()) -> garde::Result {
    if CATEGORIES.contains(&category) {
        Ok(())
    } else {
        Err(garde::Error::new(format!(
            "must be one of {}",
            CATEGORIES.join(", ")
        )))
    }
}

/// Reads `project` when it is given: a name, or empty or `null` for shared again (left out, it
/// is [`ProjectChange::Keep`], the default).
fn given_project<'de, D>(deserializer: D) -> Result<ProjectChange, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)
        .map(|project| ProjectChange::given(project.as_deref()))
}

/// Every fact the assistant keeps.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Facts;

/// `facts`' arguments.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct FactsArgs {
    /// Only this category: people, work, money, preferences, health, home or projects.
    #[garde(inner(custom(known_category)))]
    category: Option<String>,
}

impl Tool for Facts {
    type Args = FactsArgs;

    fn name(&self) -> &'static str {
        "facts"
    }

    fn description(&self) -> &'static str {
        "Everything you keep about the user in your context folder: short facts by category \
         (people, work, money, preferences, health, home, projects), each with its id, source, \
         origin and date; pending ones wait for the user to confirm them. The user reads and \
         edits them in the app. What matters most is already in your prompt (About the user)."
    }

    async fn call(&self, ctx: &ToolContext, args: FactsArgs) -> Result<ToolOutput, ToolError> {
        let facts = ctx
            .app
            .facts()
            .facts(args.category.as_deref())
            .await
            .map_err(fact_error)?;
        let facts: Vec<_> = facts.into_iter().map(fact_to_api).collect();
        ToolOutput::json(&facts)
    }
}

/// Keeps one fact.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FactSetTool;

/// `fact_set`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct FactSetArgs {
    /// The fact's id, to change it; left out: a new fact (or the one with this title).
    #[garde(length(max = 200))]
    id: Option<String>,
    /// people, work, money, preferences, health, home or projects.
    #[garde(custom(known_category))]
    category: String,
    /// A few words.
    #[garde(length(min = 1, max = 200))]
    title: String,
    /// One or two plain sentences.
    #[garde(length(max = 20_000))]
    text: String,
    /// Where it came from ("they said, 14 Oct").
    #[garde(length(max = 500))]
    source: Option<String>,
    /// The project it belongs to (its name), so that project's room starts with it; leave it
    /// out for what every room needs; empty or null: shared again.
    #[serde(default, deserialize_with = "given_project")]
    #[schema(value_type = Option<String>)]
    #[garde(skip)]
    project: ProjectChange,
}

impl Tool for FactSetTool {
    type Args = FactSetArgs;

    fn name(&self) -> &'static str {
        "fact_set"
    }

    fn description(&self) -> &'static str {
        "Keep one fact about the user (new: no id; change: its id). title: a few words. text: \
         one or two plain sentences they'd recognise, with dates as dates (\"3 Dec\", not \"next \
         week\"). source: where it came from (\"they said, 14 Oct\", \"a message from a \
         friend\"). One fact per thing: update the one there is, don't add a near copy. \
         project: the project it belongs to (its name), so that project's room starts with it; \
         leave it out for what every room needs. Kept while you handle an Event (someone else's \
         message, mail, a scheduled wake), it waits for the user to confirm it."
    }

    async fn call(&self, ctx: &ToolContext, args: FactSetArgs) -> Result<ToolOutput, ToolError> {
        // The server decides how far the fact can be trusted, from what started the turn.
        let started = ctx.app.talk().started(ctx.room);
        let by_event = ctx.worker.is_none() && started.is_some_and(|start| !start.by_owner);
        let id = ctx
            .app
            .facts()
            .set_fact(SetFact {
                id: args.id,
                category: args.category,
                title: args.title,
                text: args.text,
                source: args.source,
                project: args.project,
                origin: Some(
                    if by_event {
                        origin::MESSAGE
                    } else {
                        origin::ASSISTANT
                    }
                    .to_owned(),
                ),
                from: started.map(|start| format!("chat #{}", start.message.get())),
                pending: by_event,
            })
            .await
            .map_err(fact_error)?;
        Ok(ToolOutput::text(if by_event {
            format!(
                "Kept as {id}, waiting for the user to confirm it: it came from a turn they \
                 didn't start, so it isn't in what you know of them until they do."
            )
        } else {
            format!("Kept as {id}.")
        }))
    }
}

/// Confirms a fact that waits for the owner.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FactConfirmTool;

/// `fact_confirm`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct FactConfirmArgs {
    /// The fact's id.
    #[garde(length(min = 1, max = 200))]
    id: String,
}

impl Tool for FactConfirmTool {
    type Args = FactConfirmArgs;

    fn name(&self) -> &'static str {
        "fact_confirm"
    }

    fn description(&self) -> &'static str {
        "The user confirmed a fact that was waiting for them (pending: kept from someone else's \
         message or another event): it becomes part of what you always know. Only when the user \
         says so in their own message."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: FactConfirmArgs,
    ) -> Result<ToolOutput, ToolError> {
        let by_owner = ctx.worker.is_none()
            && ctx
                .app
                .talk()
                .started(ctx.room)
                .is_none_or(|start| start.by_owner);
        if !by_owner {
            return Err(ToolError::refused(
                "Only the user confirms a fact, in a message of their own; this turn started \
                 from an event.",
            ));
        }
        ctx.app
            .facts()
            .confirm(&args.id)
            .await
            .map_err(fact_error)?;
        Ok(ToolOutput::text(format!("{} is confirmed.", args.id)))
    }
}

/// Forgets a fact.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FactDropTool;

/// `fact_drop`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct FactDropArgs {
    /// The fact's id.
    #[garde(length(min = 1, max = 200))]
    id: String,
}

impl Tool for FactDropTool {
    type Args = FactDropArgs;

    fn name(&self) -> &'static str {
        "fact_drop"
    }

    fn description(&self) -> &'static str {
        "Forget a fact that is wrong or no longer true."
    }

    async fn call(&self, ctx: &ToolContext, args: FactDropArgs) -> Result<ToolOutput, ToolError> {
        ctx.app
            .facts()
            .drop_fact(&args.id)
            .await
            .map_err(fact_error)?;
        Ok(ToolOutput::text("Forgotten."))
    }
}

/// Keeps the assistant's card on one person.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PersonSetTool;

/// `person_set`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct PersonSetArgs {
    /// The card's id.
    #[garde(length(max = 200))]
    id: Option<String>,
    /// The person's name only, never a description.
    #[garde(length(max = 200))]
    who: Option<String>,
    /// What's going on with them right now, rewritten whole.
    #[garde(length(max = 5000))]
    now: Option<String>,
    /// A chat id from the messages, linked to the card.
    #[garde(length(max = 500))]
    chat: Option<String>,
    /// true: their messages wake you at once; false stops that.
    #[garde(skip)]
    important: Option<bool>,
    /// The project this person belongs to (its name): their messages go to that project's
    /// room. Empty: back to Main.
    #[serde(default, deserialize_with = "given_project")]
    #[schema(value_type = Option<String>)]
    #[garde(skip)]
    project: ProjectChange,
}

impl Tool for PersonSetTool {
    type Args = PersonSetArgs;

    fn name(&self) -> &'static str {
        "person_set"
    }

    fn description(&self) -> &'static str {
        "Your card on one person (or group) the user talks with, for what's going on with them \
         now. who: the person's name only (never a description: what they are goes in now or \
         fact_set); it finds or makes the card, named for them. Or id: the card's id. now: \
         what's going on with them right now (what they want from the user, what the user \
         promised, where it stands, the mood), one to three sentences, rewritten whole, not \
         appended. chat: the chat id from the messages (wa:662, slack:Team:Ada), which links \
         it so their next messages come with this card. important: true wakes you as soon as \
         they write, false stops that. Who they are stays in fact_set."
    }

    async fn call(&self, ctx: &ToolContext, args: PersonSetArgs) -> Result<ToolOutput, ToolError> {
        let id = ctx
            .app
            .facts()
            .set_person(SetPerson {
                id: args.id,
                who: args.who,
                now: args.now,
                chat: args.chat,
                important: args.important,
                project: args.project,
            })
            .await
            .map_err(fact_error)?;
        Ok(ToolOutput::text(format!("Card {id}.")))
    }
}

/// The assistant's running awareness.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Context;

impl Tool for Context {
    type Args = NoArgs;

    fn name(&self) -> &'static str {
        "context"
    }

    fn description(&self) -> &'static str {
        "Your running awareness, kept by you: open loops (who waits on the user, their \
         promises, deadlines) with ids, recent notes, and when it last looked; maybeAnswered \
         on loops the user wrote in since, stale on quiet ones. Also your routines and quiet \
         hours."
    }

    async fn call(&self, ctx: &ToolContext, _args: NoArgs) -> Result<ToolOutput, ToolError> {
        let context = ctx
            .app
            .context()
            .context()
            .await
            .map_err(ToolError::failed)?;
        ToolOutput::json(&context)
    }
}
