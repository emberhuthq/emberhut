//! `stopTask`: the person stops one of the assistant's own background tasks (`{ stopTask:
//! "<id>" }`), from the app's list of them.
//!
//! The agent runtime keeps the background tasks; the silent assistant (no agent CLI) runs with
//! [`crate::services::workers_view::NoWorkers`], so no background task runs and the action
//! stops nothing.

use serde_json::Value;

use super::{Action, ActionContext, ActionError, Body};

/// `{ stopTask: "<id>" }`: the background task stops. A value that is not text asks for
/// nothing, and an id no task has changes nothing (it may have ended), as on the reference
/// server.
#[derive(Clone, Copy, Debug)]
pub(super) struct StopTask;

impl Action for StopTask {
    fn key(&self) -> &'static str {
        "stopTask"
    }

    fn needs_device(&self) -> bool {
        true
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        if let Some(Value::String(id)) = body.get("stopTask") {
            let stopped = ctx.app.assistant_tasks().stop(id).await;
            tracing::debug!(%id, stopped, "a background task was asked to stop");
        }
        Ok(())
    }
}
