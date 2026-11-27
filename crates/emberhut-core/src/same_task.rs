//! Same-task memory across rooms: when a turn names a task (or a ticket) by its key, the
//! assistant is shown what was said about it in the last 48 hours in every room, so a room
//! doesn't contradict what was settled in another.
//!
//! A key is what the server's tasks and the trackers write: a word of capitals and digits
//! starting with a capital, a dash and a number (`TASK-12`, `ENG-42`). A turn's first
//! [`MAX_KEYS`] keys count.

use jiff::SignedDuration;

/// How far back the other rooms are read.
pub const WINDOW: SignedDuration = SignedDuration::from_hours(48);

/// The turn's own room's lines newer than this are left out: its session has them.
pub const HERE: SignedDuration = SignedDuration::from_hours(2);

/// Keys of one turn that are looked up, at most.
pub const MAX_KEYS: usize = 3;

/// Lines per key, at most (the newest).
pub const MAX_LINES: usize = 10;

/// Characters of the whole block, about.
pub const MAX_CHARS: usize = 6000;

/// The keys `text` names, in order, each once, [`MAX_KEYS`] at most.
#[must_use]
pub fn named_keys(text: &str) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut at = 0;
    while at < chars.len() && keys.len() < MAX_KEYS {
        let starts_word = at == 0 || !chars[at - 1].is_alphanumeric();
        if !(starts_word && chars[at].is_ascii_uppercase()) {
            at += 1;
            continue;
        }
        let mut end = at;
        while end < chars.len() && (chars[end].is_ascii_uppercase() || chars[end].is_ascii_digit())
        {
            end += 1;
        }
        let prefix = end - at;
        if prefix > 10 || chars.get(end) != Some(&'-') {
            at = end.max(at + 1);
            continue;
        }
        let mut digits = end + 1;
        while digits < chars.len() && chars[digits].is_ascii_digit() {
            digits += 1;
        }
        let ends_word = chars.get(digits).is_none_or(|c| !c.is_alphanumeric());
        if digits > end + 1 && ends_word {
            let key: String = chars[at..digits].iter().collect();
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        at = digits.max(at + 1);
    }
    keys
}

/// The block put after a turn's message: for each key, its lines.
#[must_use]
pub fn block(parts: &[(String, Vec<String>)]) -> String {
    let mut text = String::new();
    for (key, lines) in parts {
        if lines.is_empty() {
            continue;
        }
        let part = format!(
            "{key}:\n{}",
            lines
                .iter()
                .map(|line| format!("- {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        if !text.is_empty() && text.len() + part.len() > MAX_CHARS {
            break;
        }
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&part);
    }
    if text.is_empty() {
        return text;
    }
    format!(
        "\n\n(Same task, from all the rooms in the last 48 h: what was said there that this \
         room may not have in front of it. What the owner said or decided outranks any plan \
         made before it; don't contradict it or ask again what's settled.\n{text})"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_words_of_capitals_a_dash_and_a_number() {
        assert_eq!(
            named_keys("Move TASK-12 after ENG-42, not task-3 or XTASK-12a; TASK-12 again"),
            ["TASK-12", "ENG-42"]
        );
        assert_eq!(named_keys("A1-2 B-3 C-4 D-5"), ["A1-2", "B-3", "C-4"]);
        assert_eq!(
            named_keys("COVID-19x, ABCDEFGHIJK-1, -12, X-"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_block_names_each_key_and_says_what_outranks_what() {
        assert_eq!(block(&[]), "");
        assert_eq!(block(&[("A-1".to_owned(), Vec::new())]), "");
        let text = block(&[("A-1".to_owned(), vec!["[Atlas] Owner: hold it".to_owned()])]);
        assert!(text.starts_with("\n\n(Same task"), "{text}");
        assert!(text.ends_with("A-1:\n- [Atlas] Owner: hold it)"), "{text}");
    }
}
