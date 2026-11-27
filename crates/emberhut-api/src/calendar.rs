//! `GET /api/calendar?from=&to=`: the person's calendar for a range of days (at most 62): the
//! events from every calendar account the server reads, titled as they are on the calendar,
//! and apart from them what the person has to work on, each on its day: task and project
//! deadlines, the days asks have to be decided by, and today's planned work. Read-only.
//!
//! Days the calendar's source didn't cover this time show what was last seen, and the answer's
//! [`CalendarPage::note`] says so. A bad range is 400 with an
//! [`ErrorBody`](crate::error::ErrorBody).

use serde::{Deserialize, Deserializer, Serialize};

use crate::ids::{ProjectId, TaskId};
use crate::open_enum::open_enum;
use crate::query::push_encoded;
use crate::tasks::TaskStatus;
use crate::time::{Day, Timestamp};

/// The range asked for: `?from=2026-10-19&to=2026-10-25`, both days included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema, utoipa::IntoParams), into_params(parameter_in = Query))]
pub struct CalendarQuery {
    /// The first day.
    pub from: Day,
    /// The last day, on or after `from`.
    pub to: Day,
}

impl CalendarQuery {
    /// The range `from..=to`.
    #[must_use]
    pub fn new(from: Day, to: Day) -> Self {
        Self { from, to }
    }

    /// The query string, without its `?`.
    ///
    /// ```
    /// use emberhut_api::calendar::CalendarQuery;
    /// use emberhut_api::time::Day;
    ///
    /// let query = CalendarQuery::new(Day::new("2026-10-19"), Day::new("2026-10-25"));
    /// assert_eq!(query.to_query(), "from=2026-10-19&to=2026-10-25");
    /// ```
    #[must_use]
    pub fn to_query(&self) -> String {
        let mut query = String::from("from=");
        push_encoded(&mut query, self.from.as_str());
        query.push_str("&to=");
        push_encoded(&mut query, self.to.as_str());
        query
    }
}

/// The answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CalendarPage {
    /// The first day, as asked.
    pub from: Day,
    /// The last day, as asked.
    pub to: Day,
    /// The events, by their start.
    pub events: Vec<CalendarEvent>,
    /// What the person has to work on, each on its day.
    pub items: Vec<CalendarItem>,
    /// Why some days show what was last seen (the calendar's source didn't answer).
    pub note: Option<String>,
    /// When the server answered.
    pub at: Timestamp,
}

open_enum! {
    /// What an event is to the person.
    pub enum EventKind {
        /// Someone else is in it.
        Meeting = "meeting",
        /// Only the person.
        Event = "event",
        /// Time blocked to work, by the person or the assistant.
        Focus = "focus",
    }
}

/// An event, as the calendar has it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CalendarEvent {
    /// Its id, stable across answers.
    pub id: String,
    /// Its title, as on the calendar.
    pub title: String,
    /// When it starts.
    pub start: Timestamp,
    /// When it ends (an all-day event: the start of the day after its last).
    pub end: Timestamp,
    /// Whether it takes whole days rather than hours.
    pub all_day: bool,
    /// The calendar it is on.
    pub calendar: String,
    /// Where it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// Its notes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// How many people are in it.
    pub people: u32,
    /// The day it starts on.
    pub day: Day,
    /// What it is to the person.
    pub kind: EventKind,
}

open_enum! {
    /// What a work item on the calendar is.
    pub enum CalendarItemKind {
        /// A task's deadline, or a project's.
        Deadline = "deadline",
        /// The day an ask has to be decided by.
        Decide = "decide",
        /// One of today's planned tasks.
        Today = "today",
    }
}

/// Something the person has to work on, on its day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CalendarItem {
    /// What it is.
    pub kind: CalendarItemKind,
    /// Its day.
    pub day: Day,
    /// Its task; none on a project's deadline.
    pub task: Option<TaskId>,
    /// The task's key, as the server spells it.
    pub key: Option<String>,
    /// The project it belongs to.
    pub project: Option<ProjectId>,
    /// The task's or the project's title.
    pub title: String,
    /// A hard deadline: it can't move.
    pub hard: bool,
    /// The task's status; none on a project's deadline.
    pub status: Option<TaskStatus>,
    /// On a project's deadline: how many of its open tasks take it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    /// On an ask's day: what the person has to do, `null` when the ask names nothing (the
    /// server writes the key on every ask's day, and only there).
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub action: Option<Option<String>>,
    /// On an ask's day: whether the ask waits folded under Later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub later: Option<bool>,
}

impl CalendarItem {
    /// What the person has to do, on an ask's day.
    #[must_use]
    pub fn action(&self) -> Option<&str> {
        self.action.as_ref()?.as_deref()
    }
}

/// For `deserialize_with` on a key that may be written as `null`: present (even as `null`) is
/// `Some`, absent is the field's default, `None`.
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}
