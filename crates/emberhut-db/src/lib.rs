//! The emberhut server's database: SQLite through `SeaORM`.
//!
//! This is the only crate that knows the schema. Its tables are `SeaORM` entities, its schema
//! is made by migrations (the only SQL in the project, besides the backup's `VACUUM INTO` and
//! recall's full-text search), and
//! the server reaches it through stores ([`SessionStore`], [`PairingStore`], [`RoomStore`],
//! [`MessageStore`], [`SnapshotStore`], [`SettingStore`], [`LogStore`], [`EventStore`],
//! [`FileStore`], [`PushStore`], [`SecretLinkStore`], [`BuildStore`], [`BackupStore`],
//! [`NoteStore`], [`CorrectionStore`], [`RecallStore`], [`ConversationStore`], [`AgentStore`],
//! [`ApprovalStore`]) that take
//! and return `emberhut-core`'s types. Every operation that changes more than one row runs in one
//! transaction, and the app state is read in one.

use std::path::{Path, PathBuf};
use std::time::Duration;

use sea_orm::sqlx::sqlite::{SqliteJournalMode, SqliteSynchronous};
use sea_orm::{
    ConnectOptions, DatabaseConnection, DatabaseTransaction, DbErr, RuntimeErr,
    SqliteTransactionMode, TransactionOptions, TransactionTrait,
};
use sea_orm_migration::MigratorTrait;

mod accounts;
mod agents;
mod approvals;
mod areas;
mod artifacts;
mod backup;
mod builds;
mod calendar;
mod calls;
mod chrome;
mod conversation;
mod corrections;
mod dark;
mod entity;
mod events;
mod family;
mod files;
mod health_data;
pub mod import;
mod links;
mod log_lines;
mod machines;
mod message_refs;
mod messages;
pub mod messaging;
mod migration;
mod notes;
mod paired_runners;
mod pairing;
mod parties;
mod projects;
mod pulls;
mod push;
mod recall;
mod rooms;
mod runs;
mod secret_links;
mod sessions;
mod settings;
mod snapshot;
mod tasks;
mod threads;
mod time;
mod url;
mod workers;

pub use accounts::{PrefKey, PrefStore, UsageStore};
pub use agents::{AgentStore, AgentStoreError, AgentThread, ThreadPlace};
pub use approvals::{
    ApprovalFilter, ApprovalStore, ApprovalStoreError, DecisionMade, OverCap, StepToUse,
    UNANSWERED, UNUSED,
};
pub use areas::{AreaChange, AreaStore, AreaStoreError};
pub use artifacts::{ArtifactStore, StoredArtifact, X_REPLY};
pub use backup::BackupStore;
pub use builds::BuildStore;
pub use calendar::{CalendarStore, DayEvent};
pub use calls::{CallStore, LiveCall};
pub use chrome::{ChromeAbout, ChromeProfileRow, ChromeStore};
pub use conversation::{ConversationStore, Said};
pub use corrections::{CorrectionStore, TrialEnd};
pub use dark::{DARK_FILE, DarkLine, DarkStore};
pub use events::EventStore;
pub use family::{FamilyStore, HOME_ROOM};
pub use files::FileStore;
pub use health_data::HealthStore;
pub use links::{Link, LinkStore};
pub use log_lines::LogStore;
pub use machines::{MachineStore, StoredSample};
pub use message_refs::MessageRefStore;
pub use messages::MessageStore;
pub use messaging::{MailStore, SlackStore, VoiceNoteStore, WhatsAppStore};
pub use notes::{NoteStore, OwnerNote};
pub use paired_runners::{PairedRunner, PairedRunnerStore, RunnerPairing, RunnerRedemption};
pub use pairing::{PairingStore, Redemption};
pub use parties::PartyStore;
pub use projects::{ProjectStore, ProjectStoreError};
pub use pulls::{PullStore, RemoteFile};
pub use push::PushStore;
pub use recall::{IndexBatch, RecallStore, TranscriptOffset};
pub use rooms::RoomStore;
pub use runs::RunStore;
pub use secret_links::SecretLinkStore;
pub use sessions::SessionStore;
pub use settings::{MACHINE_ID, SettingKey, SettingStore};
pub use snapshot::{HistoryPage, RoomLine, Snapshot, SnapshotStore};
pub use tasks::{HistoryLine, NewTask, StoredPlan, TaskLine, TaskStore, TaskStoreError};
pub use threads::{Recorded, ThreadStore};
pub use url::{DatabaseUrl, InvalidDatabaseUrl};
pub use workers::{Agent, AgentSeen, StreamPlace, WorkerStore, WorkingUnit};

/// Connections in the pool. SQLite runs one writer at a time; the others read alongside it.
const MAX_CONNECTIONS: u32 = 8;

/// How long the database waits for other processes, at start-up and per statement.
#[derive(Clone, Copy, Debug)]
struct Timing {
    /// How long a statement waits for another connection's write before it fails.
    busy: Duration,
    /// How long opening and migrating wait, all told, for another process setting the same
    /// file up. Much longer than `busy`: a first start's migration is a one-off that may take
    /// a while on a slow or loaded disk, and the second start has nothing to do until it ends.
    startup: Duration,
}

impl Timing {
    /// What the server runs with.
    const SERVER: Self = Self {
        busy: Duration::from_secs(5),
        startup: Duration::from_secs(60),
    };
}

/// A statement slower than this is logged as a warning.
const SLOW_STATEMENT: Duration = Duration::from_secs(1);

/// Why the database could not do what was asked.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DbError {
    /// The database could not be opened.
    #[error("could not open the database")]
    Connect(#[source] DbErr),
    /// The schema could not be brought up to date.
    #[error("could not migrate the database")]
    Migrate(#[source] DbErr),
    /// There is no database file where one was expected.
    #[error("there is no database at {}", .0.display())]
    Missing(PathBuf),
    /// The file is a SQLite database, but not one this program made.
    #[error("{} is not an emberhut database", .0.display())]
    NotEmberhut(PathBuf),
    /// The database's schema is older than this program's (only [`Database::open_existing`]
    /// refuses it; [`Database::connect`] brings it up to date).
    #[error("the database needs {0} migration(s) that the server applies when it starts")]
    Outdated(usize),
    /// A query failed.
    #[error("database query failed")]
    Query(#[from] DbErr),
    /// A stored hash is not 32 bytes long.
    #[error("stored hash is not 32 bytes long")]
    CorruptHash,
    /// A stored value is not one its column takes (a row changed by hand, or by a newer
    /// build).
    #[error("stored {column} is not one: {value:?}")]
    CorruptValue {
        /// The table and column, `table.column`.
        column: &'static str,
        /// The value as stored.
        value: String,
    },
    /// A stored instant is outside the range the server can work with.
    #[error("stored instant out of range: {0} ms since the epoch")]
    InstantOutOfRange(i64),
    /// An archived row is not in the archive's format.
    #[error(transparent)]
    RowFormat(#[from] import::RowFormatError),
    /// An import's target is there already: an import builds a new file only.
    #[error("{} is there already; an import makes a new database only", .0.display())]
    TargetExists(PathBuf),
    /// An archived row to replace is not in the archive.
    #[error("no row of {database}.{table} is archived at {ordinal}")]
    NotArchived {
        /// The source database.
        database: String,
        /// The source table.
        table: String,
        /// The row's place.
        ordinal: i64,
    },
    /// The database is in use already (it has devices or browser profiles), where an unused one
    /// was expected.
    #[error("{} is in use already (it has devices or browser profiles)", .0.display())]
    NotUnused(PathBuf),
    /// The database was served as another machine than the one a copy carries.
    #[error("{} was served as machine {found}, not {carried}", .path.display())]
    OtherMachine {
        /// The database.
        path: PathBuf,
        /// The machine it was served as.
        found: String,
        /// The machine the copy carries.
        carried: String,
    },
    /// A copy did not end with the rows it started with.
    #[error("copied {expected} row(s) of {table} but found {found}")]
    CopyMismatch {
        /// The table.
        table: &'static str,
        /// The rows copied.
        expected: u64,
        /// The rows found after the copy.
        found: u64,
    },
    /// A setting's value is not JSON of its key's type (or could not be written as JSON).
    #[error("the setting {key} is not of its type")]
    Setting {
        /// The setting's key.
        key: String,
        /// Why not.
        #[source]
        source: serde_json::Error,
    },
}

/// The database: a pool of connections with its schema up to date. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Database {
    conn: DatabaseConnection,
    /// The database's file.
    path: PathBuf,
}

impl Database {
    /// Opens the database at `url`, making the file if it is not there, and applies the
    /// migrations it lacks, all in one transaction: a failed or interrupted migration leaves
    /// the file as it was, and of two processes starting at once the second waits for the
    /// first and then finds nothing to do.
    ///
    /// Every connection enforces foreign keys, uses the write-ahead log (readers do not wait
    /// for the writer) and waits up to a few seconds for another connection's write.
    ///
    /// # Errors
    ///
    /// [`DbError::Connect`] or [`DbError::Migrate`].
    pub async fn connect(url: &DatabaseUrl) -> Result<Self, DbError> {
        let timing = Timing::SERVER;
        let started = tokio::time::Instant::now();
        let conn = open(url, true, timing).await?;
        migrate::<migration::Migrator>(&conn, started + timing.startup)
            .await
            .map_err(DbError::Migrate)?;
        Ok(Self {
            conn,
            path: url.path().to_path_buf(),
        })
    }

    /// Opens the database at `url` only if it is there, is emberhut's, and its schema is up to
    /// date. For tools that work on the server's database beside it, such as
    /// `emberhut-server pair`: they must neither make a second, empty database nor change the
    /// schema under a running server, and must leave a file that is not emberhut's as it was.
    ///
    /// The file is first opened read-only, with no settings that persist in it (such as the
    /// journal mode), and its migrations are read without creating their table. Only a
    /// database that passes is opened for writing.
    ///
    /// # Errors
    ///
    /// [`DbError::Missing`] when there is no file, [`DbError::NotEmberhut`] when the file is some
    /// other SQLite database, [`DbError::Outdated`] when migrations are pending, and
    /// [`DbError::Connect`] or [`DbError::Query`] otherwise.
    pub async fn open_existing(url: &DatabaseUrl) -> Result<Self, DbError> {
        if !url.path().is_file() {
            return Err(DbError::Missing(url.path().to_path_buf()));
        }
        let probe = sea_orm::Database::connect(probe_options(url))
            .await
            .map_err(DbError::Connect)?;
        let checked = check_schema::<migration::Migrator>(&probe, url).await;
        probe.close().await?;
        checked?;
        Ok(Self {
            conn: open(url, false, Timing::SERVER).await?,
            path: url.path().to_path_buf(),
        })
    }

    /// The store of devices and their sessions.
    #[must_use]
    pub fn sessions(&self) -> SessionStore {
        SessionStore::new(self.conn.clone())
    }

    /// The store of pairing codes.
    #[must_use]
    pub fn pairing(&self) -> PairingStore {
        PairingStore::new(self.conn.clone())
    }

    /// The store of rooms.
    #[must_use]
    pub fn rooms(&self) -> RoomStore {
        RoomStore::new(self.conn.clone())
    }

    /// The store of messages.
    #[must_use]
    pub fn messages(&self) -> MessageStore {
        MessageStore::new(self.conn.clone())
    }

    /// The store of settings.
    #[must_use]
    pub fn settings(&self) -> SettingStore {
        SettingStore::new(self.conn.clone())
    }

    /// The activity log.
    #[must_use]
    pub fn log(&self) -> LogStore {
        LogStore::new(self.conn.clone())
    }

    /// The store of events.
    #[must_use]
    pub fn events(&self) -> EventStore {
        EventStore::new(self.conn.clone())
    }

    /// The store of the phones' push addresses.
    #[must_use]
    pub fn push(&self) -> PushStore {
        PushStore::new(self.conn.clone())
    }

    /// The store of one-time secret links.
    #[must_use]
    pub fn secret_links(&self) -> SecretLinkStore {
        SecretLinkStore::new(self.conn.clone())
    }

    /// The reader of the app state's snapshots.
    #[must_use]
    pub fn snapshots(&self) -> SnapshotStore {
        SnapshotStore::new(self.conn.clone())
    }

    /// The store of the chat's files.
    #[must_use]
    pub fn files(&self) -> FileStore {
        FileStore::new(self.conn.clone())
    }

    /// The database file's path: the server keeps the chat's files beside it.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The newest builds of the app.
    #[must_use]
    pub fn builds(&self) -> BuildStore {
        BuildStore::new(self.conn.clone())
    }

    /// The maker of consistent copies of the database, for backups.
    #[must_use]
    pub fn backups(&self) -> BackupStore {
        BackupStore::new(self.conn.clone(), self.path.clone())
    }

    /// The store of tasks and their history.
    #[must_use]
    pub fn tasks(&self) -> TaskStore {
        TaskStore::new(self.conn.clone())
    }

    /// The store of projects.
    #[must_use]
    pub fn projects(&self) -> ProjectStore {
        ProjectStore::new(self.conn.clone())
    }

    /// The store of projects' areas.
    #[must_use]
    pub fn areas(&self) -> AreaStore {
        AreaStore::new(self.conn.clone())
    }

    /// The store of what tasks produced.
    #[must_use]
    pub fn artifacts(&self) -> ArtifactStore {
        ArtifactStore::new(self.conn.clone())
    }

    /// The store of the work linked to tasks.
    #[must_use]
    pub fn links(&self) -> LinkStore {
        LinkStore::new(self.conn.clone())
    }

    /// The store of every turn of work on a task.
    #[must_use]
    pub fn runs(&self) -> RunStore {
        RunStore::new(self.conn.clone())
    }

    /// The assistant's answers' chips.
    #[must_use]
    pub fn message_refs(&self) -> MessageRefStore {
        MessageRefStore::new(self.conn.clone())
    }

    /// The owner's quick notes and the assistant's diary.
    #[must_use]
    pub fn notes(&self) -> NoteStore {
        NoteStore::new(self.conn.clone())
    }

    /// What the assistant was corrected on.
    #[must_use]
    pub fn corrections(&self) -> CorrectionStore {
        CorrectionStore::new(self.conn.clone())
    }

    /// Recall's index.
    #[must_use]
    pub fn recall(&self) -> RecallStore {
        RecallStore::new(self.conn.clone())
    }

    /// The conversation, read back.
    #[must_use]
    pub fn conversation(&self) -> ConversationStore {
        ConversationStore::new(self.conn.clone())
    }

    /// The owner's paired Chrome profiles and the tasks' browsers.
    #[must_use]
    pub fn chrome(&self) -> ChromeStore {
        ChromeStore::new(self.conn.clone())
    }

    /// The store of coding threads and their follows.
    #[must_use]
    pub fn threads(&self) -> ThreadStore {
        ThreadStore::new(self.conn.clone())
    }

    /// The runner machines' samples and points.
    #[must_use]
    pub fn machines(&self) -> MachineStore {
        MachineStore::new(self.conn.clone())
    }

    /// The runners' accounts and their use.
    #[must_use]
    pub fn usage(&self) -> UsageStore {
        UsageStore::new(self.conn.clone())
    }

    /// The person's small account choices.
    #[must_use]
    pub fn prefs(&self) -> PrefStore {
        PrefStore::new(self.conn.clone())
    }

    /// The runners paired at run time, and the codes they pair with.
    #[must_use]
    pub fn paired_runners(&self) -> PairedRunnerStore {
        PairedRunnerStore::new(self.conn.clone())
    }

    /// The files already brought from runners' machines.
    #[must_use]
    pub fn pulls(&self) -> PullStore {
        PullStore::new(self.conn.clone())
    }

    /// The store of recorded and shared calls.
    #[must_use]
    pub fn calls(&self) -> CallStore {
        CallStore::new(self.conn.clone())
    }

    /// The store of the calendar's days.
    #[must_use]
    pub fn calendar(&self) -> CalendarStore {
        CalendarStore::new(self.conn.clone())
    }

    /// The store of health and screen-time days.
    #[must_use]
    pub fn health_data(&self) -> HealthStore {
        HealthStore::new(self.conn.clone())
    }

    /// The owner's Slack messages.
    #[must_use]
    pub fn slack(&self) -> SlackStore {
        SlackStore::new(self.conn.clone())
    }

    /// The owner's mail.
    #[must_use]
    pub fn mail(&self) -> MailStore {
        MailStore::new(self.conn.clone())
    }

    /// Voice notes and what they said.
    #[must_use]
    pub fn voice_notes(&self) -> VoiceNoteStore {
        VoiceNoteStore::new(self.conn.clone())
    }

    /// The assistant's own WhatsApp number's chats, messages and log.
    #[must_use]
    pub fn whatsapp(&self) -> WhatsAppStore {
        WhatsAppStore::new(self.conn.clone())
    }

    /// The workers' and ultra runs' sub-agents, streams and runs.
    #[must_use]
    pub fn workers(&self) -> WorkerStore {
        WorkerStore::new(self.conn.clone())
    }

    /// The store of agents, their lists, approvers, policies, trusted requesters and threads.
    #[must_use]
    pub fn agents(&self) -> AgentStore {
        AgentStore::new(self.conn.clone())
    }

    /// The store of approval requests.
    #[must_use]
    pub fn approvals(&self) -> ApprovalStore {
        ApprovalStore::new(self.conn.clone())
    }

    /// The store of family members' rooms and phones.
    #[must_use]
    pub fn family(&self) -> FamilyStore {
        FamilyStore::new(self.conn.clone())
    }

    /// The store of parties: AI personas talking, and what they said.
    #[must_use]
    pub fn parties(&self) -> PartyStore {
        PartyStore::new(self.conn.clone())
    }

    /// Closes every connection, after the ones in use are returned.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] when the pool cannot be closed cleanly.
    pub async fn close(self) -> Result<(), DbError> {
        Ok(self.conn.close().await?)
    }
}

/// A pool on `url`, making the file when `create` is set.
///
/// Opening a new file can find it locked: a connection switches the file to the write-ahead
/// log, and SQLite refuses that at once, without waiting, while another process is setting the
/// same file up. So a locked open is tried again, for up to the start-up wait.
async fn open(
    url: &DatabaseUrl,
    create: bool,
    timing: Timing,
) -> Result<DatabaseConnection, DbError> {
    let deadline = tokio::time::Instant::now() + timing.startup;
    loop {
        match sea_orm::Database::connect(connect_options(url, create, timing)).await {
            Err(error) if is_busy(&error) && tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(RETRY_OPEN_AFTER).await;
            }
            result => return result.map_err(DbError::Connect),
        }
    }
}

/// How long to wait before opening, or starting to migrate, a locked file again.
const RETRY_OPEN_AFTER: Duration = Duration::from_millis(50);

fn connect_options(url: &DatabaseUrl, create: bool, timing: Timing) -> ConnectOptions {
    let mut options = ConnectOptions::new(url.as_str());
    options
        .max_connections(MAX_CONNECTIONS)
        // Statements go to `log` (bridged into the server's tracing) at debug, slow ones as
        // warnings.
        .sqlx_logging(true)
        .sqlx_logging_level(log::LevelFilter::Debug)
        .sqlx_slow_statements_logging_settings(log::LevelFilter::Warn, SLOW_STATEMENT)
        .map_sqlx_sqlite_opts(move |sqlite| {
            sqlite
                .create_if_missing(create)
                .foreign_keys(true)
                .journal_mode(SqliteJournalMode::Wal)
                .synchronous(SqliteSynchronous::Normal)
                .busy_timeout(timing.busy)
        });
    options
}

/// A single read-only connection to `url` that changes nothing in the file.
fn probe_options(url: &DatabaseUrl) -> ConnectOptions {
    let mut options = ConnectOptions::new(url.as_str());
    options
        .max_connections(1)
        .sqlx_logging(true)
        .sqlx_logging_level(log::LevelFilter::Debug)
        .map_sqlx_sqlite_opts(|sqlite| {
            sqlite
                .create_if_missing(false)
                .read_only(true)
                .busy_timeout(Timing::SERVER.busy)
        });
    options
}

/// Whether the database on `conn` is emberhut's (its first migration is recorded) and up to date,
/// read without writing anything.
async fn check_schema<M: MigratorTrait>(
    conn: &DatabaseConnection,
    url: &DatabaseUrl,
) -> Result<(), DbError> {
    let applied = M::get_applied_migrations_read_only(conn).await?;
    let first = M::migrations().first().map(|first| first.name().to_owned());
    if !applied
        .iter()
        .any(|done| Some(done.name()) == first.as_deref())
    {
        return Err(DbError::NotEmberhut(url.path().to_path_buf()));
    }
    let pending = M::get_pending_migrations_read_only(conn).await?;
    if pending.is_empty() {
        Ok(())
    } else {
        Err(DbError::Outdated(pending.len()))
    }
}

/// Whether `error` is SQLite's `SQLITE_BUSY` (any of its extended codes).
fn is_busy(error: &DbErr) -> bool {
    let (DbErr::Conn(RuntimeErr::SqlxError(error))
    | DbErr::Exec(RuntimeErr::SqlxError(error))
    | DbErr::Query(RuntimeErr::SqlxError(error))) = error
    else {
        return false;
    };
    error
        .as_database_error()
        .and_then(sea_orm::sqlx::error::DatabaseError::code)
        .and_then(|code| code.parse::<u32>().ok())
        .is_some_and(|code| code & 0xff == SQLITE_BUSY)
}

/// SQLite's primary result code for "the database file is locked".
const SQLITE_BUSY: u32 = 5;

/// Applies `M`'s pending migrations in one write transaction.
///
/// `sea-orm-migration` wraps a migration in a transaction only on Postgres; on SQLite each
/// statement would otherwise commit on its own, and a migration that stopped half-way would
/// leave tables behind that the next start trips over.
///
/// The transaction's `BEGIN` waits for another process's migration of the same file. That one
/// can hold the write lock for longer than a statement's busy timeout (a loaded disk), and the
/// answer then is to wait for it, not to fail the start: a locked `BEGIN` is tried again until
/// `deadline`.
async fn migrate<M: MigratorTrait>(
    conn: &DatabaseConnection,
    deadline: tokio::time::Instant,
) -> Result<(), DbErr> {
    let txn = loop {
        match begin_write(conn).await {
            Err(error) if is_busy(&error) && tokio::time::Instant::now() < deadline => {
                log::info!("another process is setting the database up; waiting for it");
                tokio::time::sleep(RETRY_OPEN_AFTER).await;
            }
            begun => break begun?,
        }
    };
    M::up(&txn, None).await?;
    txn.commit().await
}

/// Starts a transaction that writes.
///
/// It takes SQLite's write lock at `BEGIN`, so a second writer waits for the first (up to the
/// busy timeout) rather than failing when its read turns into a write halfway through.
async fn begin_write(conn: &DatabaseConnection) -> Result<DatabaseTransaction, DbErr> {
    conn.begin_with_options(TransactionOptions {
        sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
        ..TransactionOptions::default()
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use sea_orm::{ConnectionTrait, Statement};
    use sea_orm_migration::SchemaManager;
    use sea_orm_migration::async_trait::async_trait;
    use sea_orm_migration::prelude::*;
    use tempfile::TempDir;

    use super::*;

    /// A start-up deadline far enough ahead for any test.
    fn later() -> tokio::time::Instant {
        tokio::time::Instant::now() + Timing::SERVER.startup
    }

    /// A second start waits out a first start's migration that holds the write lock for longer
    /// than a statement's busy timeout (what a loaded disk does to it), instead of failing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_second_start_waits_out_a_long_first_migration() {
        let dir = tempfile::tempdir().unwrap();
        let url = url_in(&dir);
        let short = Timing {
            busy: Duration::from_millis(100),
            startup: Duration::from_secs(30),
        };
        let first = open(&url, true, short).await.unwrap();
        let second = open(&url, true, short).await.unwrap();
        let holding = begin_write(&first).await.unwrap();
        let release = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(600)).await;
            holding.commit().await.unwrap();
        });
        let deadline = tokio::time::Instant::now() + short.startup;
        migrate::<migration::Migrator>(&second, deadline)
            .await
            .unwrap();
        release.await.unwrap();

        // Without the wait, the same lock fails the start after the busy timeout.
        let holding = begin_write(&first).await.unwrap();
        let no_wait = tokio::time::Instant::now();
        assert!(
            migrate::<migration::Migrator>(&second, no_wait)
                .await
                .is_err()
        );
        holding.rollback().await.unwrap();
    }

    fn url_in(dir: &TempDir) -> DatabaseUrl {
        DatabaseUrl::from_path(&dir.path().join("emberhut.db")).unwrap()
    }

    /// The real migrations, then one that fails half-way.
    struct Failing;

    #[async_trait]
    impl MigratorTrait for Failing {
        fn migrations() -> Vec<Box<dyn MigrationTrait>> {
            let mut all = migration::Migrator::migrations();
            all.push(Box::new(HalfWay));
            all
        }
    }

    #[derive(DeriveMigrationName)]
    struct HalfWay;

    #[async_trait]
    #[expect(
        elided_lifetimes_in_paths,
        reason = "async_trait matches the trait's signature, which elides SchemaManager's lifetime"
    )]
    impl MigrationTrait for HalfWay {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .create_table(
                    Table::create()
                        .table("half_way")
                        .col(ColumnDef::new("id").integer().primary_key())
                        .to_owned(),
                )
                .await?;
            Err(DbErr::Migration("stopped half-way".to_owned()))
        }
    }

    #[tokio::test]
    async fn a_failed_migration_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let url = url_in(&dir);
        let conn = open(&url, true, Timing::SERVER).await.unwrap();
        assert!(migrate::<Failing>(&conn, later()).await.is_err());

        let manager = SchemaManager::new(&conn);
        for table in ["devices", "rooms", "half_way"] {
            assert!(!manager.has_table(table).await.unwrap(), "{table} was left");
        }
        // And the real migrations then run on the same file.
        migrate::<migration::Migrator>(&conn, later())
            .await
            .unwrap();
        assert!(manager.has_table("devices").await.unwrap());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn two_processes_starting_on_a_new_file_both_start() {
        let dir = tempfile::tempdir().unwrap();
        let url = url_in(&dir);
        let (a, b) = tokio::join!(Database::connect(&url), Database::connect(&url));
        a.unwrap();
        b.unwrap();
    }

    #[tokio::test]
    async fn open_existing_neither_makes_nor_migrates_a_database() {
        let dir = tempfile::tempdir().unwrap();
        let url = url_in(&dir);
        assert!(matches!(
            Database::open_existing(&url).await,
            Err(DbError::Missing(_))
        ));
        assert!(!url.path().exists(), "no file was made");

        let db = Database::connect(&url).await.unwrap();
        db.close().await.unwrap();
        // Closed, as when the server is stopped: still opened, read-only first.
        Database::open_existing(&url).await.unwrap();
    }

    #[tokio::test]
    async fn a_program_newer_than_the_database_finds_it_outdated() {
        let dir = tempfile::tempdir().unwrap();
        let url = url_in(&dir);
        Database::connect(&url)
            .await
            .unwrap()
            .close()
            .await
            .unwrap();
        let probe = sea_orm::Database::connect(probe_options(&url))
            .await
            .unwrap();
        // `Failing` is the real migrations plus one more, as a newer program would have.
        assert!(matches!(
            check_schema::<Failing>(&probe, &url).await,
            Err(DbError::Outdated(1))
        ));
        assert!(
            check_schema::<migration::Migrator>(&probe, &url)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn open_existing_leaves_another_program_s_database_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("other.db");
        let other = sea_orm::Database::connect(format!(
            "{}?mode=rwc",
            DatabaseUrl::from_path(&path).unwrap()
        ))
        .await
        .unwrap();
        SchemaManager::new(&other)
            .create_table(
                Table::create()
                    .table("notes")
                    .col(ColumnDef::new("id").integer().primary_key())
                    .to_owned(),
            )
            .await
            .unwrap();
        other.close().await.unwrap();
        let before = std::fs::read(&path).unwrap();

        let url: DatabaseUrl = DatabaseUrl::from_path(&path).unwrap();
        assert!(matches!(
            Database::open_existing(&url).await,
            Err(DbError::NotEmberhut(_))
        ));
        assert_eq!(std::fs::read(&path).unwrap(), before, "not a byte changed");
        assert!(
            !dir.path().join("other.db-wal").exists(),
            "not switched to the write-ahead log"
        );
    }

    /// A test reads the effective settings back; no application code runs SQL.
    #[tokio::test]
    async fn every_connection_has_the_settings_it_was_given() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::connect(&url_in(&dir)).await.unwrap();
        for (pragma, expected) in [
            ("journal_mode", "wal"),
            ("synchronous", "1"),
            ("foreign_keys", "1"),
            ("busy_timeout", "5000"),
        ] {
            let row = db
                .conn
                .query_one_raw(Statement::from_string(
                    db.conn.get_database_backend(),
                    format!("PRAGMA {pragma}"),
                ))
                .await
                .unwrap()
                .unwrap();
            let value = row
                .try_get_by_index::<String>(0)
                .or_else(|_| row.try_get_by_index::<i64>(0).map(|n| n.to_string()))
                .unwrap();
            assert_eq!(value, expected, "{pragma}");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_second_writer_waits_for_the_first_instead_of_failing() {
        let dir = tempfile::tempdir().unwrap();
        let url = url_in(&dir);
        let first = Database::connect(&url).await.unwrap();
        let second = Database::connect(&url).await.unwrap();

        let holding = begin_write(&first.conn).await.unwrap();
        let started = Instant::now();
        let release = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            holding.commit().await.unwrap();
        });
        let now = jiff::Timestamp::now();
        second
            .rooms()
            .mark_seen(emberhut_core::RoomId::MAIN, now)
            .await
            .unwrap();
        assert!(started.elapsed() >= Duration::from_millis(250));
        release.await.unwrap();
    }
}
