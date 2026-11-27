//! The manifest: what an import read, kept, copied and mapped.

use jiff::Timestamp;
use sea_orm::{ActiveValue::Set, EntityTrait};

use super::ImportStore;
use crate::DbError;
use crate::entity::{import_file, import_mapping, import_run, import_schema, import_table};
use crate::time::millis;

/// Rows per insert.
const BATCH: usize = 1_000;

/// One table, index, trigger or view of a source database's schema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaEntry {
    /// `table`, `index`, `trigger` or `view`.
    pub kind: String,
    /// Its name.
    pub name: String,
    /// Its `CREATE` text (none for an index SQLite made itself).
    pub sql: Option<String>,
}

/// What happened to one source table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TableRecord {
    /// The source database.
    pub source: String,
    /// The table.
    pub table: String,
    /// Rows read.
    pub read: u64,
    /// Rows archived.
    pub archived: u64,
    /// Rows mapped into emberhut's tables.
    pub mapped: u64,
    /// Rows dropped.
    pub dropped: u64,
    /// The archived rows' checksum, in lower-case hex.
    pub checksum: Option<String>,
    /// The scrubbed columns.
    pub scrubbed: Vec<String>,
    /// Why it was dropped, or what else the report says of it.
    pub note: Option<String>,
}

/// One copied file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRecord {
    /// Its tree (`uploads`, `context`, ...).
    pub tree: String,
    /// Its path in the tree, as the source wrote it.
    pub path: String,
    /// Its size.
    pub bytes: u64,
    /// Its SHA-256, in lower-case hex.
    pub sha256: String,
}

/// One mapping that ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MappingRecord {
    /// Its name.
    pub name: String,
    /// Its version.
    pub version: i32,
    /// The source table it maps from.
    pub source_table: String,
    /// The rows it wrote.
    pub rows: u64,
}

/// The import itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunRecord {
    /// When it ran.
    pub ran_at: Timestamp,
    /// The importing program's version.
    pub version: String,
    /// The kind of input.
    pub input: String,
    /// Per source database: its bytes, SHA-256 and `user_version`, as JSON.
    pub sources: String,
    /// The report, as printed.
    pub report: String,
}

fn count(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

impl ImportStore {
    /// Records the schema of the source database `source`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn record_schema(
        &self,
        source: &str,
        entries: &[SchemaEntry],
    ) -> Result<(), DbError> {
        for chunk in entries.chunks(BATCH) {
            let models = chunk.iter().map(|entry| import_schema::ActiveModel {
                source: Set(source.to_owned()),
                kind: Set(entry.kind.clone()),
                name: Set(entry.name.clone()),
                sql: Set(entry.sql.clone()),
            });
            import_schema::Entity::insert_many(models)
                .exec(&self.txn)
                .await?;
        }
        Ok(())
    }

    /// Records what happened to each source table.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn record_tables(&self, tables: &[TableRecord]) -> Result<(), DbError> {
        for chunk in tables.chunks(BATCH) {
            let models = chunk.iter().map(|table| import_table::ActiveModel {
                source: Set(table.source.clone()),
                tbl: Set(table.table.clone()),
                rows_read: Set(count(table.read)),
                rows_archived: Set(count(table.archived)),
                rows_mapped: Set(count(table.mapped)),
                rows_dropped: Set(count(table.dropped)),
                checksum: Set(table.checksum.clone()),
                scrubbed: Set(serde_json::Value::from(table.scrubbed.clone()).to_string()),
                note: Set(table.note.clone()),
            });
            import_table::Entity::insert_many(models)
                .exec(&self.txn)
                .await?;
        }
        Ok(())
    }

    /// Records the files copied.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn record_files(&self, files: &[FileRecord]) -> Result<(), DbError> {
        for chunk in files.chunks(BATCH) {
            let models = chunk.iter().map(|file| import_file::ActiveModel {
                tree: Set(file.tree.clone()),
                path: Set(file.path.clone()),
                bytes: Set(count(file.bytes)),
                sha256: Set(file.sha256.clone()),
            });
            import_file::Entity::insert_many(models)
                .exec(&self.txn)
                .await?;
        }
        Ok(())
    }

    /// Records the mappings that ran, at `now`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn record_mappings(
        &self,
        mappings: &[MappingRecord],
        now: Timestamp,
    ) -> Result<(), DbError> {
        for chunk in mappings.chunks(BATCH) {
            let models = chunk.iter().map(|mapping| import_mapping::ActiveModel {
                name: Set(mapping.name.clone()),
                version: Set(mapping.version),
                source_tbl: Set(mapping.source_table.clone()),
                rows: Set(count(mapping.rows)),
                ran_at: Set(millis(now)),
            });
            import_mapping::Entity::insert_many(models)
                .exec(&self.txn)
                .await?;
        }
        Ok(())
    }

    /// Records the import itself.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn record_run(&self, run: &RunRecord) -> Result<(), DbError> {
        import_run::Entity::insert(import_run::ActiveModel {
            ran_at: Set(millis(run.ran_at)),
            version: Set(run.version.clone()),
            input: Set(run.input.clone()),
            sources: Set(run.sources.clone()),
            report: Set(run.report.clone()),
            ..Default::default()
        })
        .exec(&self.txn)
        .await?;
        Ok(())
    }
}
