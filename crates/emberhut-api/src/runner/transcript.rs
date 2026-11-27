//! A session's transcript (`GET /api/runner/sessions/{id}/transcript?after=&limit=&agent=`) and
//! its live stream (`GET .../live?after=&agent=`).
//!
//! The transcript is the session's head and its newest steps (`after` 0), or those after a step
//! the client has. The live stream is `text/event-stream` passed through from the runner: one
//! `data:` line per [`SessionLive`] event, a `: ping` comment every 15 s. It opens with the head
//! and every step after `after`, then the words being typed and the thinking being written so
//! far, so a client that reconnects with the last step it has misses nothing. A task's worker
//! (session `w-<task>`) sends only heads and steps.

use serde::{Deserialize, Serialize};

use crate::ids::{SessionId, TaskId, ToolCallId, TurnId};
use crate::query::push_encoded;
use crate::runner::steps::SessionStep;
use crate::time::Timestamp;

/// How many steps a transcript has at most when the client names no limit (the runner's own
/// default).
pub const DEFAULT_LIMIT: u32 = 400;

/// The most steps a transcript has, whatever is asked.
pub const MAX_LIMIT: u32 = 1000;

/// What a session is and does now: the runner's own fields, what the server knows of
/// it (its place, account name and tasks), and on a live head the running turn's figures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SessionHead {
    /// Its id.
    pub id: SessionId,
    /// Its title.
    pub title: String,
    /// `running`, `queued`, `completed`, `failed`, `interrupted`, `waiting` or `idle` (the
    /// runner's words, kept as text).
    pub status: String,
    /// A turn runs now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running: Option<bool>,
    /// The folder it works in.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Its project, by name.
    #[serde(default)]
    pub project: Option<String>,
    /// The Claude account it runs on.
    #[serde(default)]
    pub account: Option<String>,
    /// That account's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_name: Option<String>,
    /// Its own git worktree's branch, when it has one.
    #[serde(default)]
    pub branch: Option<String>,
    /// Its own git worktree, when it has one.
    #[serde(default)]
    pub worktree: Option<String>,
    /// What the runner keeps of how it was opened.
    #[serde(default)]
    pub meta: Option<HeadMeta>,
    /// The model it runs on.
    #[serde(default)]
    pub model: Option<String>,
    /// Its effort.
    #[serde(default)]
    pub effort: Option<String>,
    /// When it was opened.
    #[serde(default)]
    pub created: Option<Timestamp>,
    /// When it last changed.
    #[serde(default)]
    pub updated: Option<Timestamp>,
    /// The machine it runs on, in words (`Mac`, `Linux PC`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    /// The tasks it works on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<TaskId>,
    /// Its current step, while a turn runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doing: Option<String>,
    /// When the running turn started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<Timestamp>,
    /// The running turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<TurnId>,
    /// The running turn's output tokens so far.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<f64>,
    /// Its helper agents now, with their counts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<LiveAgent>,
    /// How many messages wait behind the running turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued: Option<u32>,
}

impl SessionHead {
    /// Whether the person opened it (not a task's worker or session).
    #[must_use]
    pub fn is_direct(&self) -> bool {
        self.meta.as_ref().and_then(|meta| meta.direct) == Some(true)
    }

    /// Whether a turn runs now: its flag, else its status.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running
            .unwrap_or(matches!(self.status.as_str(), "running" | "queued"))
    }
}

/// How a session was opened, as far as a client reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HeadMeta {
    /// The person opened it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct: Option<bool>,
}

/// A helper agent of a session, as the runner counts it: by the call that started it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LiveAgent {
    /// The call that started it.
    pub id: ToolCallId,
    /// What it was asked to do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Its tokens so far.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<f64>,
    /// Its tool calls so far.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<u32>,
    /// Its latest step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<String>,
    /// It runs now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running: Option<bool>,
    /// When it started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<Timestamp>,
}

/// `GET .../transcript`'s answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Transcript {
    /// The session.
    pub session: SessionHead,
    /// Its steps, oldest first.
    #[serde(default)]
    pub events: Vec<SessionStep>,
    /// Older steps than these exist (only when no `after` was given).
    #[serde(default)]
    pub earlier: bool,
}

/// Which steps of a session to read, or to follow live from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StepsQuery {
    /// The last step the client has; 0: the newest (the transcript) or all (the live stream).
    pub after: u64,
    /// At most this many (the transcript only); [`DEFAULT_LIMIT`] when none.
    pub limit: Option<u32>,
    /// One helper agent's steps only, by the call that started it (a worker's session reads
    /// them from the agent's own stream; a runner session sends them all).
    pub agent: Option<ToolCallId>,
}

impl StepsQuery {
    /// The query string, without the `?`: `after=0&limit=400&agent=...`, values
    /// percent-encoded.
    ///
    /// ```
    /// use emberhut_api::runner::transcript::StepsQuery;
    ///
    /// let query = StepsQuery { after: 12, limit: Some(400), agent: Some("toolu 1".into()) };
    /// assert_eq!(query.to_query(), "after=12&limit=400&agent=toolu%201");
    /// assert_eq!(StepsQuery::default().to_query(), "after=0");
    /// ```
    #[must_use]
    pub fn to_query(&self) -> String {
        let mut query = format!("after={}", self.after);
        if let Some(limit) = self.limit {
            query.push_str("&limit=");
            query.push_str(&limit.to_string());
        }
        if let Some(agent) = &self.agent {
            query.push_str("&agent=");
            push_encoded(&mut query, agent.as_str());
        }
        query
    }
}

/// One event of a session's live stream: the JSON of its `data:` line, told apart by `type`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum SessionLive {
    /// A new step.
    Step {
        /// The step.
        step: SessionStep,
    },
    /// The model's words as they are typed: `start` begins a new block (the words so far are
    /// replaced), else `text` goes on the end. The block becomes a `text` step once done.
    Delta {
        /// The turn.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn: Option<TurnId>,
        /// The words.
        #[serde(default)]
        text: String,
        /// A new block starts.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<bool>,
    },
    /// The model's thinking as it is written: `start` begins a block (since when), else `text`
    /// goes on the end. It becomes a `thinking` step once done.
    Think {
        /// The turn.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn: Option<TurnId>,
        /// The words.
        #[serde(default)]
        text: String,
        /// A new block starts.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<bool>,
        /// When the block began.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        since: Option<Timestamp>,
    },
    /// The running turn's output tokens so far.
    Tokens {
        /// The turn.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn: Option<TurnId>,
        /// Its tokens.
        tokens: f64,
    },
    /// The session's head changed.
    Head {
        /// The head.
        session: SessionHead,
    },
}
