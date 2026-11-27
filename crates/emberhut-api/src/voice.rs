//! Voice: calls with the assistant from the phone, and dictation.
//!
//! - `POST /api/voice/session`: a short-lived key for one call with a realtime voice model (the
//!   phone talks to it directly), with the session already set up. The server keeps the
//!   provider's own key.
//! - `POST /api/voice/ask` (`{ request, room }`): the call asks the assistant; the request goes
//!   into the room's chat as the person's words (marked as voice), and the answer comes back
//!   when the assistant has written it (two minutes at most).
//! - `POST /api/voice/done` (`{ seconds, diag, transcript, room }`): the call ended; what was
//!   said goes to the assistant, so it follows up.
//! - `POST /api/transcribe?name=&words=`, the recording as the body: dictation's speech to
//!   text.

use serde::{Deserialize, Serialize};

use crate::ids::RoomId;

/// The name of a voice session's one tool: the realtime voice model calls it for anything it
/// can't answer at once, and the phone answers it with `POST /api/voice/ask`
/// ([`crate::wire::VOICE_ASK_TOOL`]): the phone looks for this name.
pub const ASK_TOOL: &str = crate::wire::VOICE_ASK_TOOL;

/// The label a client gives the person's lines in a call's transcript
/// (`POST /api/voice/done`'s `transcript`), [`crate::wire::VOICE_PERSON_LABEL`].
pub const PERSON_LABEL: &str = crate::wire::VOICE_PERSON_LABEL;

/// A short label for the person's lines in a call's transcript, for a client that doesn't use
/// [`PERSON_LABEL`]. A server accepts both unless configured otherwise.
pub const NEUTRAL_PERSON_LABEL: &str = "Me:";

/// `POST /api/voice/session`'s answer: a key for one call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct VoiceSession {
    /// `true`.
    pub ok: bool,
    /// The key the phone opens the realtime session with; it works for 10 minutes.
    pub key: String,
    /// When it stops working, in seconds since the epoch.
    pub expires_at: f64,
    /// The realtime model the session runs.
    pub model: String,
}

/// `POST /api/voice/ask`'s body.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct VoiceAsk {
    /// What the person wants, in their words.
    #[serde(default)]
    #[cfg_attr(feature = "validate", garde(length(utf16, max = 100_000)))]
    pub request: String,
    /// The room the call belongs to; Main when left out.
    #[serde(default)]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub room: RoomId,
}

/// `POST /api/voice/ask`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct VoiceAnswer {
    /// `true`.
    pub ok: bool,
    /// The assistant's answer, to be read out; or a line saying it is still working on it.
    pub answer: String,
}

/// `POST /api/voice/done`'s body: the phone's report of the call.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct VoiceDone {
    /// How long the call was, in seconds.
    #[serde(default)]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub seconds: Option<f64>,
    /// The phone's own view of how the call went (for the log).
    #[serde(default)]
    #[cfg_attr(feature = "validate", garde(length(max = 100_000)))]
    pub diag: Option<String>,
    /// What was said, one line per turn, each starting with who said it.
    #[serde(default)]
    #[cfg_attr(feature = "validate", garde(length(max = 1_000_000)))]
    pub transcript: Option<String>,
    /// The room the call belongs to; Main when left out.
    #[serde(default)]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub room: RoomId,
}

/// The phone's report of the last call, as the state shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LastCall {
    /// When it ended, how long it was and what the phone said of it.
    pub report: String,
    /// The start of what was said (2000 characters at most).
    pub transcript: String,
}
