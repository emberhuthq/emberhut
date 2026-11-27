//! A project page's head: its health, a bar of how much is done, its properties as chips
//! (priority, goal with its ring, deadline, More: talk about it, pause until a day, resume,
//! archive), and its filters (area, status, owner) with List / Board.

use emberhut_api::tasks::{ProjectHealth, ProjectKind};
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, Entity, FontWeight, Role, div, px, relative};

use crate::brand::APP_NAME;
use crate::store::goals::{label, progress};
use crate::store::needs_view::Layout;
use crate::store::projects::{
    PRIORITIES, ProjectFilter, ProjectPage, Stage, change_of, priority_name,
};
use crate::store::room_actions::{Pausing, RoomAction};
use crate::store::tasks::filter::OwnerPick;
use crate::store::{Page, Store};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::column;
use crate::ui::needs::segmented;
use crate::ui::palette::{Pal, room_color};
use crate::ui::places::ring::ring;
use crate::ui::tasks::marks::glyph;

/// Its health, how far it is, and its chips.
pub(super) fn top(p: &Pal, store: &Entity<Store>, shown: &ProjectPage<'_>) -> Div {
    let project = shown.project;
    let (done, total) = shown.progress();
    let share = if total == 0 {
        0.
    } else {
        f32::from(u16::try_from(done).unwrap_or(u16::MAX))
            / f32::from(u16::try_from(total).unwrap_or(u16::MAX))
    };
    let mut line = div().flex().items_center().gap(px(10.)).pt(px(8.));
    let health_word = match &project.health {
        Some(ProjectHealth::Late) => "Late",
        Some(ProjectHealth::AtRisk) => "At risk",
        Some(_) => "On track",
        None => "",
    };
    let spoken = a11y::line(&[health_word, &format!("{done} of {total} done")]);
    if let Some(health) = &project.health {
        let (word, color) = match health {
            ProjectHealth::Late => ("Late", p.danger),
            ProjectHealth::AtRisk => ("At risk", p.warn),
            _ => ("On track", p.success),
        };
        line = line.child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(color)
                .child(crate::ui::widgets::dot(color, 7.))
                .child(word),
        );
    }
    line = line
        .child(
            div()
                .w(px(160.))
                .h(px(5.))
                .rounded_full()
                .bg(p.faint.opacity(0.2))
                .child(
                    div()
                        .h_full()
                        .w(relative(share))
                        .rounded_full()
                        .bg(p.success),
                ),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(p.muted)
                .child(format!("{done} of {total} done")),
        );
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(
            line.id("project-progress")
                .read_as(Role::ProgressIndicator, spoken),
        )
        .child(chips(p, store, shown))
}

/// A property as a small, muted capsule.
fn chip(p: &Pal) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(5.))
        .px(px(8.))
        .py(px(4.))
        .rounded_full()
        .bg(p.hover)
        .text_size(px(12.))
        .text_color(p.muted)
}

/// Priority, goal, deadline and More.
fn chips(p: &Pal, store: &Entity<Store>, shown: &ProjectPage<'_>) -> Div {
    let project = shown.project;
    let id = project.id;
    let priority_color = match project.priority {
        1 => p.danger,
        2 => p.warn,
        _ => p.muted,
    };
    let picker = store.clone();
    let mut row = div().flex().flex_wrap().items_center().gap(px(6.)).child(
        Button::new("project-priority")
            .label(priority_name(project.priority))
            .icon(glyph(IconName::ChartBar, priority_color, 12.))
            .ghost()
            .small()
            .dropdown_menu(move |mut menu, _, _| {
                for (value, name) in PRIORITIES {
                    let store = picker.clone();
                    menu = menu.item(PopupMenuItem::new(name).on_click(move |_, _, cx| {
                        let change = emberhut_api::settings::ProjectChange {
                            priority: Some(value),
                            ..change_of(id)
                        };
                        store.update(cx, |store, cx| store.change_project(change, cx));
                    }));
                }
                menu
            }),
    );
    if let Some(goal) = project
        .goal
        .and_then(|goal| shown.state.goals.iter().find(|g| g.id == goal))
    {
        let ring_of = progress(goal.id, &shown.state.todos);
        let gid = goal.id;
        row = row.child(
            chip(p)
                .id("project-goal")
                .button_named(format!("Goal: {}, {}", label(goal), ring_of.words()))
                .cursor_pointer()
                .child(ring(
                    "project-goal-ring",
                    11.,
                    2.,
                    ring_of.share(),
                    room_color(Some(goal.rank), p),
                    p.faint.opacity(0.3),
                ))
                .child(label(goal))
                .on_click(move |_, window, cx| column::push(Page::Goal(gid), window, cx)),
        );
    }
    if project.kind != ProjectKind::Ongoing {
        row = row.child(super::deadline::chip(p, store, project));
    }
    row.child(more(p, store, shown))
}

/// More: talk about it, and its room's actions (pause until a day, resume, archive).
fn more(p: &Pal, store: &Entity<Store>, shown: &ProjectPage<'_>) -> AnyElement {
    let project = shown.project;
    let (id, name, status) = (project.id, project.name.clone(), project.status.clone());
    let room = shown
        .state
        .rooms
        .iter()
        .find(|room| room.project == Some(id))
        .cloned();
    let store = store.clone();
    Button::new("project-more")
        .icon(glyph(IconName::Ellipsis, p.muted, 13.))
        .ghost()
        .small()
        .tooltip("More")
        .accessibility_label("More")
        .dropdown_menu(move |mut menu, _, _| {
            let talk = store.clone();
            let (room_id, about) = (room.as_ref().map(|room| room.id), name.clone());
            menu = menu.item(
                PopupMenuItem::new(format!("Talk to {APP_NAME} about {name}")).on_click(
                    move |_, _, cx| {
                        let about = about.clone();
                        talk.update(cx, |store, cx| {
                            // In the project's room, its composer ready; else about it, here.
                            match room_id {
                                Some(room) => {
                                    store.switch_room(room, cx);
                                    store.start_message("", cx);
                                }
                                None => {
                                    store.start_message(format!("About the {about} project: "), cx);
                                }
                            }
                        });
                    },
                ),
            );
            let Some(pausing) = room.as_ref().and_then(Pausing::of) else {
                return menu;
            };
            menu = menu.separator();
            for &item in RoomAction::offered(&status) {
                let (store, name, pausing) = (store.clone(), name.clone(), pausing.clone());
                menu = menu.item(PopupMenuItem::new(item.label()).on_click(move |_, _, cx| {
                    match item.action() {
                        Some(action) => store.update(cx, |store, cx| {
                            store.act_on_room(id, &name, action, cx);
                        }),
                        None => store.update(cx, |store, cx| {
                            store.set_pausing(Some(pausing.clone()), cx);
                        }),
                    }
                }));
            }
            menu
        })
        .into_any_element()
}

/// The filters (area, status, owner) and List / Board.
pub(super) fn filter_bar(
    p: &Pal,
    store: &Entity<Store>,
    shown: &ProjectPage<'_>,
    filter: ProjectFilter,
    layout: Layout,
) -> Div {
    let mut bar = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(6.))
        .pt(px(12.));
    let facet = |id, on, off, picks, cleared| {
        facet_chip(
            p,
            store,
            Facet {
                id,
                on,
                off,
                picks,
                cleared,
            },
        )
    };
    if let Some(area) = area_facet(shown, filter) {
        bar = bar.child(facet_chip(p, store, area));
    }
    let stages = [
        Stage::NeedsYou,
        Stage::Working,
        Stage::NotStarted,
        Stage::Done,
    ]
    .into_iter()
    .map(|stage| {
        (
            stage.name().to_owned(),
            ProjectFilter {
                stage: Some(stage),
                ..filter
            },
        )
    })
    .collect();
    bar = bar.child(facet(
        "project-status",
        filter.stage.map(|stage| stage.name().to_owned()),
        "Status",
        stages,
        ProjectFilter {
            stage: None,
            ..filter
        },
    ));
    let owners = OwnerPick::ALL
        .into_iter()
        .map(|owner| {
            (
                owner.name(),
                ProjectFilter {
                    owner: Some(owner),
                    ..filter
                },
            )
        })
        .collect();
    bar = bar.child(facet(
        "project-owner",
        filter.owner.map(OwnerPick::name),
        "Owner",
        owners,
        ProjectFilter {
            owner: None,
            ..filter
        },
    ));
    let layouts = store.clone();
    bar.child(div().flex_1()).child(segmented(
        p,
        "project-layout",
        &[(Layout::List, "List"), (Layout::Board, "Board")],
        layout,
        move |layout, cx| layouts.update(cx, |store, cx| store.set_project_layout(layout, cx)),
    ))
}

/// One filter of a project's page: on, a filled chip with its value and an ✕; off, its name
/// opening its choices.
struct Facet {
    id: &'static str,
    on: Option<String>,
    off: &'static str,
    picks: Vec<(String, ProjectFilter)>,
    cleared: ProjectFilter,
}

/// `facet`'s chip.
fn facet_chip(p: &Pal, store: &Entity<Store>, facet: Facet) -> AnyElement {
    let Facet {
        id,
        on,
        off,
        picks,
        cleared,
    } = facet;
    let store = store.clone();
    match on {
        Some(value) => div()
            .id(id)
            .button_named(format!("{off}: {value}. Remove filter"))
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(10.))
            .py(px(4.))
            .rounded_full()
            .bg(p.accent)
            .text_color(p.on_accent)
            .text_size(px(12.))
            .cursor_pointer()
            .child(value)
            .child(glyph(IconName::X, p.on_accent, 11.))
            .on_click(move |_, _, cx| {
                store.update(cx, |store, cx| store.set_project_filter(cleared, cx));
            })
            .into_any_element(),
        None => Button::new(id)
            .label(off)
            .small()
            .ghost()
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                for (name, next) in &picks {
                    let (store, next) = (store.clone(), *next);
                    menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, _, cx| {
                        store.update(cx, |store, cx| store.set_project_filter(next, cx));
                    }));
                }
                menu
            })
            .into_any_element(),
    }
}

/// The Area filter, when the project has areas to pick (not the done ones).
fn area_facet(shown: &ProjectPage<'_>, filter: ProjectFilter) -> Option<Facet> {
    let areas: Vec<(emberhut_api::ids::AreaId, String)> = shown
        .areas()
        .filter(|area| area.status != emberhut_api::areas::AreaStatus::Done)
        .map(|area| (area.id, area.name.clone()))
        .collect();
    if areas.is_empty() {
        return None;
    }
    let on = filter.area.and_then(|id| {
        areas
            .iter()
            .find(|(area, _)| *area == id)
            .map(|(_, name)| name.clone())
    });
    let picks = areas
        .into_iter()
        .map(|(id, name)| {
            (
                name,
                ProjectFilter {
                    area: Some(id),
                    ..filter
                },
            )
        })
        .collect();
    Some(Facet {
        id: "project-area",
        on,
        off: "Area",
        picks,
        cleared: ProjectFilter {
            area: None,
            ..filter
        },
    })
}
