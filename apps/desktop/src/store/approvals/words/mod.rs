//! How approval requests read, a module per part: who and what a request names
//! ([`names`]), its payload ([`payload`]), what the user is asked ([`asking`]), why an answer
//! failed ([`failure`](mod@failure)) and an agent's lines ([`agents`]). Pure functions over the contract's
//! types, tested on their own. Every word a request or an agent brings (names, targets, reasons,
//! an unknown kind) goes through [`visible`](super::shown::visible), so nothing in it hides.

mod agents;
mod asking;
mod failure;
mod names;
mod payload;

pub(crate) use agents::{agent_caption, agent_line, approvals_count};
pub(crate) use asking::{approve_question, decided, withdraw_question};
pub(crate) use failure::{failure, read_one_problem, read_problem};
pub(crate) use names::{agent_name, approver_role, kind_name, state_name};
pub(crate) use payload::{amount_text, file_line, outcome_lines, payload_foot, row_facts};

#[cfg(test)]
pub(crate) mod tests {
    use emberhut_api::approvals::{Approval, ApprovalFile};
    use jiff::Timestamp;
    use serde_json::json;

    use super::*;

    /// A request on the wire, with `more` over its fields.
    pub(crate) fn approval(more: &serde_json::Value) -> Approval {
        let mut base = json!({
            "id": 7, "agent": "client-a", "agentName": "Example helper", "kind": "post",
            "target": "example blog", "summary": "The launch post", "text": "We shipped it.",
            "files": [], "payloadHash": "CD".repeat(32), "approver": "owner",
            "waitsOnOwner": true, "state": "pending",
            "createdAt": "2026-11-27T10:00:00.000Z", "expiresAt": "2026-11-30T10:00:00.000Z"
        });
        if let (Some(base), Some(more)) = (base.as_object_mut(), more.as_object()) {
            base.extend(more.clone());
        }
        serde_json::from_value(base).unwrap()
    }

    /// The tests' now: two hours after [`approval`]'s request was made.
    pub(crate) fn now() -> Timestamp {
        "2026-11-27T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn what_would_hide_in_a_requests_words_is_written_out() {
        let tricky = approval(&json!({
            "agentName": "Help\u{200B}er", "target": "example.com/\u{202E}gol",
            "approver": "lead", "approverName": "Le\u{2066}ad", "state": "rejected",
            "stateReason": "no\u{FEFF}pe"
        }));
        let (_, words) = approve_question(&tricky, &[]);
        assert!(
            words.starts_with("Help[U+200B]er may then do it once, to example.com/[U+202E]gol,"),
            "{words}"
        );
        assert_eq!(approver_role(&tricky), "Le[U+2066]ad, the approver");
        assert_eq!(outcome_lines(&tricky, now())[0], "Rejected: no[U+FEFF]pe");
        let file = ApprovalFile {
            name: "invoice\u{202E}fdp.exe".to_owned(),
            size: Some(1),
            sha256: "ab".repeat(32),
        };
        assert_eq!(
            file_line(&file),
            "invoice[U+202E]fdp.exe · 1 B · abababababab"
        );
    }
}
