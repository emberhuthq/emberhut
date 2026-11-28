//! The ember's name (`POST /api/ember/name`, the state's `emberName`): what the user calls
//! their assistant, the same on every device. A rename shows at once everywhere the name is
//! drawn: what was asked counts until the state agrees, or for [`HOLD`] at most. A name the
//! server refuses goes back to the one before, with the server's reason.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::AppContext as _;
use gpui_kit::Context;

use crate::backend::Error;
use crate::store::{Store, update_or_log};
use emberhut_api::ember_name::{DEFAULT_NAME, clean_name};

/// How long a name asked for counts over the state's word.
const HOLD: Duration = Duration::from_secs(10);

/// The rename on its way, and why the last one failed.
#[derive(Debug, Default)]
pub(crate) struct EmberNaming {
    /// The name asked for (none: the default), and when.
    asked: Option<(Option<String>, Instant)>,
    problem: Option<String>,
}

impl EmberNaming {
    /// The user's own name for the ember (none: the default): what was asked while it counts,
    /// else the state's.
    fn custom<'a>(&'a self, state: Option<&'a str>, now: Instant) -> Option<&'a str> {
        match &self.asked {
            Some((asked, at)) if now.duration_since(*at) < HOLD => asked.as_deref(),
            _ => state,
        }
    }
}

/// Why the rename didn't happen: `Not renamed: <why>`.
fn problem(error: &Error) -> String {
    let why = error
        .server_message()
        .map_or_else(|| error.to_string(), str::to_owned);
    refused(&why)
}

/// `Not renamed: <why>`, without the field's name the server's reasons start with.
fn refused(why: &str) -> String {
    format!("Not renamed: {}", why.strip_prefix("name: ").unwrap_or(why))
}

impl Store {
    /// What the ember is called: the user's name for it, or "Your ember".
    #[must_use]
    pub(crate) fn ember_name(&self) -> &str {
        self.ember_custom_name().unwrap_or(DEFAULT_NAME)
    }

    /// The user's own name for the ember; none while it is "Your ember".
    #[must_use]
    pub(crate) fn ember_custom_name(&self) -> Option<&str> {
        let state = self
            .state
            .as_ref()
            .and_then(|state| state.ember_name.as_ref())
            .and_then(|ember| ember.custom());
        self.settings.naming.custom(state, Instant::now())
    }

    /// Why the last rename failed.
    #[must_use]
    pub(crate) fn ember_name_problem(&self) -> Option<&str> {
        self.settings.naming.problem.as_deref()
    }

    /// Renames the ember to `name` (blank: back to "Your ember"): shown at once, then saved
    /// on the server; a name the server refuses goes back to the one before. A name this app
    /// can tell the server would refuse is never sent: its reason shows at once.
    pub(crate) fn rename_ember(&mut self, name: &str, cx: &mut Context<'_, Self>) {
        let name = match clean_name(name) {
            Ok(name) => name,
            Err(refusal) => {
                self.settings.naming.problem = Some(refused(&refusal.to_string()));
                cx.notify();
                return;
            }
        };
        let naming = &mut self.settings.naming;
        let asked = Instant::now();
        naming.asked = Some((name.clone(), asked));
        naming.problem = None;
        let (backend, kicker) = (Arc::clone(&self.backend), self.kicker.clone());
        cx.spawn(async move |this, cx| {
            let done = cx
                .background_spawn(async move { backend.rename_ember(name.as_deref()) })
                .await;
            kicker.kick();
            update_or_log(&this, cx, |store, cx| {
                let naming = &mut store.settings.naming;
                // Only the newest rename's answer counts.
                if naming.asked.as_ref().is_some_and(|(_, at)| *at == asked) {
                    match done {
                        Ok(kept) => naming.asked = Some((kept.custom().map(str::to_owned), asked)),
                        Err(error) => {
                            naming.asked = None;
                            naming.problem = Some(problem(&error));
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Clears the rename's problem line.
    pub(crate) fn dismiss_ember_name_problem(&mut self, cx: &mut Context<'_, Self>) {
        self.settings.naming.problem = None;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_was_asked_counts_for_a_while() {
        let now = Instant::now();
        let asked = EmberNaming {
            asked: Some((Some("Pip".to_owned()), now)),
            problem: None,
        };
        assert_eq!(asked.custom(None, now), Some("Pip"));
        assert_eq!(
            asked.custom(Some("Juniper"), now + HOLD),
            Some("Juniper"),
            "then the state's word"
        );
        let reset = EmberNaming {
            asked: Some((None, now)),
            problem: None,
        };
        assert_eq!(reset.custom(Some("Pip"), now), None, "back to the default");
        assert_eq!(
            EmberNaming::default().custom(Some("Pip"), now),
            Some("Pip")
        );
    }

    #[test]
    fn a_failure_says_why() {
        let error = Error::Refused {
            status: Some(400),
            message: Some("name: 24 characters at most".into()),
        };
        assert_eq!(problem(&error), "Not renamed: 24 characters at most");
        assert_eq!(
            problem(&Error::Unreachable("down".into())),
            "Not renamed: can't reach the server"
        );
    }
}
