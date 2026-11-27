//! A message's text as it goes on the clipboard and into the message menu: the whole message
//! ready to paste into a chat app, a draft as plain words, the web links in it, and the "Explain
//! simply" question about it.

use std::collections::{HashMap, VecDeque};

use super::links;
use super::scan::{Needle, Next};

/// How much of an answer "Explain simply" quotes.
const EXPLAIN_QUOTE: usize = 120;

/// The most web links the message menu offers to copy.
pub(crate) const MENU_LINKS: usize = 3;

/// A whole message ready to paste into a chat app: `*bold*`, links as `text (address)`, no
/// heading marks.
#[must_use]
pub(crate) fn for_pasting(markdown: &str) -> String {
    let text = links_as_words(markdown, true);
    let text = text.replace("**", "*");
    text.split('\n')
        .map(|line| {
            let hashes = line.len() - line.trim_start_matches('#').len();
            match &line[hashes..] {
                rest if (1..=6).contains(&hashes) && rest.starts_with([' ', '\t']) => {
                    rest.trim_start()
                }
                _ => line,
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A draft as a person would type it: its bold, italics and code marks gone where they come in
/// pairs (a lone `*`, as in `2 * 3`, stays), links as `text (address)`, line breaks kept.
/// Underscores stay as they are: in a chat's text they are names (`snake_case`, `__init__`)
/// far more often than emphasis.
#[must_use]
pub(crate) fn plain(markdown: &str) -> String {
    without_pairs(&links_as_words(markdown, true))
}

/// A quote's two lines: markdown and links as the words the user reads (`[x](y)` is `x`).
#[must_use]
pub(crate) fn preview(markdown: &str) -> String {
    links_as_words(markdown, false)
        .replace(['*', '`', '#', '>', '$'], "")
        .trim()
        .to_owned()
}

/// What "Explain simply" says about an answer: its first words, links as their text.
#[must_use]
pub(crate) fn explain_simply(answer: &str) -> String {
    let words = links_as_words(answer, false);
    let quote: String = words.chars().take(EXPLAIN_QUOTE).collect();
    format!("Explain this in simpler words, with a real example: “{quote}…”")
}

/// A markdown link or picture in a text: `[label](url)`, `![alt](url)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LinkSpan<'a> {
    /// Where it starts (its `[`, or the `!` before it).
    start: usize,
    /// Its label.
    label: &'a str,
    /// Its address.
    url: &'a str,
    /// Where it ends, past its `)`.
    end: usize,
}

/// The markdown links of `text`, in order, read in one pass: a label is not empty and holds no
/// `[`, an address holds no white space.
fn link_spans(text: &str) -> Vec<LinkSpan<'_>> {
    let mut spans = Vec::new();
    let mut opens = Next::new(text, Needle::Text("["));
    let mut closes = Next::new(text, Needle::Text("]"));
    let mut ends = Next::new(text, Needle::Text(")"));
    let mut spaces = Next::new(text, Needle::Space);
    let mut from = 0;
    while let Some(open) = opens.at_or_after(from) {
        from = open + 1;
        // No `]` or `)` left: no link can follow.
        let Some(close) = closes.at_or_after(open + 1) else {
            break;
        };
        if opens.at_or_after(open + 1).is_some_and(|next| next < close)
            || close == open + 1
            || text.as_bytes().get(close + 1) != Some(&b'(')
        {
            continue;
        }
        let Some(end) = ends.at_or_after(close + 2) else {
            break;
        };
        if spaces
            .at_or_after(close + 2)
            .is_some_and(|space| space < end)
        {
            continue;
        }
        let start = if text[..open].ends_with('!') {
            open - 1
        } else {
            open
        };
        spans.push(LinkSpan {
            start,
            label: &text[open + 1..close],
            url: &text[close + 2..end],
            end: end + 1,
        });
        from = end + 1;
    }
    spans
}

/// `[label](url)` as `label (url)` when `with_address` (just `label` when it is the address
/// already, its scheme aside), else as `label`.
fn links_as_words(markdown: &str, with_address: bool) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut copied = 0;
    for span in link_spans(markdown) {
        out.push_str(&markdown[copied..span.start]);
        out.push_str(span.label);
        if with_address && !names_itself(span.label, span.url) {
            out.push_str(" (");
            out.push_str(span.url.strip_prefix("mailto:").unwrap_or(span.url));
            out.push(')');
        }
        copied = span.end;
    }
    out.push_str(&markdown[copied..]);
    out
}

/// Whether a link's label is its address, written with or without its scheme (and a
/// closing `/`): `[example.com](https://example.com/)`, `[me@example.com](mailto:me@example.com)`.
fn names_itself(label: &str, url: &str) -> bool {
    let bare = |text: &str| -> String {
        let text = ["https://", "http://", "mailto:"]
            .iter()
            .find_map(|scheme| text.strip_prefix(scheme))
            .unwrap_or(text);
        text.strip_suffix('/').unwrap_or(text).to_lowercase()
    };
    bare(label) == bare(url)
}

/// A run of `*` or of backticks in a text.
#[derive(Clone, Copy, Debug)]
struct Run {
    start: usize,
    len: usize,
    mark: u8,
}

/// `text` without the emphasis and code marks that pair up: code spans (a run of backticks to
/// the next run as long), then `*` and `**` that open before a word and close after one. What
/// is inside a code span stays as it is.
fn without_pairs(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut runs: Vec<Run> = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let mark = bytes[at];
        if mark == b'`' || mark == b'*' {
            let len = bytes[at..].iter().take_while(|&&b| b == mark).count();
            runs.push(Run {
                start: at,
                len,
                mark,
            });
            at += len;
        } else {
            at += 1;
        }
    }
    // How many marks of each run stay.
    let mut kept: Vec<usize> = runs.iter().map(|run| run.len).collect();
    let mut verbatim = vec![false; runs.len()];
    let mut ticks: HashMap<usize, VecDeque<usize>> = HashMap::new();
    for (i, run) in runs.iter().enumerate() {
        if run.mark == b'`' {
            ticks.entry(run.len).or_default().push_back(i);
        }
    }
    let mut i = 0;
    while i < runs.len() {
        if runs[i].mark == b'`'
            && let Some(queue) = ticks.get_mut(&runs[i].len)
        {
            while queue.front().is_some_and(|&j| j <= i) {
                queue.pop_front();
            }
            if let Some(&close) = queue.front() {
                kept[i] = 0;
                kept[close] = 0;
                for inside in &mut verbatim[i + 1..close] {
                    *inside = true;
                }
                i = close + 1;
                continue;
            }
        }
        i += 1;
    }
    // Stars: each closing run takes from the nearest open one, two at a time when both have
    // two.
    let mut open: Vec<usize> = Vec::new();
    for (i, run) in runs.iter().enumerate() {
        if run.mark != b'*' || verbatim[i] {
            continue;
        }
        let before = text[..run.start].chars().next_back();
        let after = text[run.start + run.len..].chars().next();
        let opens = after.is_some_and(|c| !c.is_whitespace());
        let closes = before.is_some_and(|c| !c.is_whitespace());
        if closes {
            while kept[i] > 0
                && let Some(&opener) = open.last()
            {
                let taken = if kept[i] >= 2 && kept[opener] >= 2 {
                    2
                } else {
                    1
                };
                kept[i] -= taken;
                kept[opener] -= taken;
                if kept[opener] == 0 {
                    open.pop();
                }
            }
        }
        if opens && kept[i] > 0 {
            open.push(i);
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    for (run, &kept) in runs.iter().zip(&kept) {
        out.push_str(&text[copied..run.start]);
        for _ in 0..kept {
            out.push(char::from(run.mark));
        }
        copied = run.start + run.len;
    }
    out.push_str(&text[copied..]);
    out
}

/// A web link the message menu offers to copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WebLink {
    /// What is copied: the address (an email without `mailto:`).
    pub(crate) copied: String,
    /// The host it names the item by when there are several (`example.com`).
    pub(crate) host: String,
}

/// The web links in a message (`http`, `https`, `mailto`, and the bare addresses the chat links),
/// each once, in order. The app's own links (a task's) aren't web links.
#[must_use]
pub(crate) fn web_links(markdown: &str) -> Vec<WebLink> {
    let linked = links::linked(markdown);
    let mut found: Vec<String> = Vec::new();
    let mut copied = 0;
    for span in link_spans(&linked) {
        words_with_links(&linked[copied..span.start], &mut found);
        found.push(span.url.to_owned());
        copied = span.end;
    }
    words_with_links(&linked[copied..], &mut found);
    let mut links: Vec<WebLink> = Vec::new();
    for url in found {
        let Some(link) = web_link(&url) else { continue };
        if !links.contains(&link) {
            links.push(link);
        }
    }
    links
}

/// The addresses written out among `text`'s words (`https://…`, `www.…`, an email).
fn words_with_links(text: &str, found: &mut Vec<String>) {
    for word in text.split(|c: char| c.is_whitespace() || c == '[') {
        let word = word
            .trim_start_matches(['(', '<', '*', '_', '"'])
            .trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '>', '*', '_', '"']);
        if word.starts_with("https://") || word.starts_with("http://") {
            found.push(word.to_owned());
        } else if word.starts_with("www.") {
            found.push(format!("https://{word}"));
        } else if is_email(word) {
            found.push(format!("mailto:{word}"));
        }
    }
}

/// `url` as a web link, if it is one.
fn web_link(url: &str) -> Option<WebLink> {
    if let Some(address) = url.strip_prefix("mailto:") {
        let host = address.split('@').nth(1).unwrap_or(address);
        return Some(WebLink {
            copied: address.to_owned(),
            host: host.to_owned(),
        });
    }
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    (!host.is_empty()).then(|| WebLink {
        copied: url.to_owned(),
        host: host.to_owned(),
    })
}

/// `name@example.com`.
fn is_email(word: &str) -> bool {
    word.split_once('@').is_some_and(|(name, host)| {
        !name.is_empty() && host.contains('.') && !host.starts_with('.') && !host.ends_with('.')
    })
}

/// The message menu's item for `link`: Copy Link when it is the only one, else named by its
/// host.
#[must_use]
pub(crate) fn link_item(link: &WebLink, count: usize) -> String {
    if count == 1 {
        "Copy Link".to_owned()
    } else {
        format!("Copy {}", link.host)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::TASK_KEY_PREFIX;

    #[test]
    fn a_message_pastes_as_a_chat_app_reads_it() {
        let text = "# Plan\nThe **ads** are [ready](https://example.com/ads), see [https://example.com](https://example.com) and [me](mailto:me@example.com).\n## Next";
        assert_eq!(
            for_pasting(text),
            "Plan\nThe *ads* are ready (https://example.com/ads), see https://example.com and me (me@example.com).\nNext"
        );
    }

    #[test]
    fn a_links_address_goes_only_when_its_label_is_it() {
        assert_eq!(
            for_pasting(
                "[here](https://example.com/where) and [example.com](https://example.com/)"
            ),
            "here (https://example.com/where) and example.com"
        );
        assert_eq!(
            plain("[me@example.com](mailto:me@example.com)"),
            "me@example.com"
        );
    }

    #[test]
    fn a_draft_copies_as_plain_words() {
        assert_eq!(
            plain(
                "Hey, the **bakery ads** are `ready`.\n\nCheck [the folder](https://example.com/ads) 👍"
            ),
            "Hey, the bakery ads are ready.\n\nCheck the folder (https://example.com/ads) 👍"
        );
        assert_eq!(plain("snake_case stays"), "snake_case stays");
    }

    #[test]
    fn a_draft_keeps_marks_that_dont_pair() {
        assert_eq!(
            plain("2 * 3 = 6, and 2 ** 3 = 8"),
            "2 * 3 = 6, and 2 ** 3 = 8"
        );
        assert_eq!(plain("call __init__ first"), "call __init__ first");
        assert_eq!(plain("**bold *and* more** here"), "bold and more here");
        assert_eq!(plain("a *lone star"), "a *lone star");
        assert_eq!(plain("run `a * b` and ``x`y``"), "run a * b and x`y");
        assert_eq!(plain("an open `tick"), "an open `tick");
    }

    #[test]
    fn a_quote_previews_its_words() {
        assert_eq!(
            preview("> **Sure**, see [it](https://x.example) #1"),
            "Sure, see it 1"
        );
    }

    #[test]
    fn explain_simply_quotes_the_first_words() {
        assert_eq!(
            explain_simply("See [the plan](https://example.com) now"),
            "Explain this in simpler words, with a real example: “See the plan now…”"
        );
        let long = "a".repeat(200);
        let asked = explain_simply(&long);
        assert_eq!(asked.matches('a').count(), EXPLAIN_QUOTE + 4, "{asked}");
    }

    #[test]
    fn the_web_links_are_found_once_each_in_order() {
        let text = format!(
            "Read [the runbook](https://example.com/runbook), https://github.com/example/pull/3. \
             Also www.example.org, docs.example.net, me@example.com, {TASK_KEY_PREFIX}12 and \
             [again](https://example.com/runbook)."
        );
        let links = web_links(&text);
        let copied: Vec<&str> = links.iter().map(|l| l.copied.as_str()).collect();
        assert_eq!(
            copied,
            [
                "https://example.com/runbook",
                "https://github.com/example/pull/3",
                "https://www.example.org",
                "https://docs.example.net",
                "me@example.com",
            ]
        );
        assert_eq!(links[1].host, "github.com");
        assert_eq!(link_item(&links[1], 2), "Copy github.com");
        assert_eq!(link_item(&links[1], 1), "Copy Link");
        assert_eq!(
            web_links(&format!("only {TASK_KEY_PREFIX}3 and `code`")),
            []
        );
    }
}
