//! A message's text cut into what the chat draws differently: quotes (drafts with their own
//! Copy), tables, pictures on a line of their own, and the markdown around them. Long
//! paragraphs are broken every two sentences on the way, so a wall of text reads easily. Only
//! the first [`MOST_TEXT`] bytes of a text are read so ([`read_part`]).

/// The most of a message's text read into blocks, in bytes: the rest shows only as plain text,
/// when the user asks for all of it.
pub(crate) const MOST_TEXT: usize = 32 * 1024;

/// One part of a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Block {
    /// Markdown for the markdown view: paragraphs, lists, headings, code.
    Markdown(String),
    /// A run of `> ` lines: a draft the user may send as their own (markdown, without the `>`).
    Quote(String),
    /// A table: rows of cells, the first the header.
    Table(Vec<Vec<String>>),
    /// A picture alone on its line: `![alt](url)`.
    Image {
        /// Where it is: a path on the server or a web address.
        url: String,
        /// What it shows.
        alt: String,
    },
}

/// The blocks of `text`; `split` breaks each paragraph after every two sentences.
#[must_use]
pub(crate) fn blocks(text: &str, split: bool) -> Vec<Block> {
    let mut parser = Parser {
        split,
        ..Parser::default()
    };
    for raw in text.split('\n') {
        parser.line(raw);
    }
    parser.finish()
}

/// The part of `text` read into blocks: all of it, or its first [`MOST_TEXT`] bytes, to the last
/// line break among them when there is one.
#[must_use]
pub(crate) fn read_part(text: &str) -> &str {
    if text.len() <= MOST_TEXT {
        return text;
    }
    let head = &text[..text.floor_char_boundary(MOST_TEXT)];
    head.rfind('\n').map_or(head, |line_end| &head[..line_end])
}

/// The addresses of the pictures alone on a line of the part of `text` read
/// ([`read_part`]), outside code, in order.
pub(crate) fn pictures(text: &str) -> impl Iterator<Item = String> {
    blocks(read_part(text), false)
        .into_iter()
        .filter_map(|block| match block {
            Block::Image { url, .. } => Some(url),
            Block::Markdown(_) | Block::Quote(_) | Block::Table(_) => None,
        })
}

#[derive(Default)]
struct Parser {
    split: bool,
    out: Vec<Block>,
    /// Markdown since the last block of another kind.
    markdown: Vec<String>,
    /// The lines of the paragraph being read.
    para: Vec<String>,
    table: Vec<Vec<String>>,
    quote: Vec<String>,
    /// Inside a fenced code block.
    code: bool,
}

impl Parser {
    fn line(&mut self, raw: &str) {
        let line = raw.trim();
        if !self.code
            && let Some(quoted) = line.strip_prefix('>')
        {
            self.flush_para();
            self.flush_table();
            self.quote
                .push(quoted.strip_prefix(' ').unwrap_or(quoted).to_owned());
            return;
        }
        self.flush_quote();
        if line.starts_with("```") || line.starts_with("~~~") {
            self.flush_para();
            self.flush_table();
            self.code = !self.code;
            self.markdown.push(raw.to_owned());
            return;
        }
        if self.code {
            self.markdown.push(raw.to_owned());
            return;
        }
        if let Some(row) = cells(line) {
            self.flush_para();
            if !row.is_empty() {
                self.table.push(row);
            }
            return;
        }
        self.flush_table();
        if line.is_empty() {
            self.flush_para();
            self.markdown.push(String::new());
        } else if let Some((url, alt)) = picture(line) {
            self.flush_para();
            self.flush_markdown();
            self.out.push(Block::Image { url, alt });
        } else if is_heading(line) || is_rule(line) || is_item(line) {
            self.flush_para();
            self.markdown.push(raw.trim_end().to_owned());
        } else {
            self.para.push(line.to_owned());
        }
    }

    fn finish(mut self) -> Vec<Block> {
        self.flush_para();
        self.flush_table();
        self.flush_quote();
        self.flush_markdown();
        self.out
    }

    /// The paragraph read so far, into the markdown: whole, or two sentences at a time. A blank
    /// line goes before and after it, so it never runs on into a list item above it.
    fn flush_para(&mut self) {
        if self.para.is_empty() {
            return;
        }
        let joined = self.para.join(" ");
        self.para.clear();
        let parts = if self.split {
            pairs_of_sentences(&joined)
        } else {
            vec![joined]
        };
        for part in parts {
            if self.markdown.last().is_some_and(|last| !last.is_empty()) {
                self.markdown.push(String::new());
            }
            self.markdown.push(part);
            self.markdown.push(String::new());
        }
    }

    fn flush_table(&mut self) {
        if !self.table.is_empty() {
            self.flush_markdown();
            self.out.push(Block::Table(std::mem::take(&mut self.table)));
        }
    }

    fn flush_quote(&mut self) {
        if self.quote.is_empty() {
            return;
        }
        let quote = self.quote.join("\n").trim().to_owned();
        self.quote.clear();
        if !quote.is_empty() {
            self.flush_markdown();
            self.out.push(Block::Quote(quote));
        }
    }

    fn flush_markdown(&mut self) {
        let markdown = self.markdown.join("\n");
        self.markdown.clear();
        let markdown = markdown.trim_matches('\n');
        if !markdown.trim().is_empty() {
            self.out.push(Block::Markdown(markdown.to_owned()));
        }
    }
}

/// A table row's cells: `| a | b |` reads `["a", "b"]`. `\|` is a bar inside a cell and `<br>` a
/// new line in it. The `|---|:--:|` row under the header reads as no cells; a line that doesn't
/// start with `|` isn't a row.
#[must_use]
pub(crate) fn cells(line: &str) -> Option<Vec<String>> {
    let mut body = line.trim().strip_prefix('|')?;
    if body.ends_with('|') && !body.ends_with("\\|") {
        body = &body[..body.len() - 1];
    }
    let cells: Vec<String> = body
        .replace("\\|", "\u{2}")
        .split('|')
        .map(|cell| line_breaks(&cell.replace('\u{2}', "|")).trim().to_owned())
        .collect();
    let divider = cells
        .iter()
        .all(|cell| !cell.is_empty() && cell.chars().all(|c| matches!(c, '-' | ':' | ' ')));
    Some(if divider { Vec::new() } else { cells })
}

/// `<br>`, `<br/>` and `<br />` as new lines.
fn line_breaks(cell: &str) -> String {
    let mut out = String::with_capacity(cell.len());
    let mut rest = cell;
    while let Some(at) = rest.find("<br") {
        let tail = &rest[at + 3..];
        let close = tail.trim_start_matches(' ');
        let close = close.strip_prefix('/').unwrap_or(close);
        if let Some(after) = close.strip_prefix('>') {
            out.push_str(&rest[..at]);
            out.push('\n');
            rest = after;
        } else {
            out.push_str(&rest[..at + 3]);
            rest = tail;
        }
    }
    out.push_str(rest);
    out
}

/// A line that is only a picture, `![alt](url)` with no space in the address: its address and
/// alt text.
fn picture(line: &str) -> Option<(String, String)> {
    let inner = line.strip_prefix("![")?.strip_suffix(')')?;
    let (alt, url) = inner.split_once("](")?;
    (!alt.contains(']')
        && !url.is_empty()
        && !url.contains(char::is_whitespace)
        && !url.contains(')'))
    .then(|| (url.to_owned(), alt.to_owned()))
}

/// `# Title` to `###### Title`.
pub(crate) fn is_heading(line: &str) -> bool {
    let hashes = line.len() - line.trim_start_matches('#').len();
    (1..=6).contains(&hashes) && line[hashes..].starts_with([' ', '\t'])
}

/// `---`, `***` or `___`.
fn is_rule(line: &str) -> bool {
    matches!(line, "---" | "***" | "___")
}

/// A list item: `- `, `* `, `• `, `1. ` or `1) `.
pub(crate) fn is_item(line: &str) -> bool {
    if let Some(rest) = line.strip_prefix(['-', '*', '•']) {
        return rest.starts_with([' ', '\t']);
    }
    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    digits > 0
        && line[digits..]
            .strip_prefix(['.', ')'])
            .is_some_and(|rest| rest.starts_with([' ', '\t']))
}

/// A paragraph two sentences at a time. A sentence ends with `.`, `!` or `?` before a space and a
/// capital letter, a quote, or a bracket.
#[must_use]
pub(crate) fn pairs_of_sentences(paragraph: &str) -> Vec<String> {
    let mut sentences: Vec<&str> = Vec::new();
    let mut start = 0;
    let mut chars = paragraph.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if !matches!(c, '.' | '!' | '?') {
            continue;
        }
        let end = at + c.len_utf8();
        let rest = &paragraph[end..];
        let next = rest.trim_start();
        if next.len() == rest.len() {
            continue;
        }
        if next
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_uppercase() || matches!(c, '"' | '“' | '(' | '['))
        {
            sentences.push(&paragraph[start..end]);
            start = paragraph.len() - next.len();
            while chars.peek().is_some_and(|&(i, _)| i < start) {
                chars.next();
            }
        }
    }
    sentences.push(&paragraph[start..]);
    sentences
        .chunks(2)
        .map(|pair| pair.join(" "))
        .filter(|pair| !pair.trim().is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(text: &str) -> Block {
        Block::Markdown(text.to_owned())
    }

    #[test]
    fn a_wall_of_text_is_broken_every_two_sentences() {
        assert_eq!(
            pairs_of_sentences("One. Two! Three? \"Four\" said. five. Six"),
            ["One. Two!", "Three? \"Four\" said. five.", "Six"]
        );
        assert_eq!(
            blocks("First. Second. Third.\nStill third's paragraph.", true),
            [md("First. Second.\n\nThird. Still third's paragraph.")]
        );
        assert_eq!(
            blocks("First. Second. Third.", false),
            [md("First. Second. Third.")]
        );
    }

    #[test]
    fn quotes_stand_alone_as_drafts() {
        let text =
            "Two drafts:\n\n> Hey, the **ads** are ready.\n>\n> Sending soon.\n\nOr:\n\n> Done 👍";
        assert_eq!(
            blocks(text, true),
            [
                md("Two drafts:"),
                Block::Quote("Hey, the **ads** are ready.\n\nSending soon.".into()),
                md("Or:"),
                Block::Quote("Done 👍".into()),
            ]
        );
    }

    #[test]
    fn tables_and_pictures_are_their_own_blocks() {
        let text = "Compare:\n\n| | **A** | B |\n|---|:--:|---|\n| Price | 5\\|6 | 7<br>8 |\n\n![chart](/uploads/c.png)\nAfter.";
        assert_eq!(
            blocks(text, false),
            [
                md("Compare:"),
                Block::Table(vec![
                    vec![String::new(), "**A**".into(), "B".into()],
                    vec!["Price".into(), "5|6".into(), "7\n8".into()],
                ]),
                Block::Image {
                    url: "/uploads/c.png".into(),
                    alt: "chart".into()
                },
                md("After."),
            ]
        );
    }

    #[test]
    fn code_lists_and_headings_stay_markdown() {
        let text = "# Plan\n- one. Two.\n- three\n```\n> not a quote\n| not | a table |\n```\nText. More. End.";
        assert_eq!(
            blocks(text, true),
            [md(
                "# Plan\n- one. Two.\n- three\n```\n> not a quote\n| not | a table |\n```\n\nText. More.\n\nEnd."
            )]
        );
    }

    #[test]
    fn a_paragraph_after_a_list_doesnt_run_into_it() {
        assert_eq!(
            blocks("- item\nA paragraph.", false),
            [md("- item\n\nA paragraph.")]
        );
    }

    #[test]
    fn rows_read_their_cells() {
        assert_eq!(cells("| a | b |"), Some(vec!["a".into(), "b".into()]));
        assert_eq!(cells("|---|:--:|"), Some(vec![]));
        assert_eq!(cells("a | b"), None);
        assert_eq!(
            cells("| x<br/>y | z<br />w |"),
            Some(vec!["x\ny".into(), "z\nw".into()])
        );
        assert_eq!(cells("| <bring> |"), Some(vec!["<bring>".into()]));
    }

    #[test]
    fn the_pictures_of_a_text_are_found() {
        let text =
            "![a](/uploads/a.png)\n```\n![b](/uploads/b.png)\n```\n![c](https://example.com/c.png)";
        assert_eq!(
            pictures(text).collect::<Vec<_>>(),
            ["/uploads/a.png", "https://example.com/c.png"]
        );
    }

    #[test]
    fn only_the_start_of_a_long_text_is_read() {
        let short = "a line\n".repeat(10);
        assert_eq!(read_part(&short), short);
        let long = format!("{}\n{}", "é".repeat(MOST_TEXT / 4), "x".repeat(MOST_TEXT));
        assert_eq!(
            read_part(&long),
            "é".repeat(MOST_TEXT / 4),
            "to the line's end"
        );
        let one_line = "é".repeat(MOST_TEXT);
        let read = read_part(&one_line);
        assert!(
            read.len() <= MOST_TEXT && read.len() > MOST_TEXT - 2,
            "{}",
            read.len()
        );
    }

    #[test]
    fn an_empty_text_has_no_blocks() {
        assert_eq!(blocks("", true), []);
        assert_eq!(blocks("\n\n", true), []);
    }
}
