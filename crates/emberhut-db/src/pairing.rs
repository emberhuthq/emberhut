//! Pairing codes. Only their hashes are stored.

use emberhut_core::pairing::PairingRecord;
use emberhut_core::session::IssuedTokens;
use emberhut_core::{DeviceDetails, DeviceId, PairingCodeId, SecretHash};
use jiff::Timestamp;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder,
};

use crate::entity::pairing_code;
use crate::time::{instant, millis};
use crate::{DbError, begin_write, sessions};

/// What became of a code that was redeemed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Redemption {
    /// It paired this device.
    Paired(DeviceId),
    /// It had paired a device already.
    AlreadyUsed,
    /// It had expired.
    Expired,
}

/// Reads and writes pairing codes.
#[derive(Clone, Debug)]
pub struct PairingStore {
    conn: DatabaseConnection,
}

impl PairingStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Stores the code with this hash, made at `now` and working until `expires_at`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn insert(
        &self,
        hash: &SecretHash,
        now: Timestamp,
        expires_at: Timestamp,
    ) -> Result<PairingCodeId, DbError> {
        let row = pairing_code::ActiveModel {
            code_hash: Set(hash.as_bytes().to_vec()),
            created_at: Set(millis(now)),
            expires_at: Set(millis(expires_at)),
            used_at: Set(None),
            device_id: Set(None),
            ..Default::default()
        }
        .insert(&self.conn)
        .await?;
        Ok(PairingCodeId::new(row.id))
    }

    /// Deletes every code that no longer works at `now`, used or not; how many there were.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn delete_expired(&self, now: Timestamp) -> Result<u64, DbError> {
        let deleted = pairing_code::Entity::delete_many()
            .filter(pairing_code::Column::ExpiresAt.lte(millis(now)))
            .exec(&self.conn)
            .await?;
        Ok(deleted.rows_affected)
    }

    /// The newest code with this hash, if there is one.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn find(&self, hash: &SecretHash) -> Result<Option<PairingRecord>, DbError> {
        let Some(row) = pairing_code::Entity::find()
            .filter(pairing_code::Column::CodeHash.eq(hash.as_bytes().to_vec()))
            .order_by_desc(pairing_code::Column::Id)
            .one(&self.conn)
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(PairingRecord {
            id: PairingCodeId::new(row.id),
            used: row.used_at.is_some(),
            expires_at: instant(row.expires_at)?,
        }))
    }

    /// Uses up `code` at `now` and, in the same transaction, opens a session with `tokens` for
    /// the device `details` describes (as [`SessionStore::open`](crate::SessionStore::open)
    /// does). A code that another request used first, or that has expired since it was looked
    /// up, pairs nothing, and the answer says which.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn redeem(
        &self,
        code: PairingCodeId,
        details: &DeviceDetails,
        tokens: &IssuedTokens,
        now: Timestamp,
    ) -> Result<Redemption, DbError> {
        let txn = begin_write(&self.conn).await?;
        let claimed = pairing_code::Entity::update_many()
            .col_expr(pairing_code::Column::UsedAt, Expr::value(millis(now)))
            .filter(pairing_code::Column::Id.eq(code.get()))
            .filter(pairing_code::Column::UsedAt.is_null())
            .filter(pairing_code::Column::ExpiresAt.gt(millis(now)))
            .exec(&txn)
            .await?;
        if claimed.rows_affected == 0 {
            let used = pairing_code::Entity::find_by_id(code.get())
                .one(&txn)
                .await?
                .is_some_and(|row| row.used_at.is_some());
            return Ok(if used {
                Redemption::AlreadyUsed
            } else {
                Redemption::Expired
            });
        }
        let device = sessions::open_in(&txn, details, tokens, now).await?;
        pairing_code::Entity::update_many()
            .col_expr(pairing_code::Column::DeviceId, Expr::value(device.get()))
            .filter(pairing_code::Column::Id.eq(code.get()))
            .exec(&txn)
            .await?;
        txn.commit().await?;
        Ok(Redemption::Paired(device))
    }
}
