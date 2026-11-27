//! `POST /api/talk`: say something in a room. The server answers at once; the reply shows up in
//! the chat (and on the live stream while it is written).

use serde::{Deserialize, Serialize};

use crate::chat::ChatFile;
use crate::ids::{MessageId, RoomId};

/// What the person says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct TalkRequest {
    /// The text. Not blank unless files come with it; at most 100 000 UTF-16 units.
    #[cfg_attr(feature = "validate", garde(custom(crate::rules::text_or_files(&self.files)), length(utf16, max = 100_000)))]
    pub text: String,
    /// Files stored with `POST /api/uploads`, sent back whole. The server drops any whose path
    /// isn't in its uploads folder.
    #[serde(default)]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub files: Vec<ChatFile>,
    /// The message this one replies to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub reply_to: Option<MessageId>,
    /// The room; Main when left out.
    #[serde(default)]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub room: RoomId,
}

impl TalkRequest {
    /// Plain text in a room, with no files and not a reply.
    #[must_use]
    pub fn text(room: RoomId, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            files: Vec::new(),
            reply_to: None,
            room,
        }
    }

    /// The same message with these files (stored with `POST /api/uploads`) attached.
    #[must_use]
    pub fn with_files(mut self, files: Vec<ChatFile>) -> Self {
        self.files = files;
        self
    }

    /// The same message as a reply to `message`.
    #[must_use]
    pub fn with_reply_to(mut self, message: MessageId) -> Self {
        self.reply_to = Some(message);
        self
    }
}

/// `POST /api/talk`'s answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TalkReply {
    /// Always `true`: a failure is an `ErrorBody` with status 500.
    pub ok: bool,
    /// A turn started (or was queued behind the one running): the reply comes with the next
    /// polls.
    #[serde(default)]
    pub started: bool,
    /// The message's id, when the server gives it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<MessageId>,
}
