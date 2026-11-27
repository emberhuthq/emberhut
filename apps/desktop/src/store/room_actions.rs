//! What the user does to a project's room: pause it until a day, resume it, archive it, or
//! unarchive it (`POST /api/settings` with `projectSet`). The room moves in the switcher with
//! the next state, fetched at once; a refusal shows with the server's reason.

use std::sync::Arc;

use emberhut_api::ids::ProjectId;
use emberhut_api::projects::ProjectSetRequest;
use emberhut_api::rooms::RoomRow;
use emberhut_api::tasks::ProjectStatus;
use emberhut_api::time::Day;
use gpui_kit::AppContext as _;
use gpui_kit::Context;
use jiff::civil::Date;

use super::{Store, update_or_log};

/// A change to a room's project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RoomAction {
    /// Paused until the day given: it folds under Paused and comes back by itself then.
    PauseUntil(Date),
    /// Active again, from paused.
    Resume,
    /// Out of the switcher.
    Archive,
    /// Back in the switcher, from archived.
    Unarchive,
}

impl RoomAction {
    /// What the room's menu offers for a project in `status`: an archived one can only come
    /// back; a paused one can resume, or be paused until another day; any other can be paused
    /// or archived.
    #[must_use]
    pub(crate) fn offered(status: &ProjectStatus) -> &'static [MenuItem] {
        match status {
            ProjectStatus::Archived => &[MenuItem::Unarchive],
            ProjectStatus::Paused => &[
                MenuItem::Resume,
                MenuItem::PauseAnotherDay,
                MenuItem::Archive,
            ],
            _ => &[MenuItem::PauseUntil, MenuItem::Archive],
        }
    }

    /// The request for `project`.
    #[must_use]
    pub(crate) fn request(self, project: ProjectId) -> ProjectSetRequest {
        match self {
            Self::PauseUntil(day) => ProjectSetRequest::pause_until(
                project,
                Day::new(day.strftime("%Y-%m-%d").to_string()),
            ),
            Self::Resume | Self::Unarchive => ProjectSetRequest::activate(project),
            Self::Archive => ProjectSetRequest::archive(project),
        }
    }

    /// What failed, worded for the user, when the server refuses it for the room `name`.
    #[must_use]
    pub(crate) fn failure(self, name: &str) -> String {
        match self {
            Self::PauseUntil(_) => format!("Couldn't pause {name}"),
            Self::Resume => format!("Couldn't resume {name}"),
            Self::Archive => format!("Couldn't archive {name}"),
            Self::Unarchive => format!("Couldn't unarchive {name}"),
        }
    }
}

/// An item of a project room's menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MenuItem {
    /// "Pause until…": opens the day picker.
    PauseUntil,
    /// "Pause until another day…" on a paused room.
    PauseAnotherDay,
    /// "Resume now".
    Resume,
    /// "Archive".
    Archive,
    /// "Unarchive".
    Unarchive,
}

impl MenuItem {
    /// Its words.
    #[must_use]
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::PauseUntil => "Pause until…",
            Self::PauseAnotherDay => "Pause until another day…",
            Self::Resume => "Resume now",
            Self::Archive => "Archive",
            Self::Unarchive => "Unarchive",
        }
    }

    /// The action it takes at once; `None` for the ones that ask for a day first.
    #[must_use]
    pub(crate) fn action(self) -> Option<RoomAction> {
        match self {
            Self::PauseUntil | Self::PauseAnotherDay => None,
            Self::Resume => Some(RoomAction::Resume),
            Self::Archive => Some(RoomAction::Archive),
            Self::Unarchive => Some(RoomAction::Unarchive),
        }
    }
}

/// The room a pause is being picked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pausing {
    /// Its project.
    pub(crate) project: ProjectId,
    /// Its name, for the sheet's title.
    pub(crate) name: String,
}

impl Pausing {
    /// The pause for `room`; none for Main, which has no project.
    #[must_use]
    pub(crate) fn of(room: &RoomRow) -> Option<Self> {
        Some(Self {
            project: room.project?,
            name: room.name.clone(),
        })
    }
}

/// The days a pause can run to, from `today`: from tomorrow on, a week out to start with.
#[must_use]
pub(crate) fn pause_days(today: Date) -> (Date, Date) {
    let tomorrow = today.tomorrow().unwrap_or(today);
    let week = today
        .checked_add(jiff::Span::new().days(7))
        .unwrap_or(tomorrow);
    (tomorrow, week)
}

impl Store {
    /// Takes `action` on the project `project` of the room named `name`.
    pub(crate) fn act_on_room(
        &mut self,
        project: ProjectId,
        name: &str,
        action: RoomAction,
        cx: &mut Context<'_, Self>,
    ) {
        let request = action.request(project);
        let failure = action.failure(name);
        let (backend, kicker) = (Arc::clone(&self.backend), self.kicker.clone());
        self.hide_action_problem(cx);
        cx.spawn(async move |this, cx| {
            let done = cx
                .background_spawn(async move { backend.set_project(&request) })
                .await;
            kicker.kick();
            if let Err(error) = done {
                update_or_log(&this, cx, |store, cx| {
                    store.action_failed(&failure, &error, cx);
                });
            }
        })
        .detach();
    }

    /// The room whose pause day is being picked, if any.
    #[must_use]
    pub(crate) fn pausing(&self) -> Option<&Pausing> {
        self.pausing.as_ref()
    }

    /// Opens the day picker for a room's pause (`None` closes it).
    pub(crate) fn set_pausing(&mut self, pausing: Option<Pausing>, cx: &mut Context<'_, Self>) {
        self.pausing = pausing;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use emberhut_api::ids::RoomId;
    use gpui_kit::{Entity, TestAppContext};
    use jiff::civil::date;
    use serde_json::json;

    use super::*;
    use crate::backend::Error;
    use crate::backend::fake::{Call, FakeBackend};
    use crate::fixtures::room;
    use crate::store::ThemeState;

    fn store(cx: &mut TestAppContext, backend: &Arc<FakeBackend>) -> Entity<Store> {
        let backend = Arc::clone(backend) as Arc<dyn crate::backend::Backend>;
        cx.new(|_| Store::idle(backend, None, ThemeState::load(None), RoomId::MAIN))
    }

    #[test]
    fn each_status_offers_its_own_way_on() {
        let labels = |status: &str| {
            RoomAction::offered(&ProjectStatus::from(status))
                .iter()
                .map(|item| item.label())
                .collect::<Vec<_>>()
        };
        assert_eq!(labels("active"), ["Pause until…", "Archive"]);
        assert_eq!(
            labels("paused"),
            ["Resume now", "Pause until another day…", "Archive"]
        );
        assert_eq!(labels("archived"), ["Unarchive"]);
        assert_eq!(
            MenuItem::PauseUntil.action(),
            None,
            "it asks for a day first"
        );
    }

    #[test]
    fn each_action_is_its_projects_change() {
        let project = ProjectId::new(4);
        assert_eq!(
            RoomAction::PauseUntil(date(2026, 12, 1)).request(project),
            ProjectSetRequest::pause_until(project, Day::new("2026-12-01"))
        );
        assert_eq!(
            RoomAction::Resume.request(project),
            ProjectSetRequest::activate(project)
        );
        assert_eq!(
            RoomAction::Unarchive.request(project),
            ProjectSetRequest::activate(project)
        );
        assert_eq!(
            RoomAction::Archive.request(project),
            ProjectSetRequest::archive(project)
        );
    }

    #[test]
    fn a_pause_runs_from_tomorrow_a_week_out_to_start() {
        assert_eq!(
            pause_days(date(2026, 12, 30)),
            (date(2026, 12, 31), date(2027, 1, 6))
        );
    }

    #[test]
    fn main_cant_be_paused() {
        assert_eq!(Pausing::of(&room(1, json!({}))), None);
        assert_eq!(
            Pausing::of(&room(3, json!({}))),
            Some(Pausing {
                project: ProjectId::new(3),
                name: "Room 3".into()
            })
        );
    }

    #[gpui_kit::test]
    fn an_action_goes_to_the_server_and_a_refusal_says_why(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        backend.push_action(Ok(()));
        backend.push_action(Err(Error::Refused {
            status: Some(400),
            message: Some("No such project".into()),
        }));
        let store = store(cx, &backend);
        store.update(cx, |store, cx| {
            store.act_on_room(ProjectId::new(2), "Room 2", RoomAction::Archive, cx);
        });
        cx.run_until_parked();
        store.read_with(cx, |store, _| assert_eq!(store.action_problem(), None));
        store.update(cx, |store, cx| {
            store.act_on_room(ProjectId::new(9), "Room 9", RoomAction::Resume, cx);
        });
        cx.run_until_parked();
        assert_eq!(
            backend
                .calls()
                .into_iter()
                .filter(|call| matches!(call, Call::SetProject(_)))
                .collect::<Vec<_>>(),
            [
                Call::SetProject(ProjectSetRequest::archive(ProjectId::new(2))),
                Call::SetProject(ProjectSetRequest::activate(ProjectId::new(9))),
            ]
        );
        store.read_with(cx, |store, _| {
            assert_eq!(
                store.action_problem(),
                Some("Couldn't resume Room 9: No such project")
            );
        });
    }
}
