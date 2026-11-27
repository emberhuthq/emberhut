//! `calendar`, `reminders`, `add_event` and `add_reminder`: the owner's calendar and reminders
//! (every account on their Mac). Reads give the last copy, with its time, when the Mac sleeps.

use serde::Deserialize;

use super::{Tool, ToolContext, ToolError, ToolOutput};
use crate::services::calls::pick;

/// The days `calendar` reads when not told.
fn week() -> u32 {
    7
}

/// The owner's calendar from today on.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Calendar;

/// `calendar`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct CalendarArgs {
    /// How many days, from today.
    #[serde(default = "week")]
    #[garde(range(min = 1, max = 60))]
    #[schema(minimum = 1, maximum = 60, default = 7)]
    days: u32,
}

impl Tool for Calendar {
    type Args = CalendarArgs;

    fn name(&self) -> &'static str {
        "calendar"
    }

    fn description(&self) -> &'static str {
        "The user's calendar events from today on (all their accounts): title, start, end (UTC \
         ISO: say them in the user's local time), calendar, location, number of people."
    }

    async fn call(&self, ctx: &ToolContext, args: CalendarArgs) -> Result<ToolOutput, ToolError> {
        ctx.app
            .calendar()
            .upcoming(args.days)
            .await
            .map(ToolOutput::text)
            .map_err(ToolError::failed)
    }
}

/// The owner's open reminders.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Reminders;

/// `reminders` takes no arguments.
#[derive(Clone, Copy, Debug, Default, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct RemindersArgs {}

impl Tool for Reminders {
    type Args = RemindersArgs;

    fn name(&self) -> &'static str {
        "reminders"
    }

    fn description(&self) -> &'static str {
        "The user's open reminders (their Reminders app, all lists), with due dates."
    }

    async fn call(&self, ctx: &ToolContext, _args: RemindersArgs) -> Result<ToolOutput, ToolError> {
        ctx.app
            .calendar()
            .reminders()
            .await
            .map(ToolOutput::text)
            .map_err(ToolError::failed)
    }
}

/// Adds a reminder.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AddReminder;

/// `add_reminder`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct AddReminderArgs {
    /// What to be reminded of.
    #[garde(length(min = 1, max = 1000))]
    title: String,
    /// ISO date-time with offset, e.g. 2026-10-13T17:00:00+03:00.
    #[garde(length(max = 64))]
    due: Option<String>,
    /// The list it goes in.
    #[garde(length(max = 200))]
    list: Option<String>,
    /// Notes.
    #[garde(length(max = 10_000))]
    notes: Option<String>,
}

impl Tool for AddReminder {
    type Args = AddReminderArgs;

    fn name(&self) -> &'static str {
        "add_reminder"
    }

    fn description(&self) -> &'static str {
        "Add a reminder to the user's Reminders app (syncs to all their devices and alerts them \
         at `due`). For \"remind me...\" requests."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: AddReminderArgs,
    ) -> Result<ToolOutput, ToolError> {
        let reminder = pick(&[
            ("title", Some(args.title.as_str())),
            ("due", args.due.as_deref()),
            ("list", args.list.as_deref()),
            ("notes", args.notes.as_deref()),
        ]);
        ctx.app
            .calendar()
            .add_reminder(reminder)
            .await
            .map(ToolOutput::text)
            .map_err(ToolError::failed)
    }
}

/// Adds an event, only the owner in it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AddEvent;

/// `add_event`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct AddEventArgs {
    /// The event's title.
    #[garde(length(min = 1, max = 1000))]
    title: String,
    /// When it starts: ISO date-time with offset.
    #[garde(length(min = 1, max = 64))]
    start: String,
    /// When it ends: ISO date-time with offset.
    #[garde(length(min = 1, max = 64))]
    end: String,
    /// The calendar it goes in.
    #[garde(length(max = 200))]
    calendar: Option<String>,
    /// Where.
    #[garde(length(max = 1000))]
    location: Option<String>,
    /// Notes.
    #[garde(length(max = 10_000))]
    notes: Option<String>,
}

impl Tool for AddEvent {
    type Args = AddEventArgs;

    fn name(&self) -> &'static str {
        "add_event"
    }

    fn description(&self) -> &'static str {
        "Add an event to the user's calendar, only them (no invitees, ever). For blocking time \
         or noting something they asked for; it shows as focus time in their calendar."
    }

    async fn call(&self, ctx: &ToolContext, args: AddEventArgs) -> Result<ToolOutput, ToolError> {
        let event = pick(&[
            ("title", Some(args.title.as_str())),
            ("start", Some(args.start.as_str())),
            ("end", Some(args.end.as_str())),
            ("calendar", args.calendar.as_deref()),
            ("location", args.location.as_deref()),
            ("notes", args.notes.as_deref()),
        ]);
        ctx.app
            .calendar()
            .add_event(event)
            .await
            .map(ToolOutput::text)
            .map_err(ToolError::failed)
    }
}
