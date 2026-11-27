//! `recall`: one search over everything said before, by meaning and by words.

use emberhut_core::memory::recall::{Filter, LIMITS, read_bound, render};
use serde::Deserialize;

use super::{Tool, ToolContext, ToolError, ToolOutput};

/// The kinds of piece a search can keep to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RecallSource {
    /// Agent sessions' turns (and their summaries).
    Session,
    /// Agent sessions' compaction summaries.
    Summary,
    /// The chat with the assistant.
    Chat,
    /// Tasks and their history.
    Task,
    /// Facts.
    Fact,
    /// Voice notes people sent.
    Voice,
    /// The quick notes and the diary.
    Note,
}

impl RecallSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Summary => "summary",
            Self::Chat => "chat",
            Self::Task => "task",
            Self::Fact => "fact",
            Self::Voice => "voice",
            Self::Note => "note",
        }
    }
}

/// The default number of hits.
const fn eight() -> usize {
    LIMITS.0
}

/// Searches everything said before.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Recall;

/// `recall`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct RecallArgs {
    /// What to look for.
    #[garde(length(chars, min = 2, max = 2000))]
    q: String,
    /// How many hits, 1 to 30 (default 8).
    #[serde(default = "eight")]
    #[garde(range(min = 1, max = 30))]
    limit: usize,
    /// Only this kind of piece.
    #[garde(skip)]
    source: Option<RecallSource>,
    /// Only pieces at or after this (an ISO date or instant).
    #[garde(length(max = 64))]
    since: Option<String>,
    /// Only pieces at or before this (an ISO date or instant).
    #[garde(length(max = 64))]
    until: Option<String>,
}

impl Tool for Recall {
    type Args = RecallArgs;

    fn name(&self) -> &'static str {
        "recall"
    }

    fn description(&self) -> &'static str {
        "Search everything said before, by meaning and by words: the user's and your agent \
         sessions (and their compaction summaries), their chat with you, tasks and their \
         history, your facts, voice notes people sent them, and their quick notes (their \
         diary). For what isn't in your facts (\"what did we decide about X\", \"when did they \
         mention Y\"). Each hit has its date, source and where to read more (a transcript \
         path:line: sed -n '<line>p' <path>). source: session, summary, chat, task, fact, voice \
         or note. since/until: ISO dates. Ask in the language the pieces were written in \
         (translate a question; search words as they were written)."
    }

    async fn call(&self, ctx: &ToolContext, args: RecallArgs) -> Result<ToolOutput, ToolError> {
        let filter = Filter {
            source: args.source.map(|kind| kind.as_str().to_owned()),
            since: args.since.as_deref().and_then(read_bound),
            until: args.until.as_deref().and_then(read_bound),
            without_notes: false,
        };
        let found = ctx
            .app
            .recall()
            .search(&args.q, args.limit, &filter)
            .await
            .map_err(ToolError::failed)?;
        Ok(ToolOutput::text(render(&found)))
    }
}
