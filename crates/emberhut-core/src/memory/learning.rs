//! How the assistant learns from being corrected.
//!
//! Whenever the owner corrects it ("you didn't check...", "I already told them..."), the
//! correction is logged: by the assistant itself, and by the server for a message that sounds
//! like one ([`sounds_like_correction`], a candidate a weekly review confirms or drops). A
//! correction's lesson is a *trial rule* in the assistant's prompt from the moment it is logged,
//! for [`TRIAL_DAYS`]; the review keeps it (as a *standing rule*, a lesson the owner approved) or
//! drops it, and the owner can drop one in the app. Unreviewed, it lapses.

use std::sync::LazyLock;

use jiff::{SignedDuration, Timestamp};
use regex::Regex;

use super::{cut, wire};

/// How long a correction's lesson is a trial rule.
pub const TRIAL_DAYS: i64 = 14;

/// Characters a page of the chat log holds at most: well under what a tool's answer carries.
pub const CHAT_PAGE: usize = 60_000;

/// The longest message in the chat log, in characters.
pub const CHAT_LINE: usize = 1500;

/// Who logged a correction. Stored as the wire words ([`super::wire`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CorrectionSource {
    /// The assistant, with its tool ([`wire::ASSISTANT`]).
    Assistant,
    /// The server, for a message that sounds like a correction (`auto`).
    Auto,
}

impl CorrectionSource {
    /// The stored spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Assistant => wire::ASSISTANT,
            Self::Auto => "auto",
        }
    }
}

/// Who took a trial rule out of the prompt. Stored as the wire words ([`super::wire`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrialJudge {
    /// The owner, in the app ([`wire::OWNER`]).
    Owner,
    /// The weekly review (`review`).
    Review,
}

impl TrialJudge {
    /// The stored spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => wire::OWNER,
            Self::Review => "review",
        }
    }
}

/// A correction to log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewCorrection {
    /// What the owner said.
    pub said: String,
    /// What the assistant did wrong.
    pub wrong: String,
    /// The rule that would have avoided it: a trial rule from now.
    pub lesson: Option<String>,
    /// Who logged it.
    pub source: CorrectionSource,
    /// When it happened (now, unless the assistant says otherwise).
    pub at: Timestamp,
}

/// A logged correction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Correction {
    /// Its id: its trial rule is `T<id>`.
    pub id: i64,
    /// When it happened.
    pub at: Timestamp,
    /// What the owner said.
    pub said: String,
    /// What went wrong.
    pub wrong: String,
    /// Its lesson, if one was given.
    pub lesson: Option<String>,
    /// Who logged it (the stored spelling).
    pub source: String,
}

/// A standing rule the owner approved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lesson {
    /// Its number.
    pub id: i64,
    /// The rule.
    pub text: String,
    /// When it was kept.
    pub at: Timestamp,
}

/// A correction's lesson on trial.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trial {
    /// Its correction's id.
    pub id: i64,
    /// The lesson.
    pub text: String,
    /// When it was logged.
    pub at: Timestamp,
    /// When it lapses unless kept.
    pub ends: Timestamp,
    /// What the owner said that it came from (300 characters at most).
    pub said: String,
}

/// The oldest a correction can be and still be on trial at `now`.
#[must_use]
pub fn trial_window(now: Timestamp) -> Timestamp {
    now - SignedDuration::from_hours(24 * TRIAL_DAYS)
}

/// `correction`'s trial, when it has a lesson that isn't blank.
#[must_use]
pub fn trial_of(correction: &Correction) -> Option<Trial> {
    let text = correction.lesson.as_deref()?.trim();
    (!text.is_empty()).then(|| Trial {
        id: correction.id,
        text: text.to_owned(),
        at: correction.at,
        ends: correction.at + SignedDuration::from_hours(24 * TRIAL_DAYS),
        said: cut(&correction.said, 300).to_owned(),
    })
}

/// The id for a new standing rule: one more than the highest of `kept`.
#[must_use]
pub fn next_lesson_id(kept: impl IntoIterator<Item = i64>) -> i64 {
    kept.into_iter().max().unwrap_or(0).max(0) + 1
}

/// What sounds like a correction or confusion, in English or Arabic.
static SOUNDS: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(you didn'?t|you did not|you don'?t|you never|i already|i told you|already (told|sent|replied|answered|did|said)|i don'?t (get|understand)|what do you mean|what does (it|that|this) mean|that'?s (wrong|not (right|true|it|what))|not what i (said|asked|meant)|why (did|didn'?t|are|aren'?t|is|isn'?t|would|wouldn'?t|do|don'?t) you|wrong)\b|ليش|مش هيك|قلتلك",
    )
    .ok()
});

/// Whether the owner's message `said` sounds like a correction or confusion: the server logs it
/// as a candidate.
#[must_use]
pub fn sounds_like_correction(said: &str) -> bool {
    SOUNDS.as_ref().is_some_and(|sounds| sounds.is_match(said))
}

/// The automatic candidate for `said`, answered by `last_answer`, at `at`.
#[must_use]
pub fn noticed(said: &str, last_answer: Option<&str>, at: Timestamp) -> NewCorrection {
    NewCorrection {
        said: cut(said, 1000).to_owned(),
        wrong: format!(
            "Your answer before it: {}",
            cut(last_answer.unwrap_or_default(), 600)
        ),
        lesson: None,
        source: CorrectionSource::Auto,
        at,
    }
}

/// A trial rule's line in the prompt: `T12 (24 Nov). The lesson`.
#[must_use]
pub fn trial_line(trial: &Trial) -> String {
    format!(
        "T{} ({}). {}",
        trial.id,
        trial.at.strftime("%-d %b"),
        cut(&trial.text, 600)
    )
}

/// The standing and trial rules, as the assistant's prompt carries them; empty when there are
/// none.
#[must_use]
pub fn rules_text(lessons: &[Lesson], trials: &[Trial]) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !lessons.is_empty() {
        let lines: Vec<String> = lessons
            .iter()
            .map(|lesson| format!("{}. {}", lesson.id, lesson.text))
            .collect();
        parts.push(format!(
            "\n\nStanding rules the user OKed (from your corrections; follow them always):\n{}",
            lines.join("\n")
        ));
    }
    if !trials.is_empty() {
        let lines: Vec<String> = trials.iter().map(trial_line).collect();
        parts.push(format!(
            "\n\nTrial rules: the lessons of your own corrections, the last {TRIAL_DAYS} days \
             (follow them too; the weekly review keeps or drops each):\n{}",
            lines.join("\n")
        ));
    }
    parts.concat()
}

/// One message of the chat log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatLine {
    /// When it was said.
    pub at: Timestamp,
    /// The owner said it (else the assistant did).
    pub mine: bool,
    /// The room's name, when it isn't Main.
    pub room: Option<String>,
    /// What was said.
    pub text: String,
}

/// A page of the chat log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatPage {
    /// Its lines.
    pub text: String,
    /// The first message on it, counted from 1.
    pub from: usize,
    /// The last message on it.
    pub to: usize,
    /// Every message in the log's days.
    pub total: usize,
    /// When more follow: the time of its last message, for the next page's `after`.
    pub next: Option<String>,
}

/// The page of `lines` (oldest first) after the instant `after` (the last page's
/// [`ChatPage::next`]; none: from the start), at most [`CHAT_PAGE`] characters. A page never
/// splits messages of the same moment.
#[must_use]
pub fn chat_page(lines: &[ChatLine], after: Option<&str>) -> ChatPage {
    let start = after.map_or(0, |after| {
        lines
            .iter()
            .position(|line| super::iso(line.at).as_str() > after)
            .unwrap_or(lines.len())
    });
    let mut shown = Vec::new();
    let mut size = 0;
    let mut end = start;
    while end < lines.len() {
        let line = &lines[end];
        let room = line
            .room
            .as_ref()
            .map_or_else(String::new, |room| format!(" {room}"));
        let who = if line.mine { "User" } else { "Assistant" };
        let text = format!(
            "[{}{room}] {who}: {}",
            cut(&super::iso(line.at), 16),
            cut(&line.text, CHAT_LINE)
        );
        let length = text.chars().count();
        if !shown.is_empty() && size + length + 1 > CHAT_PAGE && line.at != lines[end - 1].at {
            break;
        }
        shown.push(text);
        size += length + 1;
        end += 1;
    }
    ChatPage {
        text: shown.join("\n"),
        from: start + 1,
        to: end,
        total: lines.len(),
        next: (end < lines.len()).then(|| super::iso(lines[end - 1].at)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn corrections_are_noticed_in_both_languages() {
        assert!(sounds_like_correction("You didn't check the date"));
        assert!(sounds_like_correction("I already told you that"));
        assert!(sounds_like_correction("ليش عملت هيك"));
        assert!(!sounds_like_correction("Thanks, that's great"));
        let noticed = noticed(
            "you never listen",
            Some("Done."),
            at("2026-11-24T10:00:00Z"),
        );
        assert_eq!(noticed.source, CorrectionSource::Auto);
        assert_eq!(noticed.wrong, "Your answer before it: Done.");
    }

    #[test]
    fn a_lesson_is_on_trial_for_fourteen_days() {
        let correction = Correction {
            id: 7,
            at: at("2026-11-10T10:00:00Z"),
            said: "you didn't check".to_owned(),
            wrong: "I guessed".to_owned(),
            lesson: Some("  Check first. ".to_owned()),
            source: wire::ASSISTANT.to_owned(),
        };
        let trial = trial_of(&correction).unwrap();
        assert_eq!(trial.text, "Check first.");
        assert_eq!(trial.ends, at("2026-11-24T10:00:00Z"));
        assert_eq!(trial_line(&trial), "T7 (10 Nov). Check first.");
        let blank = Correction {
            lesson: Some(" ".to_owned()),
            ..correction
        };
        assert_eq!(trial_of(&blank), None);
        assert_eq!(
            trial_window(at("2026-11-24T10:00:00Z")),
            at("2026-11-10T10:00:00Z")
        );
    }

    #[test]
    fn rules_read_as_the_prompt_has_them() {
        let lessons = [Lesson {
            id: 2,
            text: "Say when unsure.".to_owned(),
            at: at("2026-11-01T00:00:00Z"),
        }];
        assert_eq!(next_lesson_id(lessons.iter().map(|lesson| lesson.id)), 3);
        assert_eq!(next_lesson_id([]), 1);
        let text = rules_text(&lessons, &[]);
        assert!(
            text.ends_with("follow them always):\n2. Say when unsure."),
            "{text}"
        );
        assert_eq!(rules_text(&[], &[]), "");
    }

    #[test]
    fn the_chat_log_pages_without_splitting_a_moment() {
        let line = |minute: u32, text: &str| ChatLine {
            at: at(&format!("2026-11-24T10:{minute:02}:00Z")),
            mine: minute.is_multiple_of(2),
            room: (minute == 3).then(|| "Garden".to_owned()),
            text: text.to_owned(),
        };
        let long = "x".repeat(CHAT_LINE);
        let mut lines: Vec<ChatLine> = (0..50).map(|n| line(n, &long)).collect();
        lines.insert(40, line(39, "same moment"));
        let first = chat_page(&lines, None);
        assert_eq!(first.from, 1);
        assert_eq!(first.total, 51);
        assert!(first.to < 51);
        let next = first.next.clone().unwrap();
        let second = chat_page(&lines, Some(&next));
        assert_eq!(second.from, first.to + 1);
        assert_eq!(second.next, None);
        let small = chat_page(&lines[..4], None);
        assert!(
            small.text.starts_with("[2026-11-24T10:00] User: x"),
            "{}",
            small.text
        );
        assert!(
            small
                .text
                .contains("[2026-11-24T10:03 Garden] Assistant: x")
        );
        let past = chat_page(&lines[..4], Some("2027-01-01T00:00:00.000Z"));
        assert_eq!((past.from, past.to), (5, 4));
    }
}
