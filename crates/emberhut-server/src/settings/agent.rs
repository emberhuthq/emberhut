//! The assistant's switch and stop buttons: `on` (pause or resume), `interrupt` (stop the
//! answer being written in the body's room), `testLimit` (that room's next answer counts as its
//! account running out), and the app's cancel buttons for a check-in (`cancelCheckin`, its
//! time) and a watch (`stopWatch`, its number).

use serde_json::Value;

use super::{Action, ActionContext, ActionError, Body};

/// `{ on: bool }`: pause (`false`: every room's turn stops, nothing wakes the assistant) or
/// resume.
#[derive(Clone, Copy, Debug)]
pub(super) struct On;

impl Action for On {
    fn key(&self) -> &'static str {
        "on"
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(on) = body.get("on").and_then(Value::as_bool) else {
            return Ok(());
        };
        ctx.app.wake().set_on(on).await.map_err(ActionError::failed)
    }
}

/// `{ interrupt: true, room? }`: the app's stop button.
#[derive(Clone, Copy, Debug)]
pub(super) struct Interrupt;

impl Action for Interrupt {
    fn key(&self) -> &'static str {
        "interrupt"
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        if body.get("interrupt") != Some(&Value::Bool(true)) {
            return Ok(());
        }
        ctx.app
            .talk()
            .interrupt(ctx.room)
            .await
            .map_err(ActionError::failed)
    }
}

/// `{ testLimit: true, room? }`: the room's next answer is taken as its account running out.
#[derive(Clone, Copy, Debug)]
pub(super) struct TestLimit;

impl Action for TestLimit {
    fn key(&self) -> &'static str {
        "testLimit"
    }

    fn apply(
        &self,
        ctx: &ActionContext,
        body: &Body,
    ) -> impl Future<Output = Result<(), ActionError>> + Send {
        if body.get("testLimit") == Some(&Value::Bool(true)) {
            ctx.app.talk().assistant().test_limit(ctx.room);
        }
        std::future::ready(Ok(()))
    }
}

/// `{ cancelCheckin: "<its time>" }`.
#[derive(Clone, Copy, Debug)]
pub(super) struct CancelCheckin;

impl Action for CancelCheckin {
    fn key(&self) -> &'static str {
        "cancelCheckin"
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(at) = body.get("cancelCheckin").and_then(Value::as_str) else {
            return Ok(());
        };
        ctx.app
            .wake()
            .cancel_checkin(at)
            .await
            .map(drop)
            .map_err(ActionError::failed)
    }
}

/// `{ stopWatch: <its number> }`.
#[derive(Clone, Copy, Debug)]
pub(super) struct StopWatch;

impl Action for StopWatch {
    fn key(&self) -> &'static str {
        "stopWatch"
    }

    async fn apply(&self, ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
        let Some(id) = body.get("stopWatch").and_then(whole_number) else {
            return Ok(());
        };
        ctx.app
            .wake()
            .stop_watch(id)
            .await
            .map(drop)
            .map_err(ActionError::failed)
    }
}

/// A JSON number as a whole number (a fraction is cut).
fn whole_number(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        let number = value.as_f64()?.trunc();
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a watch's number; anything out of range matches no watch"
        )]
        let whole = number as i64;
        Some(whole)
    })
}
