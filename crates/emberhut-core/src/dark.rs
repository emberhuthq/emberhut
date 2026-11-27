//! The off-the-record room: a plain chat with a model that the assistant never sees. It has
//! its own database (left out of backups), no tools and none of the person's data; the model
//! gets a short system prompt and this room's own history, nothing else. Cleared, it's gone.
//!
//! A local model can be tried first, with a cloud one answering when it is off; a reply the
//! cloud model wrote for that reason says so at its top ([`FALLBACK`], [`FALLBACK_ERROR`]),
//! never a silent switch. The mark stays with the reply, so it still shows later, and is left
//! out of what the model is sent ([`unmarked`]).

/// The system prompt: the model's only instructions.
pub const SYSTEM: &str = "You are a private assistant in a chat with the user, off the record: \
    nothing here is shared with anyone or remembered elsewhere. Be direct and plain, answer what \
    they ask, and say so when you don't know. Reply in the language they write in.";

/// The top of a reply the cloud model wrote because the local one is off.
pub const FALLBACK: &str = "_via the cloud model, the local one is offline_\n\n";

/// The top of a reply the cloud model wrote because the local one failed before a word.
pub const FALLBACK_ERROR: &str = "_via the cloud model, the local one didn't answer_\n\n";

/// How many characters of history the local model's context takes, its answer's room left
/// (2 characters a token, a non-Latin script's worst case, in a 32K context).
pub const LOCAL_CONTEXT_CHARS: usize = 44_000;

/// Who said a line in the room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DarkRole {
    /// The person.
    User,
    /// The model.
    Assistant,
}

impl DarkRole {
    /// The role's word, as stored and sent.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }

    /// The role a stored word names (anything but `assistant` is the person).
    #[must_use]
    pub fn parse(word: &str) -> Self {
        if word == "assistant" {
            Self::Assistant
        } else {
            Self::User
        }
    }
}

/// `text` without a fallback mark at its top.
#[must_use]
pub fn unmarked(text: &str) -> &str {
    text.strip_prefix(FALLBACK)
        .or_else(|| text.strip_prefix(FALLBACK_ERROR))
        .unwrap_or(text)
}

/// Where the history the local model is sent starts: the latest lines (each given by its
/// length in characters, oldest first) that fit in [`LOCAL_CONTEXT_CHARS`], the last always.
#[must_use]
pub fn fit(lengths: &[usize]) -> usize {
    let Some((&last, before)) = lengths.split_last() else {
        return 0;
    };
    let mut room = LOCAL_CONTEXT_CHARS.saturating_sub(last);
    let mut from = before.len();
    while from > 0 && lengths[from - 1] < room {
        room -= lengths[from - 1];
        from -= 1;
    }
    from
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mark_is_dropped_only_at_the_top() {
        assert_eq!(unmarked(&format!("{FALLBACK}Hello")), "Hello");
        assert_eq!(unmarked(&format!("{FALLBACK_ERROR}Hi")), "Hi");
        assert_eq!(
            unmarked(&format!("Hi {FALLBACK}")),
            format!("Hi {FALLBACK}")
        );
    }

    #[test]
    fn the_local_model_gets_the_latest_lines_that_fit() {
        assert_eq!(fit(&[]), 0);
        assert_eq!(fit(&[10, 20, 30]), 0);
        assert_eq!(fit(&[30_000, 10_000, 5_000, 1_000]), 1);
        assert_eq!(fit(&[1, 50_000]), 1, "the last line always goes");
    }
}
