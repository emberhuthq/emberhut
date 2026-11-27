//! `health` and `screen_time`: the owner's Health summaries from their phone and their screen
//! time from their Mac, as they were sent in.

use serde::Deserialize;

use super::{Tool, ToolContext, ToolError, ToolOutput};

/// The days read when not told.
fn week() -> u32 {
    7
}

/// The owner's Health summaries.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Health;

/// `health`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct HealthArgs {
    /// How many days, newest first.
    #[serde(default = "week")]
    #[garde(range(min = 1, max = 365))]
    #[schema(minimum = 1, maximum = 365, default = 7)]
    days: u32,
}

impl Tool for Health {
    type Args = HealthArgs;

    fn name(&self) -> &'static str {
        "health"
    }

    fn description(&self) -> &'static str {
        "The user's Health summaries from their phone, one row per day, newest first: steps, \
         active energy (kcal), exercise minutes, sleep (minutes asleep, bedtime, wake), resting \
         heart rate, HRV (ms), weight (kg), workouts. Only their own data; summaries, no raw \
         samples. notSent: why the phone last sent no days."
    }

    async fn call(&self, ctx: &ToolContext, args: HealthArgs) -> Result<ToolOutput, ToolError> {
        match ctx
            .app
            .health_data()
            .health_days(u64::from(args.days))
            .await
            .map_err(ToolError::failed)?
        {
            Some(days) => ToolOutput::json(&days),
            None => Ok(ToolOutput::text(
                "No Health data yet: the user turns on Health sharing in the phone app \
                 (settings sheet).",
            )),
        }
    }
}

/// The owner's screen time.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ScreenTime;

/// `screen_time`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct ScreenTimeArgs {
    /// How many days back.
    #[serde(default = "week")]
    #[garde(range(min = 1, max = 90))]
    #[schema(minimum = 1, maximum = 90, default = 7)]
    days: u32,
}

impl Tool for ScreenTime {
    type Args = ScreenTimeArgs;

    fn name(&self) -> &'static str {
        "screen_time"
    }

    fn description(&self) -> &'static str {
        "The user's Screen Time, per day and device (phone, Mac), newest first: total minutes \
         and minutes per app (app ids like com.burbn.instagram = Instagram). From the Mac \
         helper, which reads the Mac's own usage and the phone's synced usage. Estimates, close \
         to the Screen Time app. Use it to help them see their patterns; never shame."
    }

    async fn call(&self, ctx: &ToolContext, args: ScreenTimeArgs) -> Result<ToolOutput, ToolError> {
        match ctx
            .app
            .health_data()
            .screentime_days(i64::from(args.days))
            .await
            .map_err(ToolError::failed)?
        {
            Some(days) => ToolOutput::json(&days),
            None => Ok(ToolOutput::text(
                "No Screen Time yet: the helper on the user's Mac sends it.",
            )),
        }
    }
}
