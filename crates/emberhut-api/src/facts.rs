//! `GET /api/facts`: what the assistant knows about the person (one short fact per line, in
//! categories), the standing rules the person agreed to, and the rules on trial: the lessons of
//! the person's recent corrections, followed for 14 days until a weekly review keeps or drops
//! each one.
//!
//! Facts are corrected, forgotten or marked (a person whose messages wake the assistant), and
//! rules on trial dropped, through `POST /api/settings` ([`crate::settings::SettingsAction`]).

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
}

impl Fact {
    /// Whether this person's messages wake the assistant at once.
    #[must_use]
    pub fn is_important(&self) -> bool {
        self.important == Some(true)
    }
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
