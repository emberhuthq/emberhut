//! Who and what a request names, in words: its kind and state, the channel it was decided
//! through, the agent's name and who decides it.
use std::borrow::Cow;

use emberhut_api::agents::AgentRow;
use emberhut_api::approvals::{ActionKind, Approval, ApprovalState, DecidedVia};

use crate::store::approvals::shown::visible;

/// The approver of a request only the user decides: the wire's word for the owner.
const OWNER: &str = "owner";

/// What a request is for, in a word; a kind this app doesn't know, as the server spells it
/// (nothing in it hidden).
#[must_use]
pub(crate) fn kind_name(kind: &ActionKind) -> String {
    match kind {
        ActionKind::Post => "Post".to_owned(),
        ActionKind::Send => "Message".to_owned(),
        ActionKind::Merge => "Merge".to_owned(),
        ActionKind::Deploy => "Deploy".to_owned(),
        ActionKind::Create => "Create".to_owned(),
        ActionKind::OtherOutward => "Other".to_owned(),
        ActionKind::Payment => "Payment".to_owned(),
        ActionKind::Credential => "Password or code".to_owned(),
        ActionKind::Secret => "Secret".to_owned(),
        ActionKind::AccountSecurity => "Account security".to_owned(),
        ActionKind::AsOwner => "In your name".to_owned(),
        other => visible(&crate::format::capitalized(
            &other.as_str().replace('_', " "),
        ))
        .into_owned(),
    }
}

/// Where a request is in its life, in a word.
#[must_use]
pub(crate) fn state_name(state: &ApprovalState) -> String {
    match state {
        ApprovalState::Pending => "Waiting".to_owned(),
        ApprovalState::Approved => "Approved".to_owned(),
        ApprovalState::Rejected => "Rejected".to_owned(),
        ApprovalState::Used => "Used".to_owned(),
        ApprovalState::Expired => "Expired".to_owned(),
        ApprovalState::Withdrawn => "Withdrawn".to_owned(),
        other => crate::format::capitalized(other.as_str()),
    }
}

/// The channel a decision came through, in a word.
pub(super) fn via_name(via: &DecidedVia) -> String {
    match via {
        DecidedVia::App => "in the app".to_owned(),
        DecidedVia::Slack => "in Slack".to_owned(),
        DecidedVia::Link => "through a link".to_owned(),
        DecidedVia::Webhook => "in their own app".to_owned(),
        other => other.as_str().to_owned(),
    }
}

/// The agent's name, nothing in it hidden: the request's own, the state's agent's, else its id.
#[must_use]
pub(crate) fn agent_name<'a>(approval: &'a Approval, agents: &'a [AgentRow]) -> Cow<'a, str> {
    let name = approval
        .agent_name
        .as_deref()
        .or_else(|| {
            agents
                .iter()
                .find(|agent| agent.id == approval.agent)
                .map(|agent| agent.name.as_str())
        })
        .unwrap_or(approval.agent.as_str());
    visible(name)
}

/// Who decides it: you, or the approver by name, as a role.
#[must_use]
pub(crate) fn approver_role(approval: &Approval) -> String {
    if approval.approver == OWNER {
        return "You".to_owned();
    }
    let name = approval
        .approver_name
        .as_deref()
        .unwrap_or(&approval.approver);
    format!("{}, the approver", visible(name))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::store::approvals::words::tests::approval;
    use crate::store::approvals::words::{agent_caption, agent_line};

    #[test]
    fn kinds_and_states_have_their_words() {
        assert_eq!(kind_name(&ActionKind::Send), "Message");
        assert_eq!(kind_name(&ActionKind::AsOwner), "In your name");
        assert_eq!(kind_name(&ActionKind::from("data_export")), "Data export");
        assert_eq!(
            kind_name(&ActionKind::from("re\u{202E}fund")),
            "Re[U+202E]fund"
        );
        assert_eq!(state_name(&ApprovalState::Pending), "Waiting");
        assert_eq!(state_name(&ApprovalState::from("paused")), "Paused");
    }

    #[test]
    fn who_decides_reads_as_a_role() {
        assert_eq!(approver_role(&approval(&json!({}))), "You");
        let other = approval(&json!({"approver": "example-lead", "approverName": "Example Lead"}));
        assert_eq!(approver_role(&other), "Example Lead, the approver");
        let unnamed = approval(&json!({"approver": "example-lead"}));
        assert_eq!(approver_role(&unnamed), "example-lead, the approver");
    }

    #[test]
    fn the_agent_is_named_by_the_request_then_the_state() {
        let agents = vec![AgentRow {
            id: emberhut_api::ids::ConfiguredAgentId::new("client-a"),
            name: "Client A".to_owned(),
            principal: "Example Ltd".to_owned(),
            room: emberhut_api::ids::RoomId::new(9),
            enabled: true,
            pending_approvals: 1,
        }];
        assert_eq!(agent_name(&approval(&json!({})), &agents), "Example helper");
        let bare = approval(&json!({"agentName": null}));
        assert_eq!(agent_name(&bare, &agents), "Client A");
        assert_eq!(agent_name(&bare, &[]), "client-a");
        assert_eq!(agent_line(&agents[0]), "Works for Example Ltd");
        let off = AgentRow {
            enabled: false,
            ..agents[0].clone()
        };
        assert_eq!(agent_line(&off), "Works for Example Ltd · Off");
        assert_eq!(
            agent_caption(&off, "Working"),
            "Off · works for Example Ltd"
        );
        assert_eq!(
            agent_caption(&agents[0], "Working"),
            "Working · works for Example Ltd"
        );
    }
}
