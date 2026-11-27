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
}

/// A routine: at `at` (`HH:MM`, server time) every day, what to do.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct RoutineArg {
    /// `HH:MM`.
    #[garde(custom(hour_minute))]
    #[schema(pattern = r"^\d\d:\d\d$")]
    at: String,
    /// What to do then.
    #[garde(length(min = 1, max = 4000))]
    what: String,
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
        "Change the owner's daily routines (each: at \"HH:MM\" server time, what to do) and/or \
         quiet hours, when they ask."
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
                    day: None,
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
        ctx.app
            .activity()
            .note("Routines updated")
            .await
            .map_err(ToolError::failed)?;
        Ok(ToolOutput::text("Saved."))
    }
}
