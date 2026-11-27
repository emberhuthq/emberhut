//! `approval_requests`: every approval request, its payload and its life.
#![expect(
    clippy::unused_async_trait_impl,
    reason = "SeaORM's model macro writes loaders that await nothing for a table without relations"
)]

use sea_orm::entity::prelude::*;

/// An approval request.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "approval_requests")]
pub(crate) struct Model {
    /// Its id.
    #[sea_orm(primary_key)]
    pub(crate) id: i64,
    /// The agent that acts.
    pub(crate) agent_id: String,
    /// The kind of action, after the fixed core's raise.
    pub(crate) kind: String,
    /// Where it goes.
    pub(crate) target: String,
    /// One line of the agent's.
    pub(crate) summary: String,
    /// The exact words, if any.
    pub(crate) text: Option<String>,
    /// The payload's SHA-256.
    pub(crate) payload_hash: Vec<u8>,
    /// `owner` or the approver's id, as the policy named it when it was made.
    pub(crate) approver: String,
    /// A payment's amount, in cents.
    pub(crate) amount_cents: Option<i64>,
    /// The task whose step asked.
    pub(crate) task_id: Option<i64>,
    /// `pending`, `approved`, `rejected`, `used`, `expired` or `withdrawn`.
    pub(crate) state: String,
    /// Why it was expired, withdrawn or rejected.
    pub(crate) state_reason: Option<String>,
    /// Where an imported row came from; none for a live one.
    pub(crate) source_ref: Option<String>,
    /// When it was made.
    pub(crate) created_at: i64,
    /// `approve` or `reject`.
    pub(crate) decision: Option<String>,
    /// When it was decided.
    pub(crate) decided_at: Option<i64>,
    /// Who decided, as the channel proved it.
    pub(crate) decided_by: Option<String>,
    /// `app`, `slack`, `link` or `webhook`.
    pub(crate) decided_via: Option<String>,
    /// When it was used.
    pub(crate) used_at: Option<i64>,
    /// The task or tool that used it.
    pub(crate) used_by: Option<String>,
    /// When it expired or was withdrawn.
    pub(crate) closed_at: Option<i64>,
    /// The SHA-256 of its signed link's nonce, until the first decision.
    #[sea_orm(unique)]
    pub(crate) link_nonce_hash: Option<Vec<u8>>,
    /// The Slack DM channel it was posted in.
    pub(crate) slack_channel: Option<String>,
    /// The Slack message's timestamp.
    pub(crate) slack_ts: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
