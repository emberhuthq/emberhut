//! A question the assistant asks, split into what it asks and the draft that comes with it: a
//! message for someone else, to be sent word for word. The draft is the decision's own field
//! when it has one; else it is cut out of the question, after a "Draft reply to …:" line or at
//! a quote that opens a line, to the last closing quote.

use emberhut_api::tasks::Decision;

/// The least a quote is long to count as a draft, in characters: a short "quoted phrase" in
/// the question isn't one.
const SHORTEST_DRAFT: usize = 40;

/// The most characters on either side of "draft" in a "Draft reply to …:" line.
const LABEL_SIDE: usize = 60;

/// A question read for its draft.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AskText {
    /// What it asks, without the draft.
    pub(crate) summary: String,
    /// The line that named the draft ("Draft reply to the bakery"), without its colon.
    pub(crate) label: Option<String>,
    /// The draft, if it has one.
    pub(crate) draft: Option<String>,
    /// What the question says after the draft.
    pub(crate) after: String,
    /// The draft came in its own field: exact, plain text, copied as it is.
    pub(crate) plain: bool,
}

impl AskText {
    /// The decision's question and draft.
    #[must_use]
    pub(crate) fn of(decision: &Decision) -> Self {
        match decision.draft.as_deref().filter(|draft| !draft.is_empty()) {
            Some(draft) => Self {
                summary: decision.question.clone(),
                label: None,
                draft: Some(draft.to_owned()),
                after: String::new(),
                plain: true,
            },
            None => Self::read(&decision.question),
        }
    }

    /// A question with any draft cut out of it.
    #[must_use]
    pub(crate) fn read(question: &str) -> Self {
        let whole = || Self {
            summary: question.to_owned(),
            label: None,
            draft: None,
            after: String::new(),
            plain: false,
        };
        let label = draft_label(question);
        let from = label.map(|(_, end)| end).or_else(|| quote_line(question));
        let Some(from) = from else { return whole() };
        let Some(open) = question[from..].find(['"', '“']).map(|at| from + at) else {
            return whole();
        };
        let open_len = question[open..].chars().next().map_or(1, char::len_utf8);
        let inside = open + open_len;
        let Some(close) = question[inside..].rfind(['"', '”']).map(|at| inside + at) else {
            return whole();
        };
        if question[open..close].chars().count() <= SHORTEST_DRAFT {
            return whole();
        }
        let cut = label.map_or(from, |(start, _)| start);
        let close_len = question[close..].chars().next().map_or(1, char::len_utf8);
        Self {
            summary: question[..cut].trim().to_owned(),
            label: label.map(|(start, end)| {
                question[start..end]
                    .trim_end_matches([' ', '\t'])
                    .trim_end_matches(':')
                    .to_owned()
            }),
            draft: Some(question[inside..close].to_owned()),
            after: question[close + close_len..].trim().to_owned(),
            plain: false,
        }
    }
}

/// A line of its own like "Draft reply to the bakery:": where it starts and where it ends.
fn draft_label(question: &str) -> Option<(usize, usize)> {
    let mut start = 0;
    for line in question.split('\n') {
        let end = start + line.len();
        let body = line.trim_end_matches([' ', '\t']);
        if let Some(body) = body.strip_suffix(':')
            && names_a_draft(body)
        {
            return Some((start, end));
        }
        start = end + 1;
    }
    None
}

/// Whether `body` says "draft" (or "Draft") with at most [`LABEL_SIDE`] characters on either
/// side of it.
fn names_a_draft(body: &str) -> bool {
    body.match_indices("raft").any(|(at, _)| {
        // The character before may be more than one byte (`éraft`): asked as text, not cut.
        body[..at].ends_with(['d', 'D'])
            && body[..at - 1].chars().count() <= LABEL_SIDE
            && body[at + 4..].chars().count() <= LABEL_SIDE
    })
}

/// The start of the first line that opens with a quote (after blanks).
fn quote_line(question: &str) -> Option<usize> {
    let mut start = 0;
    for line in question.split('\n') {
        if line.trim_start_matches([' ', '\t']).starts_with(['"', '“']) {
            return Some(start);
        }
        start += line.len() + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRAFT: &str = "Hi, the bakery ads are ready and go out on Sunday morning.";

    #[test]
    fn a_draft_after_its_label_is_cut_out() {
        let question = format!(
            "Send this to the bakery?\nDraft reply to the bakery:\n“{DRAFT}”\nThen I close it."
        );
        let text = AskText::read(&question);
        assert_eq!(text.summary, "Send this to the bakery?");
        assert_eq!(text.label.as_deref(), Some("Draft reply to the bakery"));
        assert_eq!(text.draft.as_deref(), Some(DRAFT));
        assert_eq!(text.after, "Then I close it.");
        assert!(!text.plain);
    }

    #[test]
    fn a_wide_character_before_raft_is_no_draft_and_no_panic() {
        for body in ["éraft", "Ébauche éraft", "مسودةraft", "日raft"] {
            assert!(!names_a_draft(body), "{body}");
        }
        assert!(
            names_a_draft("Le draft"),
            "a draft after a wide character still reads"
        );
        assert_eq!(draft_label("éraft:\nHi there"), None);
    }

    #[test]
    fn a_quote_opening_a_line_is_a_draft() {
        let question = format!("Shall I send it?\n\"{DRAFT}\"");
        let text = AskText::read(&question);
        assert_eq!(text.summary, "Shall I send it?");
        assert_eq!(text.draft.as_deref(), Some(DRAFT));
    }

    #[test]
    fn a_short_quoted_phrase_is_not_a_draft() {
        for question in [
            "Should the button say \"Buy now\" or not?",
            "Pick one:\n\"Yes\" or \"no\"",
            "No quotes at all",
        ] {
            let text = AskText::read(question);
            assert_eq!(text.summary, question);
            assert_eq!(text.draft, None, "{question}");
        }
    }

    #[test]
    fn a_draft_of_its_own_is_exact() {
        let decision = Decision {
            question: "Send it?".into(),
            draft: Some("**As is**".into()),
            ..Decision::default()
        };
        assert_eq!(
            AskText::of(&decision),
            AskText {
                summary: "Send it?".into(),
                label: None,
                draft: Some("**As is**".into()),
                after: String::new(),
                plain: true
            }
        );
    }
}
