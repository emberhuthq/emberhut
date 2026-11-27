//! `GET /api/scheduled`: everything the assistant has with a next run, every room's, in one
//! list: its watches (with the tasks they keep an eye on), the check-ins it set, its routines,
//! the meeting prep before each event on the person's calendar, and the timer units on the
//! machine the server runs on. Soonest first; a row with no next run last.
//!
//! A watch is stopped and a check-in cancelled through `POST /api/settings`
//! ([`SettingsAction::StopWatch`](crate::settings::SettingsAction::StopWatch),
//! [`SettingsAction::CancelCheckin`](crate::settings::SettingsAction::CancelCheckin)) with the
//! row's [`ScheduledItem::watch`] or [`ScheduledItem::checkin`].

use serde::{Deserialize, Serialize};

use crate::ids::{RoomId, TaskId, WatchId};
use crate::open_enum::open_enum;
use crate::tasks::TaskStatus;
use crate::time::Timestamp;

open_enum! {
    /// What a scheduled row is.
    pub enum ScheduledKind {
        /// A watch: the assistant looks at something again and again.
        Watch = "watch",
        /// A check-in the assistant set for itself.
        Checkin = "checkin",
        /// One of the assistant's routines (a morning brief, an evening wrap).
        Routine = "routine",
        /// The preparation before a meeting on the person's calendar.
        Prep = "prep",
        /// A unit on a timer on the server's machine.
        Unit = "unit",
    }
}

/// The page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ScheduledPage {
    /// The rows, soonest first.
    pub items: Vec<ScheduledItem>,
}

/// One thing with a next run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ScheduledItem {
    /// Its id, unique in the list (`watch:3`, `checkin:<time>`, `unit:<name>`).
    pub id: String,
    /// What it is.
    pub kind: ScheduledKind,
    /// Its title.
    pub title: String,
    /// When it runs next; none: a unit with no next run.
    pub next_at: Option<Timestamp>,
    /// How often it repeats, in words (`every 30 min`, `daily 08:30`, `weekly, Sat 20:00`);
    /// none: once.
    pub repeat: Option<String>,
    /// The room it belongs to.
    pub room: RoomId,
    /// What a watch last saw, a routine's job, a meeting's time, a unit's name.
    pub note: Option<String>,
    /// When the note was written.
    pub note_at: Option<Timestamp>,
    /// The open tasks it works for.
    pub tasks: Vec<ScheduledTask>,
    /// The watch to stop, on a watch's row.
    pub watch: Option<WatchId>,
    /// The check-in to cancel (the time it was set for), on a check-in's row.
    pub checkin: Option<Timestamp>,
}

/// A task a scheduled row works for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ScheduledTask {
    /// The task.
    pub id: TaskId,
    /// Its title.
    pub title: String,
    /// Where it is.
    pub status: TaskStatus,
}
