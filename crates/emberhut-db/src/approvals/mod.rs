//! Approval requests: made (or found, when the same payload is still pending), decided once,
//! used once, withdrawn, expired, and imported from an earlier installation's history.
//!
//! Every change of state is one conditional update that names the state it moves from, so of
//! two callers racing for the same move exactly one changes the row; the others are told
//! nothing changed.

pub(crate) mod rows;
mod use_approval;

use emberhut_core::agents::AgentId;
use emberhut_core::approvals::state::{approved_cutoff, pending_cutoff};
use emberhut_core::approvals::{
    ApprovalFile, ApprovalRequest, Approver, DecidedVia, Decision, ImportedApproval, NewApproval,
    PayloadHash, RequestError, RequestState, SlackPost,
};
use emberhut_core::task::TaskId;
use emberhut_core::{ApprovalId, SecretHash};
use jiff::Timestamp;
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect,
};

pub use use_approval::{OverCap, StepToUse};

use crate::agents::rows::cents_column;
use crate::entity::{approval_file, approval_request, configured_agent};
use crate::time::millis;
use crate::{DbError, begin_write};

/// What [`ApprovalStore::expire_due`] writes as the reason a pending request expired.
pub const UNANSWERED: &str = "unanswered after 3 days";
/// What [`ApprovalStore::expire_due`] writes as the reason an approval expired.
pub const UNUSED: &str = "approved, unused after 24 hours";

/// Why a request could not be made, imported or used.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ApprovalStoreError {
    /// The request is not what a request may be.
    #[error(transparent)]
    Invalid(#[from] RequestError),
    /// There is no such agent.
    #[error("no agent {0}")]
    NoAgent(AgentId),
    /// A payment the agent's monthly cap does not leave room for; the approval stays unused.
    #[error(transparent)]
    OverCap(#[from] OverCap),
    /// The agent is disabled: it asks for and uses no approval.
    #[error("agent {0} is disabled")]
    Disabled(AgentId),
    /// The database failed.
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<DbErr> for ApprovalStoreError {
    fn from(error: DbErr) -> Self {
        Self::Db(DbError::Query(error))
    }
}

/// Which requests [`ApprovalStore::list`] reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApprovalFilter {
    /// Only those in this state.
    pub state: Option<RequestState>,
    /// Only this agent's.
    pub agent: Option<AgentId>,
    /// At most this many; none: [`ApprovalFilter::DEFAULT_LIMIT`].
    pub limit: Option<u64>,
}

impl ApprovalFilter {
    /// How many [`ApprovalStore::list`] reads when the filter doesn't say.
    pub const DEFAULT_LIMIT: u64 = 200;
}

/// A decision, as a channel proved it: what [`ApprovalStore::decide`] records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecisionMade<'a> {
    /// The request.
    pub id: ApprovalId,
    /// The payload hash the approver was shown and decided on.
    pub payload_hash: &'a PayloadHash,
    /// Who decides, as the channel proved it: the owner from a signed-in device, or the
    /// external approver whose Slack user, link or signature it was.
    pub approver: &'a Approver,
    /// Yes or no.
    pub decision: Decision,
    /// Who, in words the history keeps (a device, a Slack user, what the approver's app said).
    pub by: &'a str,
    /// Through which channel.
    pub via: DecidedVia,
    /// Why, when the approver said (kept as the request's state reason).
    pub reason: Option<&'a str>,
}

/// Reads and writes approval requests.
#[derive(Clone, Debug)]
pub struct ApprovalStore {
    conn: DatabaseConnection,
}

impl ApprovalStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Makes the request `new` at `now`, or finds the one still pending (and unexpired) with
    /// the same agent, kind, payload hash and approver. The request, and whether it was made
    /// now. One pending on another approver (the policy moved since) is not it: the approver
    /// named now is asked.
    ///
    /// # Errors
    ///
    /// [`ApprovalStoreError::Invalid`] for a request [`NewApproval::validate`] refuses,
    /// [`ApprovalStoreError::NoAgent`], or the database's.
    pub async fn create_or_find_pending(
        &self,
        new: &NewApproval,
        now: Timestamp,
    ) -> Result<(ApprovalRequest, bool), ApprovalStoreError> {
        new.validate()?;
        let hash = new.payload_hash();
        let txn = begin_write(&self.conn).await?;
        check_agent(&txn, &new.agent).await?;
        let same = approval_request::Entity::find()
            .filter(approval_request::Column::AgentId.eq(new.agent.as_str()))
            .filter(approval_request::Column::Kind.eq(new.kind.as_str()))
            .filter(approval_request::Column::PayloadHash.eq(hash.as_bytes().to_vec()))
            .filter(approval_request::Column::State.eq(RequestState::Pending.as_str()))
            .filter(approval_request::Column::Approver.eq(new.approver.as_str().into_owned()))
            .filter(approval_request::Column::CreatedAt.gt(millis(pending_cutoff(now))))
            .order_by_asc(approval_request::Column::Id)
            .one(&txn)
            .await?;
        let (id, made) = if let Some(row) = same {
            (row.id, false)
        } else {
            let row = approval_request::ActiveModel {
                agent_id: Set(new.agent.as_str().to_owned()),
                kind: Set(new.kind.as_str().to_owned()),
                target: Set(new.target.clone()),
                summary: Set(new.summary.clone()),
                text: Set(new.text.clone()),
                payload_hash: Set(hash.as_bytes().to_vec()),
                approver: Set(new.approver.as_str().into_owned()),
                amount_cents: Set(new.amount_cents.map(cents_column)),
                task_id: Set(new.task.map(TaskId::get)),
                state: Set(RequestState::Pending.as_str().to_owned()),
                created_at: Set(millis(now)),
                ..Default::default()
            };
            let id = approval_request::Entity::insert(row)
                .exec(&txn)
                .await?
                .last_insert_id;
            let files: Vec<ApprovalFile> =
                new.files.iter().cloned().map(ApprovalFile::from).collect();
            insert_files(&txn, id, &files).await?;
            (id, true)
        };
        let request = rows::one(&txn, id).await?;
        txn.commit().await?;
        let request = request.ok_or_else(|| DbErr::RecordNotFound(format!("approval {id}")))?;
        Ok((request, made))
    }

    /// Stores a request from an earlier installation's history, as it ended: it never waits
    /// for an answer, has no link, and (its hash under the import tag, its source set) can never
    /// be used.
    ///
    /// # Errors
    ///
    /// [`ApprovalStoreError::Invalid`] for a request [`ImportedApproval::validate`] refuses,
    /// [`ApprovalStoreError::NoAgent`], or the database's.
    pub async fn import(
        &self,
        imported: &ImportedApproval,
    ) -> Result<ApprovalId, ApprovalStoreError> {
        let txn = begin_write(&self.conn).await?;
        let id = import_in(&txn, imported).await?;
        txn.commit().await?;
        Ok(id)
    }

    /// The request `id`, with its files.
    ///
    /// # Errors
    ///
    /// [`DbError`], [`DbError::CorruptValue`] or [`DbError::CorruptHash`] for a corrupt row.
    pub async fn get(&self, id: ApprovalId) -> Result<Option<ApprovalRequest>, DbError> {
        rows::one(&self.conn, id.get()).await
    }

    /// The requests `filter` names, newest first.
    ///
    /// # Errors
    ///
    /// As [`ApprovalStore::get`].
    pub async fn list(&self, filter: &ApprovalFilter) -> Result<Vec<ApprovalRequest>, DbError> {
        let mut query = approval_request::Entity::find();
        if let Some(state) = filter.state {
            query = query.filter(approval_request::Column::State.eq(state.as_str()));
        }
        if let Some(agent) = &filter.agent {
            query = query.filter(approval_request::Column::AgentId.eq(agent.as_str()));
        }
        let found = query
            .order_by_desc(approval_request::Column::Id)
            .limit(filter.limit.unwrap_or(ApprovalFilter::DEFAULT_LIMIT))
            .all(&self.conn)
            .await?;
        rows::requests(&self.conn, found).await
    }

    /// How many requests wait on an answer, per agent (agents with none left out), by agent id.
    ///
    /// # Errors
    ///
    /// [`DbError`], [`DbError::CorruptValue`] for an agent id that isn't one.
    pub async fn pending_counts(&self) -> Result<Vec<(AgentId, u32)>, DbError> {
        let counts: Vec<(String, i64)> = approval_request::Entity::find()
            .select_only()
            .column(approval_request::Column::AgentId)
            .column_as(approval_request::Column::Id.count(), "pending")
            .filter(approval_request::Column::State.eq(RequestState::Pending.as_str()))
            .group_by(approval_request::Column::AgentId)
            .order_by_asc(approval_request::Column::AgentId)
            .into_tuple()
            .all(&self.conn)
            .await?;
        counts
            .into_iter()
            .map(|(agent, n)| {
                Ok((
                    crate::agents::rows::agent_id(&agent)?,
                    u32::try_from(n).unwrap_or(u32::MAX),
                ))
            })
            .collect()
    }

    /// Records `made`, in one conditional update: only a request that is still pending, not
    /// expired at `now`, live (not imported), whose payload hash is the one decided on and
    /// whose approver is the one deciding, moves to approved or rejected. Its link's nonce is
    /// cleared with it, so a link decides once. `true` when this call decided it.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn decide(&self, made: &DecisionMade<'_>, now: Timestamp) -> Result<bool, DbError> {
        let mut update = approval_request::Entity::update_many()
            .col_expr(
                approval_request::Column::State,
                Expr::value(made.decision.state().as_str()),
            )
            .col_expr(
                approval_request::Column::Decision,
                Expr::value(made.decision.as_str()),
            )
            .col_expr(
                approval_request::Column::DecidedAt,
                Expr::value(millis(now)),
            )
            .col_expr(approval_request::Column::DecidedBy, Expr::value(made.by))
            .col_expr(
                approval_request::Column::DecidedVia,
                Expr::value(made.via.as_str()),
            )
            .col_expr(
                approval_request::Column::LinkNonceHash,
                Expr::value(Option::<Vec<u8>>::None),
            );
        if let Some(reason) = made.reason {
            update = update.col_expr(approval_request::Column::StateReason, Expr::value(reason));
        }
        let done = update
            .filter(approval_request::Column::Id.eq(made.id.get()))
            .filter(approval_request::Column::State.eq(RequestState::Pending.as_str()))
            .filter(approval_request::Column::PayloadHash.eq(made.payload_hash.as_bytes().to_vec()))
            .filter(approval_request::Column::Approver.eq(made.approver.as_str().into_owned()))
            .filter(approval_request::Column::CreatedAt.gt(millis(pending_cutoff(now))))
            .filter(approval_request::Column::SourceRef.is_null())
            .exec(&self.conn)
            .await?;
        Ok(done.rows_affected == 1)
    }

    /// Keeps `nonce` (its hash) as the request's link, while it is pending: `false` when it
    /// no longer is.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn set_link_nonce(
        &self,
        id: ApprovalId,
        nonce: &SecretHash,
    ) -> Result<bool, DbError> {
        let done = approval_request::Entity::update_many()
            .col_expr(
                approval_request::Column::LinkNonceHash,
                Expr::value(nonce.as_bytes().to_vec()),
            )
            .filter(approval_request::Column::Id.eq(id.get()))
            .filter(approval_request::Column::State.eq(RequestState::Pending.as_str()))
            .exec(&self.conn)
            .await?;
        Ok(done.rows_affected == 1)
    }

    /// The request whose link's nonce has this hash, if its link still works.
    ///
    /// # Errors
    ///
    /// As [`ApprovalStore::get`].
    pub async fn by_link_nonce(
        &self,
        nonce: &SecretHash,
    ) -> Result<Option<ApprovalRequest>, DbError> {
        let Some(row) = approval_request::Entity::find()
            .filter(approval_request::Column::LinkNonceHash.eq(nonce.as_bytes().to_vec()))
            .one(&self.conn)
            .await?
        else {
            return Ok(None);
        };
        Ok(rows::requests(&self.conn, vec![row]).await?.pop())
    }

    /// Records where the request was posted to its approver in Slack. `false` when there is
    /// no such request.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn mark_shown(&self, id: ApprovalId, post: &SlackPost) -> Result<bool, DbError> {
        let done = approval_request::Entity::update_many()
            .col_expr(
                approval_request::Column::SlackChannel,
                Expr::value(post.channel.as_str()),
            )
            .col_expr(
                approval_request::Column::SlackTs,
                Expr::value(post.ts.as_str()),
            )
            .filter(approval_request::Column::Id.eq(id.get()))
            .exec(&self.conn)
            .await?;
        Ok(done.rows_affected == 1)
    }

    /// Withdraws the request, pending or approved and unused, at `now`, with `reason` when
    /// given. `agent`: only if it is this agent's (an agent withdraws its own; the owner passes
    /// none). `true` when this call withdrew it.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn withdraw(
        &self,
        id: ApprovalId,
        agent: Option<&AgentId>,
        reason: Option<&str>,
        now: Timestamp,
    ) -> Result<bool, DbError> {
        let mut update = approval_request::Entity::update_many()
            .col_expr(
                approval_request::Column::State,
                Expr::value(RequestState::Withdrawn.as_str()),
            )
            .col_expr(approval_request::Column::ClosedAt, Expr::value(millis(now)))
            .col_expr(
                approval_request::Column::LinkNonceHash,
                Expr::value(Option::<Vec<u8>>::None),
            )
            .filter(approval_request::Column::Id.eq(id.get()))
            .filter(
                approval_request::Column::State
                    .is_in(RequestState::WITHDRAWABLE.map(RequestState::as_str)),
            )
            .filter(approval_request::Column::UsedAt.is_null());
        if let Some(reason) = reason {
            update = update.col_expr(approval_request::Column::StateReason, Expr::value(reason));
        }
        if let Some(agent) = agent {
            update = update.filter(approval_request::Column::AgentId.eq(agent.as_str()));
        }
        Ok(update.exec(&self.conn).await?.rows_affected == 1)
    }

    /// Expires what is due at `now`: requests unanswered for 3 days, and approvals unused for
    /// 24 hours. The ids expired, in order, so their approvers' messages can say so.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn expire_due(&self, now: Timestamp) -> Result<Vec<ApprovalId>, DbError> {
        let unanswered = Condition::all()
            .add(approval_request::Column::State.eq(RequestState::Pending.as_str()))
            .add(approval_request::Column::CreatedAt.lte(millis(pending_cutoff(now))));
        let unused = Condition::all()
            .add(approval_request::Column::State.eq(RequestState::Approved.as_str()))
            .add(approval_request::Column::UsedAt.is_null())
            .add(approval_request::Column::DecidedAt.lte(millis(approved_cutoff(now))));
        let txn = begin_write(&self.conn).await?;
        let mut expired = Vec::new();
        for (due, reason) in [(unanswered, UNANSWERED), (unused, UNUSED)] {
            let ids: Vec<i64> = approval_request::Entity::find()
                .select_only()
                .column(approval_request::Column::Id)
                .filter(due.clone())
                .into_tuple()
                .all(&txn)
                .await?;
            if ids.is_empty() {
                continue;
            }
            approval_request::Entity::update_many()
                .col_expr(
                    approval_request::Column::State,
                    Expr::value(RequestState::Expired.as_str()),
                )
                .col_expr(approval_request::Column::StateReason, Expr::value(reason))
                .col_expr(approval_request::Column::ClosedAt, Expr::value(millis(now)))
                .col_expr(
                    approval_request::Column::LinkNonceHash,
                    Expr::value(Option::<Vec<u8>>::None),
                )
                .filter(approval_request::Column::Id.is_in(ids.clone()))
                .filter(due)
                .exec(&txn)
                .await?;
            expired.extend(ids.into_iter().map(ApprovalId::new));
        }
        txn.commit().await?;
        expired.sort_unstable();
        Ok(expired)
    }
}

/// [`ApprovalStore::import`]'s write, on `conn`: checked, then the row and its files. The
/// importer runs it inside its own one transaction.
pub(crate) async fn import_in(
    conn: &impl ConnectionTrait,
    imported: &ImportedApproval,
) -> Result<ApprovalId, ApprovalStoreError> {
    imported.validate()?;
    check_agent(conn, &imported.agent).await?;
    let decided = imported.decided.as_ref();
    let row = approval_request::ActiveModel {
        agent_id: Set(imported.agent.as_str().to_owned()),
        kind: Set(imported.kind.as_str().to_owned()),
        target: Set(imported.target.clone()),
        summary: Set(imported.summary.clone()),
        text: Set(imported.text.clone()),
        payload_hash: Set(imported.payload_hash().as_bytes().to_vec()),
        approver: Set(imported.approver.as_str().into_owned()),
        amount_cents: Set(imported.amount_cents.map(cents_column)),
        task_id: Set(imported.task.map(TaskId::get)),
        state: Set(imported.state.as_str().to_owned()),
        state_reason: Set(imported.state_reason.clone()),
        source_ref: Set(Some(imported.source_ref.clone())),
        created_at: Set(millis(imported.created_at)),
        decision: Set(decided.map(|d| d.decision.as_str().to_owned())),
        decided_at: Set(decided.map(|d| millis(d.at))),
        decided_by: Set(decided.map(|d| d.by.clone())),
        decided_via: Set(decided.map(|d| d.via.as_str().to_owned())),
        used_at: Set(imported.used.as_ref().map(|u| millis(u.at))),
        used_by: Set(imported.used.as_ref().map(|u| u.by.clone())),
        closed_at: Set(imported.closed_at.map(millis)),
        ..Default::default()
    };
    let id = approval_request::Entity::insert(row)
        .exec(conn)
        .await?
        .last_insert_id;
    insert_files(conn, id, &imported.files).await?;
    Ok(ApprovalId::new(id))
}

/// Refuses an agent that isn't there.
async fn check_agent(
    conn: &impl ConnectionTrait,
    agent: &AgentId,
) -> Result<(), ApprovalStoreError> {
    if configured_agent::Entity::find_by_id(agent.as_str())
        .one(conn)
        .await?
        .is_none()
    {
        return Err(ApprovalStoreError::NoAgent(agent.clone()));
    }
    Ok(())
}

/// Stores `files` as request `id`'s, in order.
async fn insert_files(
    conn: &impl ConnectionTrait,
    id: i64,
    files: &[ApprovalFile],
) -> Result<(), DbErr> {
    if files.is_empty() {
        return Ok(());
    }
    approval_file::Entity::insert_many(files.iter().enumerate().map(|(i, file)| {
        approval_file::ActiveModel {
            request_id: Set(id),
            position: Set(i32::try_from(i).unwrap_or(i32::MAX)),
            name: Set(file.name.clone()),
            size: Set(file
                .size
                .map(|size| i64::try_from(size).unwrap_or(i64::MAX))),
            sha256: Set(file.sha256.as_bytes().to_vec()),
            ..Default::default()
        }
    }))
    .exec(conn)
    .await?;
    Ok(())
}
