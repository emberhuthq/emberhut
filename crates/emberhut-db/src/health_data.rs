//! The owner's health and screen time ([`HealthStore`]): the phone's summary of each day, and
//! the Mac's screen-time totals per day and device, each kept as it came (JSON).

use jiff::Timestamp;
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
};

use crate::entity::{health_day, screentime_day};
use crate::time::millis;
use crate::{DbError, begin_write};

/// Writes and reads health and screen-time days.
#[derive(Clone, Debug)]
pub struct HealthStore {
    conn: DatabaseConnection,
}

impl HealthStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Keeps each `(date, data)` as that day's health summary, in place of what the day had,
    /// arrived at `at`. One transaction.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn put_health(
        &self,
        days: Vec<(String, String)>,
        at: Timestamp,
    ) -> Result<(), DbError> {
        let txn = begin_write(&self.conn).await?;
        for (date, data) in days {
            health_day::Entity::insert(health_day::ActiveModel {
                date: Set(date),
                data: Set(data),
                at: Set(millis(at)),
            })
            .on_conflict(
                OnConflict::column(health_day::Column::Date)
                    .update_columns([health_day::Column::Data, health_day::Column::At])
                    .to_owned(),
            )
            .exec_without_returning(&txn)
            .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    /// The health summary of `date`, as stored.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn health_day(&self, date: &str) -> Result<Option<String>, DbError> {
        Ok(health_day::Entity::find_by_id(date)
            .one(&self.conn)
            .await?
            .map(|row| row.data))
    }

    /// The newest `limit` days' health summaries, newest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn recent_health(&self, limit: u64) -> Result<Vec<String>, DbError> {
        Ok(health_day::Entity::find()
            .order_by_desc(health_day::Column::Date)
            .limit(limit)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|row| row.data)
            .collect())
    }

    /// Keeps each `(date, device, data)` as that device's screen time that day, in place of
    /// what it had, arrived at `at`. One transaction.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn put_screentime(
        &self,
        days: Vec<(String, String, String)>,
        at: Timestamp,
    ) -> Result<(), DbError> {
        let txn = begin_write(&self.conn).await?;
        for (date, device, data) in days {
            screentime_day::Entity::insert(screentime_day::ActiveModel {
                date: Set(date),
                device: Set(device),
                data: Set(data),
                at: Set(millis(at)),
            })
            .on_conflict(
                OnConflict::columns([screentime_day::Column::Date, screentime_day::Column::Device])
                    .update_columns([screentime_day::Column::Data, screentime_day::Column::At])
                    .to_owned(),
            )
            .exec_without_returning(&txn)
            .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    /// The screen time of the days from `from` (`2026-10-12`) on, newest day first, then by
    /// device.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn screentime_since(&self, from: &str) -> Result<Vec<String>, DbError> {
        Ok(screentime_day::Entity::find()
            .filter(screentime_day::Column::Date.gte(from))
            .order_by_desc(screentime_day::Column::Date)
            .order_by_asc(screentime_day::Column::Device)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|row| row.data)
            .collect())
    }
}
