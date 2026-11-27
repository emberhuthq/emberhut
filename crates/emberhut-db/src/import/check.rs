//! The import's checks, inside its transaction, before it commits:
//!
//! - the archive against the source: a table's archived rows, decoded, go into an untyped
//!   `TEMP` table, and `EXCEPT` both ways against the attached private copy, comparing each
//!   value and its storage class, must find nothing;
//! - the database: `foreign_key_check` and `integrity_check`.
//!
//! The `TEMP` table's columns have no type, collation or constraint, so nothing converts a
//! value on its way in; the comparisons are made `COLLATE BINARY`, so a column's own collation
//! on the source's side cannot hide a difference.

use sea_orm::{ConnectionTrait, Statement, Value};

use super::{ImportStore, SqlValue, ident};
use crate::DbError;

/// The `TEMP` table a check fills.
const CHECK_TABLE: &str = "temp.import_check";

/// Values per insert into the `TEMP` table, under SQLite's limit on a statement's values.
const VALUES_PER_INSERT: usize = 30_000;

/// Archived rows read at a time.
const PAGE: u64 = 2_000;

/// A column the archive holds as NULL where the source has a value: in every row, or only in
/// the rows whose `when` column holds one of the given values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scrub {
    /// The column.
    pub column: String,
    /// Only in the rows whose column (the first) holds one of these texts (the second).
    pub when: Option<(String, Vec<String>)>,
}

impl Scrub {
    /// What the archive holds for the column, as an expression over the source's row.
    fn expression(&self) -> String {
        match &self.when {
            None => "NULL".to_owned(),
            Some((column, values)) => {
                let listed: Vec<String> = values
                    .iter()
                    .map(|value| format!("'{}'", value.replace('\'', "''")))
                    .collect();
                format!(
                    "CASE WHEN {} IN ({}) THEN NULL ELSE {} END",
                    ident(column),
                    listed.join(", "),
                    ident(&self.column)
                )
            }
        }
    }
}

/// What the check of one table's archive found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveCheck {
    /// Rows in the source table.
    pub source_rows: u64,
    /// Rows in the archive.
    pub archived_rows: u64,
    /// The rowid of a source row the archive does not hold as it is, if any.
    pub missing: Option<i64>,
    /// The rowid of an archived row the source does not hold as it is, if any.
    pub extra: Option<i64>,
}

impl ArchiveCheck {
    /// Whether the archive holds exactly the source's rows.
    #[must_use]
    pub const fn is_exact(&self) -> bool {
        self.source_rows == self.archived_rows && self.missing.is_none() && self.extra.is_none()
    }
}

/// A row whose reference names no row, as `foreign_key_check` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForeignKeyProblem {
    /// The table of the row.
    pub table: String,
    /// The row.
    pub rowid: Option<i64>,
    /// The table it refers to.
    pub parent: String,
}

impl ImportStore {
    /// Checks the archived rows of `table` of the database `source` against the table in the
    /// copy attached as `attached`, comparing the columns `columns` (in the archive's rows by
    /// name), each value and its storage class; a column of `scrubs` is compared with what the
    /// scrub leaves of the source's value.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::RowFormat`] for an archived row not in the format or
    /// without one of `columns`.
    pub async fn check_archive(
        &self,
        attached: &str,
        source: &str,
        table: &str,
        columns: &[String],
        scrubs: &[Scrub],
    ) -> Result<ArchiveCheck, DbError> {
        let width = columns.len() + 1;
        let names: Vec<String> = (0..columns.len()).map(|i| format!("c{i}")).collect();
        self.exec(&format!(
            "CREATE TEMP TABLE import_check (r, {})",
            names.join(", ")
        ))
        .await?;
        let filled = self.fill_check_table(source, table, columns, width).await;
        let result = match filled {
            Ok(archived_rows) => {
                self.compare(attached, table, columns, scrubs, &names, archived_rows)
                    .await
            }
            Err(error) => Err(error),
        };
        self.exec(&format!("DROP TABLE {CHECK_TABLE}")).await?;
        result
    }

    async fn fill_check_table(
        &self,
        source: &str,
        table: &str,
        columns: &[String],
        width: usize,
    ) -> Result<u64, DbError> {
        let rows_per_insert = (VALUES_PER_INSERT / width).max(1);
        let mut after = None;
        let mut archived = 0_u64;
        loop {
            let page = self.archived(source, table, after, PAGE).await?;
            let Some(last) = page.last() else {
                break;
            };
            after = Some(last.ordinal);
            for chunk in page.chunks(rows_per_insert) {
                let mut sql = format!("INSERT INTO {CHECK_TABLE} VALUES ");
                let mut values = Vec::with_capacity(chunk.len() * width);
                for (i, archived_row) in chunk.iter().enumerate() {
                    sql.push_str(if i == 0 { "(?" } else { ", (?" });
                    values.push(Value::BigInt(Some(archived_row.rowid)));
                    for column in columns {
                        let value = archived_row.row().get(column).ok_or_else(|| {
                            DbError::RowFormat(super::RowFormatError::new(format!(
                                "row {} of {table} has no column {column:?}",
                                archived_row.rowid
                            )))
                        })?;
                        // Text that is not UTF-8 is bound as a blob and cast back, which
                        // keeps its bytes.
                        sql.push_str(if matches!(value, SqlValue::TextBytes(_)) {
                            ", CAST(? AS TEXT)"
                        } else {
                            ", ?"
                        });
                        values.push(bound(value));
                    }
                    sql.push(')');
                }
                self.txn
                    .execute_raw(Statement::from_sql_and_values(
                        self.txn.get_database_backend(),
                        sql,
                        values,
                    ))
                    .await?;
                archived += chunk.len() as u64;
            }
        }
        Ok(archived)
    }

    async fn compare(
        &self,
        attached: &str,
        table: &str,
        columns: &[String],
        scrubs: &[Scrub],
        names: &[String],
        archived_rows: u64,
    ) -> Result<ArchiveCheck, DbError> {
        let source_table = format!("{}.{}", ident(attached), ident(table));
        let mut source_values = vec!["rowid".to_owned()];
        let mut check_values = vec!["r".to_owned()];
        for (column, name) in columns.iter().zip(names) {
            let quoted = scrubs
                .iter()
                .find(|scrub| &scrub.column == column)
                .map_or_else(
                    || ident(column),
                    |scrub| format!("({})", scrub.expression()),
                );
            source_values.push(format!("{quoted} COLLATE BINARY, typeof({quoted})"));
            check_values.push(format!("{name} COLLATE BINARY, typeof({name})"));
        }
        let source_select = format!("SELECT {} FROM {source_table}", source_values.join(", "));
        let check_select = format!("SELECT {} FROM {CHECK_TABLE}", check_values.join(", "));
        let source_rows = self
            .one_integer(&format!("SELECT count(*) FROM {source_table}"))
            .await?
            .unwrap_or(0);
        let missing = self
            .one_integer(&format!(
                "SELECT * FROM ({source_select} EXCEPT {check_select}) LIMIT 1"
            ))
            .await?;
        let extra = self
            .one_integer(&format!(
                "SELECT * FROM ({check_select} EXCEPT {source_select}) LIMIT 1"
            ))
            .await?;
        Ok(ArchiveCheck {
            source_rows: u64::try_from(source_rows).unwrap_or(0),
            archived_rows,
            missing,
            extra,
        })
    }

    /// The rows whose references name no row (empty when every reference holds).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn foreign_key_problems(&self) -> Result<Vec<ForeignKeyProblem>, DbError> {
        let rows = self
            .txn
            .query_all_raw(Statement::from_string(
                self.txn.get_database_backend(),
                "PRAGMA main.foreign_key_check",
            ))
            .await?;
        rows.iter()
            .map(|row| {
                Ok(ForeignKeyProblem {
                    table: row.try_get_by_index(0)?,
                    rowid: row.try_get_by_index(1)?,
                    parent: row.try_get_by_index(2)?,
                })
            })
            .collect()
    }

    /// What `integrity_check` says of the file: `["ok"]` when nothing is wrong.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn integrity(&self) -> Result<Vec<String>, DbError> {
        let rows = self
            .txn
            .query_all_raw(Statement::from_string(
                self.txn.get_database_backend(),
                "PRAGMA main.integrity_check",
            ))
            .await?;
        rows.iter()
            .map(|row| Ok(row.try_get_by_index::<String>(0)?))
            .collect()
    }

    async fn exec(&self, sql: &str) -> Result<(), DbError> {
        self.txn.execute_unprepared(sql).await?;
        Ok(())
    }

    /// The first column of the first row `sql` returns, as an integer.
    async fn one_integer(&self, sql: &str) -> Result<Option<i64>, DbError> {
        let row = self
            .txn
            .query_one_raw(Statement::from_string(self.txn.get_database_backend(), sql))
            .await?;
        Ok(match row {
            Some(row) => row.try_get_by_index::<Option<i64>>(0)?,
            None => None,
        })
    }
}

/// `value` as a statement binds it, in its own storage class.
fn bound(value: &SqlValue) -> Value {
    match value {
        SqlValue::Null => Value::BigInt(None),
        SqlValue::Integer(n) => Value::BigInt(Some(*n)),
        SqlValue::Real(x) => Value::Double(Some(*x)),
        SqlValue::Text(text) => Value::String(Some(text.clone())),
        SqlValue::TextBytes(bytes) | SqlValue::Blob(bytes) => Value::Bytes(Some(bytes.clone())),
    }
}
