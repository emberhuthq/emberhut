//! The files already brought from runners' machines ([`PullStore`], the `pulls` table): the
//! same file, unchanged (the same path, size and modification time), is fetched once.

use jiff::Timestamp;
use sea_orm::sea_query::OnConflict;
use sea_orm::{ActiveValue::Set, DatabaseConnection, EntityTrait};

use crate::DbError;
use crate::entity::pull;
use crate::time::millis;

/// A file on a runner's machine, as its runner described it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFile {
    /// The machine.
    pub machine: String,
    /// Its path there.
    pub path: String,
    /// Its size, in bytes.
    pub size: i64,
    /// When it last changed, as the runner wrote it.
    pub mtime: String,
}

/// Remembers which files were brought, and where they are here.
#[derive(Clone, Debug)]
pub struct PullStore {
    conn: DatabaseConnection,
}

impl PullStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Where `file` was brought to before (relative to the uploads folder), if it was.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn find(&self, file: &RemoteFile) -> Result<Option<String>, DbError> {
        Ok(pull::Entity::find_by_id((
            file.machine.clone(),
            file.path.clone(),
            file.size,
            file.mtime.clone(),
        ))
        .one(&self.conn)
        .await?
        .map(|row| row.local))
    }

    /// Notes that `file` was brought to `local` at `at`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn record(
        &self,
        file: &RemoteFile,
        local: &str,
        at: Timestamp,
    ) -> Result<(), DbError> {
        pull::Entity::insert(pull::ActiveModel {
            machine: Set(file.machine.clone()),
            path: Set(file.path.clone()),
            size: Set(file.size),
            mtime: Set(file.mtime.clone()),
            local: Set(local.to_owned()),
            at: Set(millis(at)),
        })
        .on_conflict(
            OnConflict::columns([
                pull::Column::Machine,
                pull::Column::Path,
                pull::Column::Size,
                pull::Column::Mtime,
            ])
            .update_columns([pull::Column::Local, pull::Column::At])
            .to_owned(),
        )
        .exec(&self.conn)
        .await?;
        Ok(())
    }
}
