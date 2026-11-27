//! The person driving a session: a message (`POST .../send`), Stop (`POST .../stop`: the step it's
//! on ends and nothing queued runs), Stop this turn (`POST .../interrupt`: what is queued runs
//! next) and its model and effort for the next turn (`POST .../settings`, the person's own
//! sessions).

use serde::{Deserialize, Deserializer, Serialize};

use crate::chat::ChatFile;
use crate::ids::TurnId;
use crate::open_enum::open_enum;

/// The words a message with files and no words of its own is sent as.
pub const FILES_ONLY_TEXT: &str = "(see the files)";

/// `POST .../send`: the person's message to a session. Sent while a turn runs, it waits behind
/// it. Not both blank: the server answers 400 `nothing to send`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SendToSession {
    /// The words.
    pub text: String,
    /// Files stored with `POST /api/uploads`, sent back whole.
    #[serde(default)]
    pub files: Vec<ChatFile>,
}

/// `POST .../send`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SentToSession {
    /// Always `true`: a refusal is an `ErrorBody`.
    pub ok: bool,
    /// The turn it starts or waits for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<TurnId>,
    /// It waits behind a turn running now.
    #[serde(default, deserialize_with = "flag_or_count")]
    pub queued: bool,
}

/// A worker answers `queued` with how many wait, a runner with whether this one does.
fn flag_or_count<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Queued {
        Flag(bool),
        Count(f64),
    }
    Ok(match Option::<Queued>::deserialize(deserializer)? {
        Some(Queued::Flag(flag)) => flag,
        Some(Queued::Count(count)) => count > 0.,
        None => false,
    })
}

/// `POST .../stop` and `.../interrupt`'s answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Stopped {
    /// `true` when it was done.
    pub ok: bool,
    /// A turn was running (it ends now); none from a task's worker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running: Option<bool>,
}

open_enum! {
    /// How hard a session thinks.
    pub enum Effort {
        /// Low.
        Low = "low",
        /// Medium.
        Medium = "medium",
        /// High: the default.
        High = "high",
        /// Extra high.
        ExtraHigh = "xhigh",
        /// The most.
        Max = "max",
    }
}

impl Effort {
    /// Every effort, least first.
    pub const ALL: [Self; 5] = [
        Self::Low,
        Self::Medium,
        Self::High,
        Self::ExtraHigh,
        Self::Max,
    ];

    /// Its name for a person (`Extra high`).
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::ExtraHigh => "Extra high",
            Self::Max => "Max",
            Self::Other(word) => word,
        }
    }
}

/// `POST .../settings`: what the person changes for the session's next turn (a running turn
/// ends first). What is left out stays as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SessionSettings {
    /// The model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The effort.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_effort_has_a_name() {
        let names: Vec<&str> = Effort::ALL.iter().map(Effort::name).collect();
        assert_eq!(names, ["Low", "Medium", "High", "Extra high", "Max"]);
        assert_eq!(Effort::from("huge").name(), "huge");
    }
}
