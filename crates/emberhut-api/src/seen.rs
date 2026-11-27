//! `POST /api/seen`: a room's messages have been seen on screen, so its `unread` clears. Answered
//! with [`crate::error::OkReply`].

use serde::{Deserialize, Serialize};

use crate::ids::RoomId;

/// The room whose messages were seen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct SeenRequest {
    /// The room; Main when left out.
    #[serde(default)]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub room: RoomId,
}
