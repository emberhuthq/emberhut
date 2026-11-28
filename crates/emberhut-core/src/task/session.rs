//! Tasks made for runner sessions: every session works for a task, so a session started
//! without one (the person's own, or one the assistant started with no task named) gets a task
//! of its own, titled from what it was first asked.
//!
//! This module holds the rules: the title a task gets from a session's first message
//! ([`title_from`], [`session_title`]), the history lines the server writes on such tasks and
//! on a session's move to another task, and which history lines are the server's own
//! bookkeeping rather than anyone's note ([`is_bookkeeping`]), which decides whether a task
//! made for a session that moved away is empty enough to be put away.

use super::TaskId;
use super::plan::UNPLANNED_TURN_LINE;

/// The most words a title made from a message keeps.
pub const TITLE_WORDS: usize = 6;

/// The longest a title made from a message may be, in characters.
pub const TITLE_MAX: usize = 60;

/// The history line of a task made for one of the person's own sessions.
#[must_use]
pub fn opened_from(place: &str) -> String {
    format!("{OPENED_FROM}{place}")
}

/// How [`opened_from`]'s line starts.
const OPENED_FROM: &str = "Opened from a session on the ";

/// The history line of a task made for a session the assistant started without naming one.
pub const MADE_FOR_SESSION: &str = "Made for a session started without a task";

/// The history line of a task made for a session that could not be started, and so was put
/// away at once.
#[must_use]
pub fn not_started(why: &str) -> String {
    format!("{NOT_STARTED}{why}")
}

/// How [`not_started`]'s line starts.
const NOT_STARTED: &str = "Its session could not be started: ";

/// The history line of the task a session moved to: `session` is how the session is named
/// ([`session_label`]), `from` the tasks it moved from.
#[must_use]
pub fn moved_here(session: &str, from: &[TaskId]) -> String {
    if from.is_empty() {
        format!("{SESSION}{session}{MOVED_HERE}")
    } else {
        let keys: Vec<String> = from.iter().map(|task| task.key()).collect();
        format!("{SESSION}{session}{MOVED_HERE} from {}", keys.join(", "))
    }
}

/// The history line of a task a session moved away from, to `to`.
#[must_use]
pub fn moved_to(session: &str, to: TaskId) -> String {
    format!("{SESSION}{session}{MOVED_TO}{}", to.key())
}

/// The history line of a task made for a session, put away when its session moved to `to` and
/// it had nothing of its own.
#[must_use]
pub fn merged_into(to: TaskId) -> String {
    format!("Merged into {}", to.key())
}

/// How a session is named in a history line: its id, and its title in quotes when it has one.
#[must_use]
pub fn session_label(id: &str, title: &str) -> String {
    let title = title.trim();
    if title.is_empty() {
        id.to_owned()
    } else {
        format!("{id} \"{title}\"")
    }
}

const SESSION: &str = "Session ";
const MOVED_HERE: &str = " moved here";
const MOVED_TO: &str = " moved to ";

/// Whether a history line is the server's own bookkeeping of a task made for a session (its
/// creation, the line saying why it was made, a turn that ended with no plan, a session moving
/// in or out), not a note of anyone's: a task made for a session whose history holds nothing
/// else has nothing of its own.
#[must_use]
pub fn is_bookkeeping(line: &str) -> bool {
    matches!(line, "Created" | "Created by the assistant")
        || line == MADE_FOR_SESSION
        || line == UNPLANNED_TURN_LINE
        || line.starts_with(OPENED_FROM)
        || (line.starts_with(SESSION) && (line.contains(MOVED_TO) || line.contains(MOVED_HERE)))
}

/// The title of a task made for a session on `place` whose first message is `message`: the
/// message's [`title_from`], else `Session on the <place>`.
#[must_use]
pub fn session_title(message: &str, place: &str) -> String {
    title_from(message).unwrap_or_else(|| format!("Session on the {place}"))
}

/// A short title made from a message: its first line with words in it, without its markdown
/// (headings, quotes, list markers, emphasis and code marks; a link reads as its text, an image
/// as its alt text, a bare address not at all), its first [`TITLE_WORDS`] words and at most
/// [`TITLE_MAX`] characters (cut between words), its first letter a capital and no trailing
/// comma, colon or full stop. `None` when no line has words.
#[must_use]
pub fn title_from(message: &str) -> Option<String> {
    let mut in_code = false;
    for line in message.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        let words = plain_words(&inline_text(strip_block_marks(trimmed)));
        if !words.is_empty() {
            return Some(shorten(&words));
        }
    }
    None
}

/// `line` without the marks markdown puts at a block's start: headings, quotes, list markers
/// and task boxes.
fn strip_block_marks(mut line: &str) -> &str {
    loop {
        let before = line;
        line = line.trim_start();
        line = line.trim_start_matches('#').trim_start_matches('>');
        for marker in ["- ", "* ", "+ "] {
            if let Some(rest) = line.strip_prefix(marker) {
                line = rest;
            }
        }
        for task_box in ["[ ] ", "[x] ", "[X] "] {
            if let Some(rest) = line.strip_prefix(task_box) {
                line = rest;
            }
        }
        let digits = line.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0
            && let Some(rest) = line[digits..]
                .strip_prefix(". ")
                .or_else(|| line[digits..].strip_prefix(") "))
        {
            line = rest;
        }
        if line == before {
            return line;
        }
    }
}

/// `line` with its links read as their text, its images as their alt text and its autolinks
/// (`<https://…>`) dropped.
fn inline_text(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find(['[', '<']) {
        let (before, from) = rest.split_at(at);
        let (mark, after) = from.split_at(1);
        if mark == "<" {
            out.push_str(before);
            match after.split_once('>') {
                Some((inside, tail)) if is_address(inside) => rest = tail,
                _ => {
                    out.push('<');
                    rest = after;
                }
            }
            continue;
        }
        // `[text](target)`; an image's `!` before it goes too.
        let parts = after
            .split_once("](")
            .and_then(|(text, target)| Some((text, target.split_once(')')?.1)));
        if let Some((text, tail)) = parts {
            out.push_str(before.strip_suffix('!').unwrap_or(before));
            out.push_str(text);
            rest = tail;
        } else {
            out.push_str(before);
            out.push('[');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// Whether `text` is a web or mail address.
fn is_address(text: &str) -> bool {
    ["http://", "https://", "mailto:", "www."]
        .iter()
        .any(|start| text.starts_with(start))
}

/// The words of `text`, each without the emphasis and code marks around it, bare addresses
/// left out.
fn plain_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter(|word| !is_address(word.trim_start_matches(['(', '*', '_', '`'])))
        .map(|word| {
            word.trim_matches(|c| matches!(c, '*' | '_' | '`' | '~'))
                .replace("**", "")
                .replace('`', "")
        })
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .collect()
}

/// `words` as a title: the first [`TITLE_WORDS`], cut between words to [`TITLE_MAX`]
/// characters (a first word longer than that, to its first [`TITLE_MAX`] characters), its
/// first letter a capital, a trailing comma, colon, semicolon, dash or full stop dropped.
fn shorten(words: &[String]) -> String {
    let mut title = String::new();
    for word in words.iter().take(TITLE_WORDS) {
        let length = title.chars().count() + usize::from(!title.is_empty()) + word.chars().count();
        if length > TITLE_MAX {
            if title.is_empty() {
                title = word.chars().take(TITLE_MAX).collect();
            }
            break;
        }
        if !title.is_empty() {
            title.push(' ');
        }
        title.push_str(word);
    }
    let title = title.trim_end_matches(['.', ',', ':', ';', '-', '–', '—']);
    let mut chars = title.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_is_the_first_line_s_first_words() {
        assert_eq!(
            title_from("fix the lexer so it reads raw strings and byte strings\nthen the parser")
                .as_deref(),
            Some("Fix the lexer so it reads")
        );
        assert_eq!(title_from("\n\n  Ship it.  \n").as_deref(), Some("Ship it"));
        assert_eq!(
            title_from("Why does CI fail?").as_deref(),
            Some("Why does CI fail?")
        );
    }

    #[test]
    fn a_title_has_no_markdown() {
        assert_eq!(
            title_from("## **Port** the `runner` to [Rust](https://example.com/rust)").as_deref(),
            Some("Port the runner to Rust")
        );
        assert_eq!(
            title_from("- [ ] look at <https://example.com/x> and ![the logo](logo.png)")
                .as_deref(),
            Some("Look at and the logo")
        );
        assert_eq!(
            title_from("> 1. _read_ https://example.com/a first").as_deref(),
            Some("Read first")
        );
        assert_eq!(
            title_from("```rust\nfn main() {}\n```\nWhy won't this build?").as_deref(),
            Some("Why won't this build?")
        );
        assert_eq!(
            title_from("Keep snake_case and a < b").as_deref(),
            Some("Keep snake_case and a b")
        );
    }

    #[test]
    fn a_title_is_cut_between_words_to_its_length() {
        let long = "Rename internationalization-configuration-loader module everywhere in it";
        let title = title_from(long).unwrap();
        assert!(title.chars().count() <= TITLE_MAX, "{title}");
        assert_eq!(
            title,
            "Rename internationalization-configuration-loader module"
        );
        let one = "a".repeat(90);
        assert_eq!(title_from(&one).unwrap().chars().count(), TITLE_MAX);
        assert_eq!(
            title_from("Überprüfe die Größe: jetzt, bitte, sofort, gleich").as_deref(),
            Some("Überprüfe die Größe: jetzt, bitte, sofort")
        );
    }

    #[test]
    fn a_message_with_no_words_falls_back_to_its_place() {
        for empty in [
            "",
            "   \n\t",
            "```\ncode only\n```",
            "https://example.com/a",
            "- **",
        ] {
            assert_eq!(title_from(empty), None, "{empty:?}");
        }
        assert_eq!(session_title("", "Mac"), "Session on the Mac");
        assert_eq!(session_title("tidy up", "Mac"), "Tidy up");
    }

    #[test]
    fn the_server_s_own_lines_are_told_from_notes() {
        let from = [TaskId::new(3)];
        let label = session_label("r-1a", "The lexer");
        assert_eq!(label, "r-1a \"The lexer\"");
        assert_eq!(session_label("r-1a", " "), "r-1a");
        assert_eq!(
            moved_here(&label, &from),
            "Session r-1a \"The lexer\" moved here from T-3"
        );
        assert_eq!(moved_here("r-1a", &[]), "Session r-1a moved here");
        assert_eq!(
            moved_to("r-1a", TaskId::new(5)),
            "Session r-1a moved to T-5"
        );
        assert_eq!(merged_into(TaskId::new(5)), "Merged into T-5");
        for line in [
            "Created".to_owned(),
            "Created by the assistant".to_owned(),
            opened_from("Mac"),
            MADE_FOR_SESSION.to_owned(),
            UNPLANNED_TURN_LINE.to_owned(),
            moved_here(&label, &from),
            moved_to(&label, TaskId::new(5)),
        ] {
            assert!(is_bookkeeping(&line), "{line}");
        }
        for line in [
            "Looked at it: the lexer is fine",
            "Plan set: 3 steps",
            "Session notes",
        ] {
            assert!(!is_bookkeeping(line), "{line}");
        }
        assert_eq!(
            not_started("offline"),
            "Its session could not be started: offline"
        );
    }
}
