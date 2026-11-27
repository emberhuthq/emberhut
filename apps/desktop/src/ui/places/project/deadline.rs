//! A project's deadline: its chip (the day, `· hard` when someone else holds the user to it)
//! opens the Deadline editor beside it: Deadline on or off (on starts a week out), the day on
//! a calendar, and Hard deadline. Each change goes to the server at once.

use emberhut_api::ids::ProjectId;
use emberhut_api::tasks::{Project, ProjectHealth};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::calendar::{Calendar, CalendarEvent, CalendarState};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::switch::Switch;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, ElementId, Entity, SharedString, Window, div, px};
use jiff::ZonedDateTime;
use jiff::civil::Date;

use crate::store::projects::{deadline_change, deadline_start};
use crate::store::{Store, lookup};
use crate::ui::palette::{Pal, pal};
use crate::ui::tasks::marks::glyph;

/// The editor's width.
const WIDTH: f32 = 290.;

/// The deadline chip: a click opens the editor under it.
pub(super) fn chip(p: &Pal, store: &Entity<Store>, project: &Project) -> AnyElement {
    let id = project.id;
    let words = project
        .deadline
        .as_ref()
        .and_then(|day| lookup::civil_day(day.as_str()))
        .map_or_else(
            || "No deadline".to_owned(),
            |day| {
                let hard = if project.hard { " · hard" } else { "" };
                format!("{}{hard}", day.strftime("%b %-d"))
            },
        );
    let late = project.health == Some(ProjectHealth::Late);
    let store = store.clone();
    Popover::new(ElementId::Name(format!("project-deadline-{id}").into()))
        .trigger(
            Button::new("project-deadline")
                .label(words)
                .icon(glyph(
                    gpui_kit::assets::IconName::Calendar,
                    if late { p.danger } else { p.muted },
                    12.,
                ))
                .ghost()
                .small()
                .tooltip("Tasks without their own date take this one"),
        )
        .content(move |_, window, cx| {
            let store = store.clone();
            window.use_keyed_state(
                ElementId::Name(format!("deadline-editor-{id}").into()),
                cx,
                move |window, cx| DeadlineEditor::new(store, id, window, cx),
            )
        })
        .into_any_element()
}

/// What the editor shows of the project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Deadline {
    day: Option<Date>,
    hard: bool,
}

/// The Deadline editor of one project.
pub(super) struct DeadlineEditor {
    store: Entity<Store>,
    project: ProjectId,
    calendar: Entity<CalendarState>,
    /// The project's deadline as last shown, so the calendar follows a change made elsewhere.
    shown: Option<Deadline>,
}

impl DeadlineEditor {
    fn new(
        store: Entity<Store>,
        project: ProjectId,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let calendar = cx.new(|cx| CalendarState::new(window, cx));
        cx.subscribe(&calendar, |this, _, event: &CalendarEvent, cx| {
            let CalendarEvent::Selected(day) = event;
            if let Some(day) = day.start() {
                this.set(Some(day), None, cx);
            }
        })
        .detach();
        cx.observe_in(&store, window, |this, _, window, cx| {
            this.follow(window, cx);
        })
        .detach();
        let mut editor = Self {
            store,
            project,
            calendar,
            shown: None,
        };
        editor.follow(window, cx);
        editor
    }

    /// The project's deadline now (none once the project is gone).
    fn deadline(&self, cx: &gpui_kit::App) -> Option<Deadline> {
        let project = self
            .store
            .read(cx)
            .state()?
            .projects
            .iter()
            .find(|project| project.id == self.project)?;
        Some(Deadline {
            day: project
                .deadline
                .as_ref()
                .and_then(|day| lookup::civil_day(day.as_str())),
            hard: project.hard,
        })
    }

    /// Puts the calendar on the project's day when it changed.
    fn follow(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let now = self.deadline(cx);
        if now == self.shown {
            return;
        }
        if let Some(day) = now.and_then(|deadline| deadline.day) {
            self.calendar
                .update(cx, |calendar, cx| calendar.set_date(day, window, cx));
        }
        self.shown = now;
        cx.notify();
    }

    /// Sets the deadline to `day` (none: no deadline) and, if given, whether it is hard.
    fn set(&mut self, day: Option<Date>, hard: Option<bool>, cx: &mut Context<'_, Self>) {
        let was = self.shown.is_some_and(|deadline| deadline.hard);
        let change = deadline_change(self.project, day, hard.unwrap_or(was));
        self.store
            .update(cx, |store, cx| store.change_project(change, cx));
    }
}

impl Render for DeadlineEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        let shown = self.shown.unwrap_or(Deadline {
            day: None,
            hard: false,
        });
        let mut editor = div()
            .id("deadline-editor")
            .w(px(WIDTH))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(line(
                &p,
                "Deadline",
                Switch::new("project-deadline-on")
                    .checked(shown.day.is_some())
                    .on_click(cx.listener(|this, on: &bool, _, cx| {
                        let day = on.then(|| deadline_start(ZonedDateTime::now().date()));
                        this.set(day, None, cx);
                    })),
            ));
        if let Some(day) = shown.day {
            editor = editor
                .child(Calendar::new(&self.calendar).small())
                .child(line(
                    &p,
                    "Hard deadline",
                    Switch::new("project-deadline-hard")
                        .checked(shown.hard)
                        .on_click(cx.listener(move |this, on: &bool, _, cx| {
                            this.set(Some(day), Some(*on), cx);
                        })),
                ))
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(p.muted)
                        .child("Someone else holds you to it: a client, an exam."),
                );
        }
        editor
    }
}

/// A labelled row: its name, then its control at the end.
fn line(p: &Pal, label: impl Into<SharedString>, control: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .text_size(px(13.))
        .child(div().flex_1().text_color(p.text).child(label.into()))
        .child(control)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use emberhut_api::patch::Patch;
    use emberhut_api::settings::{ProjectChange, SettingsAction};
    use emberhut_api::time::Day;
    use gpui_kit::TestAppContext;
    use gpui_kit::test::TestWindowExt as _;
    use serde_json::json;

    use super::*;
    use crate::backend::fake::{Call, FakeBackend};

    #[gpui_kit::test]
    fn the_editor_turns_a_deadline_hard_and_off(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let backend = Arc::new(FakeBackend::new([]));
        let shared = Arc::clone(&backend) as Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::for_tests(shared));
        let state = crate::fixtures::state(json!({"projects": [{"id": 2, "name": "Example",
            "goal": null, "priority": 1, "kind": "deliverable", "deadline": "2026-12-03",
            "hard": false, "status": "active", "pausedUntil": null, "open": 2, "doneWeek": 1,
            "health": null, "next": null, "hoursLeft": 0.0}]}));
        store.update(cx, |store, cx| store.show_state_for_tests(state, cx));
        let project = ProjectId::new(2);
        let window =
            cx.add_window(|window, cx| DeadlineEditor::new(store.clone(), project, window, cx));
        let click = |cx: &mut TestAppContext, id: &'static str| {
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                window.click(id, cx);
            })
            .unwrap();
            cx.run_until_parked();
        };
        click(cx, "project-deadline-hard");
        click(cx, "project-deadline-on");
        let change = |deadline, hard| {
            Call::Settings(SettingsAction::ProjectSet(ProjectChange {
                deadline,
                hard: Some(hard),
                ..crate::store::projects::change_of(project)
            }))
        };
        let calls = backend.calls();
        assert!(
            calls.contains(&change(Patch::Set(Day::new("2026-12-03")), true)),
            "{calls:?}"
        );
        assert!(calls.contains(&change(Patch::Clear, false)), "{calls:?}");
    }
}
