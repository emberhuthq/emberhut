//! What an answer is about, as chips under it: the links in its text that are worth a tap,
//! found without a model. A pull request on GitHub, a ticket on Linear and a report page (an
//! `.html` address that carries no secret in its query) each become a chip; other links are
//! tappable in the text already and get none.

/// Chips under one answer, at most.
pub const MAX_REFS: usize = 7;

/// What a chip points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    /// A pull request.
    PullRequest,
    /// A ticket in a tracker.
    Ticket,
    /// A report page.
    Report,
}

/// One chip: its kind, its text, a longer name and where it opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkRef {
    /// What it points at.
    pub kind: LinkKind,
    /// The chip's text: `PR #12`, `ENG-42`, `Report`.
    pub label: String,
    /// A longer name: the repository, the tracker, the report's host.
    pub title: String,
    /// Where it opens.
    pub url: String,
}

/// The chips of an answer's `text`, in the order its links come, each address once.
#[must_use]
pub fn link_refs(text: &str) -> Vec<LinkRef> {
    let mut refs: Vec<LinkRef> = Vec::new();
    for url in links(text) {
        if refs.len() >= MAX_REFS {
            break;
        }
        if let Some(chip) = chip(&url)
            && refs.iter().all(|known| known.url != chip.url)
        {
            refs.push(chip);
        }
    }
    refs
}

/// Every `https://` address in `text`, without the punctuation that ends a sentence.
fn links(text: &str) -> Vec<String> {
    let stops = |c: char| c.is_whitespace() || "\"'`<>()[]".contains(c);
    text.match_indices("https://")
        .map(|(at, _)| {
            let rest = &text[at..];
            let end = rest.find(stops).unwrap_or(rest.len());
            rest[..end]
                .trim_end_matches(|c: char| ".,;:!?'\"`*".contains(c))
                .to_owned()
        })
        .collect()
}

/// The chip for `url`, if it is one worth a chip.
fn chip(url: &str) -> Option<LinkRef> {
    let rest = url.strip_prefix("https://")?;
    if let Some(path) = rest.strip_prefix("github.com/") {
        let mut parts = path.split('/');
        let (owner, repo, pull, number) =
            (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
        let name = |part: &str| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_alphanumeric() || "_.-".contains(c))
        };
        let number: String = number.chars().take_while(char::is_ascii_digit).collect();
        if pull == "pull" && name(owner) && name(repo) && !number.is_empty() {
            return Some(LinkRef {
                kind: LinkKind::PullRequest,
                label: format!("PR #{number}"),
                title: format!("{owner}/{repo}"),
                url: format!("https://github.com/{owner}/{repo}/pull/{number}"),
            });
        }
    }
    if let Some(path) = rest.strip_prefix("linear.app/") {
        let mut parts = path.split('/');
        let (_team, issue, key) = (parts.next()?, parts.next()?, parts.next()?);
        if issue == "issue" && is_ticket_key(key) {
            return Some(LinkRef {
                kind: LinkKind::Ticket,
                label: key.to_uppercase(),
                title: "Linear".to_owned(),
                url: url.to_owned(),
            });
        }
    }
    let (address, query) = match rest.find(['?', '#']) {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, ""),
    };
    let page = std::path::Path::new(address)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm"));
    if page && !carries_a_secret(query) {
        let host = address.split('/').next().unwrap_or_default();
        return Some(LinkRef {
            kind: LinkKind::Report,
            label: "Report".to_owned(),
            title: host.to_owned(),
            url: url.to_owned(),
        });
    }
    None
}

/// `ABC-123`: letters and digits starting with a letter, a dash, digits.
fn is_ticket_key(key: &str) -> bool {
    let Some((team, number)) = key.split_once('-') else {
        return false;
    };
    team.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && team.chars().all(|c| c.is_ascii_alphanumeric())
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
}

/// Whether a query names a token, key, code or signature: such a link is not shown as a chip.
fn carries_a_secret(query: &str) -> bool {
    const SECRET: &[&str] = &[
        "t",
        "token",
        "key",
        "code",
        "secret",
        "sig",
        "signature",
        "access_token",
    ];
    query
        .trim_start_matches(['?', '#'])
        .split(['&', '?', '#'])
        .filter_map(|pair| pair.split_once('=').map(|(name, _)| name.to_lowercase()))
        .any(|name| SECRET.contains(&name.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_requests_tickets_and_reports_become_chips() {
        let text = "Opened https://github.com/acme/site/pull/12. Ticket \
            https://linear.app/acme/issue/eng-42/fix-it and the report \
            (https://reports.example.com/run/summary.html), see https://example.com/docs.";
        let refs = link_refs(text);
        let chips: Vec<(LinkKind, &str, &str)> = refs
            .iter()
            .map(|r| (r.kind, r.label.as_str(), r.title.as_str()))
            .collect();
        assert_eq!(
            chips,
            [
                (LinkKind::PullRequest, "PR #12", "acme/site"),
                (LinkKind::Ticket, "ENG-42", "Linear"),
                (LinkKind::Report, "Report", "reports.example.com"),
            ]
        );
        assert_eq!(refs[0].url, "https://github.com/acme/site/pull/12");
        assert_eq!(refs[2].url, "https://reports.example.com/run/summary.html");
    }

    #[test]
    fn a_link_with_a_secret_twice_or_of_no_kind_is_no_chip() {
        assert_eq!(
            link_refs("https://example.com/r.html?token=abc https://example.com/a"),
            []
        );
        let twice =
            link_refs("https://github.com/a/b/pull/1 and https://github.com/a/b/pull/1/files");
        assert_eq!(twice.len(), 1);
        assert_eq!(link_refs("no links here"), []);
        let many = (1..=10)
            .map(|n| format!("https://github.com/a/b/pull/{n}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(link_refs(&many).len(), MAX_REFS);
    }
}
