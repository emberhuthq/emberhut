//! The sidebar's Rooms: a section that folds under its header (kept across launches), each
//! listed room's row, and the Paused fold with the paused rooms. A project's room has a
//! right-click menu: pause it until a day, resume it, archive it or unarchive it. Hovering a
//! row shows its name and its shortcut.

use std::time::Duration;

use emberhut_api::rooms::RoomRow;
use gpui_kit::component::IconName;
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, ElementId, Entity, FontWeight, Hsla, div,
    pulsating_between, px,
};
use jiff::ZonedDateTime;

use super::controls::{tab_stop, tip};
use super::palette::{Pal, room_color};
use super::widgets::{dot, icon};
use super::window::MainWindow;
use crate::brand::APP_NAME;
use crate::shell::{WindowKind, windows};
use crate::store::room_actions::{Pausing, RoomAction};
use crate::store::{Page, RoomLine, Store, lookup};
use crate::ui::a11y::{self, Accessible as _};

/// The modifier the room shortcuts use, as their tooltips spell it.
#[cfg(target_os = "macos")]
const SHORTCUT_PREFIX: &str = "⌘";
/// The modifier the room shortcuts use, as their tooltips spell it.
#[cfg(not(target_os = "macos"))]
const SHORTCUT_PREFIX: &str = "Ctrl+";

/// The Rooms section: its header, the listed rooms, then the Paused fold. Nothing when there
/// are no rooms but Main.
pub(crate) fn section(
    window: &MainWindow,
    wide: bool,
    p: &Pal,
    cx: &Context<'_, MainWindow>,
) -> Vec<AnyElement> {
    let store = window.store.read(cx);
    let switcher = store.switcher();
    if switcher.listed().is_empty() && switcher.paused().is_empty() {
        return Vec::new();
    }
    let open = store.rooms_open();
    let mut rows = vec![fold_header(&window.store, open, p).into_any_element()];
    if !open {
        return rows;
    }
    let now = ZonedDateTime::now();
    // Where the window is: the page its column shows, else the chat's room.
    let page = window
        .column
        .read(cx)
        .current()
        .unwrap_or(Page::Room(store.room()));
    for &room in switcher.listed() {
        let help = switcher.shortcut_of(room.id).map_or_else(
            || room.name.clone(),
            |key| format!("{} ({SHORTCUT_PREFIX}{})", room.name, key.digit()),
        );
        let selected = page == Page::Room(room.id);
        rows.push(room_row(window, room, selected, wide, help, &now, p, cx));
    }
    let paused = switcher.paused();
    if !paused.is_empty() {
        let paused_open = store.paused_open();
        rows.push(paused_fold(&window.store, paused.len(), paused_open, p).into_any_element());
        if paused_open {
            for &room in paused {
                let selected = page == Page::Room(room.id);
                let help = room.name.clone();
                rows.push(room_row(window, room, selected, wide, help, &now, p, cx));
            }
        }
    }
    if windows::available(cx).contains(&WindowKind::Dark) {
        rows.push(dark_row(p));
    }
    rows
}

/// The Dark room's row, the last of Rooms: it opens its window and never lights.
fn dark_row(p: &Pal) -> AnyElement {
    let row = div()
        .id("room-dark")
        .border_1()
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(28.))
        .px(px(10.))
        .mx(px(8.))
        .rounded(px(6.))
        .text_size(px(13.))
        .text_color(p.muted)
        .hover(|d| d.bg(p.hover))
        .cursor_pointer()
        .child(
            gpui_kit::component::Icon::new(gpui_kit::assets::IconName::Lock)
                .size(px(13.))
                .text_color(p.muted),
        )
        .child("Dark")
        .on_click(|_, _, cx| {
            windows::open_single(WindowKind::Dark, cx);
        });
    tip(
        tab_stop(row, p),
        format!("Dark: off the record, {APP_NAME} can't see it"),
    )
    .into_any_element()
}

/// "Rooms", folding the section under it.
fn fold_header(store: &Entity<Store>, open: bool, p: &Pal) -> impl IntoElement {
    let store = store.clone();
    div()
        .id("rooms-fold")
        .button_named("Rooms")
        .aria_expanded(open)
        .aria_value(a11y::folded(open))
        .flex()
        .items_center()
        .gap(px(4.))
        .px(px(10.))
        .pt(px(14.))
        .pb(px(4.))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.faint)
        .cursor_pointer()
        .child("Rooms")
        .child(icon(
            if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            },
            p.faint,
            10.,
        ))
        .on_click(move |_, _, cx| store.update(cx, Store::toggle_rooms))
}

/// "Paused" with its count, opening to the paused rooms.
fn paused_fold(store: &Entity<Store>, count: usize, open: bool, p: &Pal) -> impl IntoElement {
    let store = store.clone();
    div()
        .id("paused")
        .button_named(format!("Paused, {count}"))
        .aria_expanded(open)
        .aria_value(a11y::folded(open))
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(28.))
        .px(px(10.))
        .mx(px(8.))
        .rounded(px(6.))
        .text_size(px(13.))
        .text_color(p.muted)
        .hover(|d| d.bg(p.hover))
        .cursor_pointer()
        .child(icon(IconName::Pause, p.muted, 13.))
        .child("Paused")
        .child(icon(
            if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            },
            p.faint,
            10.,
        ))
        .child(div().flex_1())
        .child(div().text_size(px(11.5)).child(count.to_string()))
        .on_click(move |_, _, cx| store.update(cx, Store::toggle_paused))
}

/// A room's row: its dot, its name (bold while unread), a pulsing teal dot while the assistant
/// works there, an unread dot, when it last spoke; under it the last line and how many need
/// the user. Narrow (or paused), one line with a badge at its end.
#[expect(
    clippy::too_many_arguments,
    reason = "a row's own facts and the frame's shared ones (now, the palette, the context)"
)]
fn room_row(
    window: &MainWindow,
    room: &RoomRow,
    selected: bool,
    wide: bool,
    help: String,
    now: &ZonedDateTime,
    p: &Pal,
    cx: &Context<'_, MainWindow>,
) -> AnyElement {
    let store = window.store.read(cx);
    let rank = store
        .state()
        .and_then(|state| lookup::goal_rank(state, room.goal));
    let color = room_color(rank, p);
    let line = RoomLine::of(room, wide, now);
    let id = room.id;
    let name_line = div()
        .flex()
        .items_center()
        .gap(px(5.))
        .text_size(px(13.))
        .child(
            div()
                .truncate()
                .when(line.unread, |d| d.font_weight(FontWeight::SEMIBOLD))
                .child(room.name.clone()),
        )
        .when(room.busy, |d| d.child(working_dot(id.get(), p.info)))
        .when(line.unread, |d| d.child(dot(p.accent, 6.)))
        .child(div().flex_1())
        .when_some(line.ago.clone(), |d, ago| {
            d.child(div().text_size(px(11.)).text_color(p.faint).child(ago))
        })
        .when_some(line.badge.clone(), |d, badge| {
            d.child(div().text_size(px(11.)).text_color(p.muted).child(badge))
        });
    let preview = line.preview.clone().map(|preview| {
        div()
            .flex()
            .items_center()
            .gap(px(4.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(11.5))
                    .text_color(p.muted)
                    .child(preview),
            )
            .when_some(line.needs, |d, needs| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .text_color(p.muted)
                        .child(needs.to_string()),
                )
            })
    });
    let spoken = a11y::room(&room.name, room.needs, room.unread, room.busy);
    let row = div()
        .id(ElementId::Name(format!("room-{id}").into()))
        // One line for the whole row: its name, what waits, what is new, whether it works.
        .button_named(spoken)
        .aria_selected(selected)
        .flex()
        .gap(px(8.))
        .px(px(10.))
        .py(px(if preview.is_some() { 5. } else { 6. }))
        .mx(px(8.))
        .rounded(px(6.))
        .when(selected, |d| d.bg(p.accent_soft))
        .when(!selected, |d| d.hover(|d| d.bg(p.hover)))
        .cursor_pointer()
        .child(
            div()
                .pt(px(if preview.is_some() { 6. } else { 5. }))
                .child(dot(color, 8.)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(1.))
                .child(name_line)
                .children(preview),
        )
        .tooltip(move |window, cx| Tooltip::new(help.clone()).build(window, cx))
        .on_click(cx.listener(move |this, _, window, cx| this.open(Page::Room(id), window, cx)));
    with_room_menu(row, room, &window.store)
}

/// A small dot in the theme's running colour that pulses while the assistant works in the room (still when the system
/// asks for less motion).
fn working_dot(room: i64, color: Hsla) -> impl IntoElement {
    dot(color, 6.).with_animation(
        ElementId::Name(format!("room-working-{room}").into()),
        Animation::new(Duration::from_millis(1600))
            .repeat()
            .with_easing(pulsating_between(0.35, 1.)),
        Styled::opacity,
    )
}

/// `row` with its room's right-click menu, for a project's room (Main has none). Also used
/// by Settings' archived rooms.
pub(crate) fn with_room_menu(
    row: gpui_kit::Stateful<gpui_kit::Div>,
    room: &RoomRow,
    store: &Entity<Store>,
) -> AnyElement {
    let (Some(project), Some(pausing)) = (room.project, Pausing::of(room)) else {
        return row.into_any_element();
    };
    let (store, name, status) = (store.clone(), room.name.clone(), room.status.clone());
    row.context_menu(move |menu: PopupMenu, _, _| {
        let mut menu = menu;
        for &item in RoomAction::offered(&status) {
            let (store, name, pausing) = (store.clone(), name.clone(), pausing.clone());
            menu = menu.item(PopupMenuItem::new(item.label()).on_click(move |_, _, cx| {
                match item.action() {
                    Some(action) => store.update(cx, |store, cx| {
                        store.act_on_room(project, &name, action, cx);
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
