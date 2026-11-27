//! Approval requests' rows to and from `emberhut-core`'s types.

use std::collections::HashMap;

use emberhut_core::ApprovalId;
use emberhut_core::approvals::{
    ApprovalFile, ApprovalRequest, DecidedVia, Decision, DecisionRecord, RequestState,
    Sha256Digest, SlackPost, UseRecord,
};
use emberhut_core::task::TaskId;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::DbError;
use crate::agents::rows::{agent_id, approver_of, corrupt, kind_of};
use crate::entity::{approval_file, approval_request};
use crate::time::instant;

fn digest(bytes: &[u8]) -> Result<Sha256Digest, DbError> {
    Sha256Digest::from_bytes(bytes).ok_or(DbError::CorruptHash)
}

pub(super) fn state(text: &str) -> Result<RequestState, DbError> {
    RequestState::parse(text).ok_or_else(|| corrupt("approval_requests.state", text))
}

fn file(row: approval_file::Model) -> Result<ApprovalFile, DbError> {
    Ok(ApprovalFile {
        size: row
            .size
            .map(|size| {
                u64::try_from(size).map_err(|_| corrupt("approval_files.size", &size.to_string()))
            })
            .transpose()?,
        sha256: digest(&row.sha256)?,
        name: row.name,
    })
}

fn decided(row: &approval_request::Model) -> Result<Option<DecisionRecord>, DbError> {
    let (Some(decision), Some(at)) = (row.decision.as_deref(), row.decided_at) else {
        return Ok(None);
    };
    let via = row.decided_via.as_deref().unwrap_or_default();
    Ok(Some(DecisionRecord {
        decision: Decision::parse(decision)
            .ok_or_else(|| corrupt("approval_requests.decision", decision))?,
        at: instant(at)?,
        by: row.decided_by.clone().unwrap_or_default(),
        via: DecidedVia::parse(via).ok_or_else(|| corrupt("approval_requests.decided_via", via))?,
    }))
}

fn request(
    row: approval_request::Model,
    files: Vec<ApprovalFile>,
) -> Result<ApprovalRequest, DbError> {
    let decided = decided(&row)?;
    let used = match row.used_at {
        Some(at) => Some(UseRecord {
            at: instant(at)?,
            by: row.used_by.clone().unwrap_or_default(),
        }),
        None => None,
    };
    let shown = match (row.slack_channel, row.slack_ts) {
        (Some(channel), Some(ts)) => Some(SlackPost { channel, ts }),
        _ => None,
    };
    Ok(ApprovalRequest {
        id: ApprovalId::new(row.id),
        agent: agent_id(&row.agent_id)?,
        kind: kind_of(&row.kind, "approval_requests.kind")?,
        target: row.target,
        summary: row.summary,
        text: row.text,
        files,
        payload_hash: digest(&row.payload_hash)?,
        approver: approver_of(&row.approver, "approval_requests.approver")?,
        amount_cents: row
            .amount_cents
            .map(|cents| {
                u64::try_from(cents)
                    .map_err(|_| corrupt("approval_requests.amount_cents", &cents.to_string()))
            })
            .transpose()?,
        task: row.task_id.map(TaskId::new),
        state: state(&row.state)?,
        state_reason: row.state_reason,
        source_ref: row.source_ref,
        created_at: instant(row.created_at)?,
        decided,
        used,
        closed_at: row.closed_at.map(instant).transpose()?,
        shown,
    })
}

/// `rows` as requests, each with its files, in the rows' order.
pub(crate) async fn requests(
    conn: &impl ConnectionTrait,
    rows: Vec<approval_request::Model>,
) -> Result<Vec<ApprovalRequest>, DbError> {
    let ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
    let mut files: HashMap<i64, Vec<ApprovalFile>> = HashMap::new();
    if !ids.is_empty() {
        for row in approval_file::Entity::find()
            .filter(approval_file::Column::RequestId.is_in(ids))
            .order_by_asc(approval_file::Column::RequestId)
            .order_by_asc(approval_file::Column::Position)
            .all(conn)
            .await?
        {
            files.entry(row.request_id).or_default().push(file(row)?);
        }
    }
    rows.into_iter()
        .map(|row| {
            let own = files.remove(&row.id).unwrap_or_default();
            request(row, own)
        })
        .collect()
}

/// The request `id`, with its files.
pub(super) async fn one(
    conn: &impl ConnectionTrait,
    id: i64,
) -> Result<Option<ApprovalRequest>, DbError> {
    let Some(row) = approval_request::Entity::find_by_id(id).one(conn).await? else {
        return Ok(None);
    };
    Ok(requests(conn, vec![row]).await?.pop())
}
