//! Consistent copies of the live database, for the server's backup.
//!
//! A copy of the database file's bytes taken while the server writes is torn: the main file and
//! its write-ahead log change under it, and recent commits are only in the log. So the copy is
//! made by SQLite itself, with `VACUUM INTO`, on one of the server's own pooled connections. It
//! reads in one transaction, so it sees one moment of the database, the log's commits included,
//! while writers go on; and no new connection to the file is opened and closed, which could
//! upset the locks the pool holds.
//! `VACUUM INTO` is this module's one statement outside `SeaORM`'s builders, as ADR 0012
//! allows.

use std::path::{Path, PathBuf};

use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, Statement};

use crate::DbError;

/// Makes consistent copies of the database.
#[derive(Clone, Debug)]
pub struct BackupStore {
    conn: DatabaseConnection,
    file: PathBuf,
}

impl BackupStore {
    pub(crate) fn new(conn: DatabaseConnection, file: PathBuf) -> Self {
        Self { conn, file }
    }

    /// The database's own file: the folder it is in is the server's data folder.
    #[must_use]
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Writes a consistent copy of the whole database to `dest`, a new file (it must not exist,
    /// or be empty). The copy is a complete database of its own, with no write-ahead log to
    /// carry along.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] when SQLite cannot make the copy (`dest` exists, its folder is
    /// missing, the disk is full) or `dest` is not a UTF-8 path.
    pub async fn snapshot_into(&self, dest: &Path) -> Result<(), DbError> {
        let Some(dest) = dest.to_str() else {
            return Err(DbErr::Custom(format!(
                "the snapshot's path is not UTF-8: {}",
                dest.display()
            ))
            .into());
        };
        let statement = Statement::from_sql_and_values(
            self.conn.get_database_backend(),
            "VACUUM INTO ?",
            [dest.into()],
        );
        self.conn.execute_raw(statement).await?;
        Ok(())
    }
}
