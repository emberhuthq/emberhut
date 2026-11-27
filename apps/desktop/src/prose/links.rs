//! The links a message's text gets before it is drawn: every task key (the key prefix and the
//! task's number) becomes a link that opens the task in the app, and a bare address
//! (`example.com/plan`) a web link. Code, links already there and full addresses stay as they
//! are: the markdown view links `https://…`, `www.…` and email addresses itself. And where a
//! link clicked in a message leads ([`LinkTarget`]).

use std::path::Path;

use emberhut_api::ids::TaskId;

use super::scan::{Needle, Next};
use crate::config::ServerUrl;
use crate::links::{self, Target};
use crate::wire::{TASK_KEY_PREFIX, TASK_LINK};

/// The top-level domains a bare address may end with to be linked (`example.com`). Kept to
/// ones that don't also name common files (`.sh`, `.md`, `.rs`, `.app`), so a file name stays
/// text.
const BARE_DOMAINS: &[&str] = &[
    "com", "org", "net", "io", "dev", "run", "ai", "co", "me", "edu", "gov", "info", "uk", "de",
    "fr", "jo", "sa", "ae", "eu", "us", "ca", "tv", "xyz", "news", "page", "site", "store", "tech",
];

/// The link that opens `task` in the app.
#[must_use]
pub(crate) fn task_url(task: TaskId) -> String {
    format!("{TASK_LINK}{task}")
}

/// The task a task link ([`TASK_LINK`] and the task's number, at most one `/` after it) opens.
#[must_use]
pub(crate) fn task_link(url: &str) -> Option<TaskId> {
    let id = url.strip_prefix(TASK_LINK)?;
    task_id(id.strip_suffix('/').unwrap_or(id))
}

/// A task's id as it is written in a link or on the command line: digits only, above zero.
#[must_use]
pub(crate) fn task_id(text: &str) -> Option<TaskId> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<i64>()
        .ok()
        .filter(|&id| id > 0)
        .map(TaskId::new)
}

/// Where a link clicked in a message's text leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LinkTarget {
    /// A task's page in the app ([`TASK_LINK`], or the app's own `emberhut://task/N`).
    Task(TaskId),
    /// Another place in the app an app link names (a page, a room, a window of its own: see
    /// [`crate::links`]), taken there by [`crate::shell::open_link`].
    Place,
    /// An email address, for the mail app.
    Mail,
    /// An HTML report on the server, opened beside the page.
    Report {
        /// Its file name.
        name: String,
    },
    /// Another file on the server: fetched signed in and shown in the viewer, as a file chip's
    /// is (a file that runs as a program is only shown in its folder, never opened).
    File {
        /// Its file name.
        name: String,
    },
    /// A web address, for the browser.
    Web,
    /// Anything else (`file:`, `smb:`, another app's scheme): never handed to the system.
    Refused,
}

impl LinkTarget {
    /// Where `url` leads.
    #[must_use]
    pub(crate) fn of(url: &str) -> Self {
        if let Some(task) = task_link(url) {
            return Self::Task(task);
        }
        match links::parse(url) {
            Ok(Target::Task(task)) => return Self::Task(task),
            Ok(_) => return Self::Place,
            Err(_) => {}
        }
        let path = url.split(['#', '?']).next().unwrap_or(url);
        if ServerUrl::is_path(path) {
            let name = path.rsplit('/').next().unwrap_or(path).to_owned();
            let html = Path::new(&name).extension().is_some_and(|ext| {
                ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm")
            });
            return if html {
                Self::Report { name }
            } else {
                Self::File { name }
            };
        }
        let scheme = url
            .split_once(':')
            .map(|(scheme, _)| scheme.to_ascii_lowercase());
        match scheme.as_deref() {
            Some("mailto") => Self::Mail,
            Some("http" | "https") => Self::Web,
            _ => Self::Refused,
        }
    }
}

/// `text` with its task keys and bare addresses as markdown links, outside fenced code.
#[must_use]
pub(crate) fn linked(text: &str) -> String {
    let mut fenced = false;
    let mut out = String::with_capacity(text.len() + 32);
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            out.push_str(line);
        } else if fenced {
            out.push_str(line);
        } else {
            link_line(line, &mut out);
        }
    }
    out
}

/// One line outside code: code spans and markdown links copied as they are, each other word
/// linked when it is a task key or a bare address. Read in one pass: the next `](` and `)` are
/// looked for once, not again from every `[`.
fn link_line(line: &str, out: &mut String) {
    let mut middles = Next::new(line, Needle::Text("]("));
    let mut closes = Next::new(line, Needle::Text(")"));
    let mut at = 0;
    while let Some(c) = line[at..].chars().next() {
        let rest = &line[at..];
        let middle = (c == '[' || rest.starts_with("!["))
            .then(|| middles.at_or_after(at))
            .flatten();
        let taken = if c == '`' {
            verbatim_code(rest)
        } else if let Some(middle) = middle {
            // A markdown link or picture, to its closing parenthesis.
            closes
                .at_or_after(middle)
                .map_or(rest.len(), |close| close + 1 - at)
        } else if c.is_whitespace() {
            c.len_utf8()
        } else {
            let end = rest
                .find(|c: char| c.is_whitespace() || c == '`' || c == '[')
                .unwrap_or(rest.len())
                .max(c.len_utf8());
            link_word(&rest[..end], out);
            at += end;
            continue;
        };
        out.push_str(&rest[..taken]);
        at += taken;
    }
}

/// How long the code span at the start of `rest` is (to its closing backtick, or the line).
fn verbatim_code(rest: &str) -> usize {
    rest[1..].find('`').map_or(rest.len(), |close| close + 2)
}

/// One word: a full address or an email stays (the markdown view links it); task keys in it
/// become links; else a bare address does.
fn link_word(word: &str, out: &mut String) {
    if word.contains("://") || word.contains('@') || word.starts_with("www.") {
        out.push_str(word);
        return;
    }
    if word.contains(TASK_KEY_PREFIX) {
        link_task_keys(word, out);
        return;
    }
    let lead = word.len()
        - word
            .trim_start_matches(['(', '*', '_', '"', '\'', '“'])
            .len();
    let core_end = word
        .trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '*', '_', '"', '\'', '”'])
        .len()
        .max(lead);
    let core = &word[lead..core_end];
    if is_bare_address(core) {
        out.push_str(&word[..lead]);
        out.push('[');
        out.push_str(core);
        out.push_str("](https://");
        out.push_str(core);
        out.push(')');
        out.push_str(&word[core_end..]);
    } else {
        out.push_str(word);
    }
}

/// The task keys in a word as links: a key in brackets and before a comma becomes a markdown
/// link to its task ([`TASK_LINK`]) in the same brackets, before the same comma. A key glued to a
/// word or path before it (`CAL-` or `x/` before the prefix) or with a word character after its
/// digits stays text.
fn link_task_keys(word: &str, out: &mut String) {
    let mut rest = word;
    let mut before: Option<char> = None;
    while let Some(at) = rest.find(TASK_KEY_PREFIX) {
        let glued = rest[..at]
            .chars()
            .next_back()
            .or(if at == 0 { before } else { None })
            .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '/' || c == '-');
        let after = &rest[at + TASK_KEY_PREFIX.len()..];
        let digits = after.len() - after.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let tail_glued = after[digits..]
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        let end = at + TASK_KEY_PREFIX.len() + digits;
        out.push_str(&rest[..at]);
        if glued || digits == 0 || tail_glued {
            out.push_str(&rest[at..end]);
        } else {
            let number = &after[..digits];
            out.push('[');
            out.push_str(TASK_KEY_PREFIX);
            out.push_str(number);
            out.push_str("](");
            out.push_str(TASK_LINK);
            out.push_str(number);
            out.push(')');
        }
        before = rest[..end].chars().next_back();
        rest = &rest[end..];
    }
    out.push_str(rest);
}

/// Whether `word` is a bare web address: `example.com`, `docs.example.org/a/b`. Its host has
/// two or more labels of letters, digits and dashes, the last a known top-level domain.
fn is_bare_address(word: &str) -> bool {
    let host = word.split(['/', '?', '#']).next().unwrap_or_default();
    let labels: Vec<&str> = host.split('.').collect();
    let Some(&tld) = labels.last() else {
        return false;
    };
    labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        && BARE_DOMAINS.contains(&tld.to_ascii_lowercase().as_str())
        && labels[0].chars().any(|c| c.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Task `n`'s key, as the server writes it.
    fn key(n: u32) -> String {
        format!("{TASK_KEY_PREFIX}{n}")
    }

    /// Task `n`'s key as a link to the task.
    fn linked_key(n: u32) -> String {
        format!("[{TASK_KEY_PREFIX}{n}]({TASK_LINK}{n})")
    }

    #[test]
    fn task_keys_become_links_to_the_task() {
        assert_eq!(
            linked(&format!("Done on {}, see ({}).", key(21), key(3))),
            format!("Done on {}, see ({}).", linked_key(21), linked_key(3))
        );
        assert_eq!(
            linked(&format!("**{}** first", key(9))),
            format!("**{}** first", linked_key(9))
        );
    }

    #[test]
    fn keys_glued_to_another_word_in_code_or_links_stay_text() {
        for text in [
            format!("CAL-{} and x/{} and {}x", key(2), key(3), key(4)),
            format!("run `git log {}` now", key(12)),
            format!("[the task {}](https://example.com/{})", key(5), key(5)),
            format!("https://example.com/{}", key(6)),
            format!("{TASK_KEY_PREFIX} alone"),
        ] {
            assert_eq!(linked(&text), text, "{text}");
        }
    }

    #[test]
    fn fenced_code_is_left_alone() {
        let code = format!("```\n{} example.com\n```\n", key(1));
        assert_eq!(
            linked(&format!("{code}{}", key(2))),
            format!("{code}{}", linked_key(2))
        );
    }

    #[test]
    fn bare_addresses_become_web_links() {
        assert_eq!(
            linked("See example.com/plan, or docs.example.org."),
            "See [example.com/plan](https://example.com/plan), or [docs.example.org](https://docs.example.org)."
        );
        for text in [
            "the script backup.sh",
            "notes.md and main.rs",
            "Emberhut.app",
            "version 1.2.3",
            "https://example.com already",
            "www.example.com is linked by the view",
            "mail me@example.com",
            "e.g. this",
        ] {
            assert_eq!(linked(text), text, "{text}");
        }
    }

    #[test]
    fn task_links_read_back_as_their_task() {
        assert_eq!(task_link(&format!("{TASK_LINK}21")), Some(TaskId::new(21)));
        assert_eq!(task_link(&task_url(TaskId::new(7))), Some(TaskId::new(7)));
        assert_eq!(task_link(&format!("{TASK_LINK}x")), None);
        assert_eq!(task_link("https://example.com/task/1"), None);
        assert_eq!(task_link(&format!("{TASK_LINK}3/")), Some(TaskId::new(3)));
        for loose in ["-3", "+3", "3//", "3///", "0", "", "/", " 3", "3 "] {
            assert_eq!(task_link(&format!("{TASK_LINK}{loose}")), None, "{loose:?}");
        }
    }

    #[test]
    fn a_link_leads_where_its_kind_goes() {
        assert_eq!(
            LinkTarget::of(&format!("{TASK_LINK}12")),
            LinkTarget::Task(TaskId::new(12))
        );
        assert_eq!(LinkTarget::of("mailto:me@example.com"), LinkTarget::Mail);
        assert_eq!(
            LinkTarget::of("/reports/week.html#top"),
            LinkTarget::Report {
                name: "week.html".into()
            }
        );
        assert_eq!(
            LinkTarget::of("/uploads/x.command?v=1"),
            LinkTarget::File {
                name: "x.command".into()
            }
        );
        assert_eq!(LinkTarget::of("https://example.com/a"), LinkTarget::Web);
        assert_eq!(LinkTarget::of("HTTP://example.com/a"), LinkTarget::Web);
        assert_eq!(
            LinkTarget::of("emberhut://task/12"),
            LinkTarget::Task(TaskId::new(12))
        );
        assert_eq!(LinkTarget::of("emberhut://chat?room=2"), LinkTarget::Place);
        assert_eq!(LinkTarget::of("emberhut://settings"), LinkTarget::Place);
        for refused in [
            "file:///etc/passwd",
            "smb://example.com/share",
            "javascript:alert(1)",
            "x-app://open",
            "//example.com/x",
            "example.com",
        ] {
            assert_eq!(LinkTarget::of(refused), LinkTarget::Refused, "{refused}");
        }
    }

    #[test]
    fn arabic_and_long_words_pass_through() {
        assert_eq!(
            linked(&format!("مرحبا {} كلمة", key(1))),
            format!("مرحبا {} كلمة", linked_key(1))
        );
    }
}
