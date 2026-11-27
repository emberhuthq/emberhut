//! External approvers and trusted requesters: set only by the owner, from a signed-in device.

use emberhut_core::agents::{TrustedRequester, validate_requesters};
use emberhut_core::approvals::{Approver, ApproverId, ApproverRecord, RequestState};
use jiff::Timestamp;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder,
};

use super::{AgentStore, AgentStoreError, check_agent, rows};
use crate::entity::{approval_policy, approval_request, approver, trusted_requester};
use crate::time::millis;
use crate::{DbError, begin_write};

impl AgentStore {
    /// Every external approver, by name.
    ///
    /// # Errors
    ///
    /// [`DbError`], [`DbError::CorruptValue`] for a row that isn't an approver.
    pub async fn approvers(&self) -> Result<Vec<ApproverRecord>, DbError> {
        approver::Entity::find()
            .order_by_asc(approver::Column::Name)
            .order_by_asc(approver::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(rows::approver)
            .collect()
    }

    /// The approver `id`, if there is one.
    ///
    /// # Errors
    ///
    /// As [`AgentStore::approvers`].
    pub async fn approver(&self, id: &ApproverId) -> Result<Option<ApproverRecord>, DbError> {
        approver::Entity::find_by_id(id.as_str())
            .one(&self.conn)
            .await?
            .map(rows::approver)
            .transpose()
    }

    /// Stores a new approver.
    ///
    /// # Errors
    ///
    /// [`AgentStoreError::Approver`] for one [`ApproverRecord::validate`] refuses,
    /// [`AgentStoreError::ApproverExists`], or the database's.
    pub async fn create_approver(
        &self,
        record: &ApproverRecord,
        now: Timestamp,
    ) -> Result<(), AgentStoreError> {
        let txn = begin_write(&self.conn).await?;
        create_approver_in(&txn, record, now).await?;
        txn.commit().await?;
        Ok(())
    }

    /// Changes the approver `record.id` to `record`.
    ///
    /// # Errors
    ///
    /// [`AgentStoreError::Approver`], [`AgentStoreError::NoApprover`], or the database's.
    pub async fn update_approver(
        &self,
        record: &ApproverRecord,
        now: Timestamp,
    ) -> Result<(), AgentStoreError> {
        record.validate()?;
        let txn = begin_write(&self.conn).await?;
        let Some(old) = approver::Entity::find_by_id(record.id.as_str())
            .one(&txn)
            .await?
        else {
            return Err(AgentStoreError::NoApprover(record.id.clone()));
        };
        rows::approver_row(record, old.created_at, now)
            .update(&txn)
            .await?;
        txn.commit().await?;
        Ok(())
    }

    /// Deletes the approver `id`; `false` when there was none. Requests it was named on keep
    /// its id, so their history still says who it was.
    ///
    /// # Errors
    ///
    /// [`AgentStoreError::ApproverInUse`] while a policy names it,
    /// [`AgentStoreError::ApproverHasRequests`] while a pending or approved request names it
    /// (a new approver given the same id must not decide or use them), or the database's.
    pub async fn delete_approver(&self, id: &ApproverId) -> Result<bool, AgentStoreError> {
        let txn = begin_write(&self.conn).await?;
        let named = approval_policy::Entity::find()
            .filter(
                approval_policy::Column::Approver
                    .eq(Approver::External(id.clone()).as_str().into_owned()),
            )
            .one(&txn)
            .await?;
        if named.is_some() {
            return Err(AgentStoreError::ApproverInUse(id.clone()));
        }
        let open = approval_request::Entity::find()
            .filter(
                approval_request::Column::Approver
                    .eq(Approver::External(id.clone()).as_str().into_owned()),
            )
            .filter(
                approval_request::Column::State
                    .is_in(RequestState::WITHDRAWABLE.map(RequestState::as_str)),
            )
            .one(&txn)
            .await?;
        if open.is_some() {
            return Err(AgentStoreError::ApproverHasRequests(id.clone()));
        }
        let deleted = approver::Entity::delete_by_id(id.as_str())
            .exec(&txn)
            .await?;
        txn.commit().await?;
        Ok(deleted.rows_affected == 1)
    }

    /// Every trusted requester, by name, then for every agent before one agent's.
    ///
    /// # Errors
    ///
    /// [`DbError`], [`DbError::CorruptValue`] for a row whose agent isn't an agent's id.
    pub async fn trusted_requesters(&self) -> Result<Vec<TrustedRequester>, DbError> {
        trusted_requester::Entity::find()
            .order_by_asc(trusted_requester::Column::Requester)
            .order_by_asc(trusted_requester::Column::AgentId)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|row| {
                Ok(TrustedRequester {
                    agent: row.agent_id.as_deref().map(rows::agent_id).transpose()?,
                    requester: row.requester,
                })
            })
            .collect()
    }

    /// Replaces the trusted requesters with `requesters`.
    ///
    /// # Errors
    ///
    /// [`AgentStoreError::Invalid`] for a list [`validate_requesters`] refuses,
    /// [`AgentStoreError::NoAgent`] for an agent that isn't there, or the database's.
    pub async fn replace_trusted_requesters(
        &self,
        requesters: &[TrustedRequester],
    ) -> Result<(), AgentStoreError> {
        let txn = begin_write(&self.conn).await?;
        replace_trusted_requesters_in(&txn, requesters).await?;
        txn.commit().await?;
        Ok(())
    }
}

/// [`AgentStore::create_approver`]'s write, on `conn`: checked, then the row. The importer runs it
/// inside its own one transaction.
pub(crate) async fn create_approver_in(
    conn: &impl ConnectionTrait,
    record: &ApproverRecord,
    now: Timestamp,
) -> Result<(), AgentStoreError> {
    record.validate()?;
    if approver::Entity::find_by_id(record.id.as_str())
        .one(conn)
        .await?
        .is_some()
    {
        return Err(AgentStoreError::ApproverExists(record.id.clone()));
    }
    rows::approver_row(record, millis(now), now)
        .insert(conn)
        .await?;
    Ok(())
}

/// [`AgentStore::replace_trusted_requesters`]'s write, on `conn`: checked, then the list
/// replaced. The importer runs it inside its own one transaction.
pub(crate) async fn replace_trusted_requesters_in(
    conn: &impl ConnectionTrait,
    requesters: &[TrustedRequester],
) -> Result<(), AgentStoreError> {
    validate_requesters(requesters)?;
    for agent in requesters.iter().filter_map(|r| r.agent.as_ref()) {
        check_agent(conn, agent).await?;
    }
    trusted_requester::Entity::delete_many().exec(conn).await?;
    if !requesters.is_empty() {
        trusted_requester::Entity::insert_many(requesters.iter().map(|r| {
            trusted_requester::ActiveModel {
                requester: sea_orm::ActiveValue::Set(r.requester.clone()),
                agent_id: sea_orm::ActiveValue::Set(
                    r.agent.as_ref().map(|a| a.as_str().to_owned()),
                ),
                ..Default::default()
            }
        }))
        .exec(conn)
        .await?;
    }
    Ok(())
}
