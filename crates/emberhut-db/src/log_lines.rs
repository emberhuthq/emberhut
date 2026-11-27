//! The activity log.

use emberhut_core::LogLineId;
use emberhut_core::log::LogLine;
use jiff::Timestamp;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, DatabaseConnection, EntityTrait, QueryOrder, QuerySelect,
};

use crate::DbError;
use crate::entity::log_line;
use crate::time::{instant, millis};

/// Writes and reads the activity log.
#[derive(Clone, Debug)]
pub struct LogStore {
    conn: DatabaseConnection,
}

impl LogStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Writes `text` as done at `at`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn add(&self, text: impl Into<String>, at: Timestamp) -> Result<LogLine, DbError> {
        let row = log_line::ActiveModel {
            at: Set(millis(at)),
            text: Set(text.into()),
            ..Default::default()
        }
        .insert(&self.conn)
        .await?;
        from_row(row)
    }

    /// The newest `limit` lines, newest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn recent(&self, limit: u64) -> Result<Vec<LogLine>, DbError> {
        log_line::Entity::find()
            .order_by_desc(log_line::Column::Id)
            .limit(limit)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect()
    }
}

fn from_row(row: log_line::Model) -> Result<LogLine, DbError> {
    Ok(LogLine {
        id: LogLineId::new(row.id),
        at: instant(row.at)?,
        text: row.text,
    })
}
