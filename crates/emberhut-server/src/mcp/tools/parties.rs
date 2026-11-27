//! `party_start` and `parties`: starting a party for the user (AI personas talking about a
//! topic on the app's Party screen), and reading the list or one party's talk.

use emberhut_core::PartyId;
use emberhut_core::party::{self, NewParty};
use serde::Deserialize;

use super::{Tool, ToolContext, ToolError, ToolOutput};
use crate::services::parties::{PartyError, cast_from_tool};

/// Starts a party.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PartyStart;

/// One persona the agent casts.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct CastArg {
    /// A name of its own, or a preset's name or id alone.
    #[garde(length(min = 1, max = 200))]
    name: String,
    /// Who they are and how they talk; left out, a preset's own.
    #[garde(length(max = 4000))]
    role: Option<String>,
    /// claude-opus-5-5 (the default), claude-sonnet-5-5 or claude-haiku-5-5.
    #[garde(length(max = 100))]
    model: Option<String>,
}

/// `party_start`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct PartyStartArgs {
    /// The topic or opening line.
    #[garde(length(min = 3))]
    topic: String,
    /// 2 to 5; a preset's name alone takes its role.
    #[garde(dive)]
    personas: Vec<CastArg>,
    /// The turn cap, 1 to 100.
    #[garde(range(min = 1, max = 100))]
    #[schema(minimum = 1, maximum = 100)]
    max_turns: Option<u32>,
}

impl Tool for PartyStart {
    type Args = PartyStartArgs;

    fn name(&self) -> &'static str {
        "party_start"
    }

    fn description(&self) -> &'static str {
        "Start a Party in the user's app: 2 to 5 AI personas (each a name, a role and a model) \
         talk about a topic, turn by turn, while the user watches live (the app's Party screen) \
         and can jump in, steer, pause or stop. Only when the user asks for one. Models: \
         claude-opus-5-5 (default), claude-sonnet-5-5, claude-haiku-5-5. Presets the user knows: \
         Nora (optimist), Victor (skeptic), Sam (engineer), Lena (historian), Omar (devil), Rita \
         (product), Dr. Kim (scientist), Jules (comedian)."
    }

    async fn call(&self, ctx: &ToolContext, args: PartyStartArgs) -> Result<ToolOutput, ToolError> {
        let new = NewParty {
            topic: args.topic,
            personas: args
                .personas
                .iter()
                .map(|p| cast_from_tool(&p.name, p.role.as_deref(), p.model.as_deref()))
                .collect(),
            title: None,
            max_turns: args.max_turns.map(f64::from),
            max_tokens: None,
        };
        match ctx.app.parties().create(&new, true).await {
            Ok(p) => {
                let names: Vec<&str> = p.personas.iter().map(|c| c.name.as_str()).collect();
                Ok(ToolOutput::text(format!(
                    "Party {} started: {}, with {}. It's on the user's Party screen; up to {} \
                     turns.",
                    p.id,
                    p.title,
                    party::names(&names),
                    p.max_turns
                )))
            }
            Err(PartyError::Refused(refusal)) => Err(ToolError::refused(refusal.to_string())),
            Err(error) => Err(ToolError::failed(error)),
        }
    }
}

/// The parties, or one party's talk.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Parties;

/// `parties`' arguments.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct PartiesArgs {
    /// One party's id: its talk in full.
    #[garde(skip)]
    id: Option<i64>,
}

impl Tool for Parties {
    type Args = PartiesArgs;

    fn name(&self) -> &'static str {
        "parties"
    }

    fn description(&self) -> &'static str {
        "The user's Parties (AI personas talking, the app's Party screen): the list, or one \
         party's talk in full (id)."
    }

    async fn call(&self, ctx: &ToolContext, args: PartiesArgs) -> Result<ToolOutput, ToolError> {
        let parties = ctx.app.parties();
        let Some(id) = args.id else {
            return parties
                .list_text()
                .await
                .map(ToolOutput::text)
                .map_err(ToolError::failed);
        };
        match parties.party_text(PartyId::new(id)).await {
            Ok(text) => Ok(ToolOutput::text(text)),
            Err(PartyError::NotFound) => Err(ToolError::refused("no such party")),
            Err(error) => Err(ToolError::failed(error)),
        }
    }
}
