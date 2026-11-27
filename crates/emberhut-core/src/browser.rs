//! The rules of the browser the assistant drives, with no I/O: what a step is (a final one, one
//! that wants a secret), how API keys and tokens on a page are masked before the assistant, a log
//! or a picture sees them, which sites nothing is taken from, how a big page's tree is folded to
//! fit, what an address typed in the toolbar means, and which files may go into a page or come
//! out of one.
//!
//! The server's browser service applies these to what the browser engine reads.

use std::sync::LazyLock;

use regex::{Captures, Regex};

/// The tree the assistant reads is cut at this many characters.
pub const SNAPSHOT_LIMIT: usize = 30_000;

/// The biggest file put into a page: 50 MB.
pub const UPLOAD_MAX: u64 = 50_000_000;

/// The biggest file taken from a page: 500 MB.
pub const DOWNLOAD_MAX: u64 = 500_000_000;

/// A regex that is a constant of this module (each one is compiled by the tests).
#[expect(
    clippy::expect_used,
    reason = "a constant pattern; the tests compile every one"
)]
fn rx(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a valid constant pattern")
}

/// Tokens known by their shape, as the page sees the pattern (JavaScript, with the lookahead
/// that keeps words like `sk-learn` out): Slack (user, bot, app-level, refresh), `OpenAI`-style
/// `sk-` keys, GitHub, AWS access key ids, Google API keys, Stripe secret and restricted keys,
/// and Cloudflare's user and account API tokens.
pub const TOKEN_SOURCE: &str = r"\b(?:xox[pbaeorsc]-[A-Za-z0-9-]{8,}|xapp-[A-Za-z0-9-]{8,}|sk-(?=[A-Za-z0-9_-]*\d)[A-Za-z0-9_-]{20,}|gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,}|(?:AKIA|ASIA)[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{35}|[sr]k_(?:live|test)_[A-Za-z0-9]{16,}|cf[ua]t_[A-Za-z0-9]{30,})";

/// [`TOKEN_SOURCE`] without the lookahead (the `regex` crate has none): an `sk-` match must
/// also hold a digit, checked by [`is_token`].
static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    rx(
        r"\b(?:xox[pbaeorsc]-[A-Za-z0-9-]{8,}|xapp-[A-Za-z0-9-]{8,}|sk-[A-Za-z0-9_-]{20,}|gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,}|(?:AKIA|ASIA)[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{35}|[sr]k_(?:live|test)_[A-Za-z0-9]{16,}|cf[ua]t_[A-Za-z0-9]{30,})",
    )
});

/// A key's kind, from its start.
static KIND: LazyLock<Regex> = LazyLock::new(|| {
    rx(r"^(xox[a-z]-|xapp-|sk-|gh[a-z]_|github_pat_|AKIA|ASIA|AIza|[sr]k_(?:live|test)_|cf[ua]t_)")
});

/// A [`TOKEN`] match that really is one: an `sk-` key holds a digit.
fn is_token(found: &str) -> bool {
    found
        .strip_prefix("sk-")
        .is_none_or(|rest| rest.bytes().any(|b| b.is_ascii_digit()))
}

/// Whether `text` holds a token shaped like a key.
#[must_use]
pub fn has_token(text: &str) -> bool {
    TOKEN.find_iter(text).any(|m| is_token(m.as_str()))
}

/// The tokens in `text`, in order.
#[must_use]
pub fn tokens_in(text: &str) -> Vec<String> {
    TOKEN
        .find_iter(text)
        .map(|m| m.as_str())
        .filter(|m| is_token(m))
        .map(str::to_owned)
        .collect()
}

/// How a masked key reads: `[secret: xoxp-…, use browser_save_secret ref e12]`.
fn masked(value: &str, at: Option<&str>) -> String {
    let kind = KIND.find(value).map_or("", |m| m.as_str());
    match at {
        Some(at) if !at.is_empty() => {
            format!("[secret: {kind}…, use browser_save_secret ref {at}]")
        }
        _ => format!("[secret: {kind}…]"),
    }
}

/// `text` with every key in it masked: the `values` found on the page first (field values,
/// longest first, 8 characters or more), then anything shaped like a token. `at` is the ref the
/// key can be saved from.
#[must_use]
pub fn mask_with(text: &str, values: &[String], at: Option<&str>) -> String {
    let mut text = text.to_owned();
    let mut longest: Vec<&String> = values.iter().filter(|v| v.chars().count() >= 8).collect();
    longest.sort_by_key(|v| std::cmp::Reverse(v.chars().count()));
    for value in longest {
        if text.contains(value.as_str()) {
            text = text.replace(value.as_str(), &masked(value, at));
        }
    }
    TOKEN
        .replace_all(&text, |caps: &Captures<'_>| {
            let found = &caps[0];
            if is_token(found) {
                masked(found, at)
            } else {
                found.to_owned()
            }
        })
        .into_owned()
}

/// `text` with anything shaped like a token masked: what every tool answer, log line and task
/// note goes through.
#[must_use]
pub fn mask(text: &str) -> String {
    mask_with(text, &[], None)
}

/// The indentation of a tree line: where its first non-space character is, or none for a blank
/// line.
fn indent(line: &str) -> Option<usize> {
    line.char_indices()
        .find(|(_, c)| !c.is_whitespace())
        .map(|(at, _)| at)
}

/// A tree line's own ref (`[ref=e12]`).
static REF: LazyLock<Regex> = LazyLock::new(|| rx(r"\[ref=(\w+)\]"));

/// The accessibility tree with every key masked, each pointing at the element that holds it:
/// its own ref, or the nearest one above it.
#[must_use]
pub fn mask_tree(tree: &str, values: &[String]) -> String {
    let mut refs: Vec<(isize, String)> = Vec::new();
    let mut lines = Vec::new();
    for line in tree.split('\n') {
        let ind = indent(line).map_or(-1, |at| isize::try_from(at).unwrap_or(isize::MAX));
        let own = REF.captures(line).map(|c| c[1].to_owned());
        while refs.last().is_some_and(|(at, _)| *at >= ind) {
            refs.pop();
        }
        if let Some(own) = &own {
            refs.push((ind, own.clone()));
        }
        let at = own.or_else(|| refs.last().map(|(_, r)| r.clone()));
        lines.push(mask_with(line, values, at.as_deref()));
    }
    lines.join("\n")
}

/// Sites nothing is ever saved from or put into: the owner's passwords and accounts.
pub const BLOCKED: [&str; 14] = [
    "1password.com",
    "1password.ca",
    "1password.eu",
    "passwords.google.com",
    "myaccount.google.com/security",
    "myaccount.google.com/signinoptions",
    "account.apple.com",
    "appleid.apple.com",
    "chromewebstore.google.com",
    "chrome.google.com/webstore",
    "bitwarden.com",
    "lastpass.com",
    "dashlane.com",
    "keepersecurity.com",
];

/// Whether `url` is on a blocked site; an address that doesn't parse, or isn't `http(s)`, is.
#[must_use]
pub fn blocked_site(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return true;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return true;
    }
    let hostname = parsed.host_str().unwrap_or_default().to_lowercase();
    let pathname = parsed.path();
    BLOCKED.iter().any(|blocked| {
        let (host, path) = blocked
            .split_once('/')
            .map_or((*blocked, String::new()), |(h, p)| (h, format!("/{p}")));
        (hostname == host || hostname.ends_with(&format!(".{host}"))) && pathname.starts_with(&path)
    })
}

/// Whether a tab at `url` has reached a blocked site, after a navigation (a click, a redirect,
/// a key): an `http(s)` address on one of [`BLOCKED`], an extension's own page, or a browser
/// page other than a new tab (settings, passwords). A blank page, an error page or an address
/// that doesn't parse has not.
#[must_use]
pub fn blocked_page(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    match parsed.scheme() {
        "http" | "https" => blocked_site(url),
        "chrome-extension" => true,
        "chrome" | "edge" | "brave" => !matches!(
            parsed.host_str().unwrap_or_default(),
            "newtab" | "new-tab-page"
        ),
        _ => false,
    }
}

/// Whether `key`, as the tools take a key (`Enter`, `Control+Enter`, `Meta+Return`,
/// `shift+enter`…), is Enter or Return with or without modifiers: the key that sends a form,
/// a message or a post.
#[must_use]
pub fn is_enter_key(key: &str) -> bool {
    if matches!(key, "\n" | "\r" | "\r\n") {
        return true;
    }
    let last = key.trim().rsplit('+').next().unwrap_or_default().trim();
    ["enter", "return", "numpadenter", "kp_enter", "keypadenter"]
        .iter()
        .any(|enter| last.eq_ignore_ascii_case(enter))
}

/// A step that can't be undone: it waits for the owner's OK. Installing an app into an account
/// (Slack's "Install to …", "Reinstall", an `OAuth` "Authorize") is one.
static FINAL: LazyLock<Regex> = LazyLock::new(|| {
    rx(
        r"(?i)\b(submit|send|post|publish|tweet|reply|buy|purchase|pay|order|checkout|subscribe|confirm|delete|remove|create|(?:re)?install|authorize|register|sign up|transfer|donate|save changes|change password|two-factor|2fa|security|disable|revoke)\b",
    )
});

/// Whether `what` (a button's label, a form's submit) reads as a final step.
#[must_use]
pub fn is_final(what: &str) -> bool {
    FINAL.is_match(what)
}

/// What a field that wants a secret says about itself.
static SECRET: LazyLock<Regex> = LazyLock::new(|| {
    rx(r"(?i)\b(password|passcode|verification code|security code|one[- ]time|2fa|otp|pin)\b")
});

/// A field described like this (type, autocomplete, name, id, label, placeholder) wants a
/// password or a code: the assistant never types one.
#[must_use]
pub fn wants_secret(field: &str) -> bool {
    static FIELD: LazyLock<Regex> = LazyLock::new(|| rx(r"\bpassword\b|one-time-code"));
    FIELD.is_match(field) || SECRET.is_match(field)
}

/// Whether a label reads as a password or a code (the Mac's fields, the screen's typing).
#[must_use]
pub fn is_secret_label(label: &str) -> bool {
    SECRET.is_match(label)
}

/// What a step is, for the OK it needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    /// Money and account security: its own OK every time.
    Hard,
    /// Said as the owner, or public (a message, a post): its text needs the owner's OK.
    Said,
    /// Anything else: the job's OK covers it.
    Ordinary,
}

/// Money and account security, by the step's label.
static HARD: LazyLock<Regex> = LazyLock::new(|| {
    rx(
        r"(?i)\b(buy|purchase|pay|payment|order|checkout|check out|subscribe|subscription|upgrade|donate|transfer|billing|banking|password|passcode|passkey|code|codes|otp|one-time|two-factor|2fa|mfa|security|recovery|authenticator|revoke|(?:delete|close|deactivate|remove) (?:my |your |this |the )?account)\b",
    )
});
/// Money and account security, by the page the step is on.
static HARD_PAGE: LazyLock<Regex> = LazyLock::new(|| {
    rx(
        r"(?i)\b(checkout|payment|billing|purchase|password|security|two-factor|2fa|recovery|delete account|close account)\b",
    )
});
/// Subscribing an app to events is setup, not a purchase.
static EVENT_SUBSCRIBE: LazyLock<Regex> = LazyLock::new(|| {
    rx(
        r"(?i)\b(?:subscribe to (?:bot |app |workspace |user )?events(?: on behalf of users)?|(?:bot |app |user )?event subscriptions?)\b",
    )
});
/// An app's own configuration pages on Slack, where "subscribe" is never money.
static SLACK_APP_PAGE: LazyLock<Regex> =
    LazyLock::new(|| rx(r"(?i)\bhttps?://api\.slack\.com/apps/A[A-Z0-9]+(?:[/?#]|$)"));
/// "Subscribe" itself.
static SUBSCRIBE: LazyLock<Regex> = LazyLock::new(|| rx(r"(?i)\b(?:subscribe|subscription)s?\b"));
/// Said as the owner or in public.
static SAID: LazyLock<Regex> = LazyLock::new(|| {
    rx(
        r"(?i)\b(send|reply|respond|post|tweet|retweet|repost|publish|message|dm|comment|email|invite|share|go live)\b",
    )
});
/// Pages where any step speaks as the owner: mail and chat.
static PERSONAL_PAGE: LazyLock<Regex> = LazyLock::new(|| {
    rx(
        r"(?i)\b(mail\.google\.com|outlook\.(?:live|office)\.com|web\.whatsapp\.com|messenger\.com|linkedin\.com/messaging|(?:x|twitter)\.com/(?:compose|messages))",
    )
});
/// An app's store console, where publishing is the owner's own app's release.
static APP_CONSOLE: LazyLock<Regex> = LazyLock::new(|| {
    rx(
        r"(?i)\b(play\.google\.com/console|appstoreconnect\.apple\.com|developer\.apple\.com/account|chromewebstore\.google\.com|chrome\.google\.com/webstore/devconsole|console\.firebase\.google\.com|expo\.dev|partner\.microsoft\.com)",
    )
});

/// Whether `what` on `page` (its title and address) is money or account security.
#[must_use]
pub fn is_hard(what: &str, page: &str) -> bool {
    let mut w = EVENT_SUBSCRIBE.replace_all(what, " ").into_owned();
    if SLACK_APP_PAGE.is_match(page) {
        w = SUBSCRIBE.replace_all(&w, " ").into_owned();
    }
    HARD.is_match(&w) || HARD_PAGE.is_match(page)
}

/// What kind of step `what` is on `page` (its title and address; empty when unknown).
#[must_use]
pub fn step_kind(what: &str, page: &str) -> StepKind {
    if is_hard(what, page) {
        StepKind::Hard
    } else if (SAID.is_match(what) && !APP_CONSOLE.is_match(page)) || PERSONAL_PAGE.is_match(page) {
        StepKind::Said
    } else {
        StepKind::Ordinary
    }
}

/// A tree line that opens a menu or a side bar, folded first when a page is too big.
static MENU: LazyLock<Regex> =
    LazyLock::new(|| rx(r"^\s*- (navigation|complementary|menu|menubar|tree)\b"));

/// The first `limit` characters of `text`.
fn head(text: &str, limit: usize) -> &str {
    text.char_indices()
        .nth(limit)
        .map_or(text, |(at, _)| &text[..at])
}

/// A tree cut to [`SNAPSHOT_LIMIT`], saying how much is left.
#[must_use]
pub fn cut(tree: &str) -> String {
    let length = tree.chars().count();
    if length <= SNAPSHOT_LIMIT {
        return tree.to_owned();
    }
    format!(
        "{}\n… cut here ({} more characters): snapshot a part of it (scope: a ref above).",
        head(tree, SNAPSHOT_LIMIT),
        length - SNAPSHOT_LIMIT
    )
}

/// A big page's tree folded to fit `limit`: what is `aria-hidden` (a closed drawer) is folded to
/// its first line always, then the biggest menus and side bars, one by one (none under 2,000
/// characters), until it fits; the page's own content stays whole. A folded part keeps its ref,
/// to read it with a scope. Then cut, as [`cut`].
#[must_use]
pub fn fold(tree: &str, limit: usize) -> String {
    let lines: Vec<&str> = tree.split('\n').collect();
    let ind: Vec<isize> = lines
        .iter()
        .map(|l| indent(l).map_or(-1, |at| isize::try_from(at).unwrap_or(isize::MAX)))
        .collect();
    let mut keep = vec![true; lines.len()];
    let mut folded: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
    let end = |i: usize| {
        let mut j = i + 1;
        while j < lines.len() && ind[j] > ind[i] {
            j += 1;
        }
        j
    };
    let size = |keep: &[bool], from: usize, to: usize| {
        (from..to)
            .filter(|&k| keep[k])
            .map(|k| lines[k].chars().count() + 1)
            .sum::<usize>()
    };
    let fold_one =
        |i: usize, keep: &mut Vec<bool>, folded: &mut std::collections::BTreeMap<usize, usize>| {
            let j = end(i);
            for k in &mut keep[i + 1..j] {
                *k = false;
            }
            folded.insert(i, j - i - 1);
        };
    for i in 0..lines.len() {
        if keep[i] && lines[i].contains("[aria-hidden]") && end(i) > i + 1 {
            fold_one(i, &mut keep, &mut folded);
        }
    }
    while size(&keep, 0, lines.len()) > limit {
        let mut best = None;
        let mut most = 0;
        for i in 0..lines.len() {
            if keep[i] && !folded.contains_key(&i) && MENU.is_match(lines[i]) && end(i) > i + 1 {
                let n = size(&keep, i, end(i));
                if n > most {
                    most = n;
                    best = Some(i);
                }
            }
        }
        match best {
            Some(i) if most >= 2_000 => fold_one(i, &mut keep, &mut folded),
            _ => break,
        }
    }
    let mut shown = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !keep[i] {
            continue;
        }
        if let Some(n) = folded.get(&i) {
            let bare = line.strip_suffix(':').unwrap_or(line);
            let scope = REF
                .captures(line)
                .map_or_default(|c| format!("; scope \"{}\" to read them", &c[1]));
            shown.push(format!("{bare} (folded: {n} lines{scope})"));
        } else {
            shown.push((*line).to_owned());
        }
    }
    cut(&shown.join("\n"))
}

/// JavaScript's `encodeURIComponent`.
#[must_use]
pub fn encode_uri_component(text: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    out
}

/// JavaScript's `decodeURIComponent`: none for a malformed escape or bytes that aren't UTF-8.
#[must_use]
pub fn decode_uri_component(text: &str) -> Option<String> {
    if !text.contains('%') {
        return Some(text.to_owned());
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// What the owner typed in the live view's address bar: a site (`example.com`, `https://…`), or
/// words to search for.
#[must_use]
pub fn address(typed: &str) -> String {
    static SCHEME: LazyLock<Regex> = LazyLock::new(|| rx(r"(?i)^[a-z][a-z0-9+.-]*://"));
    static SITE: LazyLock<Regex> = LazyLock::new(|| rx(r"^[^.\s]+(\.[^.\s]+)+(:\d+)?(/.*)?$"));
    let t = typed.trim();
    if SCHEME.is_match(t) || t.starts_with("about:") {
        return t.to_owned();
    }
    if !t.chars().any(char::is_whitespace) && SITE.is_match(t) {
        return format!("https://{t}");
    }
    format!(
        "https://www.google.com/search?q={}",
        encode_uri_component(t)
    )
}

/// The address a `browser_open` goes to: as given when it starts with `http(s)://`, else
/// `https://` and it.
#[must_use]
pub fn open_url(given: &str) -> String {
    if given.starts_with("http://") || given.starts_with("https://") {
        given.to_owned()
    } else {
        format!("https://{given}")
    }
}

/// The same page, for remembering tabs: host, path (no trailing slash) and query, never the
/// fragment. An address that doesn't parse is itself.
#[must_use]
pub fn page_key(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) if u.host_str().is_some_and(|h| !h.is_empty()) => format!(
            "{}{}{}",
            u.host_str().unwrap_or_default().to_lowercase(),
            u.path().trim_end_matches('/'),
            u.query().map_or_default(|q| format!("?{q}"))
        ),
        _ => url.to_owned(),
    }
}

/// An address's host, lowercased (empty when it has none).
#[must_use]
pub fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_lowercase))
        .unwrap_or_default()
}

/// An address's host and port as said in a line (`example.com`, `localhost:8080`); none when it
/// isn't an address with a host.
#[must_use]
pub fn url_host(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let host = u.host_str()?;
    Some(match u.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    })
}

/// host/path of an address, for what's said (never its query: a login's tokens).
#[must_use]
pub fn site(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) if u.host_str().is_some_and(|h| !h.is_empty()) => format!(
            "{}{}",
            u.host_str().unwrap_or_default(),
            u.path().trim_end_matches('/')
        ),
        _ => String::new(),
    }
}

/// What may be put into a page, by extension, in order (a type's first extension names a file
/// of that type that has none).
pub const UPLOAD_TYPES: [(&str, &str); 37] = [
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("svg", "image/svg+xml"),
    ("heic", "image/heic"),
    ("heif", "image/heif"),
    ("avif", "image/avif"),
    ("bmp", "image/bmp"),
    ("ico", "image/x-icon"),
    ("tif", "image/tiff"),
    ("tiff", "image/tiff"),
    ("pdf", "application/pdf"),
    ("txt", "text/plain"),
    ("md", "text/markdown"),
    ("csv", "text/csv"),
    ("rtf", "application/rtf"),
    ("epub", "application/epub+zip"),
    ("doc", "application/msword"),
    (
        "docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
    ("xls", "application/vnd.ms-excel"),
    (
        "xlsx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ),
    ("ppt", "application/vnd.ms-powerpoint"),
    (
        "pptx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ),
    ("odt", "application/vnd.oasis.opendocument.text"),
    ("ods", "application/vnd.oasis.opendocument.spreadsheet"),
    ("odp", "application/vnd.oasis.opendocument.presentation"),
    ("pages", "application/vnd.apple.pages"),
    ("numbers", "application/vnd.apple.numbers"),
    ("zip", "application/zip"),
    ("7z", "application/x-7z-compressed"),
    ("gz", "application/gzip"),
    ("tgz", "application/gzip"),
    ("aab", "application/octet-stream"),
    ("apk", "application/vnd.android.package-archive"),
    ("ipa", "application/octet-stream"),
];

/// The type of a file with this extension (lowercase), when it may be uploaded.
#[must_use]
pub fn upload_type(ext: &str) -> Option<&'static str> {
    UPLOAD_TYPES
        .iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, t)| *t)
}

/// A file name that says it is a key, a password or credentials: never uploaded.
#[must_use]
pub fn key_like_name(name: &str) -> bool {
    static KEY_NAME: LazyLock<Regex> = LazyLock::new(|| {
        rx(
            r"(?i)(^|[._ -])(id_(rsa|dsa|ecdsa|ed25519)|private[._ -]?key|secrets?|credentials?|passwords?|tokens?|(recovery|backup)[._ -]?codes?)([._ -]|$)",
        )
    });
    KEY_NAME.is_match(name)
}

/// A file's start that holds a private key, or (for a text type) an API token.
#[must_use]
pub fn holds_key(head: &str, mime: &str) -> bool {
    head.contains("PRIVATE KEY-----")
        || ((mime.starts_with("text/") || mime.contains("svg") || mime.contains("rtf"))
            && has_token(head))
}

/// A size as said: `1.2 MB`, `34 KB`, `12 bytes`.
#[must_use]
pub fn size_text(bytes: u64) -> String {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a file's size, said to one decimal"
    )]
    let n = bytes as f64;
    if n >= 1e6 {
        format!("{:.1} MB", n / 1e6)
    } else if n >= 1e3 {
        format!("{} KB", (n / 1e3).round())
    } else {
        format!("{bytes} bytes")
    }
}

/// A downloaded file's own name: the server's (`Content-Disposition`), else the link's last
/// part; made safe; `.pdf` (or the type's extension) added to one without any.
#[must_use]
pub fn file_name(disposition: &str, url: &str, mime: &str) -> String {
    static STAR: LazyLock<Regex> =
        LazyLock::new(|| rx(r"filename\*\s*=\s*(?:UTF-8|utf-8)?''([^;]+)"));
    static QUOTED: LazyLock<Regex> = LazyLock::new(|| rx(r#"filename\s*=\s*"([^"]+)""#));
    static BARE: LazyLock<Regex> = LazyLock::new(|| rx(r"filename\s*=\s*([^;]+)"));
    static UNSAFE: LazyLock<Regex> = LazyLock::new(|| rx(r"[/\\\x00]"));
    static LEADING: LazyLock<Regex> = LazyLock::new(|| rx(r"^[.\s]+"));
    let mut n = [&*STAR, &*QUOTED, &*BARE]
        .iter()
        .find_map(|r| r.captures(disposition).map(|c| c[1].to_owned()))
        .unwrap_or_default();
    let last = if n.is_empty() {
        url::Url::parse(url)
            .ok()
            .and_then(|u| {
                u.path()
                    .split('/')
                    .rfind(|x| !x.is_empty())
                    .map(str::to_owned)
            })
            .unwrap_or_default()
    } else {
        n.clone()
    };
    if let Some(decoded) = decode_uri_component(&last) {
        n = decoded;
    }
    let safe = UNSAFE.replace_all(&n, "-");
    let safe = LEADING.replace(&safe, "");
    let safe = safe.trim();
    let count = safe.chars().count();
    let mut name: String = safe.chars().skip(count.saturating_sub(150)).collect();
    if name.is_empty() {
        "download".clone_into(&mut name);
    }
    let bare = mime.split(';').next().unwrap_or_default().trim();
    match UPLOAD_TYPES.iter().find(|(_, t)| *t == bare) {
        Some((ext, _)) if !name.contains('.') => format!("{name}.{ext}"),
        _ => name,
    }
}

/// A secret's file name: lowercase letters, digits and dashes, 2 to 64.
#[must_use]
pub fn secret_name_ok(name: &str) -> bool {
    static NAME: LazyLock<Regex> = LazyLock::new(|| rx(r"^[a-z0-9][a-z0-9-]{1,63}$"));
    NAME.is_match(name)
}

/// The server's own secrets, never overwritten from a page: the live view's passwords and the
/// server's keys.
pub const OWN_SECRETS: [&str; 4] = ["api-key", "auth-key", "browser-control", "browser-view"];

/// Apps on the owner's Mac where Return sends what was typed: pressing it there is a final step.
#[must_use]
pub fn mac_app_sends(app: &str) -> bool {
    static SENDS: LazyLock<Regex> = LazyLock::new(|| {
        rx(
            r"(?i)^(messages|mail|slack|whatsapp|\u{200e}whatsapp|telegram|signal|discord|microsoft teams|outlook|spark|superhuman)$",
        )
    });
    SENDS.is_match(app)
}

/// Chrome and the browsers built on Chromium, by the app's bundle id (`com.google.Chrome`,
/// its betas and its web apps) or, when the Mac gives none, its name: the Mac tools never drive
/// them. The owner's Chrome profiles go through the browser extension, which keeps the
/// assistant in its own tabs and off blocked sites.
#[must_use]
pub fn mac_app_is_chromium(name: &str, bundle: &str) -> bool {
    const BUNDLES: [&str; 10] = [
        "com.google.chrome",
        "org.chromium.",
        "com.brave.browser",
        "com.microsoft.edgemac",
        "com.vivaldi.vivaldi",
        "com.operasoftware.opera",
        "company.thebrowser.browser",
        "company.thebrowser.dia",
        "ai.perplexity.comet",
        "ru.yandex.desktop.yandex-browser",
    ];
    static NAMES: LazyLock<Regex> = LazyLock::new(|| {
        rx(
            r"(?i)\b(?:chrome|chromium)\b|^(?:brave browser|microsoft edge\b.*|vivaldi|opera\b.*|arc|dia|comet|yandex(?: browser)?)$",
        )
    });
    let bundle = bundle.trim().to_lowercase();
    BUNDLES.iter().any(|id| bundle.starts_with(id)) || NAMES.is_match(name.trim())
}

/// Plain dev and system tools on the Mac, used with no ask.
#[must_use]
pub fn mac_app_pre_allowed(app: &str) -> bool {
    static PRE: LazyLock<Regex> = LazyLock::new(|| {
        rx(
            r"(?i)^(?:Finder|Xcode(?:[- ]?[\d.]+)?(?:[- ]beta)?|Simulator|Device Hub|Devices|Instruments|Accessibility Inspector|Console|Preview|FileMerge|Terminal)$",
        )
    });
    PRE.is_match(app)
}

/// Apps on the Mac that are only looked at (a terminal): never pressed, typed into or picked.
#[must_use]
pub fn mac_app_view_only(app: &str) -> bool {
    app.eq_ignore_ascii_case("terminal")
}

/// How long the Mac must be idle before the whole screen starts with no ask, in minutes.
pub const MAC_IDLE_MINUTES: u32 = 5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_masked_by_their_shape_and_sk_needs_a_digit() {
        let slack = "xoxp-1234567890-abcdef";
        assert_eq!(
            mask(&format!("key {slack} here")),
            "key [secret: xoxp-…] here"
        );
        assert_eq!(
            mask("pip install sk-learn-and-more-words-here"),
            "pip install sk-learn-and-more-words-here"
        );
        assert_eq!(mask("sk-abcdefghij1234567890xyz"), "[secret: sk-…]");
        let found = vec!["a-long-field-value-1".to_owned()];
        assert_eq!(
            mask_with("v a-long-field-value-1", &found, Some("e4")),
            "v [secret: …, use browser_save_secret ref e4]"
        );
        assert!(has_token("ghp_abcdefghijklmnopqrstuvwxyz0123456789"));
        let aws = format!("{}{} and x", "AKIA", "ABCDEFGHIJKLMNOP");
        assert_eq!(tokens_in(&aws).len(), 1);
    }

    #[test]
    fn a_tree_points_each_key_at_its_own_ref_or_the_nearest_above() {
        let tree =
            "- row [ref=e3]:\n  - text: xapp-1-AAAAAAAAAA\n- textbox [ref=e9]: xapp-2-BBBBBBBBBB";
        let masked = mask_tree(tree, &[]);
        assert!(masked.contains("- text: [secret: xapp-…, use browser_save_secret ref e3]"));
        assert!(masked.contains("[ref=e9]: [secret: xapp-…, use browser_save_secret ref e9]"));
    }

    #[test]
    fn blocked_sites_by_host_and_path() {
        assert!(blocked_site("https://my.1password.com/vault"));
        assert!(blocked_site("https://myaccount.google.com/security/x"));
        assert!(!blocked_site("https://myaccount.google.com/profile"));
        assert!(blocked_site("chrome://settings"));
        assert!(blocked_site("not a url"));
        assert!(!blocked_site("https://api.slack.com/apps"));
    }

    #[test]
    fn a_page_reached_by_a_navigation_is_blocked_on_a_blocked_site_or_a_browser_page() {
        assert!(blocked_page("https://my.1password.com/vault"));
        assert!(blocked_page("https://passwords.google.com/"));
        assert!(blocked_page("chrome://settings/passwords"));
        assert!(blocked_page("chrome-extension://abc/popup.html"));
        assert!(!blocked_page("chrome://newtab/"));
        assert!(!blocked_page("about:blank"));
        assert!(!blocked_page(""));
        assert!(!blocked_page("https://example.com/"));
    }

    #[test]
    fn enter_with_any_modifier_is_the_sending_key() {
        for key in [
            "Enter",
            "enter",
            "Return",
            "Control+Enter",
            "Meta+Enter",
            "Shift+Enter",
            "cmd+return",
            "NumpadEnter",
            "\n",
        ] {
            assert!(is_enter_key(key), "{key:?}");
        }
        for key in ["Tab", "a", "Control+a", "+", "", "Escape"] {
            assert!(!is_enter_key(key), "{key:?}");
        }
    }

    #[test]
    fn chrome_and_chromium_browsers_are_known_by_bundle_id_or_name() {
        assert!(mac_app_is_chromium("Google Chrome", "com.google.Chrome"));
        assert!(mac_app_is_chromium("Work", "com.google.Chrome.app.abcdef"));
        assert!(mac_app_is_chromium("Brave Browser", ""));
        assert!(mac_app_is_chromium("Chromium", ""));
        assert!(mac_app_is_chromium("Browser", "company.thebrowser.Browser"));
        assert!(mac_app_is_chromium("Dia", "company.thebrowser.dia"));
        assert!(mac_app_is_chromium("Comet", "ai.perplexity.comet"));
        assert!(mac_app_is_chromium(
            "Yandex",
            "ru.yandex.desktop.yandex-browser"
        ));
        assert!(mac_app_is_chromium("Comet", ""));
        assert!(!mac_app_is_chromium("Pages", "com.apple.iWork.Pages"));
        assert!(!mac_app_is_chromium("Safari", "com.apple.Safari"));
    }

    #[test]
    fn final_steps_and_secret_fields() {
        assert!(is_final("Submit order"));
        assert!(is_final("Install to workspace"));
        assert!(!is_final("Next"));
        assert!(wants_secret("password current-password pw"));
        assert!(wants_secret("text one-time-code otp"));
        assert!(!wants_secret("email username"));
    }

    #[test]
    fn step_kinds() {
        assert_eq!(step_kind("Pay now", ""), StepKind::Hard);
        assert_eq!(
            step_kind("Send", "Inbox https://mail.google.com/"),
            StepKind::Said
        );
        assert_eq!(step_kind("Create app", "Apps"), StepKind::Ordinary);
        assert_eq!(
            step_kind("Publish", "https://play.google.com/console/x"),
            StepKind::Ordinary
        );
        assert_eq!(
            step_kind(
                "Subscribe to bot events",
                "https://api.slack.com/apps/A12/event-subscriptions"
            ),
            StepKind::Ordinary
        );
    }

    #[test]
    fn a_big_tree_folds_its_menus_and_keeps_its_content() {
        let menu: String = (0..200)
            .map(|i| format!("  - link \"Menu item number {i}\" [ref=e{}]\n", i + 10))
            .collect::<Vec<_>>()
            .concat();
        let tree = format!(
            "- navigation [ref=e2]:\n{menu}- main [ref=e5]:\n  - heading \"Records\" [ref=e6]\n- dialog [aria-hidden] [ref=e7]:\n  - text: hidden"
        );
        let folded = fold(&tree, 1_000);
        assert!(
            folded.contains("- navigation [ref=e2] (folded: 200 lines; scope \"e2\" to read them)")
        );
        assert!(folded.contains("- heading \"Records\""));
        assert!(folded.contains("(folded: 1 lines; scope \"e7\" to read them)"));
        assert!(!folded.contains("Menu item number 5"));
    }

    #[test]
    fn a_tree_past_the_limit_is_cut_saying_how_much_is_left() {
        let tree = "x".repeat(SNAPSHOT_LIMIT + 5);
        let cut = cut(&tree);
        assert!(cut.ends_with(
            "… cut here (5 more characters): snapshot a part of it (scope: a ref above)."
        ));
    }

    #[test]
    fn what_is_typed_in_the_address_bar() {
        assert_eq!(address("example.com"), "https://example.com");
        assert_eq!(address(" http://a.b/c "), "http://a.b/c");
        assert_eq!(address("about:blank"), "about:blank");
        assert_eq!(
            address("best tea"),
            "https://www.google.com/search?q=best%20tea"
        );
    }

    #[test]
    fn pages_hosts_and_sites() {
        assert_eq!(
            page_key("https://Example.com/a/b/?x=1#frag"),
            "example.com/a/b?x=1"
        );
        assert_eq!(page_key("https://example.com"), "example.com");
        assert_eq!(host_of("https://Sub.Example.com/x"), "sub.example.com");
        assert_eq!(
            url_host("http://localhost:8080/x").as_deref(),
            Some("localhost:8080")
        );
        assert_eq!(site("https://a.com/login/?token=1"), "a.com/login");
        assert_eq!(open_url("example.com"), "https://example.com");
    }

    #[test]
    fn files_in_and_out() {
        assert_eq!(upload_type("pdf"), Some("application/pdf"));
        assert_eq!(upload_type("exe"), None);
        assert!(key_like_name("id_ed25519"));
        assert!(key_like_name("my-secrets.txt"));
        assert!(!key_like_name("logo.png"));
        assert!(holds_key("-----BEGIN PRIVATE KEY-----", "application/pdf"));
        assert!(holds_key("token xoxb-123456789-abc", "text/plain"));
        assert!(!holds_key("token xoxb-123456789-abc", "image/png"));
        assert_eq!(size_text(12), "12 bytes");
        assert_eq!(size_text(34_400), "34 KB");
        assert_eq!(size_text(1_250_000), "1.2 MB");
        assert_eq!(
            file_name(
                "attachment; filename=\"Week 1.pdf\"",
                "https://a.com/x",
                "application/pdf"
            ),
            "Week 1.pdf"
        );
        assert_eq!(
            file_name("", "https://a.com/files/notes%20one", "application/pdf"),
            "notes one.pdf"
        );
        assert_eq!(file_name("", "https://a.com/", ""), "download");
        assert_eq!(
            file_name("filename*=UTF-8''..%2Fevil", "https://a.com/", ""),
            "-evil"
        );
    }

    #[test]
    fn secret_names_and_mac_apps() {
        assert!(secret_name_ok("slack-acme-user"));
        assert!(!secret_name_ok("Slack"));
        assert!(!secret_name_ok("a"));
        assert!(mac_app_sends("Messages"));
        assert!(!mac_app_sends("Pages"));
        assert!(mac_app_pre_allowed("Xcode-beta"));
        assert!(mac_app_view_only("Terminal"));
    }

    #[test]
    fn uri_components() {
        assert_eq!(encode_uri_component("a b/c!"), "a%20b%2Fc!");
        assert_eq!(decode_uri_component("a%20b").as_deref(), Some("a b"));
        assert_eq!(decode_uri_component("%E0%A4%A"), None);
    }
}
