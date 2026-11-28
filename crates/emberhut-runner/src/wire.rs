//! The words of the runner protocol, in one place: the server's routes the runner calls, the
//! event kinds it reports and the live stream's event types. They match the server's contract
//! (`emberhut-api`'s runner types and `emberhut-server`'s runner routes).

use std::sync::LazyLock;

use regex::Regex;

/// The server's route for every change of a session, one event each (`POST`).
pub const EVENTS: &str = "/api/runner/events";
/// The server's route for a machine sample (`POST`).
pub const MACHINE: &str = "/api/runner/machine";
/// The server's route for the MCP servers the runner's sessions get too (`GET`).
pub const MCP: &str = "/api/runner/mcp";
/// The server's route that trades a pairing code for the runner's name and token (`POST`).
pub const PAIR: &str = "/api/runner/pair";

/// The kinds of event a runner reports. Each carries the session as it is now, so taking one
/// twice changes nothing.
pub mod event {
    /// A session was opened.
    pub const SESSION: &str = "session";
    /// A turn was asked for (`turn.prompt`, `turn.by`).
    pub const TURN_QUEUED: &str = "turn_queued";
    /// A turn started (`turn.started`).
    pub const TURN_STARTED: &str = "turn_started";
    /// What the running turn does now (`doing`).
    pub const ACTIVITY: &str = "activity";
    /// A turn ended (`turn.outcome`, `turn.last`).
    pub const TURN_FINISHED: &str = "turn_finished";
    /// The session asks its owner something (`text`).
    pub const QUESTION: &str = "question";
    /// A line for the activity log (`text`).
    pub const NOTE: &str = "note";
    /// The session moved to another account, or every account is out (`from`, `to`, `why`, `resumeAt`).
    pub const ACCOUNT: &str = "account";
}

/// The events of a session's live stream (server-sent events, one `data:` line each).
pub mod live {
    /// The session's head: status, current step, running since.
    pub const HEAD: &str = "head";
    /// A new step of the transcript.
    pub const STEP: &str = "step";
    /// Words being typed (`start`: a new block).
    pub const DELTA: &str = "delta";
    /// Thinking being written (`start`: a new block, `since`: when it began).
    pub const THINK: &str = "think";
    /// The running turn's output tokens so far.
    pub const TOKENS: &str = "tokens";
}

/// The efforts a session can run at.
pub const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// Who asked, when the server says nothing: a session it opens for work comes from the assistant.
pub const DEFAULT_BY: &str = "assistant";

/// What the agent reads before a message the assistant writes in its owner's own session.
pub const FROM_ASSISTANT: &str = "(From the assistant, writing in this session for its owner)\n\n";

static SESSION_ID: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"^r-[0-9a-f]{8}$").ok());

/// Whether `id` is a session id this runner makes: `r-` and 8 lowercase hex digits.
#[must_use]
pub fn is_session_id(id: &str) -> bool {
    SESSION_ID.as_ref().is_some_and(|re| re.is_match(id))
}

/// A request the runner turns down, with the HTTP status and the words a person reads.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct Refused {
    /// The HTTP status.
    pub status: u16,
    /// Short words for a person (they reach the app as they are).
    pub message: String,
}

impl Refused {
    /// A refusal with this status.
    #[must_use]
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    /// 400: the request is wrong.
    #[must_use]
    pub fn bad(message: impl Into<String>) -> Self {
        Self::new(400, message)
    }

    /// 403: not allowed.
    #[must_use]
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(403, message)
    }

    /// 404: no such thing.
    #[must_use]
    pub fn missing(message: impl Into<String>) -> Self {
        Self::new(404, message)
    }

    /// 409: can't now.
    #[must_use]
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(409, message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_are_r_and_eight_lowercase_hex() {
        assert!(is_session_id("r-1a2b3c4d"));
        assert!(
            !is_session_id("r-1A2B3C4D")
                && !is_session_id("r-1a2b")
                && !is_session_id("r-1a2b3c4d5")
        );
    }
}
