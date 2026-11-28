//! `GET /api/facts`: what the assistant knows about the person (one short fact per line, in
//! categories), the standing rules the person agreed to, and the rules on trial: the lessons of
//! the person's recent corrections, followed for 14 days until a weekly review keeps or drops
//! each one.
//!
//! Facts are corrected, forgotten or marked (a person whose messages wake the assistant), and
//! rules on trial kept early or dropped, through `POST /api/settings`
//! ([`crate::settings::SettingsAction`]).

use serde::{Deserialize, Serialize};

use crate::ids::{FactId, LessonId, TrialId};
use crate::open_enum::open_enum;
use crate::time::Timestamp;

open_enum! {
    /// A fact's category.
    pub enum FactCategory {
        /// People the person deals with.
        People = "people",
        /// Work.
        Work = "work",
        /// Money.
        Money = "money",
        /// How the person likes things done.
        Preferences = "preferences",
        /// Health and habits.
        Health = "health",
        /// Home and family.
        Home = "home",
        /// Projects.
        Projects = "projects",
        /// Evidence kept for later (not listed for the person).
        Proof = "proof",
    }
}

/// One fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Fact {
    /// Its id: its category and name, such as `people/example-person`.
    pub id: FactId,
    /// Its category.
    pub category: FactCategory,
    /// Its title (a person's name, for `people`).
    pub title: String,
    /// What is true.
    pub text: String,
    /// Where it came from, in words (`you corrected it, 2026-11-24` after a correction); may be
    /// empty.
    pub source: String,
    /// When it last changed.
    pub updated: Timestamp,
    /// People only: the assistant's running note on what is going on with them now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub now: Option<String>,
    /// When that note was written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub now_at: Option<Timestamp>,
    /// People only: the chats that are theirs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chats: Option<Vec<String>>,
    /// People only: their messages wake the assistant at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub important: Option<bool>,
    /// The project it belongs to (its name); none: every room has it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Who put it there: `owner`, `assistant` (in a turn the person started), `message`
    /// (in a turn something else started: someone else's message, mail, a scheduled wake),
    /// `web` or `import`; none from before this was kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// Where in the record it was said (`chat #22`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// It waits for the person to confirm it ("To confirm"), and is not in what the assistant
    /// always knows until they do.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pending: bool,
    /// The person's say on the profile: `pin` (always in it) or `hide` (never).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

impl Fact {
    /// Whether this person's messages wake the assistant at once.
    #[must_use]
    pub fn is_important(&self) -> bool {
        self.important == Some(true)
    }
}

/// One piece of the record recall holds, as "Forget everywhere" lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RecordPiece {
    /// Its id in recall's index.
    pub id: i64,
    /// Its kind: `chat`, `session`, `summary`, `task`, `voice` or `note`.
    pub source: String,
    /// What it is, in a few words.
    pub title: String,
    /// What it says (at most 600 characters of it).
    pub text: String,
    /// Where it is (`chat #12`).
    pub place: String,
    /// When it was said, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Timestamp>,
}

/// `GET /api/facts/forget`'s answer: the fact, and the pieces of the record that say it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ForgetPreview {
    /// The fact.
    pub fact: Fact,
    /// The pieces, best first.
    pub pieces: Vec<RecordPiece>,
}

/// A standing rule the person agreed to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Lesson {
    /// Its number.
    pub id: LessonId,
    /// The rule.
    pub text: String,
    /// When it was set.
    pub at: Timestamp,
}

/// A rule on trial: one correction's lesson.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Trial {
    /// Its id (the correction's).
    pub id: TrialId,
    /// The rule.
    pub text: String,
    /// When the correction was made.
    pub at: Timestamp,
    /// When the trial ends unless the review keeps it.
    pub ends: Timestamp,
    /// What the person said (its first 300 characters).
    pub said: String,
    /// The lesson in one short line addressed to the person, in the second person ("Story
    /// reels run at least 60 seconds; I check each cut's length before showing you."). None
    /// when the assistant gave none, or from a server that doesn't keep one: a client shows
    /// `text` then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lesson: Option<String>,
}

/// `GET /api/facts`'s answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FactsPage {
    /// Every fact, the most recently changed first.
    pub facts: Vec<Fact>,
    /// The standing rules.
    #[serde(default)]
    pub lessons: Vec<Lesson>,
    /// The rules on trial, the oldest first.
    #[serde(default)]
    pub trials: Vec<Trial>,
}
