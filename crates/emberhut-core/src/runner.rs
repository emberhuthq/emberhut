//! Runner sessions' text: what a session was asked (without the server's own asks), its answer's
//! status line, and the clipped lines the server keeps of them.

/// The longest ask kept of a turn, in characters.
pub const ASK_LENGTH: usize = 600;

/// The longest answer kept of a turn, in characters.
pub const LAST_LENGTH: usize = 1500;

/// The longest question kept, in characters.
pub const QUESTION_LENGTH: usize = 600;

/// The start of the line the server appends to a turn it asks for a status line.
pub const STATUS_ASK_START: &str = "End every reply with one last line";

/// `text` on one line (every run of white space one space), trimmed, cut to `n` characters.
#[must_use]
pub fn clip(text: &str, n: usize) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(n)
        .collect()
}

/// What a turn's prompt asked, without what the server added: its status-line ask and
/// everything after it, and a leading `(From ...)` label, cut to [`ASK_LENGTH`].
#[must_use]
pub fn asked(prompt: &str) -> String {
    let lower = prompt.to_lowercase();
    let mut text = match lower.find(&STATUS_ASK_START.to_lowercase()) {
        // Lowercasing can move byte offsets for some scripts: cut only where it is a boundary.
        Some(at) if prompt.is_char_boundary(at) => prompt[..at].trim_end(),
        _ => prompt,
    };
    if let Some(rest) = text.strip_prefix("(From ")
        && let Some(close) = rest.find(')')
    {
        text = rest[close + 1..].trim_start();
    }
    clip(text, ASK_LENGTH)
}

/// The status line an answer ends with: its last `STATUS: done`, `working`, `blocked` or
/// `needs <someone>`, with what follows it on its line.
#[must_use]
pub fn status_line(answer: &str) -> Option<String> {
    let lower = answer.to_ascii_lowercase();
    let mut found = None;
    let mut from = 0;
    while let Some(at) = lower[from..].find("status:") {
        let start = from + at;
        from = start + "status:".len();
        let rest = &lower[from..];
        let skipped = rest.len() - rest.trim_start().len();
        let word_at = from + skipped;
        let word_rest = &lower[word_at..];
        let word_len = ["done", "working", "blocked"]
            .iter()
            .find(|word| starts_word(word_rest, word))
            .map(|word| word.len())
            .or_else(|| needs_len(word_rest));
        if let Some(len) = word_len {
            let line_end = answer[word_at + len..]
                .find('\n')
                .map_or(answer.len(), |end| word_at + len + end);
            found = Some(answer[start..line_end].trim().to_owned());
        }
    }
    found
}

/// Whether `text` starts with `word` and a word boundary after it.
fn starts_word(text: &str, word: &str) -> bool {
    text.starts_with(word)
        && !text[word.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
}

/// `needs <word>`'s length at the start of `text`.
fn needs_len(text: &str) -> Option<usize> {
    let rest = text.strip_prefix("needs")?;
    let spaces = rest.len() - rest.trim_start_matches([' ', '\t']).len();
    if spaces == 0 {
        return None;
    }
    let word = rest[spaces..]
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .map(char::len_utf8)
        .sum::<usize>();
    (word > 0).then_some("needs".len() + spaces + word)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ask_drops_what_the_server_added() {
        assert_eq!(
            asked(
                "(From the assistant) Fix the build\n\nEnd every reply with one last line: STATUS"
            ),
            "Fix the build"
        );
        assert_eq!(asked("  plain\n ask "), "plain ask");
        assert_eq!(clip("a  b\nc", 3), "a b");
    }

    #[test]
    fn the_status_line_is_the_last_one() {
        let answer = "Did it.\nSTATUS: working: tests\nMore\n**STATUS: done** all green\n";
        assert_eq!(
            status_line(answer).as_deref(),
            Some("STATUS: done** all green")
        );
        assert_eq!(
            status_line("status: needs the owner: a key").as_deref(),
            Some("status: needs the owner: a key")
        );
        assert_eq!(status_line("STATUS: doneish"), None);
        assert_eq!(status_line("no line"), None);
    }
}
