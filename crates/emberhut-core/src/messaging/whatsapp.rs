//! The assistant's own WhatsApp number, in the groups and DMs the owner allowed.
//!
//! - The allow-list: the assistant acts only in a group the owner allowed (after its number
//!   was added there) and in DMs with people the owner named. Messages from anywhere else
//!   are dropped unread; a group nobody allowed only gets the owner asked.
//! - When: only a message that addresses the assistant (an @mention, a reply to one of its
//!   messages, or one that starts with its name; in an allowed DM, every message) wakes it,
//!   and it answers once per such message, in that chat only, never unprompted.
//! - Pace: one reply per chat every [`REPLY_GAP`], [`REPLIES_PER_HOUR`] in all,
//!   [`ASKS_PER_HOUR`] woken asks, and no links in its first [`NO_LINKS_FOR`] messages (a new
//!   number posting links gets banned).
//! - A kill switch stops every reply at once.

use jiff::{SignedDuration, Timestamp};

use crate::RoomId;

/// A group or a DM.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChatKind {
    /// A group the number is in.
    Group,
    /// DMs with one person.
    Dm,
}

impl ChatKind {
    /// The kind's word as it is stored.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Group => "group",
            Self::Dm => "dm",
        }
    }

    /// The kind a stored word names (anything but `group` is a DM).
    #[must_use]
    pub fn from_word(word: &str) -> Self {
        if word == "group" {
            Self::Group
        } else {
            Self::Dm
        }
    }
}

/// Where a chat is on the allow-list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChatStatus {
    /// The owner was asked.
    Pending,
    /// The owner allowed it.
    Allowed,
    /// The owner didn't, or took it off the list.
    Denied,
    /// The number was removed from the group.
    Left,
}

impl ChatStatus {
    /// The status's word as it is stored.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Allowed => "allowed",
            Self::Denied => "denied",
            Self::Left => "left",
        }
    }

    /// The status a stored word names (an unknown one reads as pending).
    #[must_use]
    pub fn from_word(word: &str) -> Self {
        match word {
            "allowed" => Self::Allowed,
            "denied" => Self::Denied,
            "left" => Self::Left,
            _ => Self::Pending,
        }
    }
}

/// A chat the number knows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaChat {
    /// The group's id, or the person's phone id (`<digits>@s.whatsapp.net`).
    pub jid: String,
    /// A group or a DM.
    pub kind: ChatKind,
    /// Its name.
    pub name: Option<String>,
    /// Where it is on the allow-list.
    pub status: ChatStatus,
    /// The id a DM's messages come from, when it isn't the phone id (a `@lid` one).
    pub via: Option<String>,
    /// The task the owner was asked on.
    pub task: Option<i64>,
    /// The room its asks go to (none: the WhatsApp room).
    pub room: Option<RoomId>,
    /// What the owner allowed the assistant to talk about there.
    pub topics: Option<String>,
    /// Who added the number to the group.
    pub by: Option<String>,
    /// When it was first seen.
    pub added: Timestamp,
    /// When it last changed.
    pub updated: Timestamp,
}

/// A message in an allowed chat, kept for context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaMessage {
    /// The chat's jid.
    pub chat: String,
    /// WhatsApp's id of the message.
    pub id: String,
    /// The sender's jid.
    pub sender: Option<String>,
    /// How the sender is named.
    pub who: Option<String>,
    /// What it says.
    pub text: Option<String>,
    /// When it was sent.
    pub at: Timestamp,
    /// The assistant sent it.
    pub from_me: bool,
    /// It addressed the assistant.
    pub asked: bool,
    /// The assistant replied to it.
    pub answered: bool,
}

/// A message the assistant sent: the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaSent {
    /// Its row's id.
    pub id: i64,
    /// The chat's jid.
    pub chat: String,
    /// The chat's name then.
    pub name: Option<String>,
    /// When it was sent.
    pub at: Timestamp,
    /// What it said.
    pub text: String,
    /// WhatsApp's id of it.
    pub msg: Option<String>,
    /// The message it answered.
    pub reply_to: Option<String>,
    /// The room it was shown in.
    pub room: Option<RoomId>,
}

/// The messages kept per chat.
pub const KEPT_PER_CHAT: u64 = 300;
/// The shortest gap between two replies in one chat.
pub const REPLY_GAP: SignedDuration = SignedDuration::from_secs(10);
/// The most replies in an hour, in every chat.
pub const REPLIES_PER_HOUR: u64 = 20;
/// The most asks passed to the assistant in an hour.
pub const ASKS_PER_HOUR: u64 = 10;
/// How many first messages carry no link.
pub const NO_LINKS_FOR: u64 = 10;
/// How long a message that asked can be answered.
pub const ANSWER_WITHIN: SignedDuration = SignedDuration::from_mins(30);
/// The messages of context an ask comes with.
pub const CONTEXT: u64 = 20;
/// How long the one-time QR page works.
pub const QR_FOR: SignedDuration = SignedDuration::from_mins(10);

/// The digits in `text`.
#[must_use]
pub fn digits(text: &str) -> String {
    text.chars().filter(char::is_ascii_digit).collect()
}

/// Whether `jid` is a group's.
#[must_use]
pub fn is_group_jid(jid: &str) -> bool {
    jid.ends_with("@g.us")
}

/// The phone id of a number written any way (`+1 555 010 0199`), when it has at least eight
/// digits and isn't a group's id.
#[must_use]
pub fn phone_jid(text: &str) -> Option<String> {
    let digits = digits(text);
    (digits.len() >= 8 && !is_group_jid(text.trim())).then(|| format!("{digits}@s.whatsapp.net"))
}

/// The phone number written in `text` (a `+`, digits, spaces, dots, dashes, brackets), as
/// `+` and digits; none when there is none.
///
/// ```
/// use emberhut_core::messaging::whatsapp::phone_in;
///
/// assert_eq!(phone_in("it's +1 (555) 010-0199 thanks"), Some("+15550100199".to_owned()));
/// assert_eq!(phone_in("call 12"), None);
/// ```
#[must_use]
pub fn phone_in(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    for start in 0..chars.len() {
        let first = chars[start];
        let begin = if first == '+' { start + 1 } else { start };
        if !chars.get(begin).is_some_and(char::is_ascii_digit) {
            continue;
        }
        // The longest run of number characters, ending on a digit, 10 to 20 long.
        let mut end = begin;
        let mut last_digit = None;
        while end < chars.len()
            && end - begin < 20
            && (chars[end].is_ascii_digit() || " ().-".contains(chars[end]))
        {
            if chars[end].is_ascii_digit() {
                last_digit = Some(end);
            }
            end += 1;
        }
        let Some(last) = last_digit else { continue };
        if last - begin + 1 >= 10 {
            let taken: String = chars[start..=last]
                .iter()
                .filter(|c| c.is_ascii_digit() || **c == '+')
                .collect();
            return Some(taken);
        }
    }
    None
}

/// Whether a group message's text addresses the assistant by `name`: it starts with the name
/// (after at most two other characters), as a whole word, in any case.
///
/// ```
/// use emberhut_core::messaging::whatsapp::starts_with_name;
///
/// assert!(starts_with_name("Hey, is it on?", "hey"));
/// assert!(starts_with_name("@Nova what's up", "nova"));
/// assert!(!starts_with_name("Novaland is far", "nova"));
/// ```
#[must_use]
pub fn starts_with_name(text: &str, name: &str) -> bool {
    let name = name.to_lowercase();
    if name.is_empty() {
        return false;
    }
    let lower = text.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    for skip in 0..=2 {
        if skip > 0
            && chars
                .get(skip - 1)
                .is_none_or(|c| c.is_alphanumeric() || *c == '_')
        {
            break;
        }
        let rest: String = chars.iter().skip(skip).collect();
        if let Some(after) = rest.strip_prefix(&name) {
            return after
                .chars()
                .next()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        }
    }
    false
}

/// A pairing code shown in two halves (`ABCD-EFGH`), as WhatsApp shows it.
#[must_use]
pub fn pairing_code(code: &str) -> String {
    if code.chars().count() == 8 {
        let (head, tail) = code.split_at(code.char_indices().nth(4).map_or(4, |(i, _)| i));
        format!("{head}-{tail}")
    } else {
        code.to_owned()
    }
}

/// How a chat is named to the assistant and the owner: `"<group>"`, or `DMs with <name>`.
#[must_use]
pub fn label(chat: &WaChat) -> String {
    match chat.kind {
        ChatKind::Group => format!("\"{}\"", chat.name.as_deref().unwrap_or("a group")),
        ChatKind::Dm => match &chat.name {
            Some(name) => format!("DMs with {name}"),
            None => format!("DMs with +{}", digits(&chat.jid)),
        },
    }
}

/// Why a reply may not go out now, by the pace rules; none when it may.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pace {
    /// The last reply in this chat was this long ago, under [`REPLY_GAP`].
    TooSoon(SignedDuration),
    /// [`REPLIES_PER_HOUR`] went out in the last hour.
    HourFull,
}

/// The pace rules for one more reply: `last_here` is when the assistant last wrote in this
/// chat, `last_hour` how many it sent in the last hour, everywhere.
#[must_use]
pub fn pace(last_here: Option<Timestamp>, last_hour: u64, now: Timestamp) -> Option<Pace> {
    if let Some(last) = last_here {
        let since = now.duration_since(last);
        if since < REPLY_GAP {
            return Some(Pace::TooSoon(since));
        }
    }
    (last_hour >= REPLIES_PER_HOUR).then_some(Pace::HourFull)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_jids() {
        assert_eq!(digits("+1 (555) 010"), "1555010");
        assert_eq!(
            phone_jid("+1 555 010 0199"),
            Some("15550100199@s.whatsapp.net".to_owned())
        );
        assert_eq!(phone_jid("1203630@g.us"), None);
        assert_eq!(phone_jid("12345"), None);
        assert_eq!(pairing_code("ABCDEFGH"), "ABCD-EFGH");
        assert_eq!(pairing_code("ABC"), "ABC");
    }

    #[test]
    fn labels_name_groups_and_dms() {
        let at = Timestamp::UNIX_EPOCH;
        let mut chat = WaChat {
            jid: "15550100199@s.whatsapp.net".to_owned(),
            kind: ChatKind::Dm,
            name: None,
            status: ChatStatus::Allowed,
            via: None,
            task: None,
            room: None,
            topics: None,
            by: None,
            added: at,
            updated: at,
        };
        assert_eq!(label(&chat), "DMs with +15550100199");
        chat.kind = ChatKind::Group;
        chat.name = Some("Book club".to_owned());
        assert_eq!(label(&chat), "\"Book club\"");
    }

    #[test]
    fn the_pace_rules() {
        let now = Timestamp::UNIX_EPOCH + SignedDuration::from_hours(5);
        assert_eq!(pace(None, 0, now), None);
        assert_eq!(
            pace(Some(now - SignedDuration::from_secs(4)), 0, now),
            Some(Pace::TooSoon(SignedDuration::from_secs(4)))
        );
        assert_eq!(
            pace(Some(now - SignedDuration::from_secs(11)), 19, now),
            None
        );
        assert_eq!(pace(None, 20, now), Some(Pace::HourFull));
    }

    #[test]
    fn the_name_must_lead_as_a_word() {
        assert!(starts_with_name("nova", "Nova"));
        assert!(starts_with_name("  nova, hi", "nova"));
        assert!(!starts_with_name("...nova", "nova"));
        assert!(!starts_with_name("ask nova", "nova"));
        assert!(!starts_with_name("xyznova", "nova"));
    }
}
