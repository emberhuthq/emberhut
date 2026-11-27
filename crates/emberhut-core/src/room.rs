//! Rooms: one conversation each. Every server has Main ([`RoomId::MAIN`]).

use jiff::Timestamp;

use crate::ids::{MessageId, RoomId};
use crate::task::ProjectId;

/// A room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Room {
    /// Its id.
    pub id: RoomId,
    /// Its name, as the room switcher shows it.
    pub name: String,
    /// When it was made.
    pub created_at: Timestamp,
    /// When the user last looked at it.
    pub seen_at: Option<Timestamp>,
    /// The newest message there when the user last looked; replies after it are unread.
    pub seen_through: Option<MessageId>,
    /// The project whose room it is; none for Main and every other room.
    pub project: Option<ProjectId>,
}
