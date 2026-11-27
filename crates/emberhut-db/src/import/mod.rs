//! Importing an archive of another installation's data: the only code that writes the archive,
//! the manifest and the typed rows an import maps.
//!
//! An import builds a fresh database file: [`ImportTarget::create`] makes it and applies the
//! migrations, [`ImportTarget::attach`] attaches the private copies of the source databases
//! read-only (for the check of the archive), and [`ImportTarget::begin`] starts the one write
//! transaction everything else happens in, as an [`ImportStore`]. [`ImportStore::commit`] ends
//! it and closes the file with no `-wal` or `-shm` left; dropping the store instead rolls it
//! all back.
//!
//! The importer knows the source's format; this module knows Emberhut's schema. What passes
//! between them is the archive's [`Row`]s and plain records of Emberhut's rows.

mod agents;
mod archive;
mod check;
mod devices;
mod manifest;
mod rows;
mod value;

use std::path::{Path, PathBuf};
use std::time::Duration;

use sea_orm::sqlx::sqlite::{SqliteJournalMode, SqliteSynchronous};
use sea_orm::{
    ConnectOptions, ConnectionTrait, DatabaseConnection, DatabaseTransaction, Statement,
};

pub use agents::{AgentLists, ImportedAgentThread};
pub use archive::ArchivedRow;
pub use check::{ArchiveCheck, ForeignKeyProblem, Scrub};
pub use devices::{KeptDevices, keep_devices};
pub use manifest::{FileRecord, MappingRecord, RunRecord, SchemaEntry, TableRecord};
pub use rows::{
    ImportedArea, ImportedArtifact, ImportedCall, ImportedCallHint, ImportedCallLine,
    ImportedCorrection, ImportedDoc, ImportedFile, ImportedHealthDay, ImportedLink,
    ImportedMessage, ImportedMessageFile, ImportedMessageRefs, ImportedNote, ImportedProject,
    ImportedRoom, ImportedRun, ImportedScreenTime, ImportedSetting, ImportedTask,
    ImportedTaskEvent, ImportedVector, ImportedVoiceNote,
};
pub use value::{Row, RowFormatError, SqlValue};

use crate::{DbError, begin_write, migrate, migration};

/// How long a statement on the import's file waits for a lock: nothing else opens it, so only
/// the import's own connection could hold one.
const BUSY: Duration = Duration::from_secs(5);

/// A new database file being built by an import, before its transaction starts.
#[derive(Debug)]
pub struct ImportTarget {
    conn: DatabaseConnection,
    path: PathBuf,
}

impl ImportTarget {
    /// Makes a new database at `path`, which must not exist, with emberhut's schema.
    ///
    /// Its one connection keeps SQLite's temporary files (the `TEMP` tables and sorts of the
    /// check) on disk in `temp_dir` rather than in memory or the system's temporary folder.
    ///
    /// # Errors
    ///
    /// [`DbError::TargetExists`] when there is a file at `path`, [`DbError::Connect`] and
    /// [`DbError::Migrate`].
    pub async fn create(path: &Path, temp_dir: &Path) -> Result<Self, DbError> {
        if std::fs::symlink_metadata(path).is_ok() {
            return Err(DbError::TargetExists(path.to_path_buf()));
        }
        let mut options = ConnectOptions::new(format!("sqlite://{}", path.display()));
        let filename = path.to_path_buf();
        // One connection that is never replaced: what is attached to it, and its `TEMP`
        // tables, belong to that connection.
        options
            .max_connections(1)
            .min_connections(1)
            .sqlx_logging(true)
            .sqlx_logging_level(log::LevelFilter::Trace)
            .map_sqlx_sqlite_opts(move |sqlite| {
                sqlite
                    .filename(&filename)
                    .create_if_missing(true)
                    .foreign_keys(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .synchronous(SqliteSynchronous::Normal)
                    .busy_timeout(BUSY)
                    .pragma("temp_store", "FILE")
            });
        let conn = sea_orm::Database::connect(options)
            .await
            .map_err(DbError::Connect)?;
        set_temp_dir(&conn, temp_dir).await?;
        migrate::<migration::Migrator>(&conn, tokio::time::Instant::now())
            .await
            .map_err(DbError::Migrate)?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
        })
    }

    /// Attaches the database file at `copy` as `alias`, read-only and immutable: a private copy
    /// that no other process knows of, so it takes no locks and has no journal.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn attach(&self, alias: &str, copy: &Path) -> Result<(), DbError> {
        let uri = format!("file:{}?mode=ro&immutable=1", uri_path(copy));
        self.conn
            .execute_raw(Statement::from_sql_and_values(
                self.conn.get_database_backend(),
                format!("ATTACH DATABASE ? AS {}", ident(alias)),
                [uri.into()],
            ))
            .await?;
        Ok(())
    }

    /// Starts the import's one write transaction, with foreign keys checked at its commit.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn begin(self) -> Result<ImportStore, DbError> {
        let txn = begin_write(&self.conn).await?;
        txn.execute_unprepared("PRAGMA defer_foreign_keys = ON")
            .await?;
        Ok(ImportStore {
            txn,
            conn: self.conn,
            path: self.path,
        })
    }

    /// Closes the file without importing into it.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn close(self) -> Result<(), DbError> {
        Ok(self.conn.close().await?)
    }
}

/// Points SQLite's temporary files at `dir`.
///
/// `temp_store_directory` is a deprecated pragma, kept by SQLite for this; a build without it
/// ignores it, and the temporary files then go where `SQLITE_TMPDIR` says.
async fn set_temp_dir(conn: &DatabaseConnection, dir: &Path) -> Result<(), DbError> {
    let quoted = dir.display().to_string().replace('\'', "''");
    conn.execute_unprepared(&format!("PRAGMA temp_store_directory = '{quoted}'"))
        .await?;
    Ok(())
}

/// `path` written for a `file:` URI: every byte but the unreserved ones and `/` escaped.
fn uri_path(path: &Path) -> String {
    let text = path.display().to_string();
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0F)]));
        }
    }
    out
}

/// Upper-case hex digits.
const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// `name` as an SQL identifier: in double quotes, any double quote in it doubled.
pub(crate) fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// The import's one write transaction on a new database file.
///
/// Dropping it rolls everything back; [`Self::commit`] keeps it.
#[derive(Debug)]
pub struct ImportStore {
    txn: DatabaseTransaction,
    conn: DatabaseConnection,
    path: PathBuf,
}

impl ImportStore {
    /// The file being built.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Commits, moves every change from the write-ahead log into the file and closes it, so
    /// the file stands alone with no `-wal` or `-shm` beside it.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn commit(self) -> Result<(), DbError> {
        self.txn.commit().await?;
        self.conn
            .execute_unprepared("PRAGMA wal_checkpoint(TRUNCATE)")
            .await?;
        self.conn.close().await?;
        Ok(())
    }

    /// Rolls everything back and closes the file.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn abandon(self) -> Result<(), DbError> {
        self.txn.rollback().await?;
        self.conn.close().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_quoted_whatever_they_hold() {
        assert_eq!(ident("chat"), "\"chat\"");
        assert_eq!(ident("a\"b"), "\"a\"\"b\"");
        assert_eq!(ident("where"), "\"where\"");
    }

    #[test]
    fn a_path_is_escaped_for_a_uri() {
        assert_eq!(
            uri_path(Path::new("/data/a b/x?y#z%.db")),
            "/data/a%20b/x%3Fy%23z%25.db"
        );
    }
}
