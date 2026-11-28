//! Recall's index: the pieces of text (`docs`), their full-text index (`docs_fts`), their
//! embedding vectors (`vecs`), how far each source is indexed (`marks`) and how far each agent
//! transcript is read (`transcript_offsets`).
//!
//! `docs_fts` is an SQLite FTS5 table that triggers keep in step with `docs`, so every write
//! here is an ordinary `SeaORM` one. A word search is the one query that names it
//! (`MATCH`, ranked by `bm25`), and it is the only SQL in this file, as ADR 0016 allows.

use std::collections::{HashMap, HashSet};

use emberhut_core::memory::recall::{
    Doc, NewDoc, base_reference, piece_number, piece_reference, pieces, source,
};
use sea_orm::sea_query::{OnConflict, Query};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    DbBackend, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Statement,
};

use crate::entity::{doc, doc_vector, mark, transcript_offset};
use crate::time::{instant, millis};
use crate::{DbError, begin_write};

/// The word search: the pieces matching an FTS5 query, best first, titles weighing twice the
/// text.
const WORD_SEARCH: &str = "SELECT d.id, d.source, d.reference, d.at, d.title, d.text, d.place \
     FROM docs_fts f JOIN docs d ON d.id = f.rowid WHERE docs_fts MATCH ? \
     ORDER BY bm25(docs_fts, 2.0, 1.0) LIMIT ?";

/// How many pieces match one word: the full-text index's own count.
const WORD_COUNT: &str = "SELECT count(*) FROM docs_fts WHERE docs_fts MATCH ?";

/// What one indexing pass writes, in one transaction.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexBatch {
    /// Things to index (each in pieces); one already there with the same title and text is
    /// left alone.
    pub docs: Vec<NewDoc>,
    /// Sources indexed whole each time (the notes, the facts): their pieces whose reference is
    /// not in the set are dropped.
    pub only: Vec<(String, HashSet<String>)>,
    /// How far each source is indexed now.
    pub marks: Vec<(String, String)>,
}

/// How far a transcript has been read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TranscriptOffset {
    /// The bytes read, up to the end of a line.
    pub offset: u64,
    /// The lines read.
    pub line: u64,
}

/// Reads and writes recall's index.
#[derive(Clone, Debug)]
pub struct RecallStore {
    conn: DatabaseConnection,
}

impl RecallStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// How far `key` is indexed, if it has been.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn mark(&self, key: &str) -> Result<Option<String>, DbError> {
        Ok(mark::Entity::find_by_id(key)
            .one(&self.conn)
            .await?
            .map(|row| row.value))
    }

    /// Writes `batch` in one transaction; `true` when a piece changed.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn index(&self, batch: &IndexBatch) -> Result<bool, DbError> {
        let txn = begin_write(&self.conn).await?;
        let mut changed = false;
        for new in &batch.docs {
            changed |= put(&txn, new).await?;
        }
        for (kind, keep) in &batch.only {
            let rows = doc::Entity::find()
                .filter(doc::Column::Source.eq(kind.as_str()))
                .all(&txn)
                .await?;
            let gone: Vec<i64> = rows
                .into_iter()
                .filter(|row| !keep.contains(base_reference(&row.reference)))
                .map(|row| row.id)
                .collect();
            changed |= delete(&txn, gone).await?;
        }
        for (key, value) in &batch.marks {
            set_mark(&txn, key, value).await?;
        }
        txn.commit().await?;
        Ok(changed)
    }

    /// How far every transcript read so far has been read, by path.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn transcript_offsets(&self) -> Result<HashMap<String, TranscriptOffset>, DbError> {
        Ok(transcript_offset::Entity::find()
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|row| (row.path, offset_from_row(row.offset, row.line)))
            .collect())
    }

    /// How far the transcript at `path` has been read; none when it never was.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn transcript_offset(&self, path: &str) -> Result<Option<TranscriptOffset>, DbError> {
        Ok(transcript_offset::Entity::find_by_id(path)
            .one(&self.conn)
            .await?
            .map(|row| offset_from_row(row.offset, row.line)))
    }

    /// Indexes `docs`, read from the transcript at `path`, and saves that it is read up to
    /// `read`, in one transaction; `true` when a piece changed.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn read_transcript(
        &self,
        path: &str,
        read: TranscriptOffset,
        docs: &[NewDoc],
    ) -> Result<bool, DbError> {
        let txn = begin_write(&self.conn).await?;
        let mut changed = false;
        for new in docs {
            changed |= put(&txn, new).await?;
        }
        let row = transcript_offset::ActiveModel {
            path: Set(path.to_owned()),
            offset: Set(i64::try_from(read.offset).unwrap_or(i64::MAX)),
            line: Set(i64::try_from(read.line).unwrap_or(i64::MAX)),
        };
        transcript_offset::Entity::insert(row)
            .on_conflict(
                OnConflict::column(transcript_offset::Column::Path)
                    .update_columns([
                        transcript_offset::Column::Offset,
                        transcript_offset::Column::Line,
                    ])
                    .to_owned(),
            )
            .exec_without_returning(&txn)
            .await?;
        txn.commit().await?;
        Ok(changed)
    }

    /// Forgets the transcript at `path` (it got shorter: it was rewritten), and its pieces, so it
    /// is read again from the start.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn forget_transcript(&self, path: &str) -> Result<(), DbError> {
        let txn = begin_write(&self.conn).await?;
        let prefix = format!("{path}:");
        let rows = doc::Entity::find()
            .filter(doc::Column::Source.is_in([source::SESSION, source::SUMMARY]))
            .filter(doc::Column::Reference.starts_with(prefix.as_str()))
            .all(&txn)
            .await?;
        let gone = rows
            .into_iter()
            .filter(|row| row.reference.starts_with(&prefix))
            .map(|row| row.id)
            .collect();
        delete(&txn, gone).await?;
        transcript_offset::Entity::delete_by_id(path)
            .exec(&txn)
            .await?;
        txn.commit().await?;
        Ok(())
    }

    /// The pieces matching the full-text query `query`, best first, at most `limit`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] (a query FTS5 can't read among them), or
    /// [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn search_words(&self, query: &str, limit: u64) -> Result<Vec<Doc>, DbError> {
        let statement = Statement::from_sql_and_values(
            DbBackend::Sqlite,
            WORD_SEARCH,
            [
                query.into(),
                i64::try_from(limit).unwrap_or(i64::MAX).into(),
            ],
        );
        doc::Entity::find()
            .from_raw_sql(statement)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect()
    }

    /// How many pieces hold `word` (a word of letters and digits only, searched as a quoted
    /// string, so nothing in it is read as syntax).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn word_count(&self, word: &str) -> Result<u64, DbError> {
        let quoted = format!("\"{}\"", word.replace('"', ""));
        let row = self
            .conn
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                WORD_COUNT,
                [quoted.into()],
            ))
            .await?;
        Ok(row
            .and_then(|row| row.try_get_by_index::<i64>(0).ok())
            .and_then(|n| u64::try_from(n).ok())
            .unwrap_or(0))
    }

    /// Deletes the pieces `ids` (their full-text entries and vectors go with them); how many
    /// there were.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn delete_pieces(&self, ids: &[i64]) -> Result<u64, DbError> {
        let txn = begin_write(&self.conn).await?;
        let mut gone = 0;
        for chunk in ids.chunks(500) {
            gone += doc::Entity::delete_many()
                .filter(doc::Column::Id.is_in(chunk.iter().copied()))
                .exec(&txn)
                .await?
                .rows_affected;
        }
        txn.commit().await?;
        Ok(gone)
    }

    /// The pieces with these ids that are still there, by id.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn docs(&self, ids: &[i64]) -> Result<HashMap<i64, Doc>, DbError> {
        let mut found = HashMap::with_capacity(ids.len());
        for chunk in ids.chunks(500) {
            for row in doc::Entity::find()
                .filter(doc::Column::Id.is_in(chunk.iter().copied()))
                .all(&self.conn)
                .await?
            {
                let doc = from_row(row)?;
                found.insert(doc.id, doc);
            }
        }
        Ok(found)
    }

    /// How many pieces are indexed.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn count(&self) -> Result<u64, DbError> {
        Ok(doc::Entity::find().count(&self.conn).await?)
    }

    /// How many pieces of `kind` are indexed.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn count_of(&self, kind: &str) -> Result<u64, DbError> {
        Ok(doc::Entity::find()
            .filter(doc::Column::Source.eq(kind))
            .count(&self.conn)
            .await?)
    }

    /// At most `limit` pieces with no vector yet: the facts first, then the chat and tasks,
    /// then the summaries, then the rest; newest first within each.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn unembedded(&self, limit: u64) -> Result<Vec<Doc>, DbError> {
        let groups: [&[&str]; 3] = [
            &[source::FACT],
            &[source::CHAT, source::TASK],
            &[source::SUMMARY],
        ];
        let listed: Vec<&str> = groups
            .iter()
            .flat_map(|group| group.iter().copied())
            .collect();
        let mut found = Vec::new();
        for group in groups.iter().map(Some).chain([None]) {
            let left = limit.saturating_sub(found.len() as u64);
            if left == 0 {
                break;
            }
            let without_vector = Query::select()
                .column(doc_vector::Column::Id)
                .from(doc_vector::Entity)
                .to_owned();
            let kinds = match group {
                Some(group) => doc::Column::Source.is_in(group.iter().copied()),
                None => doc::Column::Source.is_not_in(listed.iter().copied()),
            };
            for row in doc::Entity::find()
                .filter(kinds)
                .filter(doc::Column::Id.not_in_subquery(without_vector))
                .order_by_desc(doc::Column::Id)
                .limit(left)
                .all(&self.conn)
                .await?
            {
                found.push(from_row(row)?);
            }
        }
        Ok(found)
    }

    /// Stores these pieces' vectors, in place of any they had. A piece gone since is skipped.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn store_vectors(&self, vectors: &[(i64, Vec<f32>)]) -> Result<(), DbError> {
        let txn = begin_write(&self.conn).await?;
        for (id, vector) in vectors {
            if doc::Entity::find_by_id(*id).one(&txn).await?.is_none() {
                continue;
            }
            let row = doc_vector::ActiveModel {
                id: Set(*id),
                v: Set(vector.iter().flat_map(|x| x.to_le_bytes()).collect()),
            };
            doc_vector::Entity::insert(row)
                .on_conflict(
                    OnConflict::column(doc_vector::Column::Id)
                        .update_column(doc_vector::Column::V)
                        .to_owned(),
                )
                .exec_without_returning(&txn)
                .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    /// Every vector, by piece id (in id order).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn vectors(&self) -> Result<Vec<(i64, Vec<f32>)>, DbError> {
        Ok(doc_vector::Entity::find()
            .order_by_asc(doc_vector::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|row| {
                let vector = row
                    .v
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
                    .collect();
                (row.id, vector)
            })
            .collect())
    }

    /// Drops every vector (another model makes them now).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn drop_vectors(&self) -> Result<(), DbError> {
        let txn = begin_write(&self.conn).await?;
        doc_vector::Entity::delete_many().exec(&txn).await?;
        txn.commit().await?;
        Ok(())
    }

    /// Sets the mark `key` to `value`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn set_mark(&self, key: &str, value: &str) -> Result<(), DbError> {
        set_mark(&self.conn, key, value).await
    }

    /// The ids of the pieces that have a vector.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn vector_ids(&self) -> Result<HashSet<i64>, DbError> {
        Ok(doc_vector::Entity::find()
            .select_only()
            .column(doc_vector::Column::Id)
            .into_tuple::<i64>()
            .all(&self.conn)
            .await?
            .into_iter()
            .collect())
    }
}

/// Indexes `new` in pieces: each new or changed piece written, and the pieces of a longer
/// version of it dropped. `true` when anything changed.
async fn put(conn: &impl ConnectionTrait, new: &NewDoc) -> Result<bool, DbError> {
    let parts = pieces(&new.text);
    let mut changed = false;
    for (n, text) in parts.iter().enumerate() {
        let reference = piece_reference(&new.reference, n);
        let found = doc::Entity::find()
            .filter(doc::Column::Source.eq(new.source.as_str()))
            .filter(doc::Column::Reference.eq(reference.as_str()))
            .one(conn)
            .await?;
        match found {
            Some(row) if row.text == *text && row.title == new.title => {}
            Some(row) => {
                let mut change: doc::ActiveModel = row.into();
                change.at = Set(new.at.map(millis));
                change.title = Set(new.title.clone());
                change.text = Set(text.clone());
                change.place = Set(new.place.clone());
                change.update(conn).await?;
                changed = true;
            }
            None => {
                doc::ActiveModel {
                    source: Set(new.source.clone()),
                    reference: Set(reference),
                    at: Set(new.at.map(millis)),
                    title: Set(new.title.clone()),
                    text: Set(text.clone()),
                    place: Set(new.place.clone()),
                    ..Default::default()
                }
                .insert(conn)
                .await?;
                changed = true;
            }
        }
    }
    // A text that got shorter leaves no old pieces behind.
    let later = doc::Entity::find()
        .filter(doc::Column::Source.eq(new.source.as_str()))
        .filter(doc::Column::Reference.starts_with(format!("{}#", new.reference).as_str()))
        .all(conn)
        .await?;
    let gone = later
        .into_iter()
        .filter(|row| {
            piece_number(&new.reference, &row.reference).is_some_and(|n| n >= parts.len())
        })
        .map(|row| row.id)
        .collect();
    Ok(delete(conn, gone).await? || changed)
}

/// Deletes the pieces `ids` (their full-text entries and vectors go with them); `true` when
/// there were any.
async fn delete(conn: &impl ConnectionTrait, ids: Vec<i64>) -> Result<bool, DbError> {
    if ids.is_empty() {
        return Ok(false);
    }
    for chunk in ids.chunks(500) {
        doc::Entity::delete_many()
            .filter(doc::Column::Id.is_in(chunk.iter().copied()))
            .exec(conn)
            .await?;
    }
    Ok(true)
}

async fn set_mark(conn: &impl ConnectionTrait, key: &str, value: &str) -> Result<(), DbError> {
    mark::Entity::insert(mark::ActiveModel {
        key: Set(key.to_owned()),
        value: Set(value.to_owned()),
    })
    .on_conflict(
        OnConflict::column(mark::Column::Key)
            .update_column(mark::Column::Value)
            .to_owned(),
    )
    .exec_without_returning(conn)
    .await?;
    Ok(())
}

fn offset_from_row(offset: i64, line: i64) -> TranscriptOffset {
    TranscriptOffset {
        offset: u64::try_from(offset).unwrap_or_default(),
        line: u64::try_from(line).unwrap_or_default(),
    }
}

fn from_row(row: doc::Model) -> Result<Doc, DbError> {
    Ok(Doc {
        id: row.id,
        at: row.at.map(instant).transpose()?,
        source: row.source,
        reference: row.reference,
        title: row.title,
        text: row.text,
        place: row.place,
    })
}
