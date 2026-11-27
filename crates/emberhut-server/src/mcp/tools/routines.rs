//! `set_routines`: the owner's daily routines and quiet hours.

use serde::Deserialize;

use super::{Tool, ToolContext, ToolError, ToolOutput};
use crate::services::routines::{Quiet, Routine};

/// Changes the routines and/or the quiet hours.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SetRoutines;

/// `set_routines`' arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct SetRoutinesArgs {
    /// The routines, in place of the ones there are.
    #[garde(dive)]
    routines: Option<Vec<RoutineArg>>,
    /// The quiet hours.
    #[garde(dive)]
    quiet: Option<QuietArg>,
    /// Whether new messages in the owner's chats wake you (on until turned off).
    #[garde(skip)]
    news: Option<bool>,
    /// Whether you get a task to prepare the owner about an hour before each meeting (on
    /// until turned off).
    #[garde(skip)]
    meeting_prep: Option<bool>,
}

/// A routine: at `at` (`HH:MM`, server time), every day or on one weekday, what to do.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct RoutineArg {
    /// `HH:MM`.
    #[garde(custom(hour_minute))]
    #[schema(pattern = r"^\d\d:\d\d$")]
    at: String,
    /// What to do then.
    #[garde(length(min = 1, max = 4000))]
    what: String,
    /// Its weekday, 0 Sunday to 6 Saturday, for a weekly routine; left out: every day.
    #[garde(range(max = 6))]
    day: Option<u8>,
}

/// Quiet hours, `HH:MM` to `HH:MM`.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct QuietArg {
    /// When they start.
    #[garde(custom(hour_minute))]
    #[schema(pattern = r"^\d\d:\d\d$")]
    from: String,
    /// When they end.
    #[garde(custom(hour_minute))]
    #[schema(pattern = r"^\d\d:\d\d$")]
    to: String,
}

/// `HH:MM`, a time of day.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "garde hands a custom rule its context by reference"
)]
fn hour_minute(value: &str, (): &()) -> garde::Result {
    emberhut_core::wake::hour_minute(value)
        .map(drop)
        .ok_or_else(|| garde::Error::new("not a time of day as HH:MM"))
}

impl Tool for SetRoutines {
    type Args = SetRoutinesArgs;

    fn name(&self) -> &'static str {
        "set_routines"
    }

    fn description(&self) -> &'static str {
        "Change the owner's routines (each: at \"HH:MM\" server time, what to do, and day 0-6 \
         (Sunday to Saturday) for a weekly one) and/or quiet hours, and turn off or on the wakes \
         for new messages (news) and meeting prep (meeting_prep), when they ask."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: SetRoutinesArgs,
    ) -> Result<ToolOutput, ToolError> {
        let routines: Option<Vec<Routine>> = args.routines.map(|routines| {
            routines
                .into_iter()
                .map(|routine| Routine {
                    at: routine.at,
                    what: routine.what,
                    day: routine.day,
                })
                .collect()
        });
        let quiet = args.quiet.map(|quiet| Quiet {
            from: quiet.from,
            to: quiet.to,
        });
        ctx.app
            .routines()
            .set(routines.as_deref(), quiet.as_ref())
            .await
            .map_err(ToolError::failed)?;
        for (switch, on) in [
            (&crate::services::messaging::news::NEWS_WAKES, args.news),
            (
                &crate::services::meeting_prep::MEETING_PREP,
                args.meeting_prep,
            ),
        ] {
            if let Some(on) = on {
                ctx.app
                    .routines()
                    .set_switch(switch, on)
                    .await
                    .map_err(ToolError::failed)?;
            }
        }
        ctx.app
            .activity()
            .note("Routines updated")
            .await
            .map_err(ToolError::failed)?;
        Ok(ToolOutput::text("Saved."))
    }
}
