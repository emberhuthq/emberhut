//! The owner's "send it" in the chat as the OK for a message, as safe as the card's Send it:
//! it counts only when the assistant showed the owner the exact text in that room's chat
//! (naming who it goes to), or asked Send it / Cancel with it, and the owner's very next
//! message in that room is a clear go ([`super::go::is_go`]). Their own message, written within
//! [`WINDOW`] of the text and used within [`WINDOW`] of it; one message per OK.
//!
//! And the owner's yes in the chat about a task: the assistant asked about the task (a question
//! naming its key) and their very next message, or their reply to that question, is a clear
//! yes; or their own yes names the task. It covers the task's job for [`WINDOW`], unless a later
//! message of theirs in that room takes it back.

use jiff::{SignedDuration, Timestamp};
use unicode_normalization::UnicodeNormalization as _;

use super::go;

/// How long an OK in the chat counts: from the text to the go, and from the go to its use.
pub const WINDOW: SignedDuration = SignedDuration::from_mins(30);

/// One line of a room's chat, as these rules read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatLine {
    /// Its id (ids grow with time).
    pub id: i64,
    /// When it was said.
    pub at: Timestamp,
    /// What it says.
    pub text: String,
    /// The owner said it (not the assistant).
    pub mine: bool,
    /// The line it replies to.
    pub reply_to: Option<i64>,
}

/// An OK the owner gave in the chat: their message, and the room it is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatOkMsg {
    /// The owner's message.
    pub id: i64,
    /// When they wrote it.
    pub at: Timestamp,
    /// What they wrote.
    pub said: String,
}

/// For finding a text in the assistant's message: curly quotes, a quote's `>` and line breaks
/// don't matter, every word does.
#[must_use]
pub fn flat(text: &str) -> String {
    let s: String = text
        .nfc()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201c}' | '\u{201d}' => '"',
            '\u{a0}' => ' ',
            other => other,
        })
        .collect();
    // A quote's ">" at the start of each line goes (with the blanks around it).
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut line_start = true;
    while i < chars.len() {
        if line_start {
            let mut j = i;
            while j < chars.len() && matches!(chars[j], ' ' | '\t') {
                j += 1;
            }
            if chars.get(j) == Some(&'>') {
                i = j + 1;
                if matches!(chars.get(i), Some(' ' | '\t')) {
                    i += 1;
                }
            }
            line_start = false;
            continue;
        }
        let c = chars[i];
        out.push(c);
        line_start = matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}');
        i += 1;
    }
    collapse(&out)
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Who it goes to is named where the owner saw it (a word of the name, 3 letters or more):
/// never an OK for another person.
#[must_use]
pub fn named(to: &str, place: &str) -> bool {
    let place = go::loose(place);
    go::loose_words(&go::loose(to))
        .iter()
        .filter(|w| w.chars().count() >= 3)
        .any(|w| place.contains(w.as_str()))
}

/// The OK for `text` to `to` in a room whose lines (since twice [`WINDOW`] before `now`, oldest
/// first) are `lines`: the assistant's newest message there with the exact text in it, then
/// the owner's very next message there; or, when the card for this text was asked at `asked`,
/// the owner's next message after that. `to_self`: the owner's own chat (no name needed).
/// `spent`: the messages already used for an OK.
#[must_use]
pub fn find(
    lines: &[ChatLine],
    text: &str,
    to: &str,
    to_self: bool,
    asked: Option<Timestamp>,
    spent: &[i64],
    now: Timestamp,
) -> Option<ChatOkMsg> {
    let since = now - WINDOW - WINDOW;
    let wanted = flat(text);
    let ok = |m: &ChatLine, from: Timestamp| {
        (!spent.contains(&m.id)
            && go::is_go(&m.text)
            && m.at.duration_since(from) <= WINDOW
            && now.duration_since(m.at) <= WINDOW)
            .then(|| ChatOkMsg {
                id: m.id,
                at: m.at,
                said: m.text.clone(),
            })
    };
    let shown = lines
        .iter()
        .rev()
        .find(|m| !m.mine && m.at > since && flat(&m.text).contains(&wanted));
    if let Some(shown) = shown
        && let Some(next) = lines.iter().find(|m| m.mine && m.id > shown.id)
        && let Some(found) = ok(next, shown.at)
        && (to_self || named(to, &format!("{}\n{}", shown.text, next.text)))
    {
        return Some(found);
    }
    let asked = asked.filter(|asked| *asked > since)?;
    let next = lines.iter().find(|m| m.mine && m.at > asked)?;
    ok(next, asked)
}

/// Whether `text` asks something.
fn question(text: &str) -> bool {
    text.contains('?') || text.contains('؟')
}

/// Whether `text` names the task whose key is `key` (a task key, as a whole word).
#[must_use]
pub fn names_key(text: &str, key: &str) -> bool {
    text.match_indices(key).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + key.len()..].chars().next();
        before.is_none_or(|c| !c.is_alphanumeric() && c != '_')
            && after.is_none_or(|c| !c.is_alphanumeric() && c != '_')
    })
}

/// The owner's yes in a room about the task keyed `key`: in `lines` (the room's since twice
/// [`WINDOW`] before `now`, oldest first), their newest yes within [`WINDOW`] that names the
/// task, replies to the assistant's question about it, or is their very next message after
/// that question (within [`WINDOW`] of it); none once a later message of theirs stops it.
#[must_use]
pub fn task_ok(lines: &[ChatLine], key: &str, now: Timestamp) -> Option<ChatOkMsg> {
    let since = now - WINDOW;
    let asks = |m: &ChatLine| names_key(&m.text, key) && question(&m.text);
    let theirs: Vec<&ChatLine> = lines
        .iter()
        .filter(|m| m.mine && m.at > since)
        .rev()
        .collect();
    for (i, m) in theirs.iter().enumerate() {
        if theirs[..i].iter().any(|later| go::is_stop(&later.text)) {
            // They took it back since.
            break;
        }
        if !go::is_yes(&m.text) {
            continue;
        }
        let within = |q: &ChatLine| m.at.duration_since(q.at) <= WINDOW;
        let quoted = m
            .reply_to
            .and_then(|id| lines.iter().find(|l| l.id == id && !l.mine));
        let before = lines.iter().rev().find(|l| !l.mine && l.id < m.id);
        let next = before.is_some_and(|before| {
            !lines
                .iter()
                .any(|l| l.mine && l.id > before.id && l.id < m.id)
        });
        if names_key(&m.text, key)
            || quoted.is_some_and(|q| asks(q) && within(q))
            || (next && before.is_some_and(|b| asks(b) && within(b)))
        {
            return Some(ChatOkMsg {
                id: m.id,
                at: m.at,
                said: m.text.clone(),
            });
        }
    }
    None
}

/// The task's line on which message of the owner's gave the OK: `at` is its time of day.
#[must_use]
pub fn ok_line(ok: &ChatOkMsg, at: &str, room: &str) -> String {
    let said = collapse(&ok.said);
    let said = if said.chars().count() > 200 {
        format!("{}…", said.chars().take(200).collect::<String>())
    } else {
        said
    };
    format!(
        "OK from the owner in the chat: their message #{} at {at} in {room}: «{said}»",
        ok.id
    )
}
