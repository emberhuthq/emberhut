//! `ui`: what the app hides (`{ ui: { hideGoals?, hideProjects? } }`).

use super::{Action, ActionContext, ActionError, Body};

/// `{ ui: { hideGoals?, hideProjects? } }`: the switches given change, the others stay. A `ui`
/// that is not an object asks for nothing.
#[derive(Clone, Copy, Debug)]
pub(super) struct Ui;

impl Action for Ui {
    fn key(&self) -> &'static str {
        "ui"
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(change) = body.get("ui").filter(|change| change.is_object()) else {
            return Ok(());
        };
        ctx.app
            .display()
            .set_ui(change)
            .await
            .map_err(ActionError::failed)?;
        Ok(())
    }
}
