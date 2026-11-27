//! Approvals: every OK an agent's outward step needs, with the exact payload it covers.
//!
//! - `GET /api/approvals?state=&agent=` ([`ApprovalsQuery`], [`ApprovalsPage`]): what waits on
//!   the owner, and what waits on others, for information;
//! - `GET /api/approvals/{id}` ([`ApprovalReply`]);
//! - `POST /api/approvals/{id}/decision` ([`DecisionRequest`]): the owner's answer, from a
//!   signed-in device, on the payload hash they were shown;
//! - `POST /api/approvals/{id}/withdraw` ([`WithdrawRequest`]);
//! - `POST /api/approvals/{id}/use` ([`UseRequest`], [`UseReply`]): an agent's MCP server uses
//!   an approval before its step, with the agent's room token;
//! - `POST /api/approvals/{id}/webhook` ([`WebhookDecision`]): an approver's own app answers,
//!   signed with the approver's secret;
//! - `POST /api/approvals/slack`: Slack's interactivity call, signed by Slack (its own form
//!   body, not modelled here);
//! - `GET`/`POST /approve/{token}`: the one-time signed link's page and its form
//!   ([`DecisionRequest`]).
//!
//! The live stream sends an [`APPROVALS_EVENT`] event ([`ApprovalsEvent`]) when a request
//! changes.

use serde::{Deserialize, Serialize};

use crate::ids::{ApprovalId, ConfiguredAgentId, TaskId};
use crate::open_enum::open_enum;
use crate::time::Timestamp;

/// The live stream's event name for a change to an approval request.
pub const APPROVALS_EVENT: &str = "approvals";

open_enum! {
    /// What an approval is for. The last five are the fixed core: always the owner's.
    pub enum ActionKind {
        /// Publishing: a post, a comment, a public page.
        Post = "post",
        /// A message to people outside the agent's own conversations.
        Send = "send",
        /// A merge.
        Merge = "merge",
        /// A deploy.
        Deploy = "deploy",
        /// An account, a record or a resource made in another system.
        Create = "create",
        /// Anything else outward (`other`; named apart from [`ActionKind::Other`], a word this
        /// contract doesn't know).
        OtherOutward = "other",
        /// Money moves or is committed.
        Payment = "payment",
        /// A password, a code, a key.
        Credential = "credential",
        /// Reading or handing over a secret.
        Secret = "secret",
        /// Sign-in, recovery, two-factor, closing an account, permissions.
        AccountSecurity = "account_security",
        /// Anything said or sent in the owner's name.
        AsOwner = "as_owner",
    }
}

impl ActionKind {
    /// Whether this kind is the fixed core's: only the owner approves it, only in the apps.
    #[must_use]
    pub fn is_fixed_core(&self) -> bool {
        matches!(
            self,
            Self::Payment | Self::Credential | Self::Secret | Self::AccountSecurity | Self::AsOwner
        )
    }
}

open_enum! {
    /// Where a request is in its life.
    pub enum ApprovalState {
        /// Waiting for its approver.
        Pending = "pending",
        /// Approved, not used yet.
        Approved = "approved",
        /// Rejected.
        Rejected = "rejected",
        /// Approved and used once.
        Used = "used",
        /// Unanswered for 3 days, or unused for 24 hours after its approval.
        Expired = "expired",
        /// Taken back by the agent or the owner.
        Withdrawn = "withdrawn",
    }
}

open_enum! {
    /// The channel a decision came through.
    pub enum DecidedVia {
        /// The apps, from a signed-in device.
        App = "app",
        /// A button in the approver's Slack DM.
        Slack = "slack",
        /// A one-time signed link.
        Link = "link",
        /// The approver's own app.
        Webhook = "webhook",
    }
}

/// An approver's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema), schema(as = ApprovalDecision))]
pub enum Decision {
    /// Yes.
    Approve,
    /// No.
    Reject,
}

/// One file of a request, as the server read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ApprovalFile {
    /// Its name.
    pub name: String,
    /// Its size in bytes; none for an imported file that lacks it.
    #[serde(default)]
    pub size: Option<u64>,
    /// The SHA-256 of its bytes, 64 hex digits.
    pub sha256: String,
}

/// An approval request, with the exact payload it covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Approval {
    /// Its id.
    pub id: ApprovalId,
    /// The agent that acts.
    pub agent: ConfiguredAgentId,
    /// The agent's name.
    #[serde(default)]
    pub agent_name: Option<String>,
    /// What it is for.
    pub kind: ActionKind,
    /// Where it goes.
    pub target: String,
    /// One line of the agent's.
    pub summary: String,
    /// The exact words, if any.
    #[serde(default)]
    pub text: Option<String>,
    /// Its files.
    #[serde(default)]
    pub files: Vec<ApprovalFile>,
    /// The payload's hash, 64 hex digits: a decision carries it back.
    pub payload_hash: String,
    /// `owner`, or the external approver's id.
    pub approver: String,
    /// The external approver's name.
    #[serde(default)]
    pub approver_name: Option<String>,
    /// It waits on the owner: the apps offer Approve and Reject (else it is shown read-only).
    #[serde(default)]
    pub waits_on_owner: bool,
    /// A payment's amount, in cents.
    #[serde(default)]
    pub amount_cents: Option<u64>,
    /// The amount's currency (the agent's spending currency, `EUR`), set when `amountCents`
    /// is and the agent has one, so an amount reads correctly on its own.
    #[serde(default)]
    pub amount_currency: Option<String>,
    /// The task whose step asked.
    #[serde(default)]
    pub task: Option<TaskId>,
    /// Where it is in its life.
    pub state: ApprovalState,
    /// Why it was expired, withdrawn or rejected.
    #[serde(default)]
    pub state_reason: Option<String>,
    /// When it was made.
    pub created_at: Timestamp,
    /// When it stops waiting (for its answer, or to be used); none once it can't.
    #[serde(default)]
    pub expires_at: Option<Timestamp>,
    /// When it was decided.
    #[serde(default)]
    pub decided_at: Option<Timestamp>,
    /// Who decided, as the channel proved it.
    #[serde(default)]
    pub decided_by: Option<String>,
    /// Through which channel.
    #[serde(default)]
    pub decided_via: Option<DecidedVia>,
    /// When it was used.
    #[serde(default)]
    pub used_at: Option<Timestamp>,
    /// The task or tool that used it.
    #[serde(default)]
    pub used_by: Option<String>,
    /// It came from an earlier installation's history: it is over, and never usable.
    #[serde(default)]
    pub imported: bool,
}

/// The query of `GET /api/approvals`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema, utoipa::IntoParams), into_params(parameter_in = Query))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct ApprovalsQuery {
    /// Only those in this state; none: all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub state: Option<ApprovalState>,
    /// Only this agent's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub agent: Option<ConfiguredAgentId>,
}

impl ApprovalsQuery {
    /// The query string, without the `?`: `state=pending&agent=client-a`, or empty for all.
    #[must_use]
    pub fn to_query(&self) -> String {
        let mut parts = Vec::new();
        if let Some(state) = &self.state {
            parts.push(format!("state={}", query_word(state.as_str())));
        }
        if let Some(agent) = &self.agent {
            parts.push(format!("agent={}", query_word(agent.as_str())));
        }
        parts.join("&")
    }
}

/// `word` with every byte but `a-z`, `A-Z`, `0-9`, `-` and `_` percent-encoded.
fn query_word(word: &str) -> String {
    word.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// `GET /api/approvals`: the requests, newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ApprovalsPage {
    /// The requests.
    pub approvals: Vec<Approval>,
}

/// One request, as it is now: `GET /api/approvals/{id}`, and the answer to a decision or a
/// withdrawal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ApprovalReply {
    /// The request.
    pub approval: Approval,
}

/// `POST /api/approvals/{id}/decision` (and the signed link's form): yes or no, on the payload
/// hash the approver was shown. A hash that is not the request's decides nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct DecisionRequest {
    /// Yes or no.
    #[cfg_attr(feature = "validate", garde(skip))]
    pub decision: Decision,
    /// The payload hash shown, 64 hex digits.
    #[cfg_attr(feature = "validate", garde(custom(crate::rules::sha256_hex)))]
    pub payload_hash: String,
    /// Why, if the approver says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(length(chars, max = 500)))]
    pub reason: Option<String>,
}

/// `POST /api/approvals/{id}/webhook`: an approver's own app answers, signed with the
/// approver's secret (`x-emberhut-timestamp`, `x-emberhut-signature`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct WebhookDecision {
    /// Yes or no.
    #[cfg_attr(feature = "validate", garde(skip))]
    pub decision: Decision,
    /// The payload hash the app showed, 64 hex digits.
    #[cfg_attr(feature = "validate", garde(custom(crate::rules::sha256_hex)))]
    pub payload_hash: String,
    /// Who decided, as the app reports it.
    #[cfg_attr(
        feature = "validate",
        garde(custom(crate::rules::not_blank), length(chars, max = 200))
    )]
    pub by: String,
}

/// `POST /api/approvals/{id}/withdraw`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct WithdrawRequest {
    /// Why, if given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(length(chars, max = 500)))]
    pub reason: Option<String>,
}

/// `POST /api/approvals/{id}/use`: the step about to go out. The server hashes it again (the
/// request's agent, kind, target and amount with this text and these files' bytes, read now),
/// so an approval covers only the step that was approved.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct UseRequest {
    /// The step's exact words, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(length(chars, max = 40_000)))]
    pub text: Option<String>,
    /// The paths of the step's files, read by the server.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(
        feature = "validate",
        garde(length(max = 20), inner(length(min = 1, max = 4096)))
    )]
    pub files: Vec<String>,
}

/// What `POST /api/approvals/{id}/use` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UseReply {
    /// The approval was used: the step may go out.
    pub used: bool,
    /// A receipt line when used; why not otherwise.
    pub message: String,
}

/// The [`APPROVALS_EVENT`] event's data: a request changed. A client fetches what it shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ApprovalsEvent {
    /// The request that changed.
    pub id: ApprovalId,
    /// Its state now.
    pub state: ApprovalState,
    /// How many requests wait on the owner now.
    pub pending: u32,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_decision_reads_the_wire() {
        let request: DecisionRequest = serde_json::from_value(json!({
            "decision": "approve", "payloadHash": "ab".repeat(32)
        }))
        .unwrap();
        assert_eq!(request.decision, Decision::Approve);
        assert_eq!(request.reason, None);
        assert!(
            serde_json::from_value::<DecisionRequest>(
                json!({ "decision": "maybe", "payloadHash": "" })
            )
            .is_err()
        );
    }

    #[test]
    fn the_fixed_core() {
        assert!(ActionKind::AsOwner.is_fixed_core());
        assert!(!ActionKind::Post.is_fixed_core());
        assert!(!ActionKind::from("refund").is_fixed_core());
    }

    #[test]
    fn queries() {
        assert_eq!(ApprovalsQuery::default().to_query(), "");
        let query = ApprovalsQuery {
            state: Some(ApprovalState::Pending),
            agent: Some(ConfiguredAgentId::new("client-a")),
        };
        assert_eq!(query.to_query(), "state=pending&agent=client-a");
        let odd = ApprovalsQuery {
            state: None,
            agent: Some(ConfiguredAgentId::new("a&b")),
        };
        assert_eq!(odd.to_query(), "agent=a%26b");
    }

    #[test]
    fn an_approval_round_trips() {
        let body = json!({
            "id": 7, "agent": "client-a", "agentName": "Example helper", "kind": "post",
            "target": "example blog", "summary": "The launch post", "text": "We shipped it.",
            "files": [{ "name": "cover.png", "size": 9, "sha256": "ab".repeat(32) }],
            "payloadHash": "cd".repeat(32), "approver": "example-lead",
            "approverName": "Example Lead", "waitsOnOwner": false, "amountCents": null,
            "amountCurrency": null,
            "task": 12, "state": "pending", "stateReason": null,
            "createdAt": "2026-11-27T10:00:00.000Z", "expiresAt": "2026-11-30T10:00:00.000Z",
            "decidedAt": null, "decidedBy": null, "decidedVia": null, "usedAt": null,
            "usedBy": null, "imported": false
        });
        let approval: Approval = serde_json::from_value(body.clone()).unwrap();
        assert_eq!(approval.state, ApprovalState::Pending);
        assert_eq!(serde_json::to_value(&approval).unwrap(), body);
    }
}
