//! `build`: the release step's report that a store has a build ready
//! (`{ build: { platform, number } }`).

use crate::services::settings_build::{BuildError, KEY};

use super::{Action, ActionContext, ActionError, Body};

/// `{ build: { platform, number } }`: the platform's newest build, which only moves up. A
/// `build` that is not an object asks for nothing; an object that is not a platform and a whole
/// build number is refused.
#[derive(Clone, Copy, Debug)]
pub(super) struct Build;

impl Action for Build {
    fn key(&self) -> &'static str {
        KEY
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(build) = body.get(KEY) else {
            return Ok(());
        };
        match ctx.app.builds().apply(build).await {
            Ok(_) => Ok(()),
            Err(BuildError::Invalid(invalid)) => Err(ActionError::refused(invalid.to_string())),
            Err(error) => Err(ActionError::failed(error)),
        }
    }
}
