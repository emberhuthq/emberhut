//! A family member's own way in: the same assistant, as their own user, in their own rooms.
//!
//! A household server on the same machine (the one that pairs the family's devices) passes a
//! family member's requests on under `/family/...` with the device's key as the bearer; the
//! server asks that household server who the key belongs to, and only a family member gets in.
//! They see their own rooms (a `Home` room is made the first time) and nothing of the owner's.
//!
//! - `GET /family/state` ([`FamilyState`], with an `ETag`; `If-None-Match` gets a 304),
//!   `GET /family/history?before=` ([`FamilyHistory`]), `GET /family/live` (server-sent
//!   [`crate::live::LiveEvent`]s).
//! - `POST /family/talk` ([`FamilyTalk`]), `POST /family/devices` ([`FamilyDevice`]),
//!   `POST /family/seen`.
//! - `GET` and `POST /family/theme`: their own theme, as `/api/theme`.
//!
//! `?room=` picks one of their rooms on any of them; none (or another's) is their Home room.

use serde::{Deserialize, Serialize};

use crate::ids::{MessageId, RoomId};
use crate::live::Live;
use crate::theme::ThemeChoice;
use crate::time::Timestamp;

/// One of the family member's rooms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FamilyRoom {
    /// Its id.
    pub id: RoomId,
    /// Its name.
    pub name: String,
}

/// A line of their conversation: their words and the assistant's answers (no event lines).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FamilyLine {
    /// Its id.
    pub id: MessageId,
    /// They said it.
    pub mine: bool,
    /// What was said.
    pub text: String,
    /// When.
    pub at: Timestamp,
}

/// A check-in the room asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FamilyReminder {
    /// Its number in the list, from 1.
    pub id: u32,
    /// When it is due.
    pub due: Timestamp,
    /// What for.
    pub text: String,
}

/// `GET /family/state`: who they are, the room, their rooms, its latest conversation, what the
/// assistant is doing there, their reminders and their theme's choice (a newer `at` than the
/// app's own: fetch `/family/theme`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FamilyState {
    /// Who they are (the household server's name for them).
    pub me: String,
    /// The room shown.
    pub room: RoomId,
    /// Their rooms.
    pub rooms: Vec<FamilyRoom>,
    /// The room's latest 60 lines, oldest first.
    pub chat: Vec<FamilyLine>,
    /// The assistant is working in the room.
    pub working: bool,
    /// What it is doing, while it works.
    pub live: Option<Live>,
    /// The room's check-ins.
    pub reminders: Vec<FamilyReminder>,
    /// Their theme's choice.
    pub theme: ThemeChoice,
}

/// `GET /family/history?before=`: an older page of the conversation, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FamilyHistory {
    /// The lines.
    pub chat: Vec<FamilyLine>,
}

/// `POST /family/talk`: what they say (at most 8,000 characters are kept).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FamilyTalk {
    /// The text.
    #[serde(default)]
    pub text: String,
}

/// The answer to `POST /family/talk`: the message's id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FamilyTalkReply {
    /// `true`.
    pub ok: bool,
    /// The message's id.
    pub id: MessageId,
}

/// `POST /family/devices`: their iPhone's push address.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FamilyDevice {
    /// The APNs device token: 64 to 200 hex digits.
    #[serde(default)]
    pub token: String,
    /// `production`, or the sandbox for anything else.
    #[serde(default)]
    pub env: Option<String>,
}

/// The answer to `POST /family/seen`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FamilySeenReply {
    /// `true`.
    pub ok: bool,
    /// When the room was marked seen.
    pub at: Timestamp,
}
