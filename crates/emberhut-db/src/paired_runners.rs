//! Runners paired at run time ([`PairedRunnerStore`]): their rows, and the one-time codes they
//! pair with. Only a code's hash is stored; a runner's token is stored as text, since the
//! server presents it to the runner whenever it calls it.

use std::fmt;

use emberhut_core::pairing::PairingRecord;
use emberhut_core::{PairingCodeId, SecretHash};
use jiff::Timestamp;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder,
};

use crate::entity::{paired_runner, runner_code};
use crate::time::{instant, millis};
use crate::{DbError, begin_write};

/// A runner as it pairs: who it is, where it is reached, and the token it was given.
#[derive(Clone, PartialEq, Eq)]
pub struct RunnerPairing {
    /// The runner's name, the one it reports itself by.
    pub name: String,
    /// Its API's address, without a final `/`.
    pub url: String,
    /// Its machine in words, if it gave them.
    pub words: Option<String>,
    /// The token it was given.
    pub token: String,
    /// The system it said it runs on.
    pub platform: Option<String>,
    /// Its own version, as it said.
    pub version: Option<String>,
}

// Its `Debug` leaves the token out.
impl fmt::Debug for RunnerPairing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunnerPairing")
            .field("name", &self.name)
            .field("url", &self.url)
            .field("words", &self.words)
            .field("platform", &self.platform)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

/// A paired runner, as stored.
#[derive(Clone, PartialEq, Eq)]
pub struct PairedRunner {
    /// Who it is and where it is reached, as it last paired.
    pub pairing: RunnerPairing,
    /// When it first paired.
    pub created: Timestamp,
    /// When it last paired.
    pub paired: Timestamp,
}

// Its `Debug` leaves the token out, as its pairing's does.
impl fmt::Debug for PairedRunner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PairedRunner")
            .field("pairing", &self.pairing)
            .field("created", &self.created)
            .field("paired", &self.paired)
            .finish()
    }
}

/// What became of a runner code that was traded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunnerRedemption {
    /// It paired this runner, under the first of the names asked for that no paired runner
    /// had.
    Paired(PairedRunner),
    /// Every name asked for is a paired runner's: the code is left unused.
    NoFreeName,
    /// It had paired a runner already.
    AlreadyUsed,
    /// It had expired.
    Expired,
}

/// Reads and writes the paired runners and their codes.
#[derive(Clone, Debug)]
pub struct PairedRunnerStore {
    conn: DatabaseConnection,
}

impl PairedRunnerStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Every paired runner, by name.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn list(&self) -> Result<Vec<PairedRunner>, DbError> {
        paired_runner::Entity::find()
            .order_by_asc(paired_runner::Column::Name)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(runner_of)
            .collect()
    }

    /// Forgets the runner `name`: whether there was one.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn forget(&self, name: &str) -> Result<bool, DbError> {
        let deleted = paired_runner::Entity::delete_by_id(name.to_owned())
            .exec(&self.conn)
            .await?;
        Ok(deleted.rows_affected > 0)
    }

    /// Stores the code with this hash, made at `now` and working until `expires_at`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn insert_code(
        &self,
        hash: &SecretHash,
        now: Timestamp,
        expires_at: Timestamp,
    ) -> Result<PairingCodeId, DbError> {
        let row = runner_code::ActiveModel {
            code_hash: Set(hash.as_bytes().to_vec()),
            created_at: Set(millis(now)),
            expires_at: Set(millis(expires_at)),
            used_at: Set(None),
            runner: Set(None),
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
    pub async fn delete_expired_codes(&self, now: Timestamp) -> Result<u64, DbError> {
        let deleted = runner_code::Entity::delete_many()
            .filter(runner_code::Column::ExpiresAt.lte(millis(now)))
            .exec(&self.conn)
            .await?;
        Ok(deleted.rows_affected)
    }

    /// The newest code with this hash, if there is one.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn find_code(&self, hash: &SecretHash) -> Result<Option<PairingRecord>, DbError> {
        let Some(row) = runner_code::Entity::find()
            .filter(runner_code::Column::CodeHash.eq(hash.as_bytes().to_vec()))
            .order_by_desc(runner_code::Column::Id)
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

    /// Uses up `code` at `now` and, in the same transaction, keeps `runner` under the first of
    /// `names` that no paired runner has (`runner.name` is not read: `names` are the names the
    /// runner may have, in the order to try). A code that another request used first, or that
    /// has expired since it was looked up, pairs nothing, nor does one when every name is
    /// taken (the code stays unused), and the answer says which.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn pair(
        &self,
        code: PairingCodeId,
        names: &[String],
        runner: &RunnerPairing,
        now: Timestamp,
    ) -> Result<RunnerRedemption, DbError> {
        let txn = begin_write(&self.conn).await?;
        let taken: Vec<String> = paired_runner::Entity::find()
            .filter(paired_runner::Column::Name.is_in(names.iter().cloned()))
            .all(&txn)
            .await?
            .into_iter()
            .map(|row| row.name)
            .collect();
        let Some(name) = names.iter().find(|name| !taken.contains(name)) else {
            return Ok(RunnerRedemption::NoFreeName);
        };
        let claimed = runner_code::Entity::update_many()
            .col_expr(runner_code::Column::UsedAt, Expr::value(millis(now)))
            .col_expr(runner_code::Column::Runner, Expr::value(name.clone()))
            .filter(runner_code::Column::Id.eq(code.get()))
            .filter(runner_code::Column::UsedAt.is_null())
            .filter(runner_code::Column::ExpiresAt.gt(millis(now)))
            .exec(&txn)
            .await?;
        if claimed.rows_affected == 0 {
            let used = runner_code::Entity::find_by_id(code.get())
                .one(&txn)
                .await?
                .is_some_and(|row| row.used_at.is_some());
            return Ok(if used {
                RunnerRedemption::AlreadyUsed
            } else {
                RunnerRedemption::Expired
            });
        }
        let row = paired_runner::ActiveModel {
            name: Set(name.clone()),
            url: Set(runner.url.clone()),
            words: Set(runner.words.clone()),
            token: Set(runner.token.clone()),
            platform: Set(runner.platform.clone()),
            version: Set(runner.version.clone()),
            created_at: Set(millis(now)),
            paired_at: Set(millis(now)),
        }
        .insert(&txn)
        .await?;
        txn.commit().await?;
        Ok(RunnerRedemption::Paired(runner_of(row)?))
    }
}

/// A stored row as a paired runner.
fn runner_of(row: paired_runner::Model) -> Result<PairedRunner, DbError> {
    Ok(PairedRunner {
        created: instant(row.created_at)?,
        paired: instant(row.paired_at)?,
        pairing: RunnerPairing {
            name: row.name,
            url: row.url,
            words: row.words,
            token: row.token,
            platform: row.platform,
            version: row.version,
        },
    })
}
