//! What the user is asked before an answer, and what an answer that went through did.
use emberhut_api::agents::AgentRow;
use emberhut_api::approvals::{Approval, Decision};

use super::names::{agent_name, kind_name};
use super::payload::amount_text;
use crate::store::approvals::shown::visible;

/// The question before an Approve, naming the agent, what it does, for how much and where.
#[must_use]
pub(crate) fn approve_question(approval: &Approval, agents: &[AgentRow]) -> (String, String) {
    let amount = approval.amount_cents.map_or_default(|cents| {
        format!(
            " for {}",
            amount_text(cents, approval.amount_currency.as_deref())
        )
    });
    (
        format!("Approve this {}?", kind_name(&approval.kind).to_lowercase()),
        format!(
            "{} may then do it once{amount}, to {}, exactly as shown. Anything changed after \
             this needs a new OK.",
            agent_name(approval, agents),
            visible(&approval.target)
        ),
    )
}

/// The question before a withdrawal, and what it says.
#[must_use]
pub(crate) fn withdraw_question() -> (String, String) {
    (
        "Withdraw this request?".to_owned(),
        "Nothing it covers goes out. The agent has to ask again.".to_owned(),
    )
}

/// What a decision that went through did, for the line that says so.
#[must_use]
pub(crate) fn decided(decision: Option<Decision>) -> &'static str {
    match decision {
        Some(Decision::Approve) => "Approved: it may go out once, exactly as shown.",
        Some(Decision::Reject) => "Rejected: nothing it covers goes out.",
        None => "Withdrawn: nothing it covers goes out.",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::store::approvals::words::tests::approval;

    #[test]
    fn the_question_names_the_agent_the_amount_and_the_target() {
        let payment = approval(&json!({
            "kind": "payment", "target": "example-hosting.test",
            "amountCents": 4_900, "amountCurrency": "EUR"
        }));
        let (title, words) = approve_question(&payment, &[]);
        assert_eq!(title, "Approve this payment?");
        assert_eq!(
            words,
            "Example helper may then do it once for 49.00 EUR, to example-hosting.test, \
             exactly as shown. Anything changed after this needs a new OK."
        );
    }
}
