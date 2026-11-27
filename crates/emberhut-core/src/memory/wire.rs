//! The wire words memory stores and shows: its databases hold them, the diary reads by them and
//! the clients send and show them. They live here only; everything else names them through these
//! constants.
//!
//! The note's `who` on the wire (`emberhut_api::memory::NoteWho`) spells the same two words with
//! serde renames, which can't name a constant; a test in the server checks they agree.

/// Who said it: the owner. A note's `who` for the owner's quick notes, and a trial rule's
/// `trial_by` when the owner dropped it in the app.
pub const OWNER: &str = "owner";

/// Who said it: the assistant. A note's `who` for its diary entries, and a correction's
/// `source` when the assistant logged it with its tool.
pub const ASSISTANT: &str = "assistant";

/// The prefix of the assistant's diary entry's note id: `<prefix><day>`.
pub const DIARY_PREFIX: &str = "assistant:";
