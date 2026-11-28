//! The off-the-record room's own database: `dark.db` beside the server's, with one table.
//!
//! It is a file of its own so that nothing else reaches it: the backup copies the server's
//! database and its data folders, not this file, and no other store opens it. It keeps no
//! write-ahead log and deletes securely (SQLite's `secure_delete`), and clearing it ends with a
//! `VACUUM`, so a cleared chat leaves nothing behind in a side file or in freed pages. That
//! `VACUUM` is the one statement here outside `SeaORM`'s builders, as ADR 0022 allows.

use std::path::{Path, PathBuf};
use std::time::Duration;

use emberhut_core::dark::DarkRole;
use jiff::Timestamp;
use sea_orm::sqlx::sqlite::SqliteJournalMode;
use sea_orm::{
    ActiveValue::Set, ConnectOptions, ConnectionTrait, DatabaseConnection, EntityTrait, QueryOrder,
};
use sea_orm_migration::MigratorTrait;

use crate::entity::dark_message;
use crate::migration::DarkMigrator;
use crate::time::{instant, millis};
use crate::{DatabaseUrl, DbError};

/// The room's file name, in the server's data folder.
pub const DARK_FILE: &str = "dark.db";

/// One line of the room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DarkLine {
    /// Its id.
    pub id: i64,
    /// Who said it.
    pub role: DarkRole,
    /// What was said.
    pub text: String,
    /// When.
    pub at: Timestamp,
}

/// Reads and writes the room's chat.
#[derive(Clone, Debug)]
pub struct DarkStore {
    conn: DatabaseConnection,
    path: PathBuf,
}

impl DarkStore {
    /// Opens (making it if need be) the room's database at `path`, with its table.
    ///
    /// # Errors
    ///
    /// [`DbError::Connect`] or [`DbError::Migrate`].
    pub async fn open(path: &Path) -> Result<Self, DbError> {
        let url: DatabaseUrl =
            DatabaseUrl::from_path(path).map_err(|_| DbError::Missing(path.to_path_buf()))?;
        let mut options = ConnectOptions::new(url.as_str());
        options
            .max_connections(1)
            .sqlx_logging(false)
            .map_sqlx_sqlite_opts(|sqlite| {
                sqlite
                    .create_if_missing(true)
                    .journal_mode(SqliteJournalMode::Delete)
                    .pragma("secure_delete", "on")
                    .busy_timeout(Duration::from_secs(5))
            });
        let conn = sea_orm::Database::connect(options)
            .await
            .map_err(DbError::Connect)?;
        DarkMigrator::up(&conn, None)
            .await
            .map_err(DbError::Migrate)?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
        })
    }

    /// The file the room is kept in.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The whole chat, oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn lines(&self) -> Result<Vec<DarkLine>, DbError> {
        dark_message::Entity::find()
            .order_by_asc(dark_message::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|row| {
                Ok(DarkLine {
                    id: row.id,
                    role: DarkRole::parse(&row.role),
                    text: row.text,
                    at: instant(row.at)?,
                })
            })
            .collect()
    }

    /// Adds a line said by `role` at `now`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn add(
        &self,
        role: DarkRole,
        text: &str,
        now: Timestamp,
    ) -> Result<DarkLine, DbError> {
        let row = dark_message::ActiveModel {
            role: Set(role.as_str().to_owned()),
            text: Set(text.to_owned()),
            at: Set(millis(now)),
            ..Default::default()
        };
        let id = dark_message::Entity::insert(row)
            .exec(&self.conn)
            .await?
            .last_insert_id;
        Ok(DarkLine {
            id,
            role,
            text: text.to_owned(),
            at: now,
        })
    }

    /// Deletes the whole chat, and vacuums the file so nothing of it is left in freed pages.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn clear(&self) -> Result<(), DbError> {
        dark_message::Entity::delete_many().exec(&self.conn).await?;
        self.conn.execute_unprepared("VACUUM").await?;
        Ok(())
    }
}
