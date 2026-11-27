//! One-time secret links. Only their tokens' hashes are stored.

use emberhut_core::secret_link::{SecretLinkRecord, SecretName};
use emberhut_core::{SecretHash, SecretLinkId};
use jiff::Timestamp;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
};

use crate::DbError;
use crate::entity::secret_link;
use crate::time::{instant, millis};

/// Reads and writes secret links.
#[derive(Clone, Debug)]
pub struct SecretLinkStore {
    conn: DatabaseConnection,
}

impl SecretLinkStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Stores the link whose token has this hash, asking for `names` because of `why`, made at
    /// `now` and working until `expires_at`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn insert(
        &self,
        hash: &SecretHash,
        names: &[SecretName],
        why: &str,
        now: Timestamp,
        expires_at: Timestamp,
    ) -> Result<SecretLinkId, DbError> {
        let names: Vec<&str> = names.iter().map(SecretName::as_str).collect();
        let row = secret_link::ActiveModel {
            token_hash: Set(hash.as_bytes().to_vec()),
            names: Set(names.join("\n")),
            why: Set(why.to_owned()),
            created_at: Set(millis(now)),
            expires_at: Set(millis(expires_at)),
            ..Default::default()
        }
        .insert(&self.conn)
        .await?;
        Ok(SecretLinkId::new(row.id))
    }

    /// The link whose token has this hash, if it was made and not used yet (it may have
    /// expired: see [`SecretLinkRecord::works_at`]).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn find(&self, hash: &SecretHash) -> Result<Option<SecretLinkRecord>, DbError> {
        let Some(row) = secret_link::Entity::find()
            .filter(secret_link::Column::TokenHash.eq(hash.as_bytes().to_vec()))
            .one(&self.conn)
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(SecretLinkRecord {
            id: SecretLinkId::new(row.id),
            // Every name was checked when the link was made; one that no longer reads as a
            // name (a row changed by hand) is left out rather than written to.
            names: row
                .names
                .lines()
                .filter_map(|name| SecretName::parse(name).ok())
                .collect(),
            why: row.why,
            expires_at: instant(row.expires_at)?,
        }))
    }

    /// Uses the link up: `true` for the one caller that did, `false` when it was gone already.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn take(&self, id: SecretLinkId) -> Result<bool, DbError> {
        let deleted = secret_link::Entity::delete_by_id(id.get())
            .exec(&self.conn)
            .await?;
        Ok(deleted.rows_affected == 1)
    }

    /// Deletes every link that no longer works at `now`; how many there were.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn delete_expired(&self, now: Timestamp) -> Result<u64, DbError> {
        let deleted = secret_link::Entity::delete_many()
            .filter(secret_link::Column::ExpiresAt.lt(millis(now)))
            .exec(&self.conn)
            .await?;
        Ok(deleted.rows_affected)
    }
}
