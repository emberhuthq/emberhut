//! Adding a later import to a database in use (a catch-up): numbering the rows whose own id is
//! taken, finding the rows that hold a name already (a file's path, a note's `uid`), replacing a
//! task and an archived row that changed in the source since, and reading what earlier imports
//! recorded (the files they copied and their runs).

use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityName, EntityTrait, QueryFilter, QueryOrder, Statement,
};

use super::{
    ArchivedRow, FileRecord, ImportStore, ImportedFile, ImportedNote, ImportedTask, RunRecord,
    ident,
};
use crate::DbError;
use crate::entity::{
    call_hint, call_line, file, import_file, import_row, import_run, issue, issue_event, message,
    note,
};
use crate::time::instant;

/// Names per read.
const KEYS: usize = 500;

/// A table whose rows are numbered (`AUTOINCREMENT`): an imported row whose id is taken in it
/// is given the next free one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Numbered {
    /// `messages`.
    Messages,
    /// `files`.
    Files,
    /// `issues`, the tasks.
    Tasks,
    /// `issue_events`, the tasks' history.
    TaskEvents,
    /// `notes`.
    Notes,
    /// `call_lines`.
    CallLines,
    /// `call_hints`.
    CallHints,
}

impl Numbered {
    fn table(self) -> &'static str {
        match self {
            Self::Messages => message::Entity.table_name(),
            Self::Files => file::Entity.table_name(),
            Self::Tasks => issue::Entity.table_name(),
            Self::TaskEvents => issue_event::Entity.table_name(),
            Self::Notes => note::Entity.table_name(),
            Self::CallLines => call_line::Entity.table_name(),
            Self::CallHints => call_hint::Entity.table_name(),
        }
    }
}

impl ImportStore {
    /// The id the table's next row would get: above every id it holds and every id it ever
    /// gave (a deleted row's id is not given again).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn next_id(&self, table: Numbered) -> Result<i64, DbError> {
        let name = table.table();
        let backend = self.txn.get_database_backend();
        let row = self
            .txn
            .query_one_raw(Statement::from_sql_and_values(
                backend,
                format!(
                    "SELECT max(coalesce((SELECT seq FROM sqlite_sequence WHERE name = ?), 0), \
                     coalesce((SELECT max(id) FROM {}), 0)) + 1",
                    ident(name)
                ),
                [name.into()],
            ))
            .await?;
        Ok(match row {
            Some(row) => row.try_get_by_index::<i64>(0)?,
            None => 1,
        })
    }

    /// Replaces tasks by their id, every column.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], for one when a task is not there.
    pub async fn update_tasks(&self, rows: &[ImportedTask]) -> Result<(), DbError> {
        for row in rows {
            issue::Entity::update(issue::ActiveModel::from(row))
                .exec(&self.txn)
                .await?;
        }
        Ok(())
    }

    /// Replaces archived rows: each row of `table` of the database `source` at the ordinal and
    /// rowid given takes the given key and values (the source's row as it is now).
    ///
    /// # Errors
    ///
    /// [`DbError::NotArchived`] when no row is archived there (nothing is then replaced), and
    /// [`DbError::Query`].
    pub async fn rearchive(
        &self,
        source: &str,
        table: &str,
        rows: &[ArchivedRow],
    ) -> Result<(), DbError> {
        for row in rows {
            let done = import_row::Entity::update_many()
                .col_expr(import_row::Column::Key, Expr::value(row.key.clone()))
                .col_expr(import_row::Column::Row, Expr::value(row.json().to_owned()))
                .filter(import_row::Column::Source.eq(source))
                .filter(import_row::Column::Tbl.eq(table))
                .filter(import_row::Column::Ordinal.eq(row.ordinal))
                .filter(import_row::Column::Rowid.eq(row.rowid))
                .exec(&self.txn)
                .await?;
            if done.rows_affected != 1 {
                return Err(DbError::NotArchived {
                    database: source.to_owned(),
                    table: table.to_owned(),
                    ordinal: row.ordinal,
                });
            }
        }
        Ok(())
    }

    /// The files of the chat whose path in the uploads folder is one of `names`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn files_by_stored_name(
        &self,
        names: &[String],
    ) -> Result<Vec<ImportedFile>, DbError> {
        let mut found = Vec::with_capacity(names.len());
        for chunk in names.chunks(KEYS) {
            let ids: Vec<i64> = file::Entity::find()
                .filter(file::Column::StoredName.is_in(chunk.iter().cloned()))
                .all(&self.txn)
                .await?
                .into_iter()
                .map(|row| row.id)
                .collect();
            found.extend(self.files_by_id(&ids).await?);
        }
        Ok(found)
    }

    /// The notes whose `uid` is one of `uids`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn notes_by_uid(&self, uids: &[String]) -> Result<Vec<ImportedNote>, DbError> {
        let mut found = Vec::with_capacity(uids.len());
        for chunk in uids.chunks(KEYS) {
            let ids: Vec<i64> = note::Entity::find()
                .filter(note::Column::Uid.is_in(chunk.iter().cloned()))
                .all(&self.txn)
                .await?
                .into_iter()
                .map(|row| row.id)
                .collect();
            found.extend(self.notes_by_id(&ids).await?);
        }
        Ok(found)
    }

    /// Every file an import recorded, by tree and path.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn recorded_files(&self) -> Result<Vec<FileRecord>, DbError> {
        Ok(import_file::Entity::find()
            .order_by_asc(import_file::Column::Tree)
            .order_by_asc(import_file::Column::Path)
            .all(&self.txn)
            .await?
            .into_iter()
            .map(|row| FileRecord {
                tree: row.tree,
                path: row.path,
                bytes: u64::try_from(row.bytes).unwrap_or(0),
                sha256: row.sha256,
            })
            .collect())
    }

    /// Every import recorded, in the order they ran.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], and [`DbError::InstantOutOfRange`] for a time that is not one.
    pub async fn runs(&self) -> Result<Vec<RunRecord>, DbError> {
        import_run::Entity::find()
            .order_by_asc(import_run::Column::Id)
            .all(&self.txn)
            .await?
            .into_iter()
            .map(|row| {
                Ok(RunRecord {
                    ran_at: instant(row.ran_at)?,
                    version: row.version,
                    input: row.input,
                    sources: row.sources,
                    report: row.report,
                })
            })
            .collect()
    }
}
