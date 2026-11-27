//! The assistant's own appointments: check-ins (`wake_me_at`, `list_checkins`,
//! `cancel_checkin`) and watches (`watch`, `watch_note`, `list_watches`, `stop_watch`).

use serde::Deserialize;

use super::{NoArgs, Tool, ToolContext, ToolError, ToolOutput};
use crate::services::wake::{NewWatch, WakeError};

/// Schedules a check-in.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WakeMeAt;

/// `wake_me_at`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct WakeMeAtArgs {
    /// Minutes from now, up to a week.
    #[garde(range(min = 1, max = 10_080))]
    #[schema(minimum = 1, maximum = 10_080)]
    minutes: u32,
    /// Why: what to look at then.
    #[garde(length(min = 1, max = 2000))]
    why: String,
}

impl Tool for WakeMeAt {
    type Args = WakeMeAtArgs;

    fn name(&self) -> &'static str {
        "wake_me_at"
    }

    fn description(&self) -> &'static str {
        "Schedule your own check-in: the server wakes you after `minutes` with your reason, \
         e.g. 30, \"is the build on the laptop done?\". Start the reason with the task it is \
         for when there is one."
    }

    async fn call(&self, ctx: &ToolContext, args: WakeMeAtArgs) -> Result<ToolOutput, ToolError> {
        let at = ctx
            .app
            .wake()
            .wake_me_at(args.minutes, &args.why, ctx.room)
            .await
            .map_err(ToolError::failed)?;
        Ok(ToolOutput::text(format!("Check-in at {at}.")))
    }
}

/// The check-ins.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ListCheckins;

impl Tool for ListCheckins {
    type Args = NoArgs;

    fn name(&self) -> &'static str {
        "list_checkins"
    }

    fn description(&self) -> &'static str {
        "Your scheduled check-ins."
    }

    async fn call(&self, ctx: &ToolContext, _args: NoArgs) -> Result<ToolOutput, ToolError> {
        let checkins = ctx.app.wake().checkins().await.map_err(ToolError::failed)?;
        ToolOutput::json(&checkins)
    }
}

/// Cancels a check-in.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CancelCheckin;

/// `cancel_checkin`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct CancelCheckinArgs {
    /// Its time, as `list_checkins` gives it.
    #[garde(length(min = 1, max = 64))]
    at: String,
}

impl Tool for CancelCheckin {
    type Args = CancelCheckinArgs;

    fn name(&self) -> &'static str {
        "cancel_checkin"
    }

    fn description(&self) -> &'static str {
        "Cancel a scheduled check-in by its time (from list_checkins)."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: CancelCheckinArgs,
    ) -> Result<ToolOutput, ToolError> {
        let cancelled = ctx
            .app
            .wake()
            .cancel_checkin(&args.at)
            .await
            .map_err(ToolError::failed)?;
        Ok(ToolOutput::text(if cancelled {
            "Cancelled."
        } else {
            "No such check-in."
        }))
    }
}

/// Starts a watch.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WatchTool;

/// `watch`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct WatchArgs {
    /// The coding thread's id when watching one thread: the reminder then comes only after
    /// that thread changed.
    #[garde(length(max = 256))]
    thread: Option<String>,
    /// A few plain words for the app, e.g. "the release build on the laptop".
    #[garde(length(min = 1, max = 60))]
    title: String,
    /// Exactly what to look at (machine, thread id) and what counts as news.
    #[garde(length(min = 1, max = 4000))]
    what: String,
    /// Minutes between reminders.
    #[garde(range(min = 5, max = 1440))]
    #[schema(minimum = 5, maximum = 1440)]
    every_minutes: u32,
    /// Hours it lasts (24 when left out).
    #[serde(default = "a_day")]
    #[garde(range(min = 0.25, max = 168.0))]
    #[schema(minimum = 0.25, maximum = 168, default = 24)]
    hours: f64,
    /// What you see now.
    #[serde(default)]
    #[garde(length(max = 4000))]
    note: String,
}

fn a_day() -> f64 {
    24.0
}

impl Tool for WatchTool {
    type Args = WatchArgs;

    fn name(&self) -> &'static str {
        "watch"
    }

    fn description(&self) -> &'static str {
        "Keep an eye on something for a while (a coding thread, a build, a run): every \
         `every_minutes` you get a reminder to look (batched with other news; a one-thread \
         watch reminds you only after that thread changed). `what`: exactly what to look at \
         and what counts as news. `note`: what you see now; keep it current with watch_note."
    }

    async fn call(&self, ctx: &ToolContext, args: WatchArgs) -> Result<ToolOutput, ToolError> {
        let every = f64::from(args.every_minutes);
        let hours = args.hours;
        let id = ctx
            .app
            .wake()
            .add_watch(
                NewWatch {
                    title: args.title,
                    thread: args.thread.filter(|thread| !thread.is_empty()),
                    what: args.what,
                    every_minutes: every,
                    hours,
                    note: args.note,
                },
                ctx.room,
            )
            .await
            .map_err(ToolError::failed)?;
        Ok(ToolOutput::text(format!(
            "Watch #{id} started; it checks every {every} min for up to {hours} h."
        )))
    }
}

/// Notes what a watch last saw.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WatchNote;

/// `watch_note`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct WatchNoteArgs {
    /// The watch's number.
    #[garde(skip)]
    id: i64,
    /// What it saw, one or two sentences.
    #[garde(length(max = 4000))]
    note: String,
}

impl Tool for WatchNote {
    type Args = WatchNoteArgs;

    fn name(&self) -> &'static str {
        "watch_note"
    }

    fn description(&self) -> &'static str {
        "Update what a watch last saw (one or two sentences), after you looked."
    }

    async fn call(&self, ctx: &ToolContext, args: WatchNoteArgs) -> Result<ToolOutput, ToolError> {
        match ctx.app.wake().note_watch(args.id, &args.note).await {
            Ok(()) => Ok(ToolOutput::text("Noted.")),
            Err(error @ WakeError::NoWatch(_)) => Err(ToolError::refused(error.to_string())),
            Err(error) => Err(ToolError::failed(error)),
        }
    }
}

/// The watches.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ListWatches;

impl Tool for ListWatches {
    type Args = NoArgs;

    fn name(&self) -> &'static str {
        "list_watches"
    }

    fn description(&self) -> &'static str {
        "Your running watches with their last note."
    }

    async fn call(&self, ctx: &ToolContext, _args: NoArgs) -> Result<ToolOutput, ToolError> {
        let watches = ctx.app.wake().watches().await.map_err(ToolError::failed)?;
        ToolOutput::json(&watches)
    }
}

/// Stops a watch.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StopWatch;

/// `stop_watch`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct StopWatchArgs {
    /// The watch's number.
    #[garde(skip)]
    id: i64,
}

impl Tool for StopWatch {
    type Args = StopWatchArgs;

    fn name(&self) -> &'static str {
        "stop_watch"
    }

    fn description(&self) -> &'static str {
        "Stop a watch by id (when the thing is done or no longer matters)."
    }

    async fn call(&self, ctx: &ToolContext, args: StopWatchArgs) -> Result<ToolOutput, ToolError> {
        ctx.app
            .wake()
            .stop_watch(args.id)
            .await
            .map_err(ToolError::failed)?;
        Ok(ToolOutput::text("Stopped."))
    }
}
