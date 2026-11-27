//! The environments: every place the assistant works and connection it reads through, asked
//! for every 10 s while Settings or one of their pages shows in the front window, and at once
//! when one opens or the window comes back to the front. A row's action goes one at a time:
//! the row shows a spinner, every action waits, and the page comes back as it is after, with
//! what the action made (a re-pair link, a one-time sign-in link, a note) kept for that row's
//! page. Unlink asks first.

use std::sync::Arc;
use std::time::Duration;

use emberhut_api::chrome::ChromePairing;
use emberhut_api::environments::{EnvActionKind, EnvActionRequest, EnvPage, EnvState, Environment};
use emberhut_api::ids::EnvironmentId;
use gpui_kit::AppContext as _;
use gpui_kit::Context;

use super::keys::{Key, Keys};
use crate::backend::Error;
use crate::brand::APP_NAME;
use crate::store::poll::{self, Kicked, Kicker};
use crate::store::presence::Presence;
use crate::store::settings::SettingsPane;
use crate::store::settings::chrome::Health;
use crate::store::{Page, Store, update_or_log};

/// How often the environments are asked for while they show.
pub(super) const EVERY: Duration = Duration::from_secs(10);

/// An environment's page key.
pub(crate) type EnvKey = Key<EnvironmentId>;

/// What the last action on a row made, for its page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Outcome {
    /// A Chrome profile's re-pair link.
    pub(crate) pairing: Option<ChromePairing>,
    /// A one-time page to open.
    pub(crate) link: Option<String>,
    /// What it did, in a line.
    pub(crate) note: Option<String>,
}

/// The environments, as last fetched.
#[derive(Debug)]
pub(crate) struct EnvironmentsFeed {
    page: Option<EnvPage>,
    problem: Option<String>,
    /// The row whose action is on its way.
    busy: Option<EnvironmentId>,
    /// What the last action made, and on which row.
    outcome: Option<(EnvironmentId, Outcome)>,
    /// The row whose Unlink waits for the user's yes.
    unlinking: Option<EnvironmentId>,
    keys: Keys<EnvironmentId>,
    kicker: Kicker,
    kicked: Option<Kicked>,
}

impl Default for EnvironmentsFeed {
    fn default() -> Self {
        let (kicker, waiting) = poll::kicker();
        Self {
            page: None,
            problem: None,
            busy: None,
            outcome: None,
            unlinking: None,
            keys: Keys::default(),
            kicker,
            kicked: Some(waiting),
        }
    }
}

impl EnvironmentsFeed {
    /// The last page, if one came.
    #[must_use]
    pub(crate) fn page(&self) -> Option<&EnvPage> {
        self.page.as_ref()
    }

    /// Why the last fetch or action failed, worded for the user.
    #[must_use]
    pub(crate) fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// The row whose action is on its way, while one is.
    #[must_use]
    pub(crate) fn busy(&self) -> Option<&EnvironmentId> {
        self.busy.as_ref()
    }

    /// What the last action on `id` made.
    #[must_use]
    pub(crate) fn outcome(&self, id: &EnvironmentId) -> Option<&Outcome> {
        self.outcome
            .as_ref()
            .filter(|(on, _)| on == id)
            .map(|(_, outcome)| outcome)
    }

    /// Whether `id`'s Unlink waits for the user's yes.
    #[must_use]
    pub(crate) fn is_unlinking(&self, id: &EnvironmentId) -> bool {
        self.unlinking.as_ref() == Some(id)
    }

    /// The row of `key`'s page.
    #[must_use]
    pub(crate) fn id_of(&self, key: EnvKey) -> Option<&EnvironmentId> {
        self.keys.get(key)
    }

    /// Asks for the environments now if they are wanted, rather than at the next tick.
    pub(super) fn kick(&self) {
        self.kicker.kick();
    }

    /// Takes a fetch's outcome in; whether anything changed. A failure keeps the last page.
    fn take(&mut self, page: Result<EnvPage, Error>) -> bool {
        let (page, problem) = match page {
            Ok(page) => (Some(page), None),
            Err(error) => (self.page.take(), Some(read_problem(&error))),
        };
        let changed = self.page != page || self.problem != problem;
        (self.page, self.problem) = (page, problem);
        changed
    }

    /// Takes an action's outcome in: the page as it is after with what it made, or why it
    /// wasn't done.
    fn take_done(&mut self, id: EnvironmentId, kind: &EnvActionKind, page: Result<EnvPage, Error>) {
        self.busy = None;
        match page {
            Ok(mut page) => {
                let note = page.note.take().or_else(|| {
                    (*kind == EnvActionKind::Check).then(|| "Checked just now.".to_owned())
                });
                let outcome = Outcome {
                    pairing: page.pairing.take(),
                    link: page.link.take(),
                    note,
                };
                (self.page, self.problem) = (Some(page), None);
                self.outcome = Some((id, outcome));
            }
            Err(error) => self.problem = Some(action_problem(&error)),
        }
    }
}

/// Why the environments couldn't be read, as the reference client words it: the server's own
/// words when it gave them, else its status.
fn read_problem(error: &Error) -> String {
    match error {
        Error::Unreachable(_) => format!("Can't reach {APP_NAME}"),
        Error::Refused {
            message: Some(message),
            ..
        } => message.clone(),
        Error::Refused {
            status: Some(404), ..
        } => format!("{APP_NAME}'s server is on older code: this comes with its next update"),
        Error::Refused {
            status: Some(status),
            ..
        } => format!("{APP_NAME} said {status}"),
        _ => format!("Couldn't read where {APP_NAME} works"),
    }
}

/// Why an action wasn't done: the server's words, else what went wrong.
fn action_problem(error: &Error) -> String {
    match error {
        Error::Unreachable(_) => format!("Can't reach {APP_NAME}"),
        other => other
            .server_message()
            .map_or_else(|| format!("{APP_NAME} couldn't do that"), str::to_owned),
    }
}

/// How a row is doing, as Settings' rows and filter tell it.
#[must_use]
pub(crate) fn health(state: &EnvState) -> Health {
    match state {
        EnvState::Ok => Health::Fine,
        EnvState::Warn => Health::Warn,
        EnvState::Off => Health::Off,
        // Down, and a state this app doesn't know yet.
        _ => Health::Down,
    }
}

/// The actions a row's page offers: the phone's own Sync is left out on a desktop.
pub(crate) fn actions(
    env: &Environment,
) -> impl Iterator<Item = &emberhut_api::environments::EnvAction> {
    env.actions
        .iter()
        .filter(|action| action.id != EnvActionKind::HealthSync)
}

/// The footer under a row's actions.
#[must_use]
pub(crate) fn actions_footer(env: &Environment) -> String {
    if env
        .actions
        .iter()
        .any(|action| action.id == EnvActionKind::Unlink)
    {
        format!("Unlink deletes only its sign-in: what {APP_NAME} already read stays.")
    } else {
        "Nothing here stops, unpairs or deletes anything.".to_owned()
    }
}

/// Whether to ask for the environments now: while Settings' front door, Connections, Advanced
/// or one of their pages shows, in the front window.
#[must_use]
fn wanted(on_page: bool, presence: Presence) -> bool {
    on_page && presence == Presence::Front
}

impl Store {
    /// Whether the environments are [`wanted`] now.
    fn environments_wanted(&self) -> bool {
        let panes = [SettingsPane::Connections, SettingsPane::Advanced];
        let on_page = self.shows_settings(&panes)
            || self
                .columns
                .shows_any(|page| matches!(page, Page::Environment(_)));
        wanted(on_page, self.presence())
    }

    /// Starts the environments' loop (once).
    pub(super) fn start_environments(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(kicked) = self.working.envs.kicked.take() {
            super::follow::follow(
                super::follow::Feed {
                    every: EVERY,
                    wanted: Self::environments_wanted,
                    fetch: |backend| backend.environments(),
                    take: |store, page| store.working.envs.take(page),
                },
                kicked,
                cx,
            );
        }
    }

    /// The environments.
    #[must_use]
    pub(crate) fn environments(&self) -> &EnvironmentsFeed {
        &self.working.envs
    }

    /// The page key of row `id`, to open its page in a column.
    pub(crate) fn environment_key(&mut self, id: EnvironmentId) -> EnvKey {
        self.working.envs.keys.key(id)
    }

    /// Asks for the user's yes to unlink `id` (`None`: they said no).
    pub(crate) fn ask_unlink(&mut self, id: Option<EnvironmentId>, cx: &mut Context<'_, Self>) {
        self.working.envs.unlinking = id;
        cx.notify();
    }

    /// Runs `kind` on row `id` (Unlink only once the user said yes); one at a time.
    pub(crate) fn act_on_environment(
        &mut self,
        id: EnvironmentId,
        kind: EnvActionKind,
        cx: &mut Context<'_, Self>,
    ) {
        let feed = &mut self.working.envs;
        if feed.busy.is_some() {
            return;
        }
        feed.unlinking = None;
        feed.outcome = None;
        feed.busy = Some(id.clone());
        let backend = Arc::clone(&self.backend);
        let request = EnvActionRequest {
            id: id.clone(),
            kind: kind.clone(),
        };
        cx.spawn(async move |this, cx| {
            let page = cx
                .background_spawn(async move { backend.environment_action(&request) })
                .await;
            update_or_log(&this, cx, |store, cx| {
                store.working.envs.take_done(id, &kind, page);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn page(state: &str, note: Option<&str>) -> EnvPage {
        serde_json::from_value(json!({"at": "2026-01-01T00:00:00Z",
            "summary": {"ok": 1, "warn": 0, "down": 0},
            "groups": [{"id": "runners", "title": "Runners", "footer": null, "items": [
                {"id": "runner", "group": "runners", "name": "Runner", "icon": "server.rack",
                 "tint": "teal", "state": state, "status": "Online", "detail": "", "at": null,
                 "error": null, "hint": null, "facts": [], "lists": [],
                 "actions": [{"id": "check", "label": "Check now"},
                             {"id": "health-sync", "label": "Sync now"}],
                 "lastError": null}]}],
            "note": note}))
        .unwrap()
    }

    #[test]
    fn the_environments_are_asked_for_while_they_show_in_front() {
        assert!(wanted(true, Presence::Front));
        assert!(!wanted(false, Presence::Front));
        assert!(!wanted(true, Presence::Behind));
    }

    #[test]
    fn a_failed_read_says_why_and_keeps_the_page() {
        let mut feed = EnvironmentsFeed::default();
        assert!(feed.take(Ok(page("ok", None))));
        assert!(!feed.take(Ok(page("ok", None))), "the same page");
        assert!(feed.take(Err(Error::Refused {
            status: Some(500),
            message: None
        })));
        assert_eq!(feed.problem(), Some("Emberhut said 500"));
        assert!(feed.page().is_some(), "the last page stays");
        assert_eq!(
            read_problem(&Error::Unreachable("down".into())),
            "Can't reach Emberhut"
        );
    }

    #[test]
    fn a_check_says_so_and_an_action_keeps_what_it_made() {
        let mut feed = EnvironmentsFeed::default();
        let runner = EnvironmentId::new("runner");
        feed.busy = Some(runner.clone());
        feed.take_done(runner.clone(), &EnvActionKind::Check, Ok(page("ok", None)));
        assert_eq!(feed.busy(), None);
        assert_eq!(
            feed.outcome(&runner).and_then(|o| o.note.as_deref()),
            Some("Checked just now.")
        );
        assert_eq!(feed.outcome(&EnvironmentId::new("browser")), None);
        feed.take_done(
            runner.clone(),
            &EnvActionKind::Start,
            Ok(page("ok", Some("It is up."))),
        );
        assert_eq!(
            feed.outcome(&runner).and_then(|o| o.note.as_deref()),
            Some("It is up.")
        );
        assert_eq!(feed.page().and_then(|p| p.note.as_deref()), None);
        feed.take_done(
            runner,
            &EnvActionKind::Start,
            Err(Error::Refused {
                status: Some(400),
                message: Some("couldn't start it".into()),
            }),
        );
        assert_eq!(feed.problem(), Some("couldn't start it"));
    }

    #[test]
    fn a_desktop_leaves_the_phones_sync_out() {
        let page = page("warn", None);
        let runner = page.row(&EnvironmentId::new("runner")).unwrap();
        assert_eq!(actions(runner).count(), 1);
        assert_eq!(health(&runner.state), Health::Warn);
        assert!(actions_footer(runner).starts_with("Nothing here"));
    }
}
