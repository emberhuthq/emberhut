//! `import_rows`: writing the archive, and reading it back for the mappings and the check.

use sea_orm::{ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use super::{ImportStore, Row};
use crate::DbError;
use crate::entity::import_row;

/// Rows per insert: six values each, well under SQLite's limit on a statement's values.
const BATCH: usize = 1_000;

/// One archived row of a source table.
#[derive(Clone, Debug, PartialEq)]
pub struct ArchivedRow {
    /// Its place in its table's rowid order, from 0.
    pub ordinal: i64,
    /// The rowid it was read with.
    pub rowid: i64,
    /// The declared primary key as text (a JSON array for a composite key); none for a table
    /// without one, or a NULL key.
    pub key: Option<String>,
    row: Row,
    json: String,
}

impl ArchivedRow {
    /// A row to archive.
    #[must_use]
    pub fn new(ordinal: i64, rowid: i64, key: Option<String>, row: Row) -> Self {
        let json = row.to_json();
        Self {
            ordinal,
            rowid,
            key,
            row,
            json,
        }
    }

    /// The row.
    #[must_use]
    pub fn row(&self) -> &Row {
        &self.row
    }

    /// The row as the archive stores it.
    #[must_use]
    pub fn json(&self) -> &str {
        &self.json
    }
}

impl ImportStore {
    /// Archives `rows` of the source table `table` of the database `source`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], for one when a row's ordinal or rowid is archived already.
    pub async fn archive(
        &self,
        source: &str,
        table: &str,
        rows: &[ArchivedRow],
    ) -> Result<(), DbError> {
        for chunk in rows.chunks(BATCH) {
            let models = chunk.iter().map(|row| import_row::ActiveModel {
                source: Set(source.to_owned()),
                tbl: Set(table.to_owned()),
                ordinal: Set(row.ordinal),
                rowid: Set(row.rowid),
                key: Set(row.key.clone()),
                row: Set(row.json.clone()),
            });
            import_row::Entity::insert_many(models)
                .exec(&self.txn)
                .await?;
        }
        Ok(())
    }

    /// At most `limit` archived rows of `table`, in order, after the ordinal `after` (from
    /// the first when none).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::RowFormat`] for a row not in the archive's format.
    pub async fn archived(
        &self,
        source: &str,
        table: &str,
        after: Option<i64>,
        limit: u64,
    ) -> Result<Vec<ArchivedRow>, DbError> {
        let mut query = import_row::Entity::find()
            .filter(import_row::Column::Source.eq(source))
            .filter(import_row::Column::Tbl.eq(table));
        if let Some(after) = after {
            query = query.filter(import_row::Column::Ordinal.gt(after));
        }
        query
            .order_by_asc(import_row::Column::Ordinal)
            .limit(limit)
            .all(&self.txn)
            .await?
            .into_iter()
            .map(|model| {
                Ok(ArchivedRow {
                    ordinal: model.ordinal,
                    rowid: model.rowid,
                    key: model.key,
                    row: Row::from_json(&model.row)?,
                    json: model.row,
                })
            })
            .collect()
    }

    /// The keys of every archived row of `table` that has one, in order.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn archived_keys(&self, source: &str, table: &str) -> Result<Vec<String>, DbError> {
        Ok(import_row::Entity::find()
            .select_only()
            .column(import_row::Column::Key)
            .filter(import_row::Column::Source.eq(source))
            .filter(import_row::Column::Tbl.eq(table))
            .filter(import_row::Column::Key.is_not_null())
            .order_by_asc(import_row::Column::Ordinal)
            .into_tuple::<String>()
            .all(&self.txn)
            .await?)
    }
}
