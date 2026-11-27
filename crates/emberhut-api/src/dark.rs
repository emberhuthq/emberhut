//! The Dark room: a plain chat with a model, off the record. The assistant never sees it, and
//! the model there has none of the person's data or the assistant's tools.
//!
//! - `GET /api/dark`: the room ([`DarkRoom`]).
//! - `POST /api/dark` ([`DarkRequest`]): the answer as server-sent [`DarkEvent`]s: `thinking`
//!   while the model reasons, each `delta` as it is written, then `done` with the kept message,
//!   or `error`. A blank message is refused with 400, and a message while an answer is being
//!   written with 409, each with an [`ErrorBody`](crate::error::ErrorBody).
//! - `DELETE /api/dark`: everything said is deleted (409 while an answer is being written).

use serde::{Deserialize, Serialize};

use crate::ids::DarkMessageId;
use crate::open_enum::open_enum;
use crate::time::Timestamp;

open_enum! {
    /// Who wrote a message in the Dark room.
    pub enum DarkRole {
        /// The person.
        User = "user",
        /// The model.
        Assistant = "assistant",
    }
}

/// A message in the Dark room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DarkMessage {
    /// Its id.
    pub id: DarkMessageId,
    /// Who wrote it.
    pub role: DarkRole,
    /// What it says (an answer is markdown).
    pub text: String,
    /// When.
    pub at: Timestamp,
}

/// `GET /api/dark`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DarkRoom {
    /// The model that answers, in words.
    pub model: String,
    /// Where what the person writes goes, in words.
    #[serde(default, rename = "where")]
    pub place: Option<String>,
    /// An answer is being written now (one from a message sent earlier, perhaps).
    pub busy: bool,
    /// What was said, oldest first.
    pub chat: Vec<DarkMessage>,
}

/// `POST /api/dark`: a message to the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct DarkRequest {
    /// The message; not blank.
    #[cfg_attr(feature = "validate", garde(custom(crate::rules::not_blank)))]
    pub text: String,
}

/// An event of the answer's stream: one of its fields.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DarkEvent {
    /// The model is reasoning before it answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<bool>,
    /// The next piece of the answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
    /// The answer, kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done: Option<DarkMessage>,
    /// Why there is no answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
