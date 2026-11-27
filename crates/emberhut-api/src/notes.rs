//! Quick notes, the person's diary: `GET /api/notes` lists them all, newest first, and
//! `POST /api/notes` saves one ([`NoteSave`]): a new note or an edit, by the uid the device
//! gave it, so sending it again is harmless; empty text deletes it. An edit keeps the note's
//! time and day. The server files each note under the day it was written in its own time zone,
//! and adds one entry of the assistant's own a day ([`NoteAuthor::Assistant`]), which a client
//! can't change.
//!
//! A note the server refuses (a bad uid, a text over 20,000 characters) is 400 with an
//! [`ErrorBody`](crate::error::ErrorBody) carrying `ok: false`.

use serde::{Deserialize, Serialize};

use crate::ids::NoteUid;
use crate::open_enum::open_enum;
use crate::time::{Day, Timestamp};

/// The longest note the server takes, in UTF-16 units (after trimming).
pub const MAX_NOTE_LEN: usize = 20_000;

open_enum! {
    /// Who wrote a note.
    pub enum NoteAuthor {
        /// The person (its word is in [`crate::wire`]).
        Person = crate::wire::PERSON,
        /// The assistant: its entry for the day (its word is in [`crate::wire`]).
        Assistant = crate::wire::ASSISTANT,
    }
}

/// `GET /api/notes`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NotesPage {
    /// Every note, newest first.
    pub notes: Vec<Note>,
}

/// One note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Note {
    /// Its uid.
    pub uid: NoteUid,
    /// Who wrote it.
    pub who: NoteAuthor,
    /// The day it was written.
    pub day: Day,
    /// When it was written (the writing device's clock: a note written offline keeps its time).
    pub at: Timestamp,
    /// What it says.
    pub text: String,
    /// When the server first had it.
    pub created: Timestamp,
    /// When the server last changed it.
    pub updated: Timestamp,
}

impl Note {
    /// Whether it is the assistant's entry for its day.
    #[must_use]
    pub fn is_assistants(&self) -> bool {
        self.who == NoteAuthor::Assistant
    }
}

/// `POST /api/notes`: a note saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct NoteSave {
    /// The note's uid: a new one makes a note, a known one edits it.
    #[cfg_attr(feature = "validate", garde(skip))]
    pub uid: NoteUid,
    /// What it says, trimmed by the server; empty deletes the note.
    #[cfg_attr(feature = "validate", garde(length(utf16, max = MAX_NOTE_LEN)))]
    pub text: String,
    /// When it was written; none (or a time in the future): now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub at: Option<Timestamp>,
}

/// `POST /api/notes`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NoteSaved {
    /// Always `true`: a refusal is an `ErrorBody`.
    pub ok: bool,
    /// The note as saved; none when it was deleted (or there was nothing to delete).
    pub note: Option<Note>,
}
