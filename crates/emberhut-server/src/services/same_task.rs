//! Same-task memory across rooms ([`emberhut_core::same_task`]): the lines of every room that
//! name the keys a turn names, put after the turn's message, each line once per room.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use emberhut_core::message::{Author, preview};
use emberhut_core::same_task::{HERE, MAX_LINES, WINDOW, block, named_keys};
use emberhut_core::{Clock, MessageId, RoomId};
use emberhut_db::{DbError, MessageStore, RoomStore};

use crate::services::chat::one_line;

/// Characters of each line shown.
const LINE_LENGTH: usize = 300;

/// Lines read per key before the room's own are left out.
const READ_PER_KEY: u64 = 40;

/// Finds what the rooms said about a turn's keys.
#[derive(Clone, Debug)]
pub struct SameTaskService {
    messages: MessageStore,
    rooms: RoomStore,
    clock: Arc<dyn Clock>,
    /// What each room was given already.
    shown: Arc<Mutex<HashMap<RoomId, HashSet<MessageId>>>>,
}

impl SameTaskService {
    /// A service over the messages and the rooms.
    #[must_use]
    pub fn new(messages: MessageStore, rooms: RoomStore, clock: Arc<dyn Clock>) -> Self {
        Self {
            messages,
            rooms,
            clock,
            shown: Arc::default(),
        }
    }

    /// The block for a turn in `room` whose message is `text` (`said` its own message, left
    /// out): for each key it names, what was said about it in the last 48 hours in the other
    /// rooms, and in this one before its last 2 hours. Empty when it names none, or nothing
    /// new was said.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn for_turn(
        &self,
        room: RoomId,
        text: &str,
        said: Option<MessageId>,
    ) -> Result<String, DbError> {
        let keys = named_keys(text);
        if keys.is_empty() {
            return Ok(String::new());
        }
        let now = self.clock.now();
        let since = now.checked_sub(WINDOW).unwrap_or(now);
        let here = now.checked_sub(HERE).unwrap_or(now);
        let names: HashMap<RoomId, String> = self
            .rooms
            .list()
            .await?
            .into_iter()
            .map(|known| (known.id, known.name))
            .collect();
        let mut parts = Vec::new();
        let mut given = Vec::new();
        let before: HashSet<MessageId> = self
            .shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&room)
            .cloned()
            .unwrap_or_default();
        {
            for key in keys {
                let found = self.messages.naming(&key, since, READ_PER_KEY).await?;
                let mut lines: Vec<(MessageId, String)> = found
                    .into_iter()
                    .filter(|message| Some(message.id) != said)
                    .filter(|message| message.room != room || message.created_at <= here)
                    .filter(|message| !before.contains(&message.id))
                    .map(|message| {
                        let who = match message.author {
                            Author::User => "Owner",
                            Author::Assistant => "Assistant",
                            Author::Event => "Event",
                        };
                        let name = names
                            .get(&message.room)
                            .cloned()
                            .unwrap_or_else(|| format!("Room {}", message.room));
                        let at = message.created_at.strftime("%Y-%m-%d %H:%M");
                        let text = preview(&one_line(&message.text), LINE_LENGTH);
                        (message.id, format!("[{at}, {name}] {who}: {text}"))
                    })
                    .collect();
                let cut = lines.len().saturating_sub(MAX_LINES);
                lines.drain(..cut);
                given.extend(lines.iter().map(|(id, _)| *id));
                parts.push((key, lines.into_iter().map(|(_, line)| line).collect()));
            }
        }
        self.shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(room)
            .or_default()
            .extend(given);
        Ok(block(&parts))
    }
}
