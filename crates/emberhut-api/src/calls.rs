//! Calls: recording one on the owner's Mac and its transcript, and the live transcript while it
//! is recorded.
//!
//! - `POST /api/calls/record` (`{ action, room }`): the app's Record button starts or stops the
//!   Mac's recorder; the transcript goes to `room`'s chat once it is written. The answer is
//!   `{ ok: true, text }` (the recorder's words), or, when the Mac couldn't be told, still
//!   status 200 but `{ ok: false, error }`. While a call is recorded, the state says so
//!   ([`crate::state::State::recording`]).
//! - `POST /api/calls/in?call=&track=mic|sys&last=1&seconds=&title=`, a track's audio as the
//!   body: the recorder sends each track when a recording stops; `last` queues the call to be
//!   transcribed.
//! - `POST /api/calls/live?call=&track=&t=&title=&copilot=0|1`, about ten seconds of a track as
//!   the body (WAV): while it records, for the live transcript and the copilot.
//! - `GET /api/calls`: the latest calls; `GET /api/calls/{id}/transcript`: one transcript, as
//!   Markdown; `GET /api/calls/{id}/live`: the live lines and the copilot's hints.

use serde::{Deserialize, Serialize};

use crate::ids::{CallId, RoomId};
use crate::time::Timestamp;

/// Start or stop. Read leniently: `stop` stops, any other word starts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum RecordAction {
    /// Start recording.
    #[default]
    Start,
    /// Stop the recording running.
    Stop,
}

impl<'de> Deserialize<'de> for RecordAction {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let word = Option::<String>::deserialize(deserializer)?;
        Ok(if word.as_deref() == Some("stop") {
            Self::Stop
        } else {
            Self::Start
        })
    }
}

/// Start or stop recording a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct RecordCallRequest {
    /// What to do. The server reads anything but `stop` as `start`, and no action as `start`.
    #[serde(default)]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub action: RecordAction,
    /// The room the transcript goes to; Main when left out.
    #[serde(default)]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub room: RoomId,
}

impl RecordCallRequest {
    /// Start recording, the transcript going to `room`.
    #[must_use]
    pub fn start(room: RoomId) -> Self {
        Self {
            action: RecordAction::Start,
            room,
        }
    }

    /// Stop recording (`room` is the room the app shows).
    #[must_use]
    pub fn stop(room: RoomId) -> Self {
        Self {
            action: RecordAction::Stop,
            room,
        }
    }
}

/// `POST /api/calls/record`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RecordCallReply {
    /// Whether the Mac was told.
    pub ok: bool,
    /// What the server did, in its words, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Why not, when it didn't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A call being recorded on the Mac now (the newest one started in the last six hours).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CallRecording {
    /// The call's id.
    pub id: CallId,
    /// Its title, as the app shows it.
    #[serde(default)]
    pub title: String,
    /// The room its transcript goes to.
    #[serde(default)]
    pub room: RoomId,
    /// When it started.
    pub at: Timestamp,
}

/// `POST /api/calls/in`'s query. Read leniently: a missing or unusable value is a 400 from the
/// route, saying what.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams), into_params(parameter_in = Query))]
pub struct TrackQuery {
    /// The call's id: 6 to 40 letters, digits, `_` and `-`. A call the server doesn't know
    /// (started on the Mac itself) is made, in Main.
    #[serde(default)]
    pub call: Option<String>,
    /// `mic` (the owner) or `sys` (the Mac's sound: the other side).
    #[serde(default)]
    pub track: Option<String>,
    /// Not empty: this is the call's last track; it is queued to be transcribed.
    #[serde(default)]
    pub last: Option<String>,
    /// The call's length in seconds.
    #[serde(default)]
    pub seconds: Option<String>,
    /// The title of a call the server doesn't know yet.
    #[serde(default)]
    pub title: Option<String>,
}

/// `POST /api/calls/in`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReceivedTrack {
    /// `true`.
    pub ok: bool,
    /// The call's id (the recorder ends its live copilot by it).
    pub call: String,
    /// The track's size, in bytes.
    pub bytes: u64,
}

/// `POST /api/calls/live`'s query.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams), into_params(parameter_in = Query))]
pub struct ChunkQuery {
    /// The call's id, as for a track.
    #[serde(default)]
    pub call: Option<String>,
    /// `mic` or `sys`.
    #[serde(default)]
    pub track: Option<String>,
    /// Seconds from the call's start the chunk begins at.
    #[serde(default)]
    pub t: Option<String>,
    /// The title of a call the server doesn't know yet.
    #[serde(default)]
    pub title: Option<String>,
    /// `0`: the owner turned the live copilot off on the Mac; `1`: back on.
    #[serde(default)]
    pub copilot: Option<String>,
}

/// A call in the latest calls' list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CallRow {
    /// Its id.
    pub id: String,
    /// Who and what it is.
    pub title: String,
    /// `recording`, `queued`, `transcribing`, `done` or `failed`.
    pub status: String,
    /// The room its transcript goes to.
    pub room: RoomId,
    /// When it started.
    pub at: Timestamp,
}

/// `GET /api/calls`: the latest 8 calls, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CallList {
    /// The calls.
    pub calls: Vec<CallRow>,
}

/// A line of a call's live transcript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LiveCallLine {
    /// Seconds from the call's start.
    pub t: f64,
    /// Who said it: the owner or the other side.
    pub who: String,
    /// What was said.
    pub text: String,
}

/// A copilot's hint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LiveCallHint {
    /// Seconds from the call's start it is about.
    pub t: Option<f64>,
    /// The hint, one line.
    pub text: String,
    /// When it was given.
    pub at: Timestamp,
}

/// `GET /api/calls/{id}/live`: the call's last 40 lines (the microphone's echo of the other
/// side left out) and the copilot's hints, for the panel on the Mac.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CallLive {
    /// The call's id.
    pub call: String,
    /// Its title; `null` for a call the server doesn't know.
    pub title: Option<String>,
    /// Its status; `null` for a call the server doesn't know.
    pub status: Option<String>,
    /// The copilot: `waiting` (for enough to say), `on`, `ended`, `off` (none on this server),
    /// or `off: <why>` (turned off on the Mac, or it stopped).
    pub copilot: String,
    /// The latest lines, oldest first.
    pub lines: Vec<LiveCallLine>,
    /// The hints, in order.
    pub hints: Vec<LiveCallHint>,
}
