//! The person's answer to an ask: `POST /api/settings` with `{ taskDecide, choice }`.
//!
//! The answer carries the task on (or closes it: `Leave it`, `Not now` and `No` cancel it). It
//! can't be taken back once sent, so a client holds it a few seconds for an Undo first.
//!
//! It is one of the actions of `/api/settings` ([`crate::settings::SettingsAction::TaskDecide`]);
//! this type is its body on its own. Its 200 answer is `{ ok: true }` with the whole state (of
//! `room`, Main by default) spread in; [`crate::error::OkReply`] reads the `ok`. A task that
//! doesn't exist is refused with 400 and an [`crate::error::ErrorBody`].

use serde::{Deserialize, Serialize};

use crate::ids::TaskId;

/// An answer to a waiting task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct AnswerRequest {
    /// The task.
    #[cfg_attr(feature = "validate", garde(dive))]
    pub task_decide: TaskId,
    /// The answer: one of its decision's options, or `Done` for an action. Not blank.
    #[cfg_attr(feature = "validate", garde(custom(crate::rules::not_blank)))]
    pub choice: String,
}

impl AnswerRequest {
    /// The answer `choice` to `task`.
    #[must_use]
    pub fn new(task: TaskId, choice: impl Into<String>) -> Self {
        Self {
            task_decide: task,
            choice: choice.into(),
        }
    }

    /// Whether this answer closes the task instead of carrying it on (`Leave it`, `Not now`,
    /// `No`, in any case).
    #[must_use]
    pub fn cancels(&self) -> bool {
        ["leave it", "not now", "no"]
            .iter()
            .any(|word| self.choice.eq_ignore_ascii_case(word))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_answers() {
        for closing in ["Leave it", "not now", "NO"] {
            assert!(
                AnswerRequest::new(TaskId::new(1), closing).cancels(),
                "{closing}"
            );
        }
        for carrying in ["Approve", "Done", "Not yet", "No way"] {
            assert!(
                !AnswerRequest::new(TaskId::new(1), carrying).cancels(),
                "{carrying}"
            );
        }
    }
}
