//! What the window shows before the app proper, and the lines that tell the user something
//! went wrong: exactly one of "not your device", sign-in, connecting, or the app once a state
//! is on hand; the offline line once polls have failed for a few seconds; and the last action
//! the server refused, with its reason.

use std::time::{Duration, Instant};

use emberhut_api::auth::AuthMode;
use gpui_kit::Context;

use super::Store;
use super::status::Outage;
use crate::backend::Error;

/// How long polls fail before the app says it is offline: a blip (a server restart, a network
/// change) passes without a word.
pub(crate) const QUIET: Duration = Duration::from_secs(5);

/// What the window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gate {
    /// The server doesn't let this device in (a 401 without a sign-in: not on the user's
    /// network). Nothing held shows; the poll keeps checking and opens the app once it is let
    /// in.
    NotYourDevice,
    /// This device has to sign in; `can_wait` when the server only reports sessions so far, so
    /// "Not now" lets the user in for this launch.
    SignIn {
        /// Whether "Not now" is offered.
        can_wait: bool,
    },
    /// No state from the server yet (what was kept on disk waits for it, as the server may not
    /// let this device in any more): connecting, and why it can't yet once polls fail.
    Connecting,
    /// The app.
    Open,
}

/// How far the server lets this device in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Door {
    /// The server refuses this device (not on the user's network): nothing held shows.
    refused: bool,
    /// A state poll went through this launch: the server lets this device in.
    let_in: bool,
    /// "Not now" on the sign-in screen, for this launch.
    later: bool,
}

/// How polls have been going, for the offline line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Failing {
    /// Since when they fail, if they do.
    since: Option<Instant>,
    /// What the offline line says, and why polls fail, once they have failed for [`QUIET`].
    shown: Option<(String, Outage)>,
}

impl Failing {
    /// A poll at `at` failed with `problem`, part of `outage`; whether the line changed.
    pub(crate) fn failed(&mut self, problem: String, outage: Outage, at: Instant) -> bool {
        let since = *self.since.get_or_insert(at);
        if at.saturating_duration_since(since) < QUIET {
            return false;
        }
        let shown = Some((problem, outage));
        let changed = self.shown != shown;
        self.shown = shown;
        changed
    }

    /// A poll went through: the line goes at once; whether it was showing.
    pub(crate) fn passed(&mut self) -> bool {
        self.since = None;
        self.shown.take().is_some()
    }

    /// What the offline line says, if it shows.
    #[must_use]
    pub(crate) fn shown(&self) -> Option<&str> {
        self.shown.as_ref().map(|(line, _)| line.as_str())
    }

    /// Why polls fail, while the offline line shows.
    #[must_use]
    pub(crate) fn outage(&self) -> Option<Outage> {
        self.shown.as_ref().map(|&(_, outage)| outage)
    }
}

/// Why an action failed, as the server or the network put it: the server's own words, `HTTP n`
/// when it gave none, `can't reach the server` when it wasn't reached.
#[must_use]
pub(crate) fn reason(error: &Error) -> String {
    match error {
        Error::Refused {
            message: Some(message),
            ..
        } => message.clone(),
        Error::Refused {
            status: Some(status),
            message: None,
        } => format!("HTTP {status}"),
        Error::Unreachable(_) => "can't reach the server".to_owned(),
        Error::Refused {
            status: None,
            message: None,
        }
        | Error::SignInRequired
        | Error::Unreadable(_)
        | Error::Local(_)
        | Error::WrongMachine => error.to_string(),
    }
}

impl Store {
    /// What the window shows now.
    #[must_use]
    pub(crate) fn gate(&self) -> Gate {
        if self.door.refused {
            Gate::NotYourDevice
        } else if self.shows_sign_in() {
            Gate::SignIn {
                can_wait: !self.preview_sign_in && self.only_reports(),
            }
        } else if self.state.is_none() || !self.door.let_in {
            Gate::Connecting
        } else {
            Gate::Open
        }
    }

    /// Whether the server lets in devices without a session (it only reports them).
    fn only_reports(&self) -> bool {
        self.state
            .as_ref()
            .and_then(|state| state.auth.as_ref())
            .is_some_and(|auth| auth.mode == AuthMode::Report)
    }

    /// "Not now" on the sign-in screen while the server only reports: the app opens, and
    /// doesn't ask again until the next launch.
    pub(crate) fn sign_in_later(&mut self, cx: &mut Context<'_, Self>) {
        if self.only_reports() {
            self.door.later = true;
            self.needs_sign_in = false;
            cx.notify();
        }
    }

    /// A fresh state came: while the server only reports, a device without a session of its
    /// own is asked to sign in (once a launch, if the user says not now).
    pub(super) fn ask_to_sign_in(&mut self) {
        if !self.door.later && !self.is_signed_in() && self.only_reports() {
            self.needs_sign_in = true;
        }
    }

    /// A state poll went through: the server lets this device in.
    pub(super) fn let_in(&mut self) {
        self.door.let_in = true;
    }

    /// The server refused this device (`true`) or let it in again: while refused, nothing
    /// held shows. Refused, the state and the report go, and so do the chats kept on disk.
    pub(super) fn set_refused(&mut self, refused: bool, cx: &mut Context<'_, Self>) {
        if self.door.refused == refused {
            return;
        }
        self.door.refused = refused;
        if refused {
            self.state = None;
            self.etag = None;
            self.report = None;
            self.clear_kept(cx);
        }
        cx.notify();
    }

    /// This device's session ended (the server asks it to sign in, or it was signed out here):
    /// the sign-in screen shows in the app's place, and the chats kept on disk go.
    pub(super) fn signed_out(&mut self, cx: &mut Context<'_, Self>) {
        self.needs_sign_in = true;
        self.clear_kept(cx);
        cx.notify();
    }

    /// The offline line over the composer ("Can't reach the server · retrying"), once polls
    /// have failed for a few seconds.
    #[must_use]
    pub(crate) fn offline(&self) -> Option<&str> {
        self.failing.shown()
    }

    /// Why polls fail, once they have failed for a few seconds.
    #[must_use]
    pub(crate) fn outage(&self) -> Option<Outage> {
        self.failing.outage()
    }

    /// The last action the server refused: what failed and why.
    #[must_use]
    pub(crate) fn action_problem(&self) -> Option<&str> {
        self.action_problem.as_deref()
    }

    /// `what` failed with `error`: the line says so until hidden or the next action.
    pub(crate) fn action_failed(&mut self, what: &str, error: &Error, cx: &mut Context<'_, Self>) {
        log::warn!("{what}: {error}");
        self.action_problem = Some(format!("{what}: {}", reason(error)));
        cx.notify();
    }

    /// An action went through but didn't take: `line` says why until hidden or the next
    /// action.
    pub(crate) fn show_action_problem(&mut self, line: String, cx: &mut Context<'_, Self>) {
        log::warn!("{line}");
        self.action_problem = Some(line);
        cx.notify();
    }

    /// Hides the refused action's line.
    pub(crate) fn hide_action_problem(&mut self, cx: &mut Context<'_, Self>) {
        if self.action_problem.take().is_some() {
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use emberhut_api::headers::ETag;
    use emberhut_api::ids::RoomId;
    use gpui_kit::{AppContext as _, Entity, TestAppContext};
    use serde_json::json;

    use super::*;
    use crate::backend::fake::FakeBackend;
    use crate::backend::{FreshState, StateReply};
    use crate::fixtures::{message, state};
    use crate::store::ThemeState;

    fn store(cx: &mut TestAppContext) -> Entity<Store> {
        let backend = Arc::new(FakeBackend::new([])) as Arc<dyn crate::backend::Backend>;
        cx.new(|_| Store::idle(backend, None, ThemeState::load(None), RoomId::MAIN))
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "it stands for a poll's reply, which is a Result"
    )]
    fn fresh(extra: serde_json::Value) -> Result<StateReply, Error> {
        Ok(StateReply::Fresh(FreshState {
            state: Box::new(state(extra)),
            etag: Some(ETag::new("\"v1\"")),
        }))
    }

    fn report() -> crate::store::OpenReport {
        crate::store::OpenReport {
            name: "week.html".to_owned(),
            path: "/files/week.html".to_owned(),
        }
    }

    fn not_let_in() -> Result<StateReply, Error> {
        Err(Error::Refused {
            status: Some(401),
            message: Some("unauthorized".into()),
        })
    }

    #[gpui_kit::test]
    fn it_connects_until_a_state_comes(cx: &mut TestAppContext) {
        let store = store(cx);
        store.update(cx, |store, cx| {
            assert_eq!(store.gate(), Gate::Connecting);
            store.apply(RoomId::MAIN, fresh(json!({})), Instant::now(), cx);
            assert_eq!(store.gate(), Gate::Open);
        });
    }

    #[gpui_kit::test]
    fn a_device_the_server_doesnt_know_sees_nothing_held_until_let_in(cx: &mut TestAppContext) {
        let store = store(cx);
        store.update(cx, |store, cx| {
            store.apply(
                RoomId::MAIN,
                fresh(json!({"chat": [message(1)]})),
                Instant::now(),
                cx,
            );
            store.show_report(Some(report()), cx);
            store.apply(RoomId::MAIN, not_let_in(), Instant::now(), cx);
            assert_eq!(store.gate(), Gate::NotYourDevice);
            assert_eq!(store.state(), None, "nothing it held shows");
            assert!(store.open_report().is_none(), "the report closed");
            store.apply(RoomId::MAIN, fresh(json!({})), Instant::now(), cx);
            assert_eq!(store.gate(), Gate::Open, "let in: it opens by itself");
        });
    }

    #[gpui_kit::test]
    fn while_the_server_only_reports_sign_in_is_asked_for_once(cx: &mut TestAppContext) {
        let store = store(cx);
        let reporting = || fresh(json!({"auth": {"mode": "report", "bound": true, "devices": []}}));
        store.update(cx, |store, cx| {
            store.apply(RoomId::MAIN, reporting(), Instant::now(), cx);
            assert_eq!(store.gate(), Gate::SignIn { can_wait: true });
            store.sign_in_later(cx);
            assert_eq!(store.gate(), Gate::Open);
            store.apply(RoomId::MAIN, reporting(), Instant::now(), cx);
            assert_eq!(store.gate(), Gate::Open, "not asked again this launch");
        });
    }

    #[gpui_kit::test]
    fn once_the_server_enforces_sign_in_is_the_only_way_in(cx: &mut TestAppContext) {
        let store = store(cx);
        store.update(cx, |store, cx| {
            store.apply(
                RoomId::MAIN,
                fresh(json!({"auth": {"mode": "enforce", "bound": true, "devices": []}})),
                Instant::now(),
                cx,
            );
            store.apply(
                RoomId::MAIN,
                Ok(StateReply::SignInRequired),
                Instant::now(),
                cx,
            );
            assert_eq!(store.gate(), Gate::SignIn { can_wait: false });
            store.sign_in_later(cx);
            assert_eq!(store.gate(), Gate::SignIn { can_wait: false });
        });
    }

    #[gpui_kit::test]
    fn the_offline_line_shows_after_five_seconds_of_failed_polls(cx: &mut TestAppContext) {
        let store = store(cx);
        let start = Instant::now();
        let down = || Err(Error::Unreachable("refused".into()));
        store.update(cx, |store, cx| {
            store.apply(RoomId::MAIN, fresh(json!({})), start, cx);
            store.apply(RoomId::MAIN, down(), start, cx);
            assert_eq!(store.offline(), None);
            assert_eq!(store.status().text, "Watching", "a blip says nothing");
            store.apply(RoomId::MAIN, down(), start + QUIET, cx);
            assert_eq!(store.offline(), Some("Can't reach the server · retrying"));
            assert_eq!(store.status().text, "Offline");
            store.apply(RoomId::MAIN, Ok(StateReply::Unchanged), start + QUIET, cx);
            assert_eq!(store.offline(), None);
            let failing = || {
                Err(Error::Refused {
                    status: Some(500),
                    message: None,
                })
            };
            store.apply(RoomId::MAIN, failing(), start + QUIET, cx);
            store.apply(RoomId::MAIN, failing(), start + QUIET * 2, cx);
            assert_eq!(store.status().text, "Server error", "reached, but failing");
        });
    }

    #[gpui_kit::test]
    fn a_refused_action_says_what_and_why_until_hidden(cx: &mut TestAppContext) {
        let store = store(cx);
        store.update(cx, |store, cx| {
            let error = Error::Refused {
                status: Some(400),
                message: Some("No such project".into()),
            };
            store.action_failed("Couldn't archive Room 2", &error, cx);
            assert_eq!(
                store.action_problem(),
                Some("Couldn't archive Room 2: No such project")
            );
            store.hide_action_problem(cx);
            assert_eq!(store.action_problem(), None);
        });
    }

    #[test]
    fn the_offline_line_waits_out_a_blip() {
        let start = Instant::now();
        let mut failing = Failing::default();
        assert!(!failing.failed("down".into(), Outage::Unreachable, start));
        assert!(!failing.failed(
            "down".into(),
            Outage::Unreachable,
            start + Duration::from_secs(4)
        ));
        assert_eq!(failing.shown(), None, "four seconds: a blip");
        assert!(failing.failed("down".into(), Outage::Unreachable, start + QUIET));
        assert_eq!(failing.shown(), Some("down"));
        assert!(
            !failing.failed("down".into(), Outage::Unreachable, start + QUIET * 2),
            "the same line"
        );
        assert!(failing.passed(), "it goes at once");
        assert_eq!(failing.shown(), None);
        assert!(
            !failing.failed("down".into(), Outage::Unreachable, start + QUIET * 3),
            "a new blip"
        );
    }

    #[test]
    fn an_actions_reason_is_the_servers_or_says_what_happened() {
        let refused = |status, message: Option<&str>| Error::Refused {
            status,
            message: message.map(str::to_owned),
        };
        assert_eq!(
            reason(&refused(Some(400), Some("No such project"))),
            "No such project"
        );
        assert_eq!(reason(&refused(Some(502), None)), "HTTP 502");
        assert_eq!(
            reason(&Error::Unreachable("refused".into())),
            "can't reach the server"
        );
    }
}
