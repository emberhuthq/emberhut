//! `POST /api/settings`'s actions: one route for the app's many small changes.
//!
//! A body names what to do by its fields (`{ ui: { hideGoals: false } }`,
//! `{ taskDecide: 12, choice: "Report A" }`); one body may carry several, which run in the
//! order of [`ORDER`], and a field no action takes is left alone.
//! The answer is `{ ok: true }` with the whole state of the body's `room` (Main by default), or
//! 400 `{ ok: false, error }` with the first refusal or failure (the actions before it stay
//! done).
//!
//! Adding an action is a module here for its area (`settings/<area>.rs`, declared below) with a
//! type implementing [`Action`], and one line in [`actions`]: `action(<area>::TheAction),`.
//! Its key must be in [`ORDER`].

use std::fmt;
use std::future::Future;

use emberhut_core::{DeviceId, RoomId};
use futures_util::future::BoxFuture;
use serde_json::{Map, Value};

use crate::app::AppState;

mod agent;
mod auth;
mod build;
mod display;
mod jobs;
mod memory;
mod tasks;
mod workers;

/// Every action, in any order: they run in [`ORDER`].
fn actions() -> Vec<Box<dyn AnyAction>> {
    vec![
        action(display::Ui),
        action(build::Build),
        action(auth::AuthRevoke),
        action(tasks::PinFocus),
        action(tasks::DropFocus),
        action(tasks::TodoDone),
        action(tasks::TodoPark),
        action(tasks::TaskArchive),
        action(tasks::TaskUnarchive),
        action(tasks::TaskDecide),
        action(tasks::XPosted),
        action(tasks::TaskCreate),
        action(tasks::ProjectSetAction),
        action(tasks::TaskSetAction),
        action(tasks::ParkFocus),
        action(agent::On),
        action(agent::Interrupt),
        action(agent::TestLimit),
        action(agent::CancelCheckin),
        action(agent::StopWatch),
        action(memory::FactSet),
        action(memory::PersonSet),
        action(memory::FactDrop),
        action(memory::TrialDrop),
        action(jobs::StopTask),
        action(workers::TaskStop),
    ]
}

/// The body fields that name an action, in the order they run (the wire words).
pub(crate) const ORDER: &[&str] = &[
    "on",
    "interrupt",
    "testLimit",
    "cancelCheckin",
    "stopWatch",
    "stopTask",
    "pinFocus",
    "dropFocus",
    "todoDone",
    "todoPark",
    "taskArchive",
    "taskUnarchive",
    "taskDecide",
    "ui",
    "build",
    "authRevoke",
    "xPosted",
    "factSet",
    "personSet",
    "factDrop",
    "trialDrop",
    "taskStop",
    "taskCreate",
    "projectSet",
    "taskSet",
    "parkFocus",
];

/// A `POST /api/settings` body, as the actions read it: its fields, as sent.
#[derive(Clone, Debug, Default)]
pub(crate) struct Body {
    fields: Map<String, Value>,
}

impl Body {
    /// The body `value`; one that is not an object has no fields (and so asks for nothing).
    pub(crate) fn new(value: Value) -> Self {
        match value {
            Value::Object(fields) => Self { fields },
            _ => Self::default(),
        }
    }

    /// The field `key`, when the body has it (`null` included).
    pub(crate) fn get(&self, key: &str) -> Option<&Value> {
        self.fields.get(key)
    }

    /// The room the body names (`room`: a positive whole number, or such a number as text),
    /// before it is checked to exist.
    pub(crate) fn room(&self) -> Option<RoomId> {
        let room = match self.get("room")? {
            Value::Number(number) => number.as_i64().or_else(|| whole(number.as_f64()?))?,
            Value::String(text) => text.trim().parse::<i64>().ok()?,
            _ => return None,
        };
        (room > 0).then_some(RoomId::new(room))
    }
}

/// `number` as an `i64`, when it is a whole number in range.
fn whole(number: f64) -> Option<i64> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "only a whole number within i64's range is converted"
    )]
    let whole = number as i64;
    #[expect(
        clippy::cast_precision_loss,
        clippy::float_cmp,
        reason = "compared back, exactly, to the float it came from"
    )]
    let exact = whole as f64 == number;
    exact.then_some(whole)
}

/// What an action runs with: the server's services, and the room the body names (Main when it
/// names none, or one that doesn't exist).
#[derive(Clone, Debug)]
pub(crate) struct ActionContext {
    /// Every service.
    pub(crate) app: AppState,
    /// The body's room.
    pub(crate) room: RoomId,
    /// The signed-in device that sent the body; `None` for a request without a session, which
    /// report mode lets through.
    pub(crate) caller: Option<DeviceId>,
}

/// One action of `POST /api/settings`: what the body's field [`Action::key`] asks for.
pub(crate) trait Action: Send + Sync + 'static {
    /// The body field that asks for it; one of [`ORDER`].
    fn key(&self) -> &'static str;

    /// Whether only a signed-in device may ask for it: an action that changes the owner's data,
    /// runs something or acts for the owner. Report mode lets a request without a session
    /// read, never act; such a request is refused with [`ActionError::SignInRequired`] before
    /// the action runs.
    fn needs_device(&self) -> bool {
        false
    }

    /// Does what `body` asks. Called only when the body has [`Action::key`]; an action reads
    /// its field itself (and any field that goes with it), and does nothing when the field is
    /// not of the kind it takes.
    fn apply(
        &self,
        ctx: &ActionContext,
        body: &Body,
    ) -> impl Future<Output = Result<(), ActionError>> + Send;
}

/// Why an action was not done.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub(crate) enum ActionError {
    /// The action refuses this request: 400 `{ ok: false, error }` with the text.
    #[error("{0}")]
    Refused(String),
    /// The action is only for a signed-in device and the request has no session, which report
    /// mode lets through: 401 `signIn`.
    #[error("a signed-in device is required")]
    SignInRequired,
    /// Something failed underneath: 400 `{ ok: false, error }`, with a fixed sentence (the
    /// whole cause chain goes to the log, not to the client).
    #[error("the action failed")]
    Failed(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl ActionError {
    /// A refusal with this text.
    pub(crate) fn refused(text: impl Into<String>) -> Self {
        Self::Refused(text.into())
    }

    /// A failure caused by `error`.
    pub(crate) fn failed(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Failed(Box::new(error))
    }
}

/// An action, as the dispatcher runs it.
trait AnyAction: Send + Sync {
    fn key(&self) -> &'static str;
    fn needs_device(&self) -> bool;
    fn apply<'a>(
        &'a self,
        ctx: &'a ActionContext,
        body: &'a Body,
    ) -> BoxFuture<'a, Result<(), ActionError>>;
}

impl fmt::Debug for dyn AnyAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Action").field(&self.key()).finish()
    }
}

/// `action` for the list in [`actions`].
fn action<A: Action>(action: A) -> Box<dyn AnyAction> {
    Box::new(Typed(action))
}

struct Typed<A>(A);

impl<A: Action> AnyAction for Typed<A> {
    fn key(&self) -> &'static str {
        self.0.key()
    }

    fn needs_device(&self) -> bool {
        self.0.needs_device()
    }

    fn apply<'a>(
        &'a self,
        ctx: &'a ActionContext,
        body: &'a Body,
    ) -> BoxFuture<'a, Result<(), ActionError>> {
        Box::pin(self.0.apply(ctx, body))
    }
}

/// Where `key` runs: its place in [`ORDER`], after every listed one when it isn't there.
fn rank(key: &str) -> usize {
    ORDER
        .iter()
        .position(|listed| *listed == key)
        .unwrap_or(ORDER.len())
}

/// Runs every action `body` asks for, in [`ORDER`], stopping at the first that is not done.
pub(crate) async fn apply(ctx: &ActionContext, body: &Body) -> Result<(), ActionError> {
    let mut actions = actions();
    actions.sort_by_key(|action| rank(action.key()));
    let asked: Vec<&Box<dyn AnyAction>> = actions
        .iter()
        .filter(|action| body.get(action.key()).is_some())
        .collect();
    if ctx.caller.is_none() && asked.iter().any(|action| action.needs_device()) {
        return Err(ActionError::SignInRequired);
    }
    for action in asked {
        action.apply(ctx, body).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn every_action_is_in_the_order_and_has_a_key_of_its_own() {
        let actions = actions();
        let mut keys: Vec<&str> = actions.iter().map(|action| action.key()).collect();
        for key in &keys {
            assert!(ORDER.contains(key), "{key} is not in ORDER");
        }
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count);
    }

    #[test]
    fn the_order_is_fixed() {
        assert!(rank("on") < rank("taskDecide"));
        assert!(rank("taskDecide") < rank("ui"));
        assert!(rank("ui") < rank("parkFocus"));
        assert_eq!(rank("notAnAction"), ORDER.len());
    }

    #[test]
    fn a_body_names_its_room_leniently() {
        for (body, room) in [
            (json!({ "room": 2 }), Some(2)),
            (json!({ "room": 2.0 }), Some(2)),
            (json!({ "room": " 3 " }), Some(3)),
            (json!({ "room": 2.5 }), None),
            (json!({ "room": 0 }), None),
            (json!({ "room": -1 }), None),
            (json!({ "room": true }), None),
            (json!({}), None),
            (json!([1, 2]), None),
        ] {
            assert_eq!(
                Body::new(body.clone()).room(),
                room.map(RoomId::new),
                "{body}"
            );
        }
    }
}
