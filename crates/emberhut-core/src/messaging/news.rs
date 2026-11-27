//! News from the owner's chats, as it wakes the assistant: which new messages can't wait, how
//! long the rest gather, and the text of the event that brings them.
//!
//! Every couple of minutes the server looks for chats with new messages (iMessage, SMS and
//! WhatsApp from the Mac, Slack, email). They gather for [`BATCH`] and then reach Main as one
//! `messages` event, unless one can't wait: someone the owner or the assistant marked
//! important wrote, a message says it is urgent, or a group mentions the owner. Then they go
//! at once, as an `important` event, which wakes Main at once.

use std::sync::LazyLock;

use jiff::SignedDuration;
use regex::Regex;

/// How long new messages gather before they reach the assistant, when none of them can't wait.
pub const BATCH: SignedDuration = SignedDuration::from_mins(14);

/// The chats one look takes at most.
pub const MAX_CHATS: usize = 20;

/// The messages one chat brings at most.
pub const MAX_MESSAGES: u64 = 40;

/// One message from the Mac is cut to this many characters (a voice note's transcript to
/// [`VOICE_CHARS`]).
pub const MESSAGE_CHARS: usize = 500;

/// A voice note's transcript is cut to this many characters.
pub const VOICE_CHARS: usize = 6000;

/// One email is cut to this many characters.
pub const MAIL_CHARS: usize = 1500;

/// One event's chats are cut to this many characters.
pub const EVENT_CHARS: usize = 16_000;

/// The first look, with nothing seen yet, starts this long ago.
pub const FIRST_LOOK: SignedDuration = SignedDuration::from_hours(6);

/// What the assistant is asked to do with the news.
pub const KEEP: &str = "For each chat that matters: update that person's card with person_set \
    (now: what's going on with them, rewritten whole; chat: link it), loop_set for anyone \
    waiting on the owner, a promise or a deadline; close_loop when one is done. Skip noise \
    (codes, shops, ads).";

/// Words that say a message is urgent, in English or Arabic.
#[expect(
    clippy::expect_used,
    reason = "a constant pattern; the tests compile it"
)]
static URGENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:urgent|asap|emergency|right now|call me)\b|عاجل|ضروري|بسرعة|اتصل (?:علي|فيي|بي)|رد علي",
    )
    .expect("a valid constant pattern")
});

/// Whether `text` says it is urgent.
///
/// ```
/// use emberhut_core::messaging::news::is_urgent;
///
/// assert!(is_urgent("Call me when you see this"));
/// assert!(is_urgent("ASAP please"));
/// assert!(!is_urgent("the urgency of it all"));
/// ```
#[must_use]
pub fn is_urgent(text: &str) -> bool {
    URGENT.is_match(text)
}

/// Whether `text` names `name` as a whole word, in any case. A blank name is never mentioned.
///
/// ```
/// use emberhut_core::messaging::news::mentions;
///
/// assert!(mentions("ask Sam about it", "sam"));
/// assert!(!mentions("Samantha said so", "Sam"));
/// ```
#[must_use]
pub fn mentions(text: &str, name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() {
        return false;
    }
    Regex::new(&format!(r"(?i)\b{}\b", regex::escape(name))).is_ok_and(|rx| rx.is_match(text))
}

/// Why a chat's new messages can't wait, if they can't: the first that holds of someone marked
/// important wrote, one of `theirs` (the messages not the owner's) is urgent, or a `group`'s
/// mentions `owner`.
#[must_use]
pub fn why_now(
    name: &str,
    theirs: &[&str],
    important: bool,
    group: bool,
    owner: Option<&str>,
) -> Option<String> {
    if theirs.is_empty() {
        None
    } else if important {
        Some(format!("{name} wrote (marked important)"))
    } else if theirs.iter().any(|text| is_urgent(text)) {
        Some(format!("{name} says it's urgent"))
    } else if group && owner.is_some_and(|owner| theirs.iter().any(|text| mentions(text, owner))) {
        Some(format!("{name} mentions the owner"))
    } else {
        None
    }
}

/// Whether a chat's name is a group's: a Slack channel (`#…`) or a group DM (names with
/// commas).
#[must_use]
pub fn is_group_name(name: &str) -> bool {
    name.contains('#') || name.contains(',')
}

/// `text` cut to [`EVENT_CHARS`], saying where the rest is.
#[must_use]
pub fn cut(text: String) -> String {
    if text.chars().count() <= EVENT_CHARS {
        return text;
    }
    let mut cut = super::text::first_chars(&text, EVENT_CHARS);
    cut.push_str("\n… more in these chats: messages_read");
    cut
}

/// The news event's kind and text: `important` with why when some can't wait, `messages`
/// otherwise; `since` is when the last look was.
#[must_use]
pub fn event(why: &[String], since: &str, chats: &str) -> (&'static str, String) {
    let first = if why.is_empty() {
        String::new()
    } else {
        format!("Can't wait: {}.\n", why.join("; "))
    };
    let kind = if why.is_empty() {
        "messages"
    } else {
        "important"
    };
    (
        kind,
        format!(
            "{first}New messages since {since}:\n\n{chats}\n\n{KEEP} Do what helps the owner's \
             goals. Reply NO_REPLY if nothing needs them."
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urgent_words_in_english_and_arabic() {
        assert!(is_urgent("this is URGENT"));
        assert!(is_urgent("emergency at the site"));
        assert!(is_urgent("رد علي بسرعة"));
        assert!(is_urgent("اتصل بي"));
        assert!(!is_urgent("see you tomorrow"));
    }

    #[test]
    fn the_first_reason_holds() {
        let theirs = ["call me", "hi Sam"];
        assert_eq!(
            why_now("Ann", &theirs, true, true, Some("Sam")).as_deref(),
            Some("Ann wrote (marked important)")
        );
        assert_eq!(
            why_now("Ann", &theirs, false, true, Some("Sam")).as_deref(),
            Some("Ann says it's urgent")
        );
        assert_eq!(
            why_now("#team", &["hi Sam"], false, true, Some("Sam")).as_deref(),
            Some("#team mentions the owner")
        );
        assert_eq!(why_now("Ann", &["hi Sam"], false, false, Some("Sam")), None);
        assert_eq!(why_now("#team", &["hi Sam"], false, true, None), None);
        assert_eq!(why_now("Ann", &[], true, false, None), None);
    }

    #[test]
    fn a_long_event_is_cut() {
        let long = "x".repeat(EVENT_CHARS + 10);
        let cut = cut(long);
        assert!(cut.ends_with("messages_read"));
        assert!(cut.chars().count() < EVENT_CHARS + 60);
        assert_eq!(super::cut("short".to_owned()), "short");
    }

    #[test]
    fn the_event_says_why() {
        let (kind, text) = event(&[], "2026-11-26T10:00:00Z", "Ann (WhatsApp)");
        assert_eq!(kind, "messages");
        assert!(text.starts_with("New messages since 2026-11-26T10:00:00Z:\n\nAnn (WhatsApp)"));
        let (kind, text) = event(&["Ann says it's urgent".to_owned()], "t", "c");
        assert_eq!(kind, "important");
        assert!(text.starts_with("Can't wait: Ann says it's urgent.\n"));
    }
}
