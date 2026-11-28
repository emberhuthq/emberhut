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
//! A later import can also add to a database that is in use: [`ImportTarget::open_existing`]
//! opens it as its server left it (up to date, never migrated here), [`ImportTarget::read`]
//! reads it in a snapshot that keeps no writer waiting, and [`ImportTarget::begin`] then takes
//! the write lock for one short transaction of appends (`append`).
//!
//! The importer knows the source's format; this module knows Emberhut's schema. What passes
//! between them is the archive's [`Row`]s and plain records of Emberhut's rows.

mod agents;
mod append;
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
    TransactionTrait,
};

pub use agents::{AgentLists, ImportedAgentThread};
pub use append::Numbered;
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

use crate::{DatabaseUrl, DbError, begin_write, check_schema, migrate, migration, probe_options};

/// How long a statement on the import's file waits for a lock: nothing else opens it, so only
/// the import's own connection could hold one.
const BUSY: Duration = Duration::from_secs(5);

/// How long a statement on a database in use waits for its server's write to end: the server's
/// writes are short, and the append's one transaction must not fail because one was under way.
const BUSY_IN_USE: Duration = Duration::from_secs(30);

/// A database file an import writes into, before its transaction starts: a new one being built,
/// or one in use that an import adds to.
#[derive(Debug)]
pub struct ImportTarget {
    conn: DatabaseConnection,
    path: PathBuf,
    /// Whether the file was there already (and may be in use by its server).
    existing: bool,
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
        let conn = connect_one(path, true, BUSY).await?;
        set_temp_dir(&conn, temp_dir).await?;
        migrate::<migration::Migrator>(&conn, tokio::time::Instant::now())
            .await
            .map_err(DbError::Migrate)?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
            existing: false,
        })
    }

    /// Opens the database at `url`, which must be there, be emberhut's and be up to date, to
    /// add an import's rows to it while its server may be running. Nothing in its schema is
    /// changed: a database with migrations pending is refused, not migrated.
    ///
    /// Its one connection waits for the server's writes rather than failing, and keeps SQLite's
    /// temporary files in `temp_dir`. Its transactions are short: [`Self::read`] reads in a
    /// snapshot, which keeps no writer waiting, and [`Self::begin`] takes the write lock.
    ///
    /// # Errors
    ///
    /// [`DbError::Missing`], [`DbError::NotEmberhut`], [`DbError::Outdated`],
    /// [`DbError::Connect`] and [`DbError::Query`].
    pub async fn open_existing(url: &DatabaseUrl, temp_dir: &Path) -> Result<Self, DbError> {
        let path = url.path();
        if !path.is_file() {
            return Err(DbError::Missing(path.to_path_buf()));
        }
        let probe = sea_orm::Database::connect(probe_options(url))
            .await
            .map_err(DbError::Connect)?;
        let checked = check_schema::<migration::Migrator>(&probe, url).await;
        probe.close().await?;
        checked?;
        let conn = connect_one(path, false, BUSY_IN_USE).await?;
        set_temp_dir(&conn, temp_dir).await?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
            existing: true,
        })
    }

    /// Starts a transaction that only reads: a snapshot of the file as it is now, which does
    /// not keep its server from writing. [`ImportStore::end_read`] ends it.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn read(self) -> Result<ImportStore, DbError> {
        let txn = self.conn.begin().await?;
        Ok(ImportStore {
            txn,
            conn: self.conn,
            path: self.path,
            existing: self.existing,
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
            existing: self.existing,
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

/// One connection to the file at `path` that is never replaced (what is attached to it, and its
/// `TEMP` tables, belong to that connection), making the file only when `create`.
async fn connect_one(
    path: &Path,
    create: bool,
    busy: Duration,
) -> Result<DatabaseConnection, DbError> {
    // The URL only picks the driver: the file is the one `filename` names.
    let url = DatabaseUrl::from_path(path).map_err(|_| DbError::Missing(path.to_path_buf()))?;
    let mut options = ConnectOptions::new(url.as_str());
    let filename = path.to_path_buf();
    options
        .max_connections(1)
        .min_connections(1)
        .sqlx_logging(true)
        .sqlx_logging_level(log::LevelFilter::Trace)
        .map_sqlx_sqlite_opts(move |sqlite| {
            sqlite
                .filename(&filename)
                .create_if_missing(create)
                .foreign_keys(true)
                .journal_mode(SqliteJournalMode::Wal)
                .synchronous(SqliteSynchronous::Normal)
                .busy_timeout(busy)
                .pragma("temp_store", "FILE")
        });
    sea_orm::Database::connect(options)
        .await
        .map_err(DbError::Connect)
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

/// The import's one write transaction on a new database file, or a transaction on a database
/// in use that an import adds to ([`ImportTarget::open_existing`]).
///
/// Dropping it rolls everything back; [`Self::commit`] keeps it.
#[derive(Debug)]
pub struct ImportStore {
    txn: DatabaseTransaction,
    conn: DatabaseConnection,
    path: PathBuf,
    existing: bool,
}

impl ImportStore {
    /// The file being built.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Commits and closes the file. A new file is first checkpointed, every change moved from
    /// the write-ahead log into it, so it stands alone with no `-wal` or `-shm` beside it; a
    /// file that was there already is left to its server's checkpoints.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn commit(self) -> Result<(), DbError> {
        self.txn.commit().await?;
        if !self.existing {
            self.conn
                .execute_unprepared("PRAGMA wal_checkpoint(TRUNCATE)")
                .await?;
        }
        self.conn.close().await?;
        Ok(())
    }

    /// Ends a transaction that only read ([`ImportTarget::read`]), keeping the connection for
    /// the next.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn end_read(self) -> Result<ImportTarget, DbError> {
        self.txn.rollback().await?;
        Ok(ImportTarget {
            conn: self.conn,
            path: self.path,
            existing: self.existing,
        })
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
