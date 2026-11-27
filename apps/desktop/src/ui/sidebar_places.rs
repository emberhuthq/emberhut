//! The sidebar's Projects and Goals sections, and More's Weekly report row. Projects: each
//! active project with its goal's colour, a dot when it is at risk or late, its open count and
//! a hover line; its active areas under it (each opening the project on that area). Goals: each
//! with a small ring and `done/total`. Each section folds under its header (the fold kept across
//! launches in `window.json`, as Recent sessions' is), and hides while the server hides
//! projects or goals.

use emberhut_api::areas::AreaStatus;
use emberhut_api::tasks::{ProjectHealth, ProjectStatus};
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, ElementId, Entity, FontWeight, Role, Stateful, div, px};

use super::a11y;
use super::column;
use super::controls::{tab_stop, tip};
use super::palette::{Pal, room_color};
use super::places::ring::ring;
use super::tasks::marks::glyph;
use super::widgets::dot;
use super::window::MainWindow;
use crate::brand::APP_NAME;
use crate::prefs::{Kept, Sidebar};
use crate::prose::direction;
use crate::store::goals::{label, progress};
use crate::store::projects::{ProjectFilter, priority_name};
use crate::store::{Page, Store};

/// A sidebar row, lit when `selected`.
fn row(id: impl Into<ElementId>, selected: bool, p: &Pal) -> Stateful<gpui_kit::Div> {
    let row = div()
        .id(id)
        // A screen reader hears a row as a button, named by its caller.
        .role(Role::Button)
        .aria_selected(selected)
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(28.))
        .px(px(10.))
        .mx(px(8.))
        .rounded(px(6.))
        .border_1()
        .text_size(px(13.))
        .when(selected, |d| d.bg(p.accent_soft))
        .when(!selected, |d| d.hover(|d| d.bg(p.hover)))
        .cursor_pointer();
    tab_stop(row, p)
}

/// A folding section of the sidebar: its fold's key in the store, and where its fold is kept.
#[derive(Clone, Copy)]
struct Section {
    /// The fold's key ([`Store::flip_place_fold`]), also the header's element id.
    key: &'static str,
    /// Its header's title.
    title: &'static str,
    /// Keeps whether it is unfolded in the sidebar's prefs.
    keep: fn(&mut Sidebar, bool),
    /// Whether the prefs have it unfolded.
    kept: fn(&Sidebar) -> bool,
}

/// The Projects section.
const PROJECTS: Section = Section {
    key: "sidebar-projects",
    title: "Projects",
    keep: |sidebar, open| sidebar.projects_open = open,
    kept: |sidebar| sidebar.projects_open,
};

/// The Goals section.
const GOALS: Section = Section {
    key: "sidebar-goals",
    title: "Goals",
    keep: |sidebar, open| sidebar.goals_open = open,
    kept: |sidebar| sidebar.goals_open,
};

/// Folds Projects and Goals in `store` as `sidebar` (the prefs read at launch) left them;
/// both start unfolded.
pub(crate) fn start_folds(store: &mut Store, sidebar: Sidebar, cx: &mut Context<'_, Store>) {
    for section in [PROJECTS, GOALS] {
        if !(section.kept)(&sidebar) && !store.places().is_flipped(section.key) {
            store.flip_place_fold(section.key, cx);
        }
    }
}

/// A section's header: its title, which folds it (in every window, and for the next launch).
fn header(section: Section, open: bool, store: &Entity<Store>, p: &Pal) -> AnyElement {
    let Section { key, title, .. } = section;
    let store = store.clone();
    row(key, false, p)
        // Read as the section's name, open or folded.
        .aria_label(title)
        .aria_expanded(open)
        .aria_value(a11y::folded(open))
        .mt(px(10.))
        .text_color(p.muted)
        .font_weight(FontWeight::MEDIUM)
        .child(glyph(
            if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            },
            p.muted,
            12.,
        ))
        .child(title)
        .on_click(move |_, _, cx| {
            let open = store.update(cx, |store, cx| {
                store.flip_place_fold(key, cx);
                !store.places().is_flipped(key)
            });
            Kept::set_sidebar(cx, |sidebar| (section.keep)(sidebar, open));
        })
        .into_any_element()
}

/// The Projects section.
pub(super) fn projects(
    window: &MainWindow,
    p: &Pal,
    cx: &Context<'_, MainWindow>,
) -> Vec<AnyElement> {
    let store = window.store.read(cx);
    let Some(state) = store.state() else {
        return Vec::new();
    };
    let hidden = state.ui.is_none_or(|ui| ui.hide_projects);
    let active: Vec<_> = state
        .projects
        .iter()
        .filter(|project| project.status == ProjectStatus::Active)
        .collect();
    if hidden || active.is_empty() {
        return Vec::new();
    }
    // Open by default: a project's tasks shouldn't be folded away.
    let open = !store.places().is_flipped(PROJECTS.key);
    let mut rows = vec![header(PROJECTS, open, &window.store, p)];
    if !open {
        return rows;
    }
    let shown = window.column.read(cx).current();
    for project in active {
        let id = project.id;
        let goal = project
            .goal
            .and_then(|goal| state.goals.iter().find(|g| g.id == goal))
            .map(|goal| goal.rank);
        let health = match project.health {
            Some(ProjectHealth::Late) => Some(p.danger),
            Some(ProjectHealth::AtRisk) => Some(p.warn),
            _ => None,
        };
        let line = format!(
            "{} · {} open, {} done this week",
            priority_name(project.priority),
            project.open,
            project.done_week
        );
        let opener = window.store.clone();
        rows.push(
            tip(
                row(
                    ElementId::Name(format!("sidebar-project-{id}").into()),
                    shown == Some(Page::Project(id)),
                    p,
                ),
                line,
            )
            .aria_label(a11y::project(
                &project.name,
                project.health.as_ref(),
                project.open,
            ))
            .child(dot(room_color(goal, p), 8.))
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .child(direction::marked(&project.name)),
            )
            .children(health.map(|color| dot(color, 6.)))
            .when(project.open > 0, |d| {
                d.child(
                    div()
                        .text_size(px(11.5))
                        .text_color(p.muted)
                        .child(project.open.to_string()),
                )
            })
            .on_click(move |_, window, cx| {
                opener.update(cx, |store, cx| {
                    store.set_project_filter(ProjectFilter::default(), cx);
                });
                column::open(Page::Project(id), window, cx);
            })
            .into_any_element(),
        );
        rows.extend(area_rows(window, p, state, id));
    }
    rows
}

/// The Goals section.
pub(super) fn goals(window: &MainWindow, p: &Pal, cx: &Context<'_, MainWindow>) -> Vec<AnyElement> {
    let store = window.store.read(cx);
    let Some(state) = store.state() else {
        return Vec::new();
    };
    let hidden = state.ui.is_none_or(|ui| ui.hide_goals);
    if hidden || state.goals.is_empty() {
        return Vec::new();
    }
    let open = !store.places().is_flipped(GOALS.key);
    let mut rows = vec![header(GOALS, open, &window.store, p)];
    if !open {
        return rows;
    }
    let shown = window.column.read(cx).current();
    for goal in &state.goals {
        let id = goal.id;
        let ring_of = progress(id, &state.todos);
        let count = if ring_of.total == 0 {
            "–".to_owned()
        } else {
            format!("{}/{}", ring_of.done, ring_of.total)
        };
        let help = format!(
            "{}\n{}",
            goal.title,
            goal.status.clone().unwrap_or_else(|| goal.why.clone())
        );
        rows.push(
            tip(
                row(
                    ElementId::Name(format!("sidebar-goal-{id}").into()),
                    shown == Some(Page::Goal(id)),
                    p,
                ),
                help,
            )
            .child(ring(
                ElementId::Name(format!("sidebar-ring-{id}").into()),
                13.,
                2.5,
                ring_of.share(),
                room_color(Some(goal.rank), p),
                p.faint.opacity(0.3),
            ))
            .aria_label(a11y::line(&[&label(goal), &ring_of.words()]))
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .child(direction::marked(&label(goal))),
            )
            .child(div().text_size(px(11.5)).text_color(p.muted).child(count))
            .on_click(move |_, window, cx| column::open(Page::Goal(id), window, cx))
            .into_any_element(),
        );
    }
    rows
}

/// More's Weekly report row, with how many were finished this week.
pub(super) fn report_row(
    window: &MainWindow,
    p: &Pal,
    cx: &Context<'_, MainWindow>,
) -> Stateful<gpui_kit::Div> {
    let store = window.store.read(cx);
    let this_week = store
        .state()
        .and_then(|state| state.report.as_ref())
        .and_then(|report| report.weeks.last().copied())
        .unwrap_or(0);
    let shown = window.column.read(cx).current() == Some(Page::Report);
    tip(
        row("weekly-report", shown, p).pl(px(26.)),
        format!("Done each week, yours and {APP_NAME}'s"),
    )
    .aria_label(if this_week > 0 {
        format!("Weekly report, {this_week} done this week")
    } else {
        "Weekly report".to_owned()
    })
    .child(glyph(IconName::ChartColumn, p.muted, 14.))
    .child(div().flex_1().child("Weekly report"))
    .when(this_week > 0, |d| {
        d.child(
            div()
                .text_size(px(11.5))
                .text_color(p.muted)
                .child(this_week.to_string()),
        )
    })
    .on_click(|_, window, cx| column::open(Page::Report, window, cx))
}

/// A project's active areas under it, each opening the project's page on that area.
fn area_rows(
    window: &MainWindow,
    p: &Pal,
    state: &emberhut_api::state::State,
    id: emberhut_api::ids::ProjectId,
) -> Vec<AnyElement> {
    let mut rows = Vec::new();
    for area in state
        .areas
        .iter()
        .filter(|area| area.pid == id && area.status == AreaStatus::Active)
    {
        let area_id = area.id;
        let opener = window.store.clone();
        let help = area
            .about
            .clone()
            .or_else(|| area.goal.clone())
            .unwrap_or_else(|| area.name.clone());
        rows.push(
            tip(
                row(
                    ElementId::Name(format!("sidebar-area-{area_id}").into()),
                    false,
                    p,
                ),
                help,
            )
            .pl(px(28.))
            .aria_label(a11y::area(&area.name, area.open()))
            .child(glyph(
                match area.state {
                    emberhut_api::areas::WorkStage::Done => IconName::CircleCheck,
                    emberhut_api::areas::WorkStage::NeedsYou => IconName::Hand,
                    emberhut_api::areas::WorkStage::Working => IconName::CircleDot,
                    _ => IconName::Circle,
                },
                p.muted,
                11.,
            ))
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .text_size(px(12.5))
                    .child(direction::marked(&area.name)),
            )
            .when(area.open() > 0, |d| {
                d.child(
                    div()
                        .text_size(px(11.5))
                        .text_color(p.muted)
                        .child(area.open().to_string()),
                )
            })
            .on_click(move |_, window, cx| {
                opener.update(cx, |store, cx| {
                    store.set_project_filter(
                        ProjectFilter {
                            area: Some(area_id),
                            ..ProjectFilter::default()
                        },
                        cx,
                    );
                });
                column::open(Page::Project(id), window, cx);
            })
            .into_any_element(),
        );
    }
    rows
}
