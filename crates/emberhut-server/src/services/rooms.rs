//! The rooms as the assistant sees them across its sessions: finding one by what it is
//! called, handing something to another room (`room_send`), and what was said in each since a
//! time (`rooms`, and the morning brief).

use std::sync::Arc;

use emberhut_core::event::NewEvent;
use emberhut_core::message::{Author, preview};
use emberhut_core::room::Room;
use emberhut_core::{Clock, RoomId};
use emberhut_db::{DbError, EventStore, MessageStore, RoomStore};
use jiff::Timestamp;

use crate::services::chat::one_line;

/// Lines of each room in [`RoomsService::said_since`].
const LINES_PER_ROOM: u64 = 12;
/// Characters of each of those lines.
const LINE_LENGTH: usize = 400;

/// Why something could not be handed to another room.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RoomSendError {
    /// No room is called that.
    #[error("no room \"{0}\"")]
    NoRoom(String),
    /// The room named is the one sending.
    #[error("that's this room")]
    SameRoom,
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
}

/// Finds rooms, hands things between them and says what was said in them.
#[derive(Clone, Debug)]
pub struct RoomsService {
    rooms: RoomStore,
    messages: MessageStore,
    events: EventStore,
    clock: Arc<dyn Clock>,
}

impl RoomsService {
    /// A service over the rooms, their messages and the events.
    #[must_use]
    pub fn new(
        rooms: RoomStore,
        messages: MessageStore,
        events: EventStore,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            rooms,
            messages,
            events,
            clock,
        }
    }

    /// The room `name` means: its id (digits), `main`, or a room's name in any case (`'s room`
    /// after it ignored), whole or its start.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn find(&self, name: &str) -> Result<Option<Room>, DbError> {
        let name = name.trim();
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_digit()) {
            return match name.parse::<i64>() {
                Ok(id) => self.rooms.get(RoomId::new(id)).await,
                Err(_) => Ok(None),
            };
        }
        let mut key = name.to_lowercase();
        for ending in ["'s room", " room"] {
            if let Some(cut) = key.strip_suffix(ending) {
                key = cut.trim_end().to_owned();
            }
        }
        if key.is_empty() {
            return Ok(None);
        }
        // The owner's rooms: a family member's room is never handed work by name.
        let rooms = self.rooms.owners().await?;
        if key == "main" {
            return Ok(rooms.into_iter().find(|room| room.id == RoomId::MAIN));
        }
        let whole = rooms
            .iter()
            .position(|room| room.name.to_lowercase() == key);
        let start = || {
            rooms
                .iter()
                .position(|room| room.name.to_lowercase().starts_with(&key))
        };
        Ok(whole.or_else(start).map(|at| rooms[at].clone()))
    }

    /// The owner's room a number names, as a program sends it for a project: the room of the
    /// project with that id, else the room with that id; none when neither is the owner's (a
    /// family member's room is never one).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn numbered(&self, id: i64) -> Result<Option<Room>, DbError> {
        let rooms = self.rooms.owners().await?;
        let of_project = rooms
            .iter()
            .find(|room| room.project.is_some_and(|project| project.get() == id));
        Ok(of_project
            .or_else(|| rooms.iter().find(|room| room.id.get() == id))
            .cloned())
    }

    /// The room `room`, as a sentence names it: `the Main room`, `the Atlas room`.
    async fn called(&self, room: RoomId) -> Result<String, DbError> {
        let name = self
            .rooms
            .get(room)
            .await?
            .map_or_else(|| format!("Room {room}"), |room| room.name);
        Ok(format!("the {name} room"))
    }

    /// Hands `text` from room `from` to the room `to` names: it lands there as an event of
    /// kind `room` (`From the Main room: ...`), which wakes that room's session. The room it
    /// went to.
    ///
    /// # Errors
    ///
    /// [`RoomSendError::NoRoom`], [`RoomSendError::SameRoom`], [`RoomSendError::Database`].
    pub async fn send(&self, from: RoomId, to: &str, text: &str) -> Result<Room, RoomSendError> {
        let target = self
            .find(to)
            .await?
            .ok_or_else(|| RoomSendError::NoRoom(to.trim().to_owned()))?;
        if target.id == from {
            return Err(RoomSendError::SameRoom);
        }
        let who = self.called(from).await?;
        self.events
            .add(
                NewEvent::new(target.id, "room", format!("From {who}: {text}")),
                self.clock.now(),
            )
            .await?;
        Ok(target)
    }

    /// What the owner and the assistant said in each room after `since` (the last 12 lines of
    /// each, oldest first, under the room's name), or in `only`; empty when nothing was.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn said_since(
        &self,
        since: Timestamp,
        only: Option<RoomId>,
    ) -> Result<String, DbError> {
        let mut parts = Vec::new();
        for room in self.rooms.owners().await? {
            if only.is_some_and(|only| only != room.id) {
                continue;
            }
            let said = self
                .messages
                .said_since(room.id, since, LINES_PER_ROOM)
                .await?;
            if said.is_empty() {
                continue;
            }
            let lines: Vec<String> = said
                .iter()
                .map(|message| {
                    let at = message.created_at.strftime("%Y-%m-%dT%H:%M");
                    let who = if message.author == Author::User {
                        "Owner"
                    } else {
                        "Assistant"
                    };
                    let text = preview(&one_line(&message.text), LINE_LENGTH);
                    format!("[{at}] {who}: {text}")
                })
                .collect();
            parts.push(format!("## {}\n{}", room.name, lines.join("\n")));
        }
        Ok(parts.join("\n\n"))
    }
}
