//! Devices and their sessions.

use emberhut_core::session::{
    AccessRecord, DeviceSummary, IssuedTokens, RefreshMatch, RefreshRecord, idle_before,
};
use emberhut_core::{DeviceDetails, DeviceId, SecretHash};
use jiff::Timestamp;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait,
    DatabaseConnection, DbErr, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, QuerySelect,
};

use crate::entity::{device, fcm_device, push_device, session};
use crate::time::{instant, millis};
use crate::{DbError, begin_write};

/// Reads and writes devices and their sessions.
#[derive(Clone, Debug)]
pub struct SessionStore {
    conn: DatabaseConnection,
}

impl SessionStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Opens a session with `tokens` for the device `details` describes, at `now`.
    ///
    /// A device whose install id is known takes over that row (signed back in if it was
    /// signed out), and all its old tokens stop working; any other device gets a new row.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn open(
        &self,
        details: &DeviceDetails,
        tokens: &IssuedTokens,
        now: Timestamp,
    ) -> Result<DeviceId, DbError> {
        let txn = begin_write(&self.conn).await?;
        let device = open_in(&txn, details, tokens, now).await?;
        txn.commit().await?;
        Ok(device)
    }

    /// The session whose current or previous access token has this hash, if there is one,
    /// with that token's own expiry.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn find_by_access(&self, hash: &SecretHash) -> Result<Option<AccessRecord>, DbError> {
        let hash = hash.as_bytes().to_vec();
        let found = session::Entity::find()
            .filter(
                Condition::any()
                    .add(session::Column::AccessHash.eq(hash.clone()))
                    .add(session::Column::PreviousAccessHash.eq(hash.clone())),
            )
            .find_also_related(device::Entity)
            .one(&self.conn)
            .await?;
        let Some((session, Some(device))) = found else {
            return Ok(None);
        };
        // Hashes, not secrets: comparing them says which of the two matched.
        let expires_at = if session.access_hash == hash {
            session.access_expires_at
        } else if let Some(previous) = session.previous_access_expires_at {
            previous
        } else {
            // `rotate` always writes both; a previous hash without its expiry works for nobody.
            return Ok(None);
        };
        Ok(Some(AccessRecord {
            device: DeviceId::new(device.id),
            expires_at: instant(expires_at)?,
            device_seen_at: instant(device.seen_at)?,
        }))
    }

    /// The session whose current or previous refresh token has this hash, if there is one.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn find_by_refresh(
        &self,
        hash: &SecretHash,
    ) -> Result<Option<RefreshRecord>, DbError> {
        let hash = hash.as_bytes().to_vec();
        let found = session::Entity::find()
            .filter(
                Condition::any()
                    .add(session::Column::RefreshHash.eq(hash.clone()))
                    .add(session::Column::PreviousRefreshHash.eq(hash.clone())),
            )
            .find_also_related(device::Entity)
            .one(&self.conn)
            .await?;
        let Some((session, Some(device))) = found else {
            return Ok(None);
        };
        let matched = if session.refresh_hash == hash {
            RefreshMatch::Current
        } else if let Some(rotated_at) = session.rotated_at {
            RefreshMatch::Previous {
                rotated_at: instant(rotated_at)?,
                current: SecretHash::from_bytes(&session.refresh_hash)
                    .ok_or(DbError::CorruptHash)?,
            }
        } else {
            // `rotate` always writes both; a previous hash without its time works for nobody.
            return Ok(None);
        };
        Ok(Some(RefreshRecord {
            device: DeviceId::new(device.id),
            matched,
            device_seen_at: instant(device.seen_at)?,
            device_install: device.install,
        }))
    }

    /// Replaces `device`'s tokens with `tokens`, if its current refresh token still has the
    /// hash `current`; `false` when it does not (another request used it first). The tokens
    /// being replaced become the previous ones: the access token keeps working until its own
    /// expiry. The device is seen at `now`, and takes the install id `claim` if it still has
    /// none and no other device has it.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn rotate(
        &self,
        device: DeviceId,
        current: &SecretHash,
        tokens: &IssuedTokens,
        claim: Option<&str>,
        now: Timestamp,
    ) -> Result<bool, DbError> {
        let txn = begin_write(&self.conn).await?;
        // In an UPDATE every column on the right reads its value from before the update, so
        // the current hashes move into the previous ones as the new ones come in.
        let replaced = session::Entity::update_many()
            .col_expr(
                session::Column::PreviousAccessHash,
                Expr::col(session::Column::AccessHash),
            )
            .col_expr(
                session::Column::PreviousAccessExpiresAt,
                Expr::col(session::Column::AccessExpiresAt),
            )
            .col_expr(
                session::Column::PreviousRefreshHash,
                Expr::col(session::Column::RefreshHash),
            )
            .col_expr(
                session::Column::AccessHash,
                Expr::value(tokens.access.hash().as_bytes().to_vec()),
            )
            .col_expr(
                session::Column::AccessExpiresAt,
                Expr::value(millis(tokens.access_expires_at)),
            )
            .col_expr(
                session::Column::RefreshHash,
                Expr::value(tokens.refresh.hash().as_bytes().to_vec()),
            )
            .col_expr(session::Column::IssuedAt, Expr::value(millis(now)))
            .col_expr(session::Column::RotatedAt, Expr::value(millis(now)))
            .filter(session::Column::DeviceId.eq(device.get()))
            .filter(session::Column::RefreshHash.eq(current.as_bytes().to_vec()))
            .exec(&txn)
            .await?;
        if replaced.rows_affected == 0 {
            return Ok(false);
        }

        device::Entity::update_many()
            .col_expr(device::Column::SeenAt, Expr::value(millis(now)))
            .filter(device::Column::Id.eq(device.get()))
            .exec(&txn)
            .await?;
        if let Some(install) = claim {
            let taken = device::Entity::find()
                .select_only()
                .column(device::Column::Id)
                .filter(device::Column::Install.eq(install))
                .into_tuple::<i64>()
                .one(&txn)
                .await?
                .is_some();
            if !taken {
                device::Entity::update_many()
                    .col_expr(device::Column::Install, Expr::value(install))
                    .filter(device::Column::Id.eq(device.get()))
                    .filter(device::Column::Install.is_null())
                    .exec(&txn)
                    .await?;
            }
        }
        txn.commit().await?;
        Ok(true)
    }

    /// Replaces `device`'s current tokens with `tokens`, if the refresh token they replaced
    /// still has the hash `previous` and the current refresh token still has the hash
    /// `current` (as the caller read it); `false` when either has changed, so of two re-issues
    /// racing from one read exactly one wins. For a client that lost the
    /// answer to a refresh: the pair issued moments before stops working, and the previous
    /// pair and the time it was replaced stay as they are, so the grace does not grow.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn reissue(
        &self,
        device: DeviceId,
        previous: &SecretHash,
        current: &SecretHash,
        tokens: &IssuedTokens,
        now: Timestamp,
    ) -> Result<bool, DbError> {
        let txn = begin_write(&self.conn).await?;
        let replaced = session::Entity::update_many()
            .col_expr(
                session::Column::AccessHash,
                Expr::value(tokens.access.hash().as_bytes().to_vec()),
            )
            .col_expr(
                session::Column::AccessExpiresAt,
                Expr::value(millis(tokens.access_expires_at)),
            )
            .col_expr(
                session::Column::RefreshHash,
                Expr::value(tokens.refresh.hash().as_bytes().to_vec()),
            )
            .col_expr(session::Column::IssuedAt, Expr::value(millis(now)))
            .filter(session::Column::DeviceId.eq(device.get()))
            .filter(session::Column::PreviousRefreshHash.eq(previous.as_bytes().to_vec()))
            .filter(session::Column::RefreshHash.eq(current.as_bytes().to_vec()))
            .exec(&txn)
            .await?;
        if replaced.rows_affected == 0 {
            return Ok(false);
        }
        device::Entity::update_many()
            .col_expr(device::Column::SeenAt, Expr::value(millis(now)))
            .filter(device::Column::Id.eq(device.get()))
            .exec(&txn)
            .await?;
        txn.commit().await?;
        Ok(true)
    }

    /// Notes that `device` was seen at `now`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn mark_seen(&self, device: DeviceId, now: Timestamp) -> Result<(), DbError> {
        device::Entity::update_many()
            .col_expr(device::Column::SeenAt, Expr::value(millis(now)))
            .filter(device::Column::Id.eq(device.get()))
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// Signs `device` out at `now`: its session is deleted, so every token it had (current
    /// and previous) stops working at once, and so are the push addresses it registered, so it
    /// is sent no more alerts.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn sign_out(&self, device: DeviceId, now: Timestamp) -> Result<(), DbError> {
        let txn = begin_write(&self.conn).await?;
        session::Entity::delete_many()
            .filter(session::Column::DeviceId.eq(device.get()))
            .exec(&txn)
            .await?;
        push_device::Entity::delete_many()
            .filter(push_device::Column::DeviceId.eq(device.get()))
            .exec(&txn)
            .await?;
        fcm_device::Entity::delete_many()
            .filter(fcm_device::Column::DeviceId.eq(device.get()))
            .exec(&txn)
            .await?;
        device::Entity::update_many()
            .col_expr(device::Column::RevokedAt, Expr::value(millis(now)))
            .filter(device::Column::Id.eq(device.get()))
            .exec(&txn)
            .await?;
        txn.commit().await?;
        Ok(())
    }

    /// The devices signed in at `now`, the most recently seen first: neither signed out nor
    /// idle (an idle device is signed out at its next refresh, and is not listed meanwhile).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn signed_in(&self, now: Timestamp) -> Result<Vec<DeviceSummary>, DbError> {
        device::Entity::find()
            .filter(device::Column::RevokedAt.is_null())
            .filter(device::Column::SeenAt.gte(millis(idle_before(now))))
            .order_by_desc(device::Column::SeenAt)
            .order_by_desc(device::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|row| {
                Ok(DeviceSummary {
                    id: DeviceId::new(row.id),
                    name: row.name,
                    model: row.hardware,
                    created_at: instant(row.created_at)?,
                    seen_at: instant(row.seen_at)?,
                })
            })
            .collect()
    }
}

/// [`SessionStore::open`] inside the caller's transaction, so that what proved the device
/// (a pairing code, say) is used up in the same commit.
pub(crate) async fn open_in(
    conn: &impl ConnectionTrait,
    details: &DeviceDetails,
    tokens: &IssuedTokens,
    now: Timestamp,
) -> Result<DeviceId, DbErr> {
    let known = match details.install() {
        Some(install) => {
            device::Entity::find()
                .filter(device::Column::Install.eq(install))
                .one(conn)
                .await?
        }
        None => None,
    };
    let device = match known {
        Some(known) => {
            let mut device = known.into_active_model();
            device.name = Set(details.name().to_owned());
            device.hardware = Set(details.model().to_owned());
            device.seen_at = Set(millis(now));
            device.revoked_at = Set(None);
            device.update(conn).await?
        }
        None => {
            device::ActiveModel {
                name: Set(details.name().to_owned()),
                hardware: Set(details.model().to_owned()),
                install: Set(details.install().map(str::to_owned)),
                created_at: Set(millis(now)),
                seen_at: Set(millis(now)),
                revoked_at: Set(None),
                ..Default::default()
            }
            .insert(conn)
            .await?
        }
    };

    // A new sign-in replaces every token the device had, previous ones included.
    session::Entity::insert(session::ActiveModel {
        device_id: Set(device.id),
        access_hash: Set(tokens.access.hash().as_bytes().to_vec()),
        access_expires_at: Set(millis(tokens.access_expires_at)),
        refresh_hash: Set(tokens.refresh.hash().as_bytes().to_vec()),
        issued_at: Set(millis(now)),
        previous_access_hash: Set(None),
        previous_access_expires_at: Set(None),
        previous_refresh_hash: Set(None),
        rotated_at: Set(None),
    })
    .on_conflict(
        OnConflict::column(session::Column::DeviceId)
            .update_columns([
                session::Column::AccessHash,
                session::Column::AccessExpiresAt,
                session::Column::RefreshHash,
                session::Column::IssuedAt,
                session::Column::PreviousAccessHash,
                session::Column::PreviousAccessExpiresAt,
                session::Column::PreviousRefreshHash,
                session::Column::RotatedAt,
            ])
            .to_owned(),
    )
    .exec_without_returning(conn)
    .await?;
    Ok(DeviceId::new(device.id))
}

#[cfg(test)]
mod tests {
    use emberhut_core::OsRandom;
    use jiff::Timestamp;

    use super::*;
    use crate::{Database, DatabaseUrl};

    #[tokio::test]
    async fn only_the_hashes_of_tokens_are_stored() {
        let dir = tempfile::tempdir().unwrap();
        let url: DatabaseUrl = DatabaseUrl::from_path(&dir.path().join("emberhut.db")).unwrap();
        let db = Database::connect(&url).await.unwrap();
        let now = Timestamp::from_second(1_800_000_000).unwrap();
        let tokens = IssuedTokens::issue(&OsRandom, now).unwrap();
        db.sessions()
            .open(&DeviceDetails::new(None, None, None), &tokens, now)
            .await
            .unwrap();

        let rows = session::Entity::find().all(&db.conn).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].access_hash, tokens.access.hash().as_bytes());
        assert_eq!(rows[0].refresh_hash, tokens.refresh.hash().as_bytes());
        for stored in [&rows[0].access_hash, &rows[0].refresh_hash] {
            assert_ne!(stored.as_slice(), tokens.access.as_str().as_bytes());
            assert_ne!(stored.as_slice(), tokens.refresh.as_str().as_bytes());
        }
    }
}
