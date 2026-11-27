//! `POST /api/tasks/{id}/message`: the person writes on a task's own page. It goes where an
//! answer goes, never to Main: the task's worker carries on with it, or the runner session of
//! its latest run, or the assistant in the task's room; an ask it had is cleared. A refusal (no
//! such task, nothing to send) is 400 with an [`ErrorBody`](crate::error::ErrorBody).

use serde::{Deserialize, Serialize};

use crate::chat::ChatFile;
use crate::ids::SessionId;
use crate::open_enum::open_enum;

/// What the person writes on a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct TaskMessageRequest {
    /// The text; not blank unless files come with it.
    #[cfg_attr(feature = "validate", garde(custom(crate::rules::text_or_files(&self.files))))]
    pub text: String,
    /// Files stored with `POST /api/uploads`, sent back whole.
    #[serde(default)]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub files: Vec<ChatFile>,
}

open_enum! {
    /// Where a message on a task went.
    pub enum MessageTo {
        /// The assistant, in the task's room (its word is in [`crate::wire`]).
        Assistant = crate::wire::ASSISTANT,
        /// The task's worker.
        Worker = "worker",
        /// The runner session of its latest run.
        Session = "session",
    }
}

impl MessageTo {
    /// The assistant's id on the wire: where a message went, and the owner of the assistant's
    /// own tasks. Its one home is [`crate::wire::ASSISTANT`].
    pub const ASSISTANT: &'static str = crate::wire::ASSISTANT;
}

/// `POST /api/tasks/{id}/message`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TaskMessageReply {
    /// Always `true`: a refusal is an `ErrorBody`.
    pub ok: bool,
    /// Where it went.
    pub to: MessageTo,
    /// The session it went to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionId>,
    /// It waits for the run on now to end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::MessageTo;

    #[test]
    fn the_assistants_id_is_its_wire_word() {
        assert_eq!(MessageTo::ASSISTANT, MessageTo::Assistant.as_str());
        assert_eq!(MessageTo::from(MessageTo::ASSISTANT), MessageTo::Assistant);
    }
}
