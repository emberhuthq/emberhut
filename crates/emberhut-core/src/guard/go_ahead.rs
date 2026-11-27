//! Go-ahead cards for the final steps of a job in the browser or on the owner's Mac: the
//! rules on words. A final step goes through on the job's grant ([`super::grants`]), on the
//! exact text the owner OK'd for a message or a post, or on that step's own Go ahead card; the
//! hard steps ask every time.

use std::sync::LazyLock;

use regex::Regex;

use super::go;

/// How long a planned job's OK lasts.
pub const PLAN_HOURS: i64 = 4;

/// How long the owner's request in a room carries: a task made there that long after their
/// last message there is still theirs.
pub const CARRY_HOURS: i64 = 72;

/// The owner's lines in a task's history that aren't words of theirs: a tap, and what their
/// app's buttons note.
static TAPPED: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"^(?:Chose: |Created$|Started$|Back to to do$|Waiting on .+$|Done$|Canceled$|Archived by .+$|Archived with [A-Z]+-\d+$|Unarchived by .+$|Decide by |No decide-by date$|Turned ultra mode |Changed the description$|Posted reply )").ok()
});

/// Whether a history line of the owner's is a tap or a button's note, not their words.
#[must_use]
pub fn tapped(text: &str) -> bool {
    TAPPED.as_ref().is_some_and(|re| re.is_match(text))
}

/// Whether a history line is the owner's tap (`Chose: <answer>`).
#[must_use]
pub fn is_choice(text: &str) -> bool {
    text.starts_with("Chose: ")
}

/// A plan's step and a button's label are the same step when one's words hold all of the
/// other's ("Create app" and "Create"); "Create app" never covers "Create API key".
#[must_use]
pub fn matches(step: &str, label: &str) -> bool {
    let a = go::loose_words(&go::loose(step));
    let b = go::loose_words(&go::loose(label));
    !a.is_empty()
        && !b.is_empty()
        && (a.iter().all(|w| b.contains(w)) || b.iter().all(|w| a.contains(w)))
}

/// Every pattern here compiles.
#[must_use]
pub fn patterns_compile() -> bool {
    TAPPED.is_some()
}
