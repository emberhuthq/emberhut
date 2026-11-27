//! `taskStop`: the person stops a task's worker (`{ taskStop: <task id> }`), from the task's
//! page: its session stops, its run closes, and a task still marked working goes back to the
//! assistant's to do.

use emberhut_core::task::TaskId;
use serde_json::Value;

use super::{Action, ActionContext, ActionError, Body};

/// `{ taskStop: <id> }`: the task's worker stops. A value that is not a number asks for
/// nothing.
#[derive(Clone, Copy, Debug)]
pub(super) struct TaskStop;

impl Action for TaskStop {
    fn key(&self) -> &'static str {
        "taskStop"
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(Value::Number(id)) = body.get("taskStop") else {
            return Ok(());
        };
        let Some(id) = id.as_i64().or_else(|| {
            id.as_f64().filter(|n| n.fract() == 0.0).map(|n| {
                #[expect(clippy::cast_possible_truncation, reason = "a whole task id")]
                let id = n as i64;
                id
            })
        }) else {
            return Ok(());
        };
        match ctx.app.tasks().stop_worker(TaskId::new(id)).await {
            Ok(()) => Ok(()),
            Err(crate::services::tasks::TaskError::Refused(why)) => Err(ActionError::Refused(why)),
            Err(other) => Err(ActionError::failed(other)),
        }
    }
}
