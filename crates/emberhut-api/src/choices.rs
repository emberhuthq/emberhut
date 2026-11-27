//! A decision's options as cards to read and compare: each with its name, a sentence or two,
//! the facts to weigh, a short number and the assistant's reason when it is the pick.

use serde::{Deserialize, Serialize};

use crate::ids::ArtifactId;

/// One option of a decision as a card to read and compare. A plain answer beside the cards
/// ("More ideas") is one with only its `label`. Only the fields it has are sent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Choice {
    /// Its button: the answer sent back when it is picked.
    pub label: String,
    /// Its name as a card; none for a plain answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// A sentence or two about it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// A short number to compare by (a score, a price).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<String>,
    /// What picking it starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub then: Option<String>,
    /// The long version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub more: Option<String>,
    /// The facts to compare, the same keys on every card, in order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<ChoiceField>>,
    /// The assistant's reason, when it is the pick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// Its pictures and videos.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<Vec<ArtifactId>>,
}

impl Choice {
    /// Whether it reads as a card (it has a name), not as a plain answer.
    #[must_use]
    pub fn is_card(&self) -> bool {
        self.title.is_some()
    }
}

/// One fact on a card: `{ "k": "Price", "v": "5 000" }`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChoiceField {
    /// What it is (`k` on the wire).
    #[serde(rename = "k")]
    pub key: String,
    /// Its value (`v` on the wire).
    #[serde(rename = "v")]
    pub value: String,
}
