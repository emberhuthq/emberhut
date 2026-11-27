//! Voice notes and audio clips people send the owner (Slack clips, WhatsApp voice notes,
//! iMessage audio), as text. Each is transcribed once and kept by its file's id; a reader
//! sees `[voice, 0:42] what they said` where a file name used to be.
//!
//! A note is tried at once (and once more at once after a hiccup); a failed round is tried
//! again after 1, 5 and 30 minutes, then given up. A Slack clip refused for want of the
//! `files:read` scope is tried every 30 minutes for 3 days instead, so it fills in once the
//! scope is added. A clip over the length limit is skipped and says so.

use jiff::{SignedDuration, Timestamp};

/// Where a note's sound comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VoiceSource {
    /// A Slack file, fetched with the owner's user token.
    Slack,
    /// A WhatsApp voice note, from the Mac helper.
    WhatsApp,
    /// An iMessage audio message, from the Mac helper.
    IMessage,
}

impl VoiceSource {
    /// The source's word as it is stored (`slack`, `wa`, `im`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Slack => "slack",
            Self::WhatsApp => "wa",
            Self::IMessage => "im",
        }
    }

    /// The source a stored word names.
    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            "slack" => Some(Self::Slack),
            "wa" => Some(Self::WhatsApp),
            "im" => Some(Self::IMessage),
            _ => None,
        }
    }

    /// The source of a Mac message's note id (`wa:<message>` or `im:<attachment>`).
    #[must_use]
    pub fn of_mac_id(id: &str) -> Self {
        if id.starts_with("wa:") {
            Self::WhatsApp
        } else {
            Self::IMessage
        }
    }

    /// How a search hit names it.
    #[must_use]
    pub fn shown(self) -> &'static str {
        match self {
            Self::Slack => "Slack",
            Self::WhatsApp => "WhatsApp",
            Self::IMessage => "iMessage",
        }
    }
}

/// Where a note is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VoiceStatus {
    /// Not tried yet.
    New,
    /// Transcribed.
    Done,
    /// Over the length limit: skipped.
    Long,
    /// Failed; tried again at its next time.
    Failed,
    /// Failed for good.
    GaveUp,
}

impl VoiceStatus {
    /// The status's word as it is stored.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Done => "done",
            Self::Long => "long",
            Self::Failed => "failed",
            Self::GaveUp => "gave_up",
        }
    }

    /// The status a stored word names; an unknown one reads as new.
    #[must_use]
    pub fn from_word(word: &str) -> Self {
        match word {
            "done" => Self::Done,
            "long" => Self::Long,
            "failed" => Self::Failed,
            "gave_up" => Self::GaveUp,
            _ => Self::New,
        }
    }
}

/// A voice note as it is kept.
#[derive(Clone, Debug, PartialEq)]
pub struct VoiceNote {
    /// Its id: `slack:<file>`, `wa:<message>` or `im:<attachment>`.
    pub id: String,
    /// Where its sound comes from.
    pub source: VoiceSource,
    /// What fetches it: `<workspace>|<url>` for Slack, the Mac's id otherwise.
    pub reference: String,
    /// Its file name.
    pub name: Option<String>,
    /// How long it is, when known.
    pub seconds: Option<f64>,
    /// Where it is.
    pub status: VoiceStatus,
    /// What was said, once done.
    pub text: Option<String>,
    /// Why the last try failed.
    pub error: Option<String>,
    /// How many rounds failed.
    pub tries: u32,
    /// When it is tried again.
    pub next_at: Option<Timestamp>,
    /// The chat it was sent in.
    pub chat: Option<String>,
    /// That chat's name.
    pub chat_name: Option<String>,
    /// Who sent it.
    pub sender: Option<String>,
    /// When its message was sent.
    pub msg_at: Option<Timestamp>,
    /// The kept message that shows it (a Slack message's id), whose text follows it.
    pub msg: Option<String>,
    /// What that message shows for it now.
    pub shown: Option<String>,
    /// When it was first seen.
    pub at: Timestamp,
    /// When it was done (or skipped).
    pub done_at: Option<Timestamp>,
}

/// A note seen in a message: what is kept of it before it is tried.
#[derive(Clone, Debug, PartialEq)]
pub struct SeenNote {
    /// Its id.
    pub id: String,
    /// Where its sound comes from.
    pub source: VoiceSource,
    /// What fetches it.
    pub reference: String,
    /// Its file name.
    pub name: Option<String>,
    /// How long it is, when known.
    pub seconds: Option<f64>,
    /// The chat it was sent in.
    pub chat: Option<String>,
    /// That chat's name.
    pub chat_name: Option<String>,
    /// Who sent it.
    pub sender: Option<String>,
    /// When its message was sent.
    pub msg_at: Option<Timestamp>,
    /// The kept message that shows it.
    pub msg: Option<String>,
}

/// The longest note transcribed, by default: ten minutes.
pub const MAX_SECONDS: f64 = 600.0;

/// The biggest file the speech-to-text service takes.
pub const MAX_BYTES: usize = 25 << 20;

/// The waits after each failed round; after the last, it gives up.
pub const BACKOFF: [SignedDuration; 3] = [
    SignedDuration::from_mins(1),
    SignedDuration::from_mins(5),
    SignedDuration::from_mins(30),
];

/// How often a Slack clip refused for want of `files:read` is tried, and for how long.
pub const BLOCKED_EVERY: SignedDuration = SignedDuration::from_mins(30);
/// How long a refused Slack clip keeps being tried after its message.
pub const BLOCKED_FOR: SignedDuration = SignedDuration::from_hours(72);

/// A length as `m:ss`.
///
/// ```
/// use emberhut_core::messaging::voice::clock;
///
/// assert_eq!(clock(42.0), "0:42");
/// assert_eq!(clock(125.4), "2:05");
/// ```
#[must_use]
pub fn clock(seconds: f64) -> String {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a note's minutes and seconds fit in i64; whole numbers are the point"
    )]
    let (minutes, rest) = (
        (seconds / 60.0).floor() as i64,
        (seconds % 60.0).round() as i64,
    );
    format!("{minutes}:{rest:02}")
}

/// The tag a note's text starts with: `[voice, m:ss]`, or `[voice]` with no length.
#[must_use]
pub fn tag(seconds: Option<f64>) -> String {
    match seconds {
        Some(s) if s != 0.0 && s.is_finite() => format!("[voice, {}]", clock(s)),
        _ => "[voice]".to_owned(),
    }
}

/// What a reader sees for a note now: its transcript once done, else its file name and why
/// there is no text yet. `note` is none when it was never seen; `name` names it then.
#[must_use]
pub fn line(note: Option<&VoiceNote>, name: Option<&str>, max_seconds: f64) -> String {
    let file = format!(
        "[{}]",
        note.and_then(|n| n.name.as_deref())
            .or(name)
            .unwrap_or("voice note")
    );
    let Some(note) = note else {
        return file;
    };
    match note.status {
        VoiceStatus::Done => {
            let said = note.text.as_deref().filter(|text| !text.is_empty());
            format!("{} {}", tag(note.seconds), said.unwrap_or("(nothing said)"))
        }
        VoiceStatus::Long => format!(
            "{} {} (not transcribed: over {} minutes)",
            tag(note.seconds),
            note.name.as_deref().unwrap_or("null"),
            minutes(max_seconds)
        ),
        VoiceStatus::Failed | VoiceStatus::GaveUp => format!("{file} (couldn't transcribe)"),
        VoiceStatus::New => format!("{file} (transcribing…)"),
    }
}

fn minutes(seconds: f64) -> i64 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a limit in minutes fits in i64"
    )]
    let minutes = (seconds / 60.0).round() as i64;
    minutes
}

/// When a note that just failed its `tries`-th round is tried again: none to give up.
/// `blocked` says the failure was Slack refusing for want of `files:read`, and `sent` when
/// its message was sent.
#[must_use]
pub fn next_try(
    tries: u32,
    blocked: bool,
    sent: Timestamp,
    now: Timestamp,
) -> Option<SignedDuration> {
    if blocked && now.duration_since(sent) < BLOCKED_FOR {
        return Some(BLOCKED_EVERY);
    }
    let index = usize::try_from(tries.checked_sub(1)?).ok()?;
    BACKOFF.get(index).copied()
}

/// The prompt that helps the speech-to-text service: the languages to expect, and names that
/// may come up.
#[must_use]
pub fn prompt(names: &[&str]) -> String {
    let hint = "A voice message sent to the owner. The speaker may use Arabic, English, or both \
                mixed in one sentence. Write it exactly as spoken: Arabic in Arabic script, \
                English in English.";
    let names: Vec<&str> = names.iter().copied().filter(|n| !n.is_empty()).collect();
    if names.is_empty() {
        hint.to_owned()
    } else {
        format!("{hint} Names that may come up: {}.", names.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(status: VoiceStatus) -> VoiceNote {
        VoiceNote {
            id: "slack:F1".to_owned(),
            source: VoiceSource::Slack,
            reference: "acme|https://files.example/F1".to_owned(),
            name: Some("clip.m4a".to_owned()),
            seconds: Some(42.0),
            status,
            text: Some("see you at five".to_owned()),
            error: None,
            tries: 0,
            next_at: None,
            chat: None,
            chat_name: None,
            sender: None,
            msg_at: None,
            msg: None,
            shown: None,
            at: Timestamp::UNIX_EPOCH,
            done_at: None,
        }
    }

    #[test]
    fn each_status_reads_its_own_way() {
        assert_eq!(
            line(Some(&note(VoiceStatus::Done)), None, MAX_SECONDS),
            "[voice, 0:42] see you at five"
        );
        assert_eq!(
            line(Some(&note(VoiceStatus::Long)), None, MAX_SECONDS),
            "[voice, 0:42] clip.m4a (not transcribed: over 10 minutes)"
        );
        assert_eq!(
            line(Some(&note(VoiceStatus::Failed)), None, MAX_SECONDS),
            "[clip.m4a] (couldn't transcribe)"
        );
        assert_eq!(
            line(Some(&note(VoiceStatus::New)), None, MAX_SECONDS),
            "[clip.m4a] (transcribing…)"
        );
        assert_eq!(line(None, Some("a.ogg"), MAX_SECONDS), "[a.ogg]");
        assert_eq!(line(None, None, MAX_SECONDS), "[voice note]");
        let mut silent = note(VoiceStatus::Done);
        silent.text = Some(String::new());
        silent.seconds = None;
        assert_eq!(
            line(Some(&silent), None, MAX_SECONDS),
            "[voice] (nothing said)"
        );
    }

    #[test]
    fn retries_back_off_then_give_up_unless_slack_is_refusing() {
        let sent = Timestamp::UNIX_EPOCH;
        let now = sent + SignedDuration::from_hours(1);
        assert_eq!(
            next_try(1, false, sent, now),
            Some(SignedDuration::from_mins(1))
        );
        assert_eq!(
            next_try(3, false, sent, now),
            Some(SignedDuration::from_mins(30))
        );
        assert_eq!(next_try(4, false, sent, now), None);
        assert_eq!(next_try(9, true, sent, now), Some(BLOCKED_EVERY));
        let late = sent + SignedDuration::from_hours(80);
        assert_eq!(next_try(9, true, sent, late), None);
    }

    #[test]
    fn the_prompt_names_who_may_come_up() {
        assert!(prompt(&["Ann", ""]).ends_with("Names that may come up: Ann."));
        assert!(!prompt(&[]).contains("Names"));
    }
}
