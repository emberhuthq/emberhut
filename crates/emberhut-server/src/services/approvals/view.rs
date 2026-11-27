//! Requests as the contract shows them (`emberhut_api::approvals::Approval`), with the agents' and
//! approvers' names.

use std::collections::HashMap;

use emberhut_api::approvals as api;
use emberhut_core::approvals::{ApprovalRequest, Approver, RequestState};
use jiff::Timestamp;

use crate::convert::timestamp_to_api;

/// The names a request is shown with: each agent's and each external approver's, by id.
#[derive(Clone, Debug, Default)]
pub(crate) struct Names {
    /// Agents' names by id.
    pub(crate) agents: HashMap<String, String>,
    /// Approvers' names by id.
    pub(crate) approvers: HashMap<String, String>,
    /// Agents' spending currencies by id, for those with a cap.
    pub(crate) currencies: HashMap<String, String>,
}

/// `request` as the contract shows it at `now`.
pub(crate) fn approval(request: &ApprovalRequest, names: &Names, now: Timestamp) -> api::Approval {
    let approver_name = match &request.approver {
        Approver::External(id) => names.approvers.get(id.as_str()).cloned(),
        Approver::Owner | Approver::Nobody => None,
    };
    let decided = request.decided.as_ref();
    api::Approval {
        id: emberhut_api::ids::ApprovalId::new(request.id.get()),
        agent: emberhut_api::ids::ConfiguredAgentId::new(request.agent.as_str()),
        agent_name: names.agents.get(request.agent.as_str()).cloned(),
        kind: api::ActionKind::from(request.kind.as_str()),
        target: request.target.clone(),
        summary: request.summary.clone(),
        text: request.text.clone(),
        files: request
            .files
            .iter()
            .map(|file| api::ApprovalFile {
                name: file.name.clone(),
                size: file.size,
                sha256: file.sha256.to_hex(),
            })
            .collect(),
        payload_hash: request.payload_hash.to_hex(),
        approver: request.approver.as_str().into_owned(),
        approver_name,
        waits_on_owner: request.approver == Approver::Owner && request.is_open(now),
        amount_cents: request.amount_cents,
        amount_currency: request
            .amount_cents
            .and_then(|_| names.currencies.get(request.agent.as_str()).cloned()),
        task: request
            .task
            .map(|task| emberhut_api::ids::TaskId::new(task.get())),
        state: state(request.state),
        state_reason: request.state_reason.clone(),
        created_at: timestamp_to_api(request.created_at),
        expires_at: request.expires_at().map(timestamp_to_api),
        decided_at: decided.map(|d| timestamp_to_api(d.at)),
        decided_by: decided.map(|d| d.by.clone()),
        decided_via: decided.map(|d| api::DecidedVia::from(d.via.as_str())),
        used_at: request.used.as_ref().map(|u| timestamp_to_api(u.at)),
        used_by: request.used.as_ref().map(|u| u.by.clone()),
        imported: request.source_ref.is_some(),
    }
}

/// A request's state as the contract writes it.
pub(crate) fn state(state: RequestState) -> api::ApprovalState {
    api::ApprovalState::from(state.as_str())
}
