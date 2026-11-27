//! `notes`, `note_search` and `diary_write`: the owner's quick notes (the diary the owner and
//! the assistant share) and the assistant's own entry a day.

use emberhut_core::memory::notes::render_days;
use emberhut_core::memory::recall::{Filter, LIMITS, render, source};
use serde::Deserialize;

use super::{Tool, ToolContext, ToolError, ToolOutput};
use crate::services::notes::NoteError;

/// What every notes tool says about them.
macro_rules! private {
    () => {
        " Theirs alone: never quote or pass them to anyone else (another person's chat, a \
         worker's brief, another person's room) unless they ask."
    };
}

/// The owner's quick notes, day by day.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Notes;

/// `notes`' arguments.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct NotesArgs {
    /// One day (YYYY-MM-DD); default today.
    #[garde(length(max = 10))]
    day: Option<String>,
    /// The first day of a range.
    #[garde(length(max = 10))]
    from: Option<String>,
    /// The last day of a range.
    #[garde(length(max = 10))]
    to: Option<String>,
}

impl Tool for Notes {
    type Args = NotesArgs;

    fn name(&self) -> &'static str {
        "notes"
    }

    fn description(&self) -> &'static str {
        concat!(
            "The user's quick notes, their diary: what they wrote during the day (a thought, \
             what happened, an idea, how they feel, a to-do), with times, day by day, and your \
             own entry for each day. day: one day (YYYY-MM-DD, default today); or from/to for \
             a range.",
            private!()
        )
    }

    async fn call(&self, ctx: &ToolContext, args: NotesArgs) -> Result<ToolOutput, ToolError> {
        let notes = ctx.app.notes();
        let list = if args.from.is_some() || args.to.is_some() {
            notes.list(args.from.as_deref(), args.to.as_deref()).await
        } else {
            let day = args.day.unwrap_or_else(|| notes.today());
            notes.list(Some(&day), Some(&day)).await
        }
        .map_err(ToolError::failed)?;
        Ok(ToolOutput::text(render_days(&list, notes.zone())))
    }
}

/// The default number of hits.
const fn eight() -> usize {
    LIMITS.0
}

/// Searches the owner's quick notes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NoteSearch;

/// `note_search`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct NoteSearchArgs {
    /// What to look for.
    #[garde(length(chars, min = 2, max = 2000))]
    q: String,
    /// How many hits, 1 to 30 (default 8).
    #[serde(default = "eight")]
    #[garde(range(min = 1, max = 30))]
    limit: usize,
}

impl Tool for NoteSearch {
    type Args = NoteSearchArgs;

    fn name(&self) -> &'static str {
        "note_search"
    }

    fn description(&self) -> &'static str {
        concat!(
            "Search the user's quick notes (their diary) by meaning and by words, e.g. \"what \
             did they say about sleep\".",
            private!()
        )
    }

    async fn call(&self, ctx: &ToolContext, args: NoteSearchArgs) -> Result<ToolOutput, ToolError> {
        let filter = Filter {
            source: Some(source::NOTE.to_owned()),
            ..Filter::default()
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

/// Writes the assistant's entry in the diary.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DiaryWrite;

/// `diary_write`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct DiaryWriteArgs {
    /// The entry: a plain, warm few lines.
    #[garde(length(chars, min = 10, max = 2000))]
    text: String,
    /// The day (YYYY-MM-DD, today or before); default today.
    #[garde(length(max = 32))]
    day: Option<String>,
}

impl Tool for DiaryWrite {
    type Args = DiaryWriteArgs;

    fn name(&self) -> &'static str {
        "diary_write"
    }

    fn description(&self) -> &'static str {
        "Write your own entry in the user's diary for a day (default today): a plain, warm few \
         lines on what the day was (what moved, what they did, what's left), in your voice, \
         shown as yours under their notes. One per day: writing again replaces it. The evening \
         wrap writes it."
    }

    async fn call(&self, ctx: &ToolContext, args: DiaryWriteArgs) -> Result<ToolOutput, ToolError> {
        match ctx.app.notes().write(&args.text, args.day.as_deref()).await {
            Ok(note) => Ok(ToolOutput::text(format!(
                "Written in the user's diary for {}.",
                note.day
            ))),
            Err(NoteError::Refused(refusal)) => Ok(ToolOutput::error(refusal.to_string())),
            Err(error) => Err(ToolError::failed(error)),
        }
    }
}
