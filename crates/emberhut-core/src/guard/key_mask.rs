//! Keys and tokens on a page or in a line (a service's developer pages, a new API token): the
//! assistant never reads one. What the browser tools show it, a task's browser log and its
//! history have them masked, so a value never passes through the assistant's context, the chat
//! or a log. Masked, a key reads `[secret: xoxp-…, use browser_save_secret ref e12]`: the
//! browser's tool that moves it from the page into the server's secrets folder.
//!
//! The browser's own part (finding keys in the page's fields, blacking them out of a
//! screenshot) runs in the page and is the browser area's; the rules on text are here.

use std::sync::LazyLock;

use regex::Regex;

/// The token shapes, as one pattern (for the page's side, which runs it in JavaScript): Slack
/// (user, bot, app-level, refresh), `OpenAI` and Anthropic, GitHub, AWS access key ids, Google
/// API keys, Stripe secret and restricted keys, Cloudflare's user and account API tokens. An
/// `sk-` key needs a digit in it, so words like "sk-learn" stay.
pub const TOKEN_SOURCE: &str = r"\b(?:xox[pbaeorsc]-[A-Za-z0-9-]{8,}|xapp-[A-Za-z0-9-]{8,}|sk-(?=[A-Za-z0-9_-]*\d)[A-Za-z0-9_-]{20,}|gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,}|(?:AKIA|ASIA)[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{35}|[sr]k_(?:live|test)_[A-Za-z0-9]{16,}|cf[ua]t_[A-Za-z0-9]{30,})";

/// [`TOKEN_SOURCE`] without the `sk-` shape, whose digit rule needs a look-ahead this crate's
/// regexes don't have: [`SK`] and [`has_digit`] read it instead.
static TOKEN: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"\b(?:xox[pbaeorsc]-[A-Za-z0-9-]{8,}|xapp-[A-Za-z0-9-]{8,}|gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,}|(?:AKIA|ASIA)[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{35}|[sr]k_(?:live|test)_[A-Za-z0-9]{16,}|cf[ua]t_[A-Za-z0-9]{30,})",
    )
    .ok()
});

/// An `sk-` key's shape; it counts with a digit in it.
static SK: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\bsk-[A-Za-z0-9_-]{20,}").ok());

/// The kind a masked key keeps: its prefix.
static KIND: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"^(xox[a-z]-|xapp-|sk-|gh[a-z]_|github_pat_|AKIA|ASIA|AIza|[sr]k_(?:live|test)_|cf[ua]t_)",
    )
    .ok()
});

/// The `[ref=e12]` an accessibility tree's line names its element by.
static REF: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\[ref=(\w+)\]").ok());

fn has_digit(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_digit())
}

/// What a key reads as, masked: its kind, and the ref to save it from when there is one.
#[must_use]
pub fn masked(value: &str, reference: Option<&str>) -> String {
    let kind = KIND
        .as_ref()
        .and_then(|re| re.captures(value))
        .map_or(String::new(), |caps| caps[1].to_owned());
    let save = reference
        .filter(|r| !r.is_empty())
        .map_or(String::new(), |r| {
            format!(", use browser_save_secret ref {r}")
        });
    format!("[secret: {kind}…{save}]")
}

/// `text` with every key in it masked: the `values` found on the page first (8 characters or
/// longer, the longest first), then anything shaped like a token.
#[must_use]
pub fn mask(text: &str, values: &[String], reference: Option<&str>) -> String {
    let mut text = text.to_owned();
    let mut found: Vec<&String> = values.iter().filter(|v| v.chars().count() >= 8).collect();
    found.sort_by_key(|v| std::cmp::Reverse(v.chars().count()));
    for value in found {
        if text.contains(value.as_str()) {
            text = text.replace(value.as_str(), &masked(value, reference));
        }
    }
    if let Some(re) = TOKEN.as_ref() {
        text = re
            .replace_all(&text, |caps: &regex::Captures<'_>| {
                masked(&caps[0], reference)
            })
            .into_owned();
    }
    if let Some(re) = SK.as_ref() {
        text = re
            .replace_all(&text, |caps: &regex::Captures<'_>| {
                if has_digit(&caps[0]) {
                    masked(&caps[0], reference)
                } else {
                    caps[0].to_owned()
                }
            })
            .into_owned();
    }
    text
}

/// Whether `text` holds anything shaped like a token.
#[must_use]
pub fn has_token(text: &str) -> bool {
    TOKEN.as_ref().is_some_and(|re| re.is_match(text))
        || SK
            .as_ref()
            .is_some_and(|re| re.find_iter(text).any(|m| has_digit(m.as_str())))
}

/// An accessibility tree with its keys masked: each one points at the element that holds it
/// (its own ref, or the nearest one above it).
#[must_use]
pub fn mask_tree(tree: &str, values: &[String]) -> String {
    let mut refs: Vec<(i64, String)> = Vec::new();
    let mut lines = Vec::new();
    for line in tree.split('\n') {
        // A blank line is indented less than any: every element above it is closed.
        let indent = line
            .chars()
            .position(|c| !c.is_whitespace())
            .and_then(|at| i64::try_from(at).ok())
            .unwrap_or(-1);
        let own = REF
            .as_ref()
            .and_then(|re| re.captures(line))
            .map(|caps| caps[1].to_owned());
        while refs.last().is_some_and(|(at, _)| *at >= indent) {
            refs.pop();
        }
        if let Some(own) = &own {
            refs.push((indent, own.clone()));
        }
        let reference = own.or_else(|| refs.last().map(|(_, r)| r.clone()));
        lines.push(mask(line, values, reference.as_deref()));
    }
    lines.join("\n")
}

/// The sites nothing is ever saved from (password managers, account security pages, the
/// browser's extension store).
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

/// Whether `url` is a site keys are never saved from: one of [`BLOCKED`] (a host or any of
/// its subdomains, and the path under it), anything but `http(s)`, or an address that doesn't
/// read as one.
#[must_use]
pub fn blocked_site(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return true;
    };
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return true;
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let host = host_port
        .rsplit_once(':')
        .filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit()))
        .map_or(host_port, |(host, _)| host)
        .to_ascii_lowercase();
    if host.is_empty() {
        return true;
    }
    let path_end = rest[end..]
        .find(['?', '#'])
        .map_or(rest.len(), |at| end + at);
    let path = if end < path_end {
        &rest[end..path_end]
    } else {
        "/"
    };
    let path = if path.is_empty() { "/" } else { path };
    BLOCKED.iter().any(|blocked| {
        let (site, under) = blocked
            .split_once('/')
            .map_or((*blocked, String::new()), |(site, under)| {
                (site, format!("/{under}"))
            });
        (host == site || host.ends_with(&format!(".{site}"))) && path.starts_with(&under)
    })
}

/// Every pattern here compiles.
#[must_use]
pub fn patterns_compile() -> bool {
    TOKEN.is_some() && SK.is_some() && KIND.is_some() && REF.is_some()
}
