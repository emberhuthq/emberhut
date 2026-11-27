//! Text as the channels see it: the one normal form a message is compared in, what Slack and
//! WhatsApp would turn into formatting, and short previews.

use std::sync::LazyLock;

use regex::Regex;

/// Whether `c` is whitespace as JavaScript's `trim` sees it.
#[must_use]
pub fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// `text` without JavaScript whitespace at either end.
#[must_use]
pub fn js_trim(text: &str) -> &str {
    text.trim_matches(is_js_space)
}

/// The form two texts are compared in, and the one that is sent: line breaks as `\n`,
/// non-breaking spaces as spaces, no whitespace at the end of a line or around the whole.
///
/// ```
/// use emberhut_core::messaging::text::same;
///
/// assert_eq!(same("  hi there \r\nsee you\u{a0}\r\n"), "hi there\nsee you");
/// ```
#[must_use]
pub fn same(text: &str) -> String {
    let unified = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\u{a0}', " ");
    let lines: Vec<&str> = unified
        .split('\n')
        .map(|line| line.trim_end_matches(is_js_space))
        .collect();
    js_trim(&lines.join("\n")).to_owned()
}

/// `text` cut to `limit` characters at a word's end, with `…`; shorter text as it is.
///
/// ```
/// use emberhut_core::messaging::text::preview;
///
/// assert_eq!(preview("one two three", 9), "one two…");
/// assert_eq!(preview("short", 9), "short");
/// ```
#[must_use]
pub fn preview(text: &str, limit: usize) -> String {
    static LAST_WORD: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\s+\S*$").ok());
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let head: String = text.chars().take(limit).collect();
    let cut = match LAST_WORD.as_ref() {
        Some(last) => last.replace(&head, "").into_owned(),
        None => head,
    };
    format!("{cut}…")
}

/// The first `limit` characters of `text`.
#[must_use]
pub fn first_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

/// A rule: what it finds, and how it is named to the assistant.
struct Rule {
    found: Option<Regex>,
    what: &'static str,
}

impl Rule {
    fn new(pattern: &str, what: &'static str) -> Self {
        Self {
            found: Regex::new(pattern).ok(),
            what,
        }
    }

    fn matches(&self, text: &str) -> bool {
        self.found
            .as_ref()
            .is_some_and(|found| found.is_match(text))
    }
}

/// What Slack would show as formatting or a link.
static SLACK_FORMATTING: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    vec![
        Rule::new(r"\[[^\]\n]*\]\([^)\n]*\)", "a markdown link [text](url)"),
        Rule::new(
            r"<[^\s>][^>\n]*>",
            "a <link> or <@mention> in angle brackets",
        ),
        Rule::new("`", "backticks"),
        Rule::new(r"\*\*|__|~~", "markdown bold or strikethrough"),
        Rule::new(
            r"(?m)(?:^|[\s(])(?:\*[^\s*_~][^\n]*?\*|_[^\s*_~][^\n]*?_|~[^\s*_~][^\n]*?~)(?:[\s).,!?:;]|$)",
            "*bold*, _italic_ or ~strike~ formatting",
        ),
        Rule::new(r"(?m)^\s*#{1,6}\s", "a markdown heading"),
        Rule::new(r"(?m)^\s*[-*•▪◦]\s", "bullet symbols"),
        Rule::new(r"(?m)^\s*>", "a quote (>)"),
    ]
});

/// What in `text` Slack would show as formatting, named; none for plain text. Such text is
/// refused rather than stripped, so the owner only ever approves what goes out.
///
/// ```
/// use emberhut_core::messaging::text::slack_formatting;
///
/// assert_eq!(slack_formatting("see you at 5, ok?"), None);
/// assert_eq!(slack_formatting("it's *done*"), Some("*bold*, _italic_ or ~strike~ formatting"));
/// assert_eq!(slack_formatting("- one\n- two"), Some("bullet symbols"));
/// ```
#[must_use]
pub fn slack_formatting(text: &str) -> Option<&'static str> {
    SLACK_FORMATTING
        .iter()
        .find(|rule| rule.matches(text))
        .map(|rule| rule.what)
}

/// Whether `text` has markdown WhatsApp would show as it is typed: `**bold**`, a
/// `[text](url)` link, a `#` heading or backticks.
#[must_use]
pub fn whatsapp_markdown(text: &str) -> bool {
    static MARKDOWN: LazyLock<Rule> = LazyLock::new(|| {
        Rule::new(
            r"(?m)\*\*|\[[^\]\n]*\]\([^)\n]*\)|^\s*#{1,6}\s|`",
            "markdown",
        )
    });
    MARKDOWN.matches(text)
}

/// Whether `text` holds a link (an address, `www.`, or a bare domain on a common ending).
#[must_use]
pub fn has_link(text: &str) -> bool {
    static LINK: LazyLock<Rule> = LazyLock::new(|| {
        Rule::new(
            r"(?i)https?://|www\.|\b[a-z0-9-]+\.(?:com|net|org|io|app|co|me|ly|dev|ai)\b",
            "a link",
        )
    });
    LINK.matches(text)
}

/// `text` with HTML's character references decoded: numeric ones, and the named ones mail
/// uses. An unknown name stays as it was.
#[must_use]
pub fn html_decode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let end = tail.find(';').filter(|end| *end <= 12);
        let decoded = end.and_then(|end| entity(&tail[1..end]).map(|c| (c, end)));
        if let Some((c, end)) = decoded {
            out.push(c);
            rest = &tail[end + 1..];
        } else {
            out.push('&');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

/// The character an entity's name (between `&` and `;`) stands for.
fn entity(name: &str) -> Option<char> {
    if let Some(number) = name.strip_prefix('#') {
        let code = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse().ok()?,
        };
        return char::from_u32(code);
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "hellip" => '…',
        "mdash" => '—',
        "ndash" => '–',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "laquo" => '«',
        "raquo" => '»',
        "bull" => '•',
        "middot" => '·',
        "euro" => '€',
        "pound" => '£',
        "times" => '×',
        "deg" => '°',
        "zwnj" => '\u{200c}',
        "zwj" => '\u{200d}',
        "lrm" => '\u{200e}',
        "rlm" => '\u{200f}',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_passes_and_each_kind_of_formatting_is_named() {
        for plain in [
            "Thanks, see you tomorrow at 10.",
            "2 * 3 = 6",
            "a_b and c_d",
            "email me at someone@example.com",
        ] {
            assert_eq!(slack_formatting(plain), None, "{plain}");
        }
        for (text, what) in [
            ("[here](https://example.com)", "a markdown link [text](url)"),
            ("ping <@U123>", "a <link> or <@mention> in angle brackets"),
            ("run `make`", "backticks"),
            ("**big**", "markdown bold or strikethrough"),
            ("so _very_ nice", "*bold*, _italic_ or ~strike~ formatting"),
            ("# Title", "a markdown heading"),
            ("• one", "bullet symbols"),
            ("> said", "a quote (>)"),
        ] {
            assert_eq!(slack_formatting(text), Some(what), "{text}");
        }
    }

    #[test]
    fn whatsapp_sees_markdown_and_links() {
        assert!(whatsapp_markdown("**hi**"));
        assert!(whatsapp_markdown("line\n## head"));
        assert!(!whatsapp_markdown("*hi* is fine on WhatsApp"));
        assert!(has_link("see https://x.test"));
        assert!(has_link("go to example.com now"));
        assert!(!has_link("no link here."));
    }

    #[test]
    fn entities_decode_and_unknown_ones_stay() {
        assert_eq!(
            html_decode("Tom &amp; Jerry &#39;x&#x27; &nbsp;&bogus; & done"),
            "Tom & Jerry 'x' \u{a0}&bogus; & done"
        );
    }

    #[test]
    fn same_is_idempotent() {
        let once = same(" a \r\n\r\n b\u{a0} ");
        assert_eq!(once, "a\n\n b");
        assert_eq!(same(&once), once);
    }
}
