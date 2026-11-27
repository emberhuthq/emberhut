//! The stop button: `POST /api/settings` with `{ interrupt: true, room }` drops the answer the
//! assistant is writing in that room now. Its background work keeps running.
//!
//! Answered like every `/api/settings` change: `{ ok: true }` with the room's whole state spread
//! in, which [`crate::error::OkReply`] reads the `ok` of.

use serde::{Deserialize, Serialize};

use crate::ids::RoomId;

/// Stop the turn running in a room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct InterruptRequest {
    /// Always `true` (the server stops the turn only when it is), so it is private: built only
    /// by [`InterruptRequest::new`].
    #[cfg_attr(feature = "validate", garde(skip))]
    interrupt: bool,
    /// The room whose turn stops; Main when left out.
    #[serde(default)]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub room: RoomId,
}

impl InterruptRequest {
    /// Stop the turn running in `room`.
    #[must_use]
    pub fn new(room: RoomId) -> Self {
        Self {
            interrupt: true,
            room,
        }
    }
}
