//! A project's page: its name; its health, a bar of how much is done and its properties as
//! chips (priority, goal with its ring, deadline, and More: talk about it, pause until a day,
//! resume, archive); the filters (area, status, owner) and List / Board; what of it needs the
//! user, one row each; then its work by area: as a list (each lane's open work with its steps
//! under it, its done work folded) or a board (a row of status columns per area, Done folded);
//! its paused and done areas folded at the foot.

//!
//! The header and filters are in `header.rs`, the Deadline editor in `deadline.rs`, the list in
//! `list.rs` and the board in `board.rs`.

mod board;
mod deadline;
mod header;
mod list;

use emberhut_api::ids::ProjectId;
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{Div, ElementId, Entity, FontWeight, div, px};
use jiff::ZonedDateTime;

use super::{MainWindow, body, gone, section, title};
use crate::prose::direction;
use crate::store::needs_view::Layout;
use crate::store::projects::{ProjectPage, Stage};
use crate::store::tasks::search::Found;
use crate::store::{Page, Store};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::column;
use crate::ui::palette::Pal;
use crate::ui::tasks::marks::glyph;

/// The page.
pub(in crate::ui::places) fn page(
    window: &mut MainWindow,
    p: &Pal,
    project: ProjectId,
    cx: &Context<'_, MainWindow>,
) -> Div {
    let store_entity = window.store.clone();
    let store = window.store.read(cx);
    let Some(state) = store.state() else {
        return gone(p, "Loading…");
    };
    let Some(shown) = ProjectPage::of(state, project) else {
        return gone(p, "Project removed");
    };
    let today = ZonedDateTime::now().date();
    let layout = store.needs_view().project_layout();
    let filter = store.places().project_filter();
    let done = match store
        .places()
        .project_done(project)
        .map(|lookup| lookup.found())
    {
        Some(Found::Tasks(tasks)) => Some(tasks),
        _ => None,
    };
    let lanes = shown.lanes(filter, done, &|task| store.is_gone(task));
    let mut page = body()
        .child(title(shown.project.name.clone()))
        .child(header::top(p, &store_entity, &shown))
        .child(header::filter_bar(p, &store_entity, &shown, filter, layout));
    page = page.children(store.needs().map(|needs| needs_inbox(p, &shown, &needs)));
    if lanes.is_empty() {
        page = page.child(
            div()
                .pt(px(12.))
                .text_size(px(13.))
                .text_color(p.muted)
                .child(if filter.is_empty() {
                    "Nothing open."
                } else {
                    "Nothing here for this filter."
                }),
        );
    }
    page = match layout {
        Layout::List => lanes.iter().fold(page, |page, lane| {
            page.child(list::lane(p, &store_entity, store, lane, today))
        }),
        Layout::Board => page.child(board::board(p, &store_entity, store, &lanes, today)),
    };
    if filter.is_empty() {
        page = page.child(quiet_areas(p, &store_entity, store, &shown));
    }
    page
}

/// A folded part's row: its title and a chevron.
pub(super) fn fold_row(p: &Pal, id: &str, text: String, open: bool) -> gpui_kit::Stateful<Div> {
    div()
        .id(ElementId::Name(id.to_owned().into()))
        // Read as its title, open or folded.
        .button_named(text.clone())
        .aria_expanded(open)
        .aria_value(a11y::folded(open))
        .flex()
        .items_center()
        .gap(px(6.))
        .mt(px(8.))
        .py(px(5.))
        .cursor_pointer()
        .text_size(px(13.))
        .child(div().flex_1().child(text))
        .child(glyph(
            if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            },
            p.faint,
            13.,
        ))
}

/// A stage's small mark.
pub(super) fn stage_mark(p: &Pal, stage: Stage) -> impl IntoElement {
    let (name, color) = match stage {
        Stage::NeedsYou => (IconName::Hand, p.accent),
        Stage::NotStarted => (IconName::Circle, p.muted),
        Stage::Working => (IconName::CircleDot, p.info),
        Stage::Scheduled => (IconName::Clock, p.info),
        Stage::Done => (IconName::CircleCheck, p.success),
    };
    glyph(name, color, 13.)
}

/// What of the project needs the user, one row each: what to do, the task, its decide-by chip;
/// a click opens the task.
fn needs_inbox(p: &Pal, shown: &ProjectPage<'_>, needs: &crate::store::needs::Needs<'_>) -> Div {
    let rows = shown.needs(needs);
    let mut page = div().flex().flex_col();
    if rows.is_empty() {
        return page;
    }
    page = page.child(section(p, format!("Needs you · {}", rows.len())));
    for row in rows {
        let task = row.task;
        page = page.child(
            div()
                .id(ElementId::Name(format!("project-need-{}", row.id).into()))
                .button_named(a11y::line(&[
                    row.action
                        .as_deref()
                        .or(row.ask.as_deref())
                        .unwrap_or_default(),
                    &row.title,
                ]))
                .flex()
                .items_center()
                .gap(px(9.))
                .py(px(5.))
                .cursor_pointer()
                .hover(|d| d.bg(p.hover))
                .child(glyph(IconName::Hand, p.accent, 12.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_size(px(13.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(direction::marked(
                                    row.action
                                        .as_deref()
                                        .or(row.ask.as_deref())
                                        .unwrap_or(&row.title),
                                )),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(p.muted)
                                .child(direction::marked(&row.title)),
                        ),
                )
                .children(
                    crate::store::need_card::decide_chip(row, needs.today(), false)
                        .map(|chip| crate::ui::needs::chip(p, &chip)),
                )
                .on_click(move |_, window, cx| {
                    if let Some(task) = task {
                        column::push(Page::Task(task), window, cx);
                    }
                }),
        );
    }
    page
}

/// The project's paused and done areas, folded at the foot.
fn quiet_areas(
    p: &Pal,
    store_entity: &Entity<Store>,
    store: &Store,
    shown: &ProjectPage<'_>,
) -> Div {
    let mut page = div().flex().flex_col();
    let quiet = shown.quiet_areas();
    if !quiet.is_empty() {
        let open = store.places().is_flipped("quiet-areas");
        let flip = store_entity.clone();
        page = page.child(
            fold_row(
                p,
                "quiet-areas",
                format!("Paused and done areas · {}", quiet.len()),
                open,
            )
            .on_click(move |_, _, cx| {
                flip.update(cx, |store, cx| store.flip_place_fold("quiet-areas", cx));
            }),
        );
        if open {
            for area in quiet {
                page = page.child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .py(px(4.))
                        .pl(px(16.))
                        .text_size(px(13.))
                        .child(div().flex_1().child(area.name.clone()))
                        .child(
                            div()
                                .text_color(p.muted)
                                .child(area.status.as_str().to_owned()),
                        ),
                );
            }
        }
    }
    page
}
