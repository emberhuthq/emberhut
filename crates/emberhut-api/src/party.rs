//! Party: two to five AI personas, each a name, a role and a model, talk about the person's
//! topic turn by turn while the person watches; the person jumps in, steers, pauses, resumes
//! and stops them.
//!
//! - `GET /api/parties`: the parties, newest first, with the ready-made personas, the person's
//!   own from earlier parties and the models ([`PartiesPage`]).
//! - `POST /api/parties` ([`NewParty`]): a new one, started; the answer is a [`PartyReply`].
//! - `GET /api/parties/{id}/live`: server-sent [`PartyEvent`]s, a snapshot first, with a
//!   `: ping` comment every 15 s.
//! - `POST /api/parties/{id}/say|pause|resume|stop|delete` ([`PartyAction`]): a
//!   [`PartyReply`], its party `null` after `delete`.
//!
//! A refusal is 400 (a bad topic or cast) or 404 (no such party) with
//! `{ "ok": false, "error": … }` ([`ErrorBody`](crate::error::ErrorBody)).

use serde::{Deserialize, Serialize};

use crate::ids::{PartyId, PartyLineId, PersonaId};
use crate::open_enum::open_enum;
use crate::time::Timestamp;

/// The fewest personas a party has.
pub const MIN_CAST: usize = 2;
/// The most personas a party has.
pub const MAX_CAST: usize = 5;

open_enum! {
    /// Where a party stands.
    pub enum PartyStatus {
        /// They are talking.
        Running = "running",
        /// Waiting for the person (paused, or the server restarted).
        Paused = "paused",
        /// It reached a cap, or its natural end.
        Done = "done",
        /// The person stopped it.
        Stopped = "stopped",
    }
}

open_enum! {
    /// What a line of a party is.
    pub enum LineKind {
        /// A persona's turn.
        Turn = "turn",
        /// The person jumped in: their words join the talk.
        Say = "say",
        /// The person's direction, which the cast follows.
        Steer = "steer",
    }
}

/// A persona: in a party, among the presets, or one of the person's own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Persona {
    /// Its id.
    pub id: PersonaId,
    /// Its name.
    pub name: String,
    /// Who they are and how they talk: their prompt's heart.
    pub role: String,
    /// Its model's id (`claude-opus-5-5`).
    pub model: String,
    /// Its colour in a party (`blue`, `orange`, `green`, `purple`, `pink`); empty elsewhere.
    #[serde(default)]
    pub color: String,
}

/// A model a persona can talk with, with its list prices.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PartyModel {
    /// Its id.
    pub id: String,
    /// Its name (`Opus 5.5`).
    pub name: String,
    /// US dollars per million tokens in.
    #[serde(rename = "in")]
    pub price_in: f64,
    /// US dollars per million tokens out.
    #[serde(rename = "out")]
    pub price_out: f64,
    /// What it is good for and what it costs, in words.
    pub note: String,
}

/// Who is speaking now and what they have said so far.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Speaking {
    /// The persona's id.
    pub who: PersonaId,
    /// Its name.
    pub name: String,
    /// What it has said so far; empty before its first word.
    pub text: String,
}

/// A party.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Party {
    /// Its id.
    pub id: PartyId,
    /// Its title: the topic's first 80 characters unless one was given.
    pub title: String,
    /// The topic or opening line.
    pub topic: String,
    /// Where it stands.
    pub status: PartyStatus,
    /// Why it paused or ended, in words.
    #[serde(default)]
    pub why: Option<String>,
    /// The cast.
    pub personas: Vec<Persona>,
    /// The turns taken.
    pub turns: u32,
    /// Its turn cap.
    pub max_turns: u32,
    /// The tokens used.
    pub tokens: u64,
    /// Its token cap.
    pub max_tokens: u64,
    /// What it cost, in US dollars at list prices.
    pub cost: f64,
    /// When it was made.
    pub created: Timestamp,
    /// When it last changed.
    pub updated: Timestamp,
    /// Who is speaking now.
    #[serde(default)]
    pub speaking: Option<Speaking>,
    /// The list's preview: the last line said (`Name: text`); only in the list.
    #[serde(default)]
    pub last: Option<String>,
}

impl Party {
    /// The persona with this id.
    #[must_use]
    pub fn persona(&self, who: &PersonaId) -> Option<&Persona> {
        self.personas.iter().find(|persona| persona.id == *who)
    }
}

/// A line said in a party.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PartyLine {
    /// Its id.
    pub id: PartyLineId,
    /// The party it was said in.
    pub party: PartyId,
    /// Who said it: a persona's id, or the person's (`owner`).
    pub who: PersonaId,
    /// Their name.
    pub name: String,
    /// What it is.
    pub kind: LineKind,
    /// What was said.
    pub text: String,
    /// The model a persona's turn came from.
    #[serde(default)]
    pub model: Option<String>,
    /// The tokens it took.
    #[serde(default)]
    pub tokens: u64,
    /// What it cost, in US dollars.
    #[serde(default)]
    pub cost: f64,
    /// It was stopped mid-turn.
    #[serde(default)]
    pub cut: bool,
    /// When it was said.
    pub at: Timestamp,
}

/// The caps a new party starts with, unless the person sets others.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PartyDefaults {
    /// The turn cap.
    pub max_turns: u32,
    /// The token cap.
    pub max_tokens: u64,
    /// The model a persona talks with.
    pub model: String,
}

/// `GET /api/parties`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PartiesPage {
    /// The parties, the one changed last first.
    pub parties: Vec<Party>,
    /// The ready-made personas.
    pub presets: Vec<Persona>,
    /// The person's own personas from earlier parties, newest first, one per name.
    pub recent: Vec<Persona>,
    /// The models a persona can talk with.
    pub models: Vec<PartyModel>,
    /// The caps and model a new party starts with.
    #[serde(default)]
    pub defaults: Option<PartyDefaults>,
}

/// A persona of a new party: its id and colour are the server's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct CastMember {
    /// Its name, 40 characters at most; each persona's differs.
    #[cfg_attr(
        feature = "validate",
        garde(custom(crate::rules::not_blank), length(utf16, max = 40))
    )]
    pub name: String,
    /// Who they are and how they talk; a plain guest when empty.
    #[cfg_attr(feature = "validate", garde(length(utf16, max = 2000)))]
    pub role: String,
    /// The model's id; an unknown one talks with the default.
    #[cfg_attr(feature = "validate", garde(skip))]
    pub model: String,
}

/// `POST /api/parties`: a new party, started at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct NewParty {
    /// The topic or opening line, 4,000 characters at most.
    #[cfg_attr(
        feature = "validate",
        garde(custom(crate::rules::not_blank), length(utf16, max = 4000))
    )]
    pub topic: String,
    /// The turn cap (the server keeps it within 1 to 100).
    #[cfg_attr(feature = "validate", garde(range(min = 1, max = 100)))]
    pub max_turns: u32,
    /// The token cap (the server keeps it within 5,000 to 2,000,000).
    #[cfg_attr(feature = "validate", garde(range(min = 5_000, max = 2_000_000)))]
    pub max_tokens: u64,
    /// The cast: [`MIN_CAST`] to [`MAX_CAST`] personas.
    #[cfg_attr(feature = "validate", garde(length(min = 2, max = 5), dive))]
    pub personas: Vec<CastMember>,
    /// Its title; the topic's start when none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub title: Option<String>,
}

/// What the person's line in a party is: their words, or a direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum SayKind {
    /// Jump in: the words join the talk; a persona named answers.
    Say,
    /// Steer: a direction everyone follows.
    Steer,
}

/// `POST /api/parties/{id}/say`: the person jumps in or steers. A party that waits starts
/// again to answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct PartySay {
    /// What they say, 4,000 characters at most.
    #[cfg_attr(
        feature = "validate",
        garde(custom(crate::rules::not_blank), length(utf16, max = 4000))
    )]
    pub text: String,
    /// What it is.
    #[cfg_attr(feature = "validate", garde(skip))]
    pub kind: SayKind,
}

/// One of the person's controls on a party: `POST /api/parties/{id}/{route}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartyAction {
    /// Jump in or steer.
    Say(PartySay),
    /// Pause after the turn being spoken.
    Pause,
    /// Carry on; one that reached a cap gets 10 more turns (and half its token cap again).
    Resume,
    /// Stop now, keeping what was said.
    Stop,
    /// Delete it and its talk.
    Delete,
}

impl PartyAction {
    /// The last segment of its route.
    #[must_use]
    pub fn route(&self) -> &'static str {
        match self {
            Self::Say(_) => "say",
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Stop => "stop",
            Self::Delete => "delete",
        }
    }
}

/// The answer to a new party and to the person's controls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PartyReply {
    /// Always `true`: a refusal is an `ErrorBody`.
    pub ok: bool,
    /// The party as it is now; `null` after a delete.
    #[serde(default)]
    pub party: Option<Party>,
}

/// An event of a party's live stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum PartyEvent {
    /// The party and all its talk, first on every stream.
    Snapshot {
        /// The party.
        party: Option<Party>,
        /// What was said, oldest first.
        #[serde(default)]
        messages: Vec<PartyLine>,
    },
    /// The party changed: its status, caps, meter or who speaks.
    Party {
        /// The party as it is now.
        party: Option<Party>,
    },
    /// A line was said.
    Message {
        /// The line.
        message: PartyLine,
    },
    /// What the persona speaking has said so far; `null` once the turn is over.
    Speaking {
        /// Who and what.
        speaking: Option<Speaking>,
    },
}
