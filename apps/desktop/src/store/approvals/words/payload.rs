//! A request's payload in words: a file's line with its short hash, a payment's exact amount,
//! a row's facts, the foot under the payload and how it ended.
use emberhut_api::agents::AgentRow;
use emberhut_api::approvals::{Approval, ApprovalFile, ApprovalState};
use jiff::Timestamp;

use super::names::{agent_name, state_name, via_name};
use crate::format::{file_size, until};
use crate::store::approvals::shown::visible;
use crate::store::devices::relative;

/// How many hex digits of a SHA-256 are shown: enough to tell two files or payloads apart by
/// eye.
pub(crate) const SHORT_HASH: usize = 12;

/// A SHA-256 as the app shows it: its first [`SHORT_HASH`] hex digits, in lower case.
#[must_use]
pub(crate) fn short_hash(hex: &str) -> String {
    hex.chars()
        .take(SHORT_HASH)
        .collect::<String>()
        .to_lowercase()
}

/// One file's line: its name, size and short hash (`cover.png · 12 KiB · 3f2a9c0b1d4e`).
#[must_use]
pub(crate) fn file_line(file: &ApprovalFile) -> String {
    let size = file
        .size
        .map_or_else(|| "size unknown".to_owned(), file_size);
    format!(
        "{} · {size} · {}",
        visible(&file.name),
        short_hash(&file.sha256)
    )
}

/// A payment's amount, exactly: whole cents, the thousands grouped, the currency's code after
/// (`1,234.50 EUR`). Never rounded nor put in a local style: this is what the user says yes to.
#[must_use]
pub(crate) fn amount_text(cents: u64, currency: Option<&str>) -> String {
    let whole = (cents / 100).to_string();
    let mut grouped = String::with_capacity(whole.len() + whole.len() / 3);
    for (at, digit) in whole.chars().enumerate() {
        if at > 0 && (whole.len() - at).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let currency = currency.map_or_default(|code| format!(" {code}"));
    format!("{grouped}.{:02}{currency}", cents % 100)
}

/// A request's facts in its list's row: its agent, its files, when it was asked.
#[must_use]
pub(crate) fn row_facts(approval: &Approval, agents: &[AgentRow], now: Timestamp) -> String {
    let files = match approval.files.len() {
        0 => None,
        1 => Some("1 file".to_owned()),
        n => Some(format!("{n} files")),
    };
    let asked = time(&approval.created_at).map(|at| relative(at, now));
    [
        Some(agent_name(approval, agents).into_owned()),
        files,
        asked,
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

/// The line under the payload: when it was asked, when it expires while it waits, and the
/// start of the payload's hash, which the decision sends back.
#[must_use]
pub(crate) fn payload_foot(approval: &Approval, now: Timestamp) -> String {
    let asked = time(&approval.created_at).map(|at| format!("Asked {}", relative(at, now)));
    let expires = approval
        .expires_at
        .as_ref()
        .and_then(time)
        .filter(|_| approval.state == ApprovalState::Pending)
        .map(|at| format!("expires {}", until(at, now)));
    let hash = Some(format!("payload {}", short_hash(&approval.payload_hash)));
    [asked, expires, hash]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
}

/// How it ended, or where it stands, for a request no longer waiting: its state and why, who
/// decided and through what, when it was used and by what, and whether it came from an earlier
/// installation's history.
#[must_use]
pub(crate) fn outcome_lines(approval: &Approval, now: Timestamp) -> Vec<String> {
    let mut lines = vec![match &approval.state_reason {
        Some(reason) => format!("{}: {}", state_name(&approval.state), visible(reason)),
        None => state_name(&approval.state),
    }];
    if let Some(at) = approval.decided_at.as_ref().and_then(time) {
        let by = approval
            .decided_by
            .as_deref()
            .map_or_default(|by| format!(" by {}", visible(by)));
        let via = approval
            .decided_via
            .as_ref()
            .map_or_default(|via| format!(" ({})", via_name(via)));
        lines.push(format!("Decided {}{by}{via}", relative(at, now)));
    }
    if let Some(at) = approval.used_at.as_ref().and_then(time) {
        let by = approval
            .used_by
            .as_deref()
            .map_or_default(|by| format!(" by {}", visible(by)));
        lines.push(format!("Used {}{by}", relative(at, now)));
    }
    if approval.imported {
        lines.push("From an earlier installation's history: it can't be used.".to_owned());
    }
    lines
}

/// A time the server wrote, if it reads as one.
fn time(text: &emberhut_api::time::Timestamp) -> Option<Timestamp> {
    text.as_str().parse().ok()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::store::approvals::words::tests::{approval, now};

    #[test]
    fn a_files_line_has_its_name_size_and_short_hash() {
        let file = ApprovalFile {
            name: "cover.png".to_owned(),
            size: Some(12_288),
            sha256: "3F2A9C0B1D4E".to_owned() + &"0".repeat(52),
        };
        assert_eq!(file_line(&file), "cover.png · 12 KiB · 3f2a9c0b1d4e");
        let unknown = ApprovalFile { size: None, ..file };
        assert_eq!(
            file_line(&unknown),
            "cover.png · size unknown · 3f2a9c0b1d4e"
        );
        assert_eq!(short_hash("ab"), "ab");
    }

    #[test]
    fn an_amount_reads_exactly_with_its_currency() {
        assert_eq!(amount_text(123_450, Some("EUR")), "1,234.50 EUR");
        assert_eq!(amount_text(5, None), "0.05");
        assert_eq!(amount_text(100_000_000, Some("USD")), "1,000,000.00 USD");
        assert_eq!(amount_text(99_999, Some("EUR")), "999.99 EUR");
    }

    #[test]
    fn a_rows_facts_and_the_payloads_foot() {
        let two = approval(&json!({"files": [
            {"name": "a.png", "size": 1, "sha256": "ab".repeat(32)},
            {"name": "b.png", "size": 2, "sha256": "cd".repeat(32)}
        ]}));
        assert_eq!(
            row_facts(&two, &[], now()),
            "Example helper · 2 files · 2 hours ago"
        );
        assert_eq!(
            payload_foot(&two, now()),
            "Asked 2 hours ago · expires in 2 days · payload cdcdcdcdcdcd"
        );
        let used = approval(&json!({"state": "used"}));
        assert_eq!(
            payload_foot(&used, now()),
            "Asked 2 hours ago · payload cdcdcdcdcdcd",
            "only a waiting request expires"
        );
    }

    #[test]
    fn the_outcome_says_who_decided_and_how() {
        let rejected = approval(&json!({
            "state": "rejected", "stateReason": "Wrong blog",
            "decidedAt": "2026-11-27T11:00:00Z", "decidedBy": "Example Lead",
            "decidedVia": "slack", "imported": true
        }));
        assert_eq!(
            outcome_lines(&rejected, now()),
            [
                "Rejected: Wrong blog",
                "Decided 1 hour ago by Example Lead (in Slack)",
                "From an earlier installation's history: it can't be used."
            ]
        );
    }
}
