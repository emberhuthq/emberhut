//! Pausing and resuming the assistant (`{ on }` on `/api/settings`). Paused, it wakes for
//! nothing and stops the turn it is in; it still answers in the chat. The switch shows its new
//! place at once: what was asked counts until the state agrees, or for [`HOLD`] at most (a
//! poll that finds the server unchanged keeps the old state on hand).

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::AppContext as _;
use gpui_kit::Context;

use crate::backend::Error;
use crate::brand::APP_NAME;
use crate::store::{Store, update_or_log};
use emberhut_api::settings::SettingsAction;

/// How long a switch asked for counts over the state's word.
const HOLD: Duration = Duration::from_secs(10);

/// The switch's change on its way, and why the last one failed.
#[derive(Debug, Default)]
pub(crate) struct OnDuty {
    /// What was asked, and when.
    asked: Option<(bool, Instant)>,
    problem: Option<String>,
}

impl OnDuty {
    /// Whether the assistant is on duty: what was asked while it counts, else the state's.
    #[must_use]
    fn is_on(&self, state_on: bool, now: Instant) -> bool {
        match self.asked {
            Some((on, at)) if now.duration_since(at) < HOLD => on,
            _ => state_on,
        }
    }
}

/// Why the switch didn't move: `Emberhut not paused: <why>`.
fn problem(on: bool, error: &Error) -> String {
    let what = if on { "resumed" } else { "paused" };
    let why = error
        .server_message()
        .map_or_else(|| error.to_string(), str::to_owned);
    format!("{APP_NAME} not {what}: {why}")
}

impl Store {
    /// Whether the assistant is on duty (not paused). On until a state says otherwise.
    #[must_use]
    pub(crate) fn is_on_duty(&self) -> bool {
        let state_on = self.state.as_ref().is_none_or(|state| state.on);
        self.settings.on_duty.is_on(state_on, Instant::now())
    }

    /// Why the last pause or resume failed.
    #[must_use]
    pub(crate) fn on_duty_problem(&self) -> Option<&str> {
        self.settings.on_duty.problem.as_deref()
    }

    /// Clears that line.
    pub(crate) fn dismiss_on_duty_problem(&mut self, cx: &mut Context<'_, Self>) {
        self.settings.on_duty.problem = None;
        cx.notify();
    }

    /// Pauses (`false`) or resumes (`true`) the assistant.
    pub(crate) fn set_on_duty(&mut self, on: bool, cx: &mut Context<'_, Self>) {
        let duty = &mut self.settings.on_duty;
        duty.asked = Some((on, Instant::now()));
        duty.problem = None;
        let (backend, kicker) = (Arc::clone(&self.backend), self.kicker.clone());
        cx.spawn(async move |this, cx| {
            let done = cx
                .background_spawn(async move { backend.settings(SettingsAction::SetOn(on)) })
                .await;
            kicker.kick();
            update_or_log(&this, cx, |store, cx| {
                if let Err(error) = done {
                    let duty = &mut store.settings.on_duty;
                    duty.asked = None;
                    duty.problem = Some(problem(on, &error));
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_was_asked_counts_for_a_while() {
        let now = Instant::now();
        let asked = OnDuty {
            asked: Some((false, now)),
            problem: None,
        };
        assert!(!asked.is_on(true, now + Duration::from_secs(1)));
        assert!(asked.is_on(true, now + HOLD), "then the state's word");
        assert!(OnDuty::default().is_on(true, now));
    }

    #[test]
    fn a_failure_says_which_way() {
        let error = Error::Refused {
            status: Some(400),
            message: Some("busy".into()),
        };
        assert_eq!(problem(false, &error), "Emberhut not paused: busy");
        assert_eq!(
            problem(true, &Error::Unreachable("down".into())),
            "Emberhut not resumed: can't reach the server"
        );
    }
}
