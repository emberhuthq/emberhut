//! `authRevoke`: the owner signs a device out (`{ authRevoke: <device id> }`).

use emberhut_core::DeviceId;
use serde_json::Value;

use super::{Action, ActionContext, ActionError, Body};

/// `{ authRevoke: <device id> }`: the device's tokens stop working at once, until it pairs
/// again. A value that is not a number asks for nothing; an id no device has changes nothing.
/// Only a signed-in device may sign one out, in either auth mode:
/// report mode's pass for a request without a session would otherwise let anyone who reaches
/// the server sign every device out, one id after another.
#[derive(Clone, Copy, Debug)]
pub(super) struct AuthRevoke;

impl Action for AuthRevoke {
    fn key(&self) -> &'static str {
        "authRevoke"
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        if ctx.caller.is_none() {
            return Err(ActionError::SignInRequired);
        }
        let Some(Value::Number(number)) = body.get("authRevoke") else {
            return Ok(());
        };
        let Some(id) = number.as_i64().or_else(|| truncated(number.as_f64()?)) else {
            return Ok(());
        };
        ctx.app
            .auth()
            .sign_out(DeviceId::new(id))
            .await
            .map_err(ActionError::failed)
    }
}

/// `number` cut to a whole number (an id given as a fraction is cut), when it is in range.
fn truncated(number: f64) -> Option<i64> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "truncating toward zero is the point; the range is checked first"
    )]
    let whole = number.trunc() as i64;
    #[expect(
        clippy::cast_precision_loss,
        reason = "only a bound for the range check"
    )]
    let in_range = number.is_finite() && number.abs() < i64::MAX as f64;
    in_range.then_some(whole)
}
