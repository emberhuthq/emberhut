//! Findings: a scan's or a review's results, filed under it, each landed or not, and the lines
//! a run reports them in (`FINDING: <title> | <state> | <where and what>`).

use std::sync::LazyLock;

use regex::Regex;

use super::{Landed, TaskId, wire};

/// One finding to file or update under a scan: found by id, else by its title there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindingIn {
    /// The finding, when it is known.
    pub id: Option<TaskId>,
    /// Its title.
    pub title: Option<String>,
    /// What and where, a few lines.
    pub body: Option<String>,
    /// Its state.
    pub landed: Option<Landed>,
    /// The task that fixes it.
    pub fixed_by: Option<TaskId>,
    /// `fixed_by` was given (a `null` clears it).
    pub fixed_by_given: bool,
    /// It was filed twice: the finding it repeats (it is put away).
    pub duplicate_of: Option<TaskId>,
}

/// A state in plain words ("not landed", "Partly landed", "Landed ✅") as its stored word;
/// none for words that aren't a state.
#[must_use]
pub fn landed_of(words: &str) -> Option<Landed> {
    let w: String = words
        .to_lowercase()
        .replace(['_', '-'], " ")
        .chars()
        .filter(|c| c.is_ascii_lowercase() || *c == ' ')
        .collect();
    let w = w.trim();
    if w.starts_with("not landed")
        || w.starts_with("notlanded")
        || matches!(w, "open" | "missing" | "not fixed")
    {
        Some(Landed::NotLanded)
    } else if w.starts_with("partly") || w.starts_with("partial") {
        Some(Landed::Partly)
    } else if w.starts_with("landed") || matches!(w, "fixed" | "done") {
        Some(Landed::Landed)
    } else if w.starts_with("superseded") || matches!(w, "moot" | "dropped") {
        Some(Landed::Superseded)
    } else if w.starts_with("unchecked") || matches!(w, "unknown" | "not checked") {
        Some(Landed::Unchecked)
    } else {
        None
    }
}

/// Two titles are the same finding: the same words, any case and spacing.
#[must_use]
pub fn same_title(a: &str, b: &str) -> bool {
    let squash = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    squash(a) == squash(b)
}

/// A pattern of this module's: none if it didn't compile (its tests say so), and then no line
/// reads as a finding.
fn rx(pattern: &str) -> Option<Regex> {
    Regex::new(pattern).ok()
}

static LINE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    rx(&format!(
        r"(?im)^[\s>*_`-]*FINDING(?:\s+{}(\d+))?[*_`]*:[*_`]*\s*([^\n]+)$",
        wire::KEY_PREFIX
    ))
});
static KEYED: LazyLock<Option<Regex>> = LazyLock::new(|| {
    rx(&format!(
        r"(?i)^{}(\d+)\b\s*[:—–-]?\s*(.*)$",
        wire::KEY_PREFIX
    ))
});

/// Markdown's marks off a line, but not the underscores inside words (`not_landed`).
fn unmark(line: &str) -> String {
    let chars: Vec<char> = line.chars().filter(|c| *c != '*' && *c != '`').collect();
    let mut out = String::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '_' {
            let start = i;
            while i < chars.len() && chars[i] == '_' {
                i += 1;
            }
            let before = start
                .checked_sub(1)
                .and_then(|b| chars.get(b))
                .is_some_and(char::is_ascii_alphanumeric);
            let after = chars.get(i).is_some_and(char::is_ascii_alphanumeric);
            if before && after {
                out.extend(&chars[start..i]);
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// A run's `FINDING` lines. A finding filed already is named either way:
/// `FINDING <key>: <title> | ...` or `FINDING: <key> | <state> | ...`; a new one gets
/// `unchecked` when its line names no state.
#[must_use]
pub fn parse_findings(text: &str) -> Vec<FindingIn> {
    let mut found = Vec::new();
    let Some(lines) = LINE.as_ref() else {
        return found;
    };
    for caps in lines.captures_iter(text) {
        let mut id = caps
            .get(1)
            .and_then(|m| m.as_str().parse::<i64>().ok())
            .map(TaskId::new);
        let line = unmark(&caps[2]);
        let mut parts: Vec<String> = line.split('|').map(|p| p.trim().to_owned()).collect();
        if parts[0].is_empty() && id.is_none() {
            continue;
        }
        if id.is_none()
            && let Some(keyed) = KEYED.as_ref().and_then(|k| k.captures(&parts[0]))
        {
            id = keyed[1].parse::<i64>().ok().map(TaskId::new);
            let title = keyed[2].trim().to_owned();
            parts[0] = title;
        }
        let landed = parts.get(1).and_then(|p| landed_of(p));
        let skip = if landed.is_some() { 2 } else { 1 };
        let detail = parts
            .iter()
            .skip(skip)
            .cloned()
            .collect::<Vec<_>>()
            .join(" | ");
        let detail = detail.trim();
        found.push(FindingIn {
            id,
            title: (!parts[0].is_empty()).then(|| parts[0].chars().take(200).collect()),
            body: (!detail.is_empty()).then(|| detail.to_owned()),
            landed: if id.is_some() {
                landed
            } else {
                Some(landed.unwrap_or(Landed::Unchecked))
            },
            ..FindingIn::default()
        });
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::wire::KEY_PREFIX;

    #[test]
    fn states_are_read_from_plain_words() {
        assert_eq!(landed_of("not landed"), Some(Landed::NotLanded));
        assert_eq!(landed_of("Not_Landed"), Some(Landed::NotLanded));
        assert_eq!(landed_of("Partly landed"), Some(Landed::Partly));
        assert_eq!(landed_of("Landed ✅"), Some(Landed::Landed));
        assert_eq!(landed_of("moot"), Some(Landed::Superseded));
        assert_eq!(landed_of("unknown"), Some(Landed::Unchecked));
        assert_eq!(landed_of("in src/main.rs"), None);
    }

    #[test]
    fn every_pattern_compiles() {
        assert!(LINE.is_some(), "LINE");
        assert!(KEYED.is_some(), "KEYED");
    }

    #[test]
    fn finding_lines_are_read_new_and_named() {
        let text = [
            "Report.",
            "- **FINDING:** Leak in the cache | not landed | src/cache.rs keeps K8S_ROUTER_URL",
            &format!("FINDING {KEY_PREFIX}12: Old title | landed | fixed in v2"),
            &format!("FINDING: {KEY_PREFIX}13 | partly"),
            "FINDING: Loose end with no state",
            "FINDING:",
        ]
        .join("\n");
        let found = parse_findings(&text);
        assert_eq!(found.len(), 4, "{found:?}");
        assert_eq!(found[0].title.as_deref(), Some("Leak in the cache"));
        assert_eq!(found[0].landed, Some(Landed::NotLanded));
        assert_eq!(
            found[0].body.as_deref(),
            Some("src/cache.rs keeps K8S_ROUTER_URL")
        );
        assert_eq!(found[1].id, Some(TaskId::new(12)));
        assert_eq!(found[1].landed, Some(Landed::Landed));
        assert_eq!(found[2].id, Some(TaskId::new(13)));
        assert_eq!(found[2].title, None);
        assert_eq!(found[2].landed, Some(Landed::Partly));
        assert_eq!(found[3].landed, Some(Landed::Unchecked));
        assert!(same_title(" Leak  in the CACHE", "leak in the cache"));
    }
}
