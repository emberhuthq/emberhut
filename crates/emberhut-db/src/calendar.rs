//! The calendar's days ([`CalendarStore`]): each event the Mac's calendars gave, on each day it
//! covers, as last seen.

use jiff::Timestamp;
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
};

use crate::entity::calendar_day;
use crate::time::millis;
use crate::{DbError, begin_write};

/// An event on one of its days, to store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DayEvent {
    /// The day, `2026-10-19`.
    pub day: String,
    /// The event's id.
    pub id: String,
    /// The event, as JSON.
    pub event: String,
}

/// Writes and reads the calendar's days.
#[derive(Clone, Debug)]
pub struct CalendarStore {
    conn: DatabaseConnection,
}

impl CalendarStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// What the Mac said for `days`, seen at `at`: each of those days loses what it had, and
    /// gets `events` (each on its own day; one on a day not in `days` is left out). One
    /// transaction.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn replace(
        &self,
        days: &[String],
        events: Vec<DayEvent>,
        at: Timestamp,
    ) -> Result<(), DbError> {
        let txn = begin_write(&self.conn).await?;
        if !days.is_empty() {
            calendar_day::Entity::delete_many()
                .filter(calendar_day::Column::Day.is_in(days.iter().cloned()))
                .exec(&txn)
                .await?;
        }
        for event in events.into_iter().filter(|e| days.contains(&e.day)) {
            calendar_day::Entity::insert(calendar_day::ActiveModel {
                day: Set(event.day),
                id: Set(event.id),
                event: Set(event.event),
                seen: Set(Some(millis(at))),
            })
            .on_conflict(
                OnConflict::columns([calendar_day::Column::Day, calendar_day::Column::Id])
                    .update_columns([calendar_day::Column::Event, calendar_day::Column::Seen])
                    .to_owned(),
            )
            .exec_without_returning(&txn)
            .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    /// The events kept for the days `from..=to`, by day (the JSON as stored).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn between(&self, from: &str, to: &str) -> Result<Vec<String>, DbError> {
        Ok(calendar_day::Entity::find()
            .filter(calendar_day::Column::Day.gte(from))
            .filter(calendar_day::Column::Day.lte(to))
            .order_by_asc(calendar_day::Column::Day)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|row| row.event)
            .collect())
    }
}
