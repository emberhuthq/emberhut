//! Events: what programs and the server tell the assistant, for it to read when it next wakes
//! in the event's room. Each is read once (`handled`), and one with a key happens at most once.

use jiff::Timestamp;

use crate::{EventId, RoomId};

/// An event to store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewEvent {
    /// The room whose session reads it.
    pub room: RoomId,
    /// What kind of news it is (`plan`, `fact`, `ask`, a program's own word, ...).
    pub kind: String,
    /// The news, in words for the assistant.
    pub text: String,
    /// Makes the event happen once: a second event with the same key is not stored.
    pub key: Option<String>,
    /// Stored as already read: kept for the record, but nothing wakes for it.
    pub handled: bool,
}

impl NewEvent {
    /// A `kind` event in `room`, with no key, waiting to be read.
    #[must_use]
    pub fn new(room: RoomId, kind: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            room,
            kind: kind.into(),
            text: text.into(),
            key: None,
            handled: false,
        }
    }

    /// The same event, made to happen at most once under `key`.
    #[must_use]
    pub fn once(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }
}

/// A stored event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// Its id; ids grow, so they order the events.
    pub id: EventId,
    /// The room whose session reads it.
    pub room: RoomId,
    /// What kind of news it is.
    pub kind: String,
    /// The news.
    pub text: String,
    /// When it was stored.
    pub at: Timestamp,
    /// The key that makes it happen once, if it has one.
    pub key: Option<String>,
    /// Whether it has been read.
    pub handled: bool,
}
