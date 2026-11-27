//! The directory's own SQLite file, through `SeaORM` (ADR 0002's rules: no SQL outside the
//! migrations, writes in `BEGIN IMMEDIATE` transactions, instants as milliseconds).
//!
//! It holds accounts (the Clerk user id only) and machines (id, public key, name, account, when
//! linked, last seen, and the box holding it), and, for the replay rules of ADR 0007's
//! section 5, the signatures of accepted statements until their 5 minutes have passed and each
//! machine's latest link or unlink time. Both are pruned in every statement's transaction: a
//! statement older than the 5 minutes is stale anyway.
//!
//! The relay's database is not the server's, so it lives here rather than in `emberhut-db`, which
//! holds the server's schema.

use std::path::Path;
use std::time::Duration;

use emberhut_identity::{
    AccountId, MachineId, MachineName, PUBLIC_KEY_LEN, PublicKey, Signature, UnixMillis,
};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::sqlx::ConnectOptions as _;
use sea_orm::sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
};
use sea_orm::{
    ActiveValue::Set, ColumnTrait as _, ConnectionTrait, DatabaseConnection, DatabaseTransaction,
    DbErr, EntityTrait as _, PaginatorTrait as _, QueryFilter as _, QueryOrder as _,
    SqliteTransactionMode, SqlxSqliteConnector, TransactionOptions, TransactionTrait as _,
};
use sea_orm_migration::MigratorTrait as _;

mod entity;
mod migration;

use entity::{accepted_signature, account, latest_statement, machine};

/// Connections in the pool. SQLite runs one writer at a time; the others read alongside it.
const MAX_CONNECTIONS: u32 = 8;

/// How long a statement waits for another connection's write before it fails.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// A statement slower than this is logged as a warning.
const SLOW_STATEMENT: Duration = Duration::from_secs(1);

/// Why the directory's database failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// The file couldn't be opened.
    #[error("the directory's database couldn't be opened")]
    Open(#[source] sea_orm::sqlx::Error),
    /// The schema couldn't be brought up to date.
    #[error("the directory's database couldn't be migrated")]
    Migrate(#[source] DbErr),
    /// A query failed.
    #[error("a query on the directory's database failed")]
    Query(#[from] DbErr),
    /// A stored value isn't what the directory writes.
    #[error("the directory's database holds a {0} it can't read")]
    Corrupt(&'static str),
}

/// Why a statement wasn't accepted, or the database failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum WriteError {
    /// Its signature was already accepted.
    #[error("the statement was already accepted")]
    Replayed,
    /// It is older than the machine's latest accepted statement.
    #[error("the statement is older than the machine's latest")]
    Stale,
    /// The machine is linked to another account.
    #[error("the machine is linked to another account")]
    LinkedElsewhere,
    /// The account has as many machines as it may hold.
    #[error("the account is full")]
    AccountFull,
    /// The machine isn't linked.
    #[error("the machine isn't linked")]
    NotLinked,
    /// The database failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl From<DbErr> for WriteError {
    fn from(error: DbErr) -> Self {
        Self::Store(StoreError::Query(error))
    }
}

/// A link or unlink statement the directory is about to accept: whose, its signature, and the
/// time it names.
#[derive(Clone, Debug)]
pub(crate) struct Statement {
    /// The machine that signed it.
    pub(crate) machine: MachineId,
    /// Its signature, kept to refuse it a second time.
    pub(crate) signature: Signature,
    /// The time it names.
    pub(crate) time: UnixMillis,
}

/// A machine linked to an account, as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredMachine {
    /// Its id.
    pub(crate) id: MachineId,
    /// Its key.
    pub(crate) public_key: PublicKey,
    /// Its name.
    pub(crate) name: MachineName,
    /// When it was linked.
    pub(crate) linked_at: UnixMillis,
    /// When its uplink was last up, if ever.
    pub(crate) last_seen: Option<UnixMillis>,
    /// Whether a gateway holds its uplink now.
    pub(crate) connected: bool,
}

/// A machine to link: the statement's machine, key and name.
#[derive(Clone, Debug)]
pub(crate) struct NewLink {
    /// The machine's key.
    pub(crate) public_key: PublicKey,
    /// Its name.
    pub(crate) name: MachineName,
    /// The account it is linked to.
    pub(crate) account: AccountId,
    /// The statement that links it.
    pub(crate) statement: Statement,
}

/// The directory's database. Cheap to clone.
#[derive(Clone, Debug)]
pub(crate) struct Store {
    conn: DatabaseConnection,
}

impl Store {
    /// Opens the file at `path`, making it if it isn't there, and applies the migrations it
    /// lacks in one transaction. Every connection enforces foreign keys, uses the write-ahead
    /// log and waits up to 5 s for another connection's write.
    pub(crate) async fn open(path: &Path) -> Result<Self, StoreError> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(BUSY_TIMEOUT)
            .log_statements(log::LevelFilter::Debug)
            .log_slow_statements(log::LevelFilter::Warn, SLOW_STATEMENT);
        let pool = SqlitePoolOptions::new()
            .max_connections(MAX_CONNECTIONS)
            .connect_with(options)
            .await
            .map_err(StoreError::Open)?;
        let store = Self {
            conn: SqlxSqliteConnector::from_sqlx_sqlite_pool(pool),
        };
        let txn = store.begin_write().await.map_err(StoreError::Migrate)?;
        migration::Migrator::up(&txn, None)
            .await
            .map_err(StoreError::Migrate)?;
        txn.commit().await.map_err(StoreError::Migrate)?;
        Ok(store)
    }

    /// Starts a transaction that writes, taking SQLite's write lock at `BEGIN`, so a second
    /// writer waits for the first rather than failing when its read turns into a write.
    async fn begin_write(&self) -> Result<DatabaseTransaction, DbErr> {
        self.conn
            .begin_with_options(TransactionOptions {
                sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
                ..TransactionOptions::default()
            })
            .await
    }

    /// The account's machines, oldest link first.
    pub(crate) async fn machines(
        &self,
        account: &AccountId,
    ) -> Result<Vec<StoredMachine>, StoreError> {
        machine::Entity::find()
            .filter(machine::Column::Account.eq(account.as_str()))
            .order_by_asc(machine::Column::LinkedAt)
            .order_by_asc(machine::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|row| from_row(&row))
            .collect()
    }

    /// Links a machine to `link.account`, at most `cap` machines to an account, at `now`. A
    /// machine already in that account only takes the new name.
    pub(crate) async fn link(
        &self,
        link: NewLink,
        cap: u32,
        now: UnixMillis,
    ) -> Result<StoredMachine, WriteError> {
        let txn = self.begin_write().await?;
        prune(&txn, now).await?;
        check_statement(&txn, &link.statement).await?;
        let id = link.statement.machine.to_string();
        match machine::Entity::find_by_id(id.clone()).one(&txn).await? {
            Some(row) if row.account != link.account.as_str() => {
                return Err(WriteError::LinkedElsewhere);
            }
            Some(_) => {
                machine::Entity::update_many()
                    .col_expr(machine::Column::Name, Expr::value(link.name.as_str()))
                    .filter(machine::Column::Id.eq(id.as_str()))
                    .exec(&txn)
                    .await?;
            }
            None => {
                let held = machine::Entity::find()
                    .filter(machine::Column::Account.eq(link.account.as_str()))
                    .count(&txn)
                    .await?;
                if held >= u64::from(cap) {
                    return Err(WriteError::AccountFull);
                }
                account::Entity::insert(account::ActiveModel {
                    user_id: Set(link.account.as_str().to_owned()),
                })
                .on_conflict(
                    OnConflict::column(account::Column::UserId)
                        .do_nothing()
                        .to_owned(),
                )
                .exec_without_returning(&txn)
                .await?;
                machine::Entity::insert(machine::ActiveModel {
                    id: Set(id.clone()),
                    public_key: Set(link.public_key.to_bytes().to_vec()),
                    name: Set(link.name.as_str().to_owned()),
                    account: Set(link.account.as_str().to_owned()),
                    linked_at: Set(millis(now)),
                    last_seen: Set(None),
                    held_by: Set(None),
                })
                .exec_without_returning(&txn)
                .await?;
            }
        }
        record_statement(&txn, &link.statement).await?;
        let row = machine::Entity::find_by_id(id)
            .one(&txn)
            .await?
            .ok_or(StoreError::Corrupt("machine"))?;
        txn.commit().await?;
        Ok(from_row(&row)?)
    }

    /// Unlinks machine `id` from `account`. Whether it was there.
    pub(crate) async fn unlink_from(
        &self,
        account: &AccountId,
        id: &MachineId,
    ) -> Result<bool, StoreError> {
        let txn = self.begin_write().await?;
        let removed = machine::Entity::delete_many()
            .filter(machine::Column::Id.eq(id.to_string()))
            .filter(machine::Column::Account.eq(account.as_str()))
            .exec(&txn)
            .await?
            .rows_affected;
        if removed > 0 {
            forget_if_empty(&txn, account.as_str()).await?;
        }
        txn.commit().await?;
        Ok(removed > 0)
    }

    /// Unlinks the machine that signed `statement`, at `now`.
    pub(crate) async fn unlink_itself(
        &self,
        statement: &Statement,
        now: UnixMillis,
    ) -> Result<(), WriteError> {
        // Checked with a read first: anyone can send a validly signed unlink for a machine that
        // isn't linked (keys are free to make), and those must not queue for the write lock
        // that real links wait on.
        check_statement(&self.conn, statement).await?;
        let id = statement.machine.to_string();
        if machine::Entity::find_by_id(id.clone())
            .one(&self.conn)
            .await?
            .is_none()
        {
            return Err(WriteError::NotLinked);
        }
        // And again in the write, which another request may have overtaken.
        let txn = self.begin_write().await?;
        prune(&txn, now).await?;
        check_statement(&txn, statement).await?;
        let row = machine::Entity::find_by_id(id)
            .one(&txn)
            .await?
            .ok_or(WriteError::NotLinked)?;
        machine::Entity::delete_by_id(row.id.clone())
            .exec(&txn)
            .await?;
        forget_if_empty(&txn, &row.account).await?;
        record_statement(&txn, statement).await?;
        txn.commit().await?;
        Ok(())
    }

    /// The account each of `ids` is linked to, if any, in the same order.
    pub(crate) async fn accounts_of(
        &self,
        ids: &[MachineId],
    ) -> Result<Vec<Option<AccountId>>, StoreError> {
        let rows = machine::Entity::find()
            .filter(machine::Column::Id.is_in(ids.iter().map(ToString::to_string)))
            .all(&self.conn)
            .await?;
        ids.iter()
            .map(|id| {
                let id = id.to_string();
                rows.iter()
                    .find(|row| row.id == id)
                    .map(|row| AccountId::new(row.account.clone()))
                    .transpose()
                    .map_err(|_| StoreError::Corrupt("account"))
            })
            .collect()
    }

    /// Records that machine `id`'s uplink is held by `held_by` from `now` on, or (with `None`)
    /// that it went down at `now`. Whether the machine is linked.
    pub(crate) async fn presence(
        &self,
        id: &MachineId,
        held_by: Option<&str>,
        now: UnixMillis,
    ) -> Result<bool, StoreError> {
        Ok(machine::Entity::update_many()
            .col_expr(machine::Column::HeldBy, Expr::value(held_by))
            .col_expr(machine::Column::LastSeen, Expr::value(millis(now)))
            .filter(machine::Column::Id.eq(id.to_string()))
            .exec(&self.conn)
            .await?
            .rows_affected
            > 0)
    }

    /// Records that every uplink box `held_by` held went down at `now`. How many there were.
    pub(crate) async fn release(&self, held_by: &str, now: UnixMillis) -> Result<u64, StoreError> {
        Ok(machine::Entity::update_many()
            .col_expr(machine::Column::HeldBy, Expr::value(Option::<String>::None))
            .col_expr(machine::Column::LastSeen, Expr::value(millis(now)))
            .filter(machine::Column::HeldBy.eq(held_by))
            .exec(&self.conn)
            .await?
            .rows_affected)
    }
}

/// How long a statement stays fresh, in milliseconds: 5 minutes.
const FRESH_MILLIS: i64 = 5 * 60 * 1_000;

/// Forgets signatures whose statements can no longer be fresh, and latest times older than
/// any fresh statement.
async fn prune(txn: &DatabaseTransaction, now: UnixMillis) -> Result<(), DbErr> {
    accepted_signature::Entity::delete_many()
        .filter(accepted_signature::Column::ExpiresAt.lt(millis(now)))
        .exec(txn)
        .await?;
    latest_statement::Entity::delete_many()
        .filter(latest_statement::Column::Time.lt(millis(now) - FRESH_MILLIS))
        .exec(txn)
        .await?;
    Ok(())
}

/// Refuses a statement already accepted, or older than the machine's latest.
async fn check_statement(
    txn: &impl ConnectionTrait,
    statement: &Statement,
) -> Result<(), WriteError> {
    let seen = accepted_signature::Entity::find_by_id(statement.signature.to_bytes().to_vec())
        .one(txn)
        .await?;
    if seen.is_some() {
        return Err(WriteError::Replayed);
    }
    let latest = latest_statement::Entity::find_by_id(statement.machine.to_string())
        .one(txn)
        .await?;
    if latest.is_some_and(|latest| millis(statement.time) < latest.time) {
        return Err(WriteError::Stale);
    }
    Ok(())
}

/// Keeps an accepted statement's signature until it can no longer be fresh, and its time as
/// the machine's latest (no older than the one before: [`check_statement`] saw to that).
async fn record_statement(txn: &DatabaseTransaction, statement: &Statement) -> Result<(), DbErr> {
    let time = millis(statement.time);
    accepted_signature::Entity::insert(accepted_signature::ActiveModel {
        signature: Set(statement.signature.to_bytes().to_vec()),
        expires_at: Set(time + FRESH_MILLIS),
    })
    .exec_without_returning(txn)
    .await?;
    latest_statement::Entity::insert(latest_statement::ActiveModel {
        machine: Set(statement.machine.to_string()),
        time: Set(time),
    })
    .on_conflict(
        OnConflict::column(latest_statement::Column::Machine)
            .update_column(latest_statement::Column::Time)
            .to_owned(),
    )
    .exec_without_returning(txn)
    .await?;
    Ok(())
}

/// Forgets an account that has no machine left: the directory keeps nothing else about it.
async fn forget_if_empty(txn: &DatabaseTransaction, account: &str) -> Result<(), DbErr> {
    let left = machine::Entity::find()
        .filter(machine::Column::Account.eq(account))
        .count(txn)
        .await?;
    if left == 0 {
        account::Entity::delete_by_id(account.to_owned())
            .exec(txn)
            .await?;
    }
    Ok(())
}

/// An instant as stored.
fn millis(time: UnixMillis) -> i64 {
    // At most 2^53 - 1, so it always fits.
    time.get().cast_signed()
}

/// A stored instant.
fn instant(millis: i64) -> Result<UnixMillis, StoreError> {
    u64::try_from(millis)
        .ok()
        .and_then(|millis| UnixMillis::new(millis).ok())
        .ok_or(StoreError::Corrupt("time"))
}

fn from_row(row: &machine::Model) -> Result<StoredMachine, StoreError> {
    let key: [u8; PUBLIC_KEY_LEN] = row
        .public_key
        .as_slice()
        .try_into()
        .map_err(|_| StoreError::Corrupt("public key"))?;
    let public_key = PublicKey::from_bytes(&key).map_err(|_| StoreError::Corrupt("public key"))?;
    let id: MachineId = row
        .id
        .parse()
        .map_err(|_| StoreError::Corrupt("machine id"))?;
    id.check(&public_key)
        .map_err(|_| StoreError::Corrupt("machine id"))?;
    Ok(StoredMachine {
        id,
        public_key,
        name: row
            .name
            .parse()
            .map_err(|_| StoreError::Corrupt("machine name"))?,
        linked_at: instant(row.linked_at)?,
        last_seen: row.last_seen.map(instant).transpose()?,
        connected: row.held_by.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use emberhut_identity::SecretKey;
    use sea_orm_migration::SchemaManager;

    use super::*;

    const NOW: u64 = 1_795_561_073_000;

    fn at(millis: u64) -> UnixMillis {
        UnixMillis::new(millis).unwrap()
    }

    fn statement(seed: u8, signature: u8, time: u64) -> Statement {
        Statement {
            machine: SecretKey::from_bytes(&[seed; 32]).machine_id(),
            signature: Signature::from_bytes(&[signature; 64]),
            time: at(time),
        }
    }

    fn new_link(seed: u8, signature: u8, account: &str, time: u64) -> NewLink {
        let key = SecretKey::from_bytes(&[seed; 32]);
        NewLink {
            public_key: key.public_key(),
            name: "Main computer".parse().unwrap(),
            account: AccountId::new(account).unwrap(),
            statement: statement(seed, signature, time),
        }
    }

    async fn store() -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("directory.db")).await.unwrap();
        (store, dir)
    }

    #[tokio::test]
    async fn the_migration_goes_down_and_up_again() {
        let (store, _dir) = store().await;
        let manager = SchemaManager::new(&store.conn);
        migration::Migrator::down(&store.conn, None).await.unwrap();
        assert!(!manager.has_table("machines").await.unwrap());
        migration::Migrator::up(&store.conn, None).await.unwrap();
        assert!(manager.has_table("machines").await.unwrap());
    }

    #[tokio::test]
    async fn signatures_and_latest_times_are_pruned_once_they_cant_matter() {
        let (store, _dir) = store().await;
        store
            .link(new_link(1, 1, "user_a", NOW), 5, at(NOW))
            .await
            .unwrap();
        // The same signature, five minutes later still: refused.
        assert!(matches!(
            store
                .unlink_itself(&statement(1, 1, NOW), at(NOW + 300_000))
                .await,
            Err(WriteError::Replayed)
        ));
        // A moment later it is forgotten, and so is the latest time.
        store
            .unlink_itself(&statement(1, 2, NOW + 300_001), at(NOW + 600_002))
            .await
            .unwrap();
        let rows = accepted_signature::Entity::find()
            .all(&store.conn)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "only the unlink's signature is kept");
        assert_eq!(rows[0].signature, vec![2; 64]);
        assert!(
            account::Entity::find()
                .all(&store.conn)
                .await
                .unwrap()
                .is_empty(),
            "an account without machines is forgotten"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unlinked_machines_own_unlink_takes_no_write_lock() {
        let (store, _dir) = store().await;
        store
            .link(new_link(1, 1, "user_a", NOW), 5, at(NOW))
            .await
            .unwrap();
        // Another writer holds the lock, as a link in progress does.
        let held = store.begin_write().await.unwrap();
        for seed in 2..50 {
            let unlink = tokio::time::timeout(
                Duration::from_secs(1),
                store.unlink_itself(&statement(seed, seed, NOW), at(NOW)),
            )
            .await
            .expect("answered without waiting for the write lock");
            assert!(matches!(unlink, Err(WriteError::NotLinked)));
        }
        // A replay is told apart without the lock too.
        held.rollback().await.unwrap();
        store
            .unlink_itself(&statement(1, 9, NOW), at(NOW))
            .await
            .unwrap();
        let held = store.begin_write().await.unwrap();
        let replay = tokio::time::timeout(
            Duration::from_secs(1),
            store.unlink_itself(&statement(1, 9, NOW), at(NOW)),
        )
        .await
        .expect("answered without waiting for the write lock");
        assert!(matches!(replay, Err(WriteError::Replayed)));
        held.rollback().await.unwrap();
    }

    #[tokio::test]
    async fn statuses_and_presence_follow_the_links() {
        let (store, _dir) = store().await;
        store
            .link(new_link(1, 1, "user_a", NOW), 5, at(NOW))
            .await
            .unwrap();
        let linked = SecretKey::from_bytes(&[1; 32]).machine_id();
        let other = SecretKey::from_bytes(&[2; 32]).machine_id();
        assert_eq!(
            store.accounts_of(&[other, linked]).await.unwrap(),
            [None, Some(AccountId::new("user_a").unwrap())]
        );
        assert!(
            store
                .presence(&linked, Some("box-1"), at(NOW + 1))
                .await
                .unwrap()
        );
        assert!(
            !store
                .presence(&other, Some("box-1"), at(NOW + 1))
                .await
                .unwrap()
        );
        let machines = store
            .machines(&AccountId::new("user_a").unwrap())
            .await
            .unwrap();
        assert!(machines[0].connected);
        assert!(store.presence(&linked, None, at(NOW + 2)).await.unwrap());
        let machines = store
            .machines(&AccountId::new("user_a").unwrap())
            .await
            .unwrap();
        assert!(!machines[0].connected);
        assert_eq!(machines[0].last_seen, Some(at(NOW + 2)));
    }
}
