//! One step of a runner session's transcript, as the runner writes it: what was asked, the
//! model's words and thinking, each tool call and its result, a question it asks the person, its
//! to-do list, a turn's end and the runner's own notes. Each kind has its own fields; the others
//! are left out, and are left out again when written.

use serde::{Deserialize, Serialize};

use crate::chat::ChatFile;
use crate::ids::{ToolCallId, TurnId};
use crate::open_enum::open_enum;
use crate::time::Timestamp;

open_enum! {
    /// What a step is.
    pub enum StepKind {
        /// A message to the session (see [`SessionStep::by`]).
        User = "user",
        /// The model's words.
        Text = "text",
        /// A tool call, with its input.
        Tool = "tool",
        /// What a tool call returned.
        Result = "result",
        /// A turn ended: its number and outcome.
        End = "end",
        /// The runner's own note: the session moved to another account, restarted...
        System = "system",
        /// A question the session asks the person, with its options.
        Question = "question",
        /// A thinking block, done.
        Thinking = "thinking",
        /// The session's to-do list as it is now.
        Todos = "todos",
    }
}

open_enum! {
    /// Who sent a message to a session.
    pub enum Sender {
        /// The person.
        ///
        /// Its word on the wire is in [`crate::wire`].
        Person = crate::wire::PERSON,
        /// The assistant, briefing it for a task.
        ///
        /// Its word on the wire is in [`crate::wire`].
        Assistant = crate::wire::ASSISTANT,
        /// The runner itself (a turn carried on after a restart).
        Runner = "runner",
        /// The session carried on by itself.
        Itself = "self",
    }
}

open_enum! {
    /// Where an item of a session's to-do list is.
    pub enum TodoStatus {
        /// Not started.
        Pending = "pending",
        /// Being done now.
        InProgress = "in_progress",
        /// Done.
        Completed = "completed",
    }
}

/// One step of a session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SessionStep {
    /// Its place in the session, rising: the live stream starts after the last one a client
    /// has.
    pub seq: u64,
    /// The turn it happened in.
    #[serde(default)]
    pub turn: Option<TurnId>,
    /// When.
    #[serde(default)]
    pub at: Option<Timestamp>,
    /// What it is.
    pub kind: StepKind,
    /// Its words: a message, the model's text or thinking, a result, a note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Who sent a message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Sender>,
    /// A tool call's tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// A tool call in brief: the command, the pattern searched for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// A tool call in words (`Running: npm test`, `Reading main.rs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doing: Option<String>,
    /// A tool call's input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// A result is an error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<bool>,
    /// A turn's outcome (`completed`, `interrupted`, `failed`...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// A turn's number, at its end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    /// The helper agent's call this step happened inside.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ToolCallId>,
    /// A tool call's id (its result and its helper's steps point to it).
    #[serde(default, rename = "id", skip_serializing_if = "Option::is_none")]
    pub call: Option<ToolCallId>,
    /// An edit's changed lines, `- old` and `+ new`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    /// The pictures and files sent with a message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<ChatFile>,
    /// A question's questions, each with its options.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<SessionQuestion>,
    /// How long a thinking block took, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
    /// How many lines a result had (its text may be cut in the middle).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<u64>,
    /// The to-do list, on a `todos` step.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub todos: Vec<TodoItem>,
}

impl SessionStep {
    /// A step of `kind` with nothing else in it but its place.
    #[must_use]
    pub fn new(seq: u64, kind: StepKind) -> Self {
        Self {
            seq,
            turn: None,
            at: None,
            kind,
            text: None,
            by: None,
            name: None,
            summary: None,
            doing: None,
            input: None,
            error: None,
            outcome: None,
            n: None,
            parent: None,
            call: None,
            diff: None,
            files: Vec::new(),
            questions: Vec::new(),
            ms: None,
            lines: None,
            todos: Vec::new(),
        }
    }

    /// Whether a result is an error.
    #[must_use]
    pub fn failed(&self) -> bool {
        self.error == Some(true)
    }
}

/// One question a session asks the person (its tool `AskUserQuestion`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SessionQuestion {
    /// The question.
    pub question: String,
    /// A short header over it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    /// Several options may be picked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multi_select: Option<bool>,
    /// Its options.
    #[serde(default)]
    pub options: Vec<QuestionOption>,
}

/// One option of a [`SessionQuestion`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct QuestionOption {
    /// What it says: sent back as the answer.
    pub label: String,
    /// What it means.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One item of a session's to-do list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TodoItem {
    /// Its id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// What it is.
    pub content: String,
    /// Where it is.
    pub status: TodoStatus,
    /// What it is while being done (`Running the tests`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_form: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_step_has_only_its_place_and_kind() {
        let step = SessionStep::new(4, StepKind::Text);
        assert_eq!((step.seq, &step.kind), (4, &StepKind::Text));
        assert!(!step.failed() && step.files.is_empty());
    }
}
