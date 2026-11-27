//! Using an approval: the one step that lets something go out, and, for a payment, the
//! spending it adds to the agent's month.

use emberhut_core::ApprovalId;
use emberhut_core::agents::AgentId;
use emberhut_core::approvals::fixed_core::effective_approver;
use emberhut_core::approvals::state::approved_cutoff;
use emberhut_core::approvals::{
    ActionKind, ApprovalRequest, MonthKey, PayloadHash, RequestState, within_cap,
};
use jiff::Timestamp;
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
};

use super::{ApprovalStore, ApprovalStoreError, rows};
use crate::agents::rows::{agent, approver_of, cents_column, corrupt};
use crate::entity::{agent_spend, approval_policy, approval_request, configured_agent};
use crate::time::millis;
use crate::{DbError, begin_write};

/// A step about to go out, as the server hashed it from the step itself (its text, and its
/// files' bytes read now), never from the request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepToUse<'a> {
    /// The agent whose step it is: the one whose room's token called.
    pub agent: &'a AgentId,
    /// The request the step names, when it names one: only it is used. None: the oldest
    /// matching approval (a guard rails' step, which names none).
    pub request: Option<ApprovalId>,
    /// The kind of action, after the fixed core's raise.
    pub kind: ActionKind,
    /// The step's payload hash.
    pub payload_hash: &'a PayloadHash,
    /// The task or tool that uses it.
    pub by: &'a str,
    /// The month a payment counts in, in the server's time zone.
    pub month: &'a MonthKey,
}

/// A payment the agent's monthly cap does not leave room for.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "the payment of {amount_cents} cents is over the monthly cap: {spent_cents} spent of {cap_cents}"
)]
pub struct OverCap {
    /// The payment's amount.
    pub amount_cents: u64,
    /// What the month's payments add up to so far.
    pub spent_cents: u64,
    /// The cap; 0 when the agent has none (no spending at all).
    pub cap_cents: u64,
}

impl ApprovalStore {
    /// Uses an approval for `step` at `now`, in one transaction: the oldest request of the
    /// step's agent, kind and payload hash that is approved, unused, unexpired and live (not
    /// imported), and whose approver is still the one the agent's policy names now, becomes
    /// used, by one conditional update. A payment also needs its amount to fit the agent's
    /// monthly cap, and is recorded as spending in the same transaction.
    ///
    /// The used request; none when nothing matched, and then nothing goes out.
    ///
    /// # Errors
    ///
    /// [`ApprovalStoreError::OverCap`] for a payment over the cap (the approval stays unused),
    /// [`ApprovalStoreError::Disabled`] for a disabled agent, [`ApprovalStoreError::NoAgent`],
    /// or the database's.
    pub async fn use_approval(
        &self,
        step: &StepToUse<'_>,
        now: Timestamp,
    ) -> Result<Option<ApprovalRequest>, ApprovalStoreError> {
        let txn = begin_write(&self.conn).await?;
        let Some(agent_row) = configured_agent::Entity::find_by_id(step.agent.as_str())
            .one(&txn)
            .await?
        else {
            return Err(ApprovalStoreError::NoAgent(step.agent.clone()));
        };
        if !agent_row.enabled {
            return Err(ApprovalStoreError::Disabled(step.agent.clone()));
        }
        let policy = approval_policy::Entity::find()
            .filter(approval_policy::Column::AgentId.eq(step.agent.as_str()))
            .filter(approval_policy::Column::Kind.eq(step.kind.as_str()))
            .one(&txn)
            .await?
            .map(|row| approver_of(&row.approver, "approval_policies.approver"))
            .transpose()?;
        let approver_now = effective_approver(step.agent, step.kind, policy.as_ref())
            .as_str()
            .into_owned();
        let usable = Condition::all()
            .add(approval_request::Column::AgentId.eq(step.agent.as_str()))
            .add(approval_request::Column::Kind.eq(step.kind.as_str()))
            .add(approval_request::Column::PayloadHash.eq(step.payload_hash.as_bytes().to_vec()))
            .add(approval_request::Column::State.eq(RequestState::Approved.as_str()))
            .add(approval_request::Column::UsedAt.is_null())
            .add(approval_request::Column::SourceRef.is_null())
            .add(approval_request::Column::Approver.eq(approver_now.as_str()))
            .add(approval_request::Column::DecidedAt.gt(millis(approved_cutoff(now))));
        let usable = match step.request {
            Some(id) => usable.add(approval_request::Column::Id.eq(id.get())),
            None => usable,
        };
        let Some(candidate) = approval_request::Entity::find()
            .filter(usable.clone())
            .order_by_asc(approval_request::Column::Id)
            .one(&txn)
            .await?
        else {
            return Ok(None);
        };
        let payment = if step.kind == ActionKind::Payment {
            let amount = candidate
                .amount_cents
                .and_then(|cents| u64::try_from(cents).ok())
                .ok_or_else(|| {
                    corrupt(
                        "approval_requests.amount_cents",
                        &format!("{:?}", candidate.amount_cents),
                    )
                })?;
            let cap = agent(agent_row)?.spend_cap;
            let spent = month_spend(&txn, step.agent, step.month).await?;
            if !within_cap(spent, amount, cap.as_ref()) {
                return Err(OverCap {
                    amount_cents: amount,
                    spent_cents: spent,
                    cap_cents: cap.map_or(0, |cap| cap.cents),
                }
                .into());
            }
            Some(amount)
        } else {
            None
        };
        let used = approval_request::Entity::update_many()
            .col_expr(
                approval_request::Column::State,
                Expr::value(RequestState::Used.as_str()),
            )
            .col_expr(approval_request::Column::UsedAt, Expr::value(millis(now)))
            .col_expr(approval_request::Column::UsedBy, Expr::value(step.by))
            .filter(approval_request::Column::Id.eq(candidate.id))
            .filter(usable)
            .exec(&txn)
            .await?;
        if used.rows_affected != 1 {
            return Ok(None);
        }
        if let Some(amount) = payment {
            agent_spend::Entity::insert(agent_spend::ActiveModel {
                agent_id: Set(step.agent.as_str().to_owned()),
                request_id: Set(candidate.id),
                amount_cents: Set(cents_column(amount)),
                month: Set(step.month.as_str().to_owned()),
                at: Set(millis(now)),
                ..Default::default()
            })
            .exec(&txn)
            .await?;
        }
        let request = rows::one(&txn, candidate.id).await?;
        txn.commit().await?;
        Ok(request)
    }

    /// What the agent's used payments add up to in `month`, in cents.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn month_spend(&self, agent: &AgentId, month: &MonthKey) -> Result<u64, DbError> {
        month_spend(&self.conn, agent, month).await
    }
}

async fn month_spend(
    conn: &impl ConnectionTrait,
    agent: &AgentId,
    month: &MonthKey,
) -> Result<u64, DbError> {
    let total: Option<Option<i64>> = agent_spend::Entity::find()
        .select_only()
        .column_as(agent_spend::Column::AmountCents.sum(), "total")
        .filter(agent_spend::Column::AgentId.eq(agent.as_str()))
        .filter(agent_spend::Column::Month.eq(month.as_str()))
        .into_tuple()
        .one(conn)
        .await?;
    Ok(total
        .flatten()
        .map_or(0, |total| u64::try_from(total).unwrap_or_default()))
}
