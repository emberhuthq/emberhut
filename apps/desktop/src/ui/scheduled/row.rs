//! One scheduled row: when it runs (in the user's time, with what is left under a day off), its
//! kind's icon and title, `Watch · every 4 h · Room`, what it last saw, and its tasks. With one
//! task the row opens it; with more, each is a link. Its right-click menu stops a watch or
//! cancels a check-in.

use emberhut_api::ids::TaskId;
use emberhut_api::scheduled::{ScheduledItem, ScheduledKind, ScheduledTask};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::Icon;
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, ElementId, Entity, FontWeight, Role, div, px};
use jiff::ZonedDateTime;

use crate::prose::direction;
use crate::store::scheduled::filter::kind_name;
use crate::store::scheduled::when;
use crate::store::tasks::clock::instant;
use crate::store::{Page, Store};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::column;
use crate::ui::controls::{tab_stop, tip};
use crate::ui::palette::Pal;
use crate::wire::TASK_KEY_PREFIX;

/// A kind's icon.
fn kind_icon(kind: &ScheduledKind) -> AssetIcon {
    match kind {
        ScheduledKind::Watch => AssetIcon::Eye,
        ScheduledKind::Checkin => AssetIcon::AlarmClock,
        ScheduledKind::Routine => AssetIcon::Repeat,
        ScheduledKind::Prep => AssetIcon::Users,
        ScheduledKind::Unit => AssetIcon::Cog,
        _ => AssetIcon::Clock,
    }
}

/// The line under the title: its kind, how often (`once` when it doesn't repeat), its room.
fn about_line(item: &ScheduledItem, store: &Store) -> String {
    let room = store
        .state()
        .and_then(|state| state.rooms.iter().find(|row| row.id == item.room))
        .map(|row| row.name.clone());
    [
        Some(kind_name(&item.kind)),
        Some(item.repeat.clone().unwrap_or_else(|| "once".to_owned())),
        room,
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

/// The row.
pub(super) fn row(
    store: &Entity<Store>,
    read: &Store,
    item: &ScheduledItem,
    now: &ZonedDateTime,
    p: &Pal,
) -> AnyElement {
    let when_column = when_column(item, now, p);
    let mut what = what_column(item, read, p);
    let one_task = match item.tasks.as_slice() {
        [task] => Some(task.id),
        _ => None,
    };
    if item.tasks.len() > 1 {
        for task in &item.tasks {
            what = what.child(task_link(task, &item.id, p));
        }
    } else if let Some(task) = item.tasks.first() {
        what = what.child(task_line(task, p));
    }
    let next = item.next_at.as_ref().and_then(instant);
    let at = next.map_or_else(|| "No next run".to_owned(), |next| when::full(next, now));
    // One line for the row: what, when and how often; a button when it opens its task.
    let spoken = a11y::line(&[&item.title, &at, &about_line(item, read)]);
    let role = if one_task.is_some() {
        Role::Button
    } else {
        Role::Group
    };
    let line = div()
        .id(ElementId::Name(format!("scheduled-{}", item.id).into()))
        .read_as(role, spoken)
        .flex()
        .items_start()
        .gap(px(12.))
        .px(px(12.))
        .py(px(6.))
        .rounded(px(6.))
        .border_1()
        .child(when_column)
        .child(what)
        .when_some(one_task, |d, task| {
            d.cursor_pointer()
                .hover(|d| d.bg(p.hover))
                .on_click(move |_, window, cx| column::push(Page::Task(task), window, cx))
        });
    let line = tab_stop(line, p);
    let (watch, checkin) = (item.watch, item.checkin.clone());
    if watch.is_none() && checkin.is_none() {
        return line.into_any_element();
    }
    let store = store.clone();
    line.context_menu(move |menu, _, _| {
        let mut menu = menu;
        if let Some(watch) = watch {
            let store = store.clone();
            menu = menu.item(PopupMenuItem::new("Stop watch").on_click(move |_, _, cx| {
                store.update(cx, |store, cx| store.stop_watch(watch, cx));
            }));
        }
        if let Some(at) = checkin.clone() {
            let store = store.clone();
            menu = menu.item(
                PopupMenuItem::new("Cancel check-in").on_click(move |_, _, cx| {
                    let at = at.clone();
                    store.update(cx, |store, cx| store.cancel_checkin(at, cx));
                }),
            );
        }
        menu
    })
    .into_any_element()
}

/// When the row runs: its time, and what is left under a day off (in the accent within the
/// hour); the whole date on hover.
fn when_column(item: &ScheduledItem, now: &ZonedDateTime, p: &Pal) -> impl IntoElement {
    let next = item.next_at.as_ref().and_then(instant);
    let time = next.map_or_else(|| "—".to_owned(), |next| when::time(next, now));
    let left =
        next.and_then(|next| when::left(next, now).map(|left| (left, when::is_soon(next, now))));
    let full = next.map_or_else(|| "No next run".to_owned(), |next| when::full(next, now));
    tip(
        div()
            .id(ElementId::Name(
                format!("scheduled-when-{}", item.id).into(),
            ))
            .flex()
            .flex_col()
            .gap(px(1.))
            .w(px(84.))
            .flex_none()
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(time),
            )
            .when_some(left, |d, (left, soon)| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .text_color(if soon { p.info } else { p.muted })
                        .child(left),
                )
            }),
        full,
    )
    // Seen by the headless tests (a no-op outside them).
    .test_support()
}

/// What the row is: its kind's icon and title, the line under it, and what it last saw.
fn what_column(item: &ScheduledItem, read: &Store, p: &Pal) -> Div {
    let note = item
        .note
        .clone()
        .filter(|note| !note.trim().is_empty() && item.kind != ScheduledKind::Unit);
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .flex_1()
        .min_w_0()
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(6.))
                .child(
                    div().pt(px(2.)).child(
                        Icon::new(kind_icon(&item.kind))
                            .size(px(12.))
                            .text_color(p.muted),
                    ),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .line_clamp(2)
                        .child(direction::marked(&item.title)),
                ),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(p.muted)
                .child(about_line(item, read)),
        )
        .when_some(note, |d, note| {
            d.child(
                div()
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .line_clamp(2)
                    .child(note),
            )
        })
}

/// A task's key and title.
fn task_words(task: &ScheduledTask, p: &Pal, title_color: gpui_kit::Hsla) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(5.))
        .min_w_0()
        .child(
            div()
                .flex_none()
                .text_size(px(11.))
                .text_color(p.muted)
                .child(format!("{TASK_KEY_PREFIX}{}", task.id)),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(title_color)
                .truncate()
                .child(direction::marked(&task.title)),
        )
}

/// The one task of a row that opens it.
fn task_line(task: &ScheduledTask, p: &Pal) -> Div {
    task_words(task, p, p.muted)
}

/// One of a row's tasks, as a link to its page.
fn task_link(task: &ScheduledTask, row: &str, p: &Pal) -> impl IntoElement {
    let id: TaskId = task.id;
    tab_stop(
        div()
            .id(ElementId::Name(format!("scheduled-{row}-task-{id}").into()))
            .button_named(format!("{TASK_KEY_PREFIX}{id} {}", task.title))
            .hint("Opens its page")
            .border_1()
            .rounded(px(4.))
            .cursor_pointer()
            .hover(|d| d.bg(p.hover))
            .child(task_words(task, p, p.text))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                column::push(Page::Task(id), window, cx);
            }),
        p,
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::backend::fake::FakeBackend;

    #[test]
    fn the_line_under_the_title_says_kind_repeat_and_room() {
        let store = Store::for_tests(Arc::new(FakeBackend::new([])));
        let item: ScheduledItem = serde_json::from_value(json!({
            "id": "watch:3", "kind": "watch", "title": "A reply", "nextAt": null,
            "repeat": "every 4 h", "room": 1, "note": null, "noteAt": null, "tasks": [],
            "watch": 3, "checkin": null
        }))
        .unwrap();
        assert_eq!(about_line(&item, &store), "Watch · every 4 h");
        let once = ScheduledItem {
            repeat: None,
            kind: ScheduledKind::Checkin,
            ..item
        };
        assert_eq!(about_line(&once, &store), "Check-in · once");
    }
}
