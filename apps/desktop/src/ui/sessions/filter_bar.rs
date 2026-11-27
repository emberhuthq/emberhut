//! The Sessions screen's filter bar: Room (one pick, only rooms that have something here), Who
//! runs it and Where (machines and accounts): each a chip, filled with what is picked, that
//! opens a popover of checkboxes which stays open for several picks, each with its count; and
//! Clear when more than one filter is on.

use emberhut_api::ids::RoomId;
use emberhut_api::state::State;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::popover::Popover;
use gpui_kit::prelude::*;
use gpui_kit::{Div, ElementId, Entity, FontWeight, SharedString, div, px};

use crate::store::Store;
use crate::store::sessions::filter::{Choice, Group, Pick, Picks, SessionFilter, chip_title};
use crate::ui::a11y::Accessible as _;
use crate::ui::controls::tab_stop;
use crate::ui::palette::{Pal, pal};

/// A multi-pick menu of the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Menu {
    Worker,
    Environment,
}

impl Menu {
    fn name(self) -> &'static str {
        match self {
            Self::Worker => "Worker",
            Self::Environment => "Environment",
        }
    }

    fn picks(self, picks: &Picks) -> &[Pick] {
        match self {
            Self::Worker => &picks.workers,
            Self::Environment => &picks.environment,
        }
    }

    fn cleared(self, filter: &SessionFilter) -> SessionFilter {
        let mut filter = filter.clone();
        match self {
            Self::Worker => filter.clear_workers(),
            Self::Environment => filter.clear_environment(),
        }
        filter
    }
}

/// A room's name for the bar.
fn room_name(state: &State, room: RoomId) -> String {
    state
        .rooms
        .iter()
        .find(|r| r.id == room)
        .map_or_else(|| format!("Room {room}"), |r| r.name.clone())
}

fn set(store: &Entity<Store>, filter: SessionFilter, cx: &mut gpui_kit::App) {
    store.update(cx, |store, cx| store.set_session_filter(filter, cx));
}

/// The bar, for `rooms` (those that have something here) and the menus' `picks`.
pub(crate) fn bar(
    store: &Entity<Store>,
    rooms: &[RoomId],
    picks: &Picks,
    p: &Pal,
    cx: &gpui_kit::App,
) -> Div {
    let read = store.read(cx);
    let filter = read.session_filter().clone();
    let mut bar = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(6.))
        .px(px(8.))
        .py(px(6.));
    if let Some(state) = read.state() {
        bar = bar.child(room_chip(store, state, rooms, &filter));
    }
    for menu in [Menu::Worker, Menu::Environment] {
        if menu.picks(picks).is_empty() {
            continue;
        }
        bar = bar.child(multi_chip(store, menu, menu.picks(picks), p));
    }
    if filter.count() > 1 {
        let store = store.clone();
        bar = bar.child(tab_stop(
            div()
                .id("sessions-filter-clear")
                .button_named("Clear the filters")
                .border_1()
                .rounded(px(6.))
                .px(px(6.))
                .text_size(px(12.))
                .text_color(p.accent)
                .cursor_pointer()
                .child("Clear")
                .on_click(move |_, _, cx| set(&store, SessionFilter::default(), cx)),
            p,
        ));
    }
    bar
}

/// The Room chip: All, or one room.
fn room_chip(
    store: &Entity<Store>,
    state: &State,
    rooms: &[RoomId],
    filter: &SessionFilter,
) -> impl IntoElement {
    let label: SharedString = filter
        .room
        .map_or_else(|| "Room".to_owned(), |room| room_name(state, room))
        .into();
    let mut options: Vec<(String, Option<RoomId>)> = vec![("All".to_owned(), None)];
    let mut offered: Vec<RoomId> = rooms.to_vec();
    if let Some(picked) = filter.room
        && !offered.contains(&picked)
    {
        offered.push(picked);
    }
    options.extend(
        offered
            .into_iter()
            .map(|room| (room_name(state, room), Some(room))),
    );
    let (store, filter) = (store.clone(), filter.clone());
    let chip = Button::new("sessions-filter-room")
        .label(label)
        .small()
        .dropdown_caret(true);
    let chip = if filter.room.is_some() {
        chip.primary()
    } else {
        chip.ghost()
    };
    chip.dropdown_menu(move |mut menu, _, _| {
        for (name, room) in &options {
            let (store, room) = (store.clone(), *room);
            let mut next = filter.clone();
            next.room = room;
            menu = menu.item(
                PopupMenuItem::new(name.clone())
                    .checked(filter.room == room)
                    .on_click(move |_, _, cx| set(&store, next.clone(), cx)),
            );
        }
        menu
    })
}

/// A multi-pick chip and its popover of checkboxes.
fn multi_chip(store: &Entity<Store>, menu: Menu, picks: &[Pick], p: &Pal) -> Popover {
    let title: SharedString = chip_title(menu.name(), picks).into();
    let any = picks.iter().any(|pick| pick.on);
    let chip = Button::new(ElementId::Name(
        format!("sessions-filter-{}", menu.name()).into(),
    ))
    .label(title)
    .small()
    .dropdown_caret(true);
    let chip = if any { chip.primary() } else { chip.ghost() };
    let store = store.clone();
    let ring = p.border;
    Popover::new(ElementId::Name(
        format!("sessions-filter-pop-{}", menu.name()).into(),
    ))
    .trigger(chip)
    .content(move |_, _, cx| {
        let p = pal(cx);
        let read = store.read(cx);
        let filter = read.session_filter().clone();
        let picks = read
            .sessions_screen()
            .map_or_default(|screen| menu.picks(&screen.picks).to_vec());
        let any = picks.iter().any(|pick| pick.on);
        let all = {
            let (store, cleared) = (store.clone(), menu.cleared(&filter));
            Checkbox::new(ElementId::Name(format!("pick-all-{}", menu.name()).into()))
                .label("All")
                .checked(!any)
                .on_click(move |_, _, cx| set(&store, cleared.clone(), cx))
        };
        let mut list = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .min_w(px(220.))
            .p(px(4.))
            .child(all);
        let mut group_shown: Option<Option<Group>> = None;
        for pick in picks {
            if group_shown != Some(pick.group) {
                group_shown = Some(pick.group);
                list = list.child(div().h(px(1.)).bg(ring).my(px(2.)));
                if let Some(name) = pick.group.map(Group::name) {
                    list = list.child(
                        div()
                            .text_size(px(11.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.muted)
                            .child(name),
                    );
                }
            }
            list = list.child(pick_row(&store, &filter, &pick, &p));
        }
        list
    })
}

/// One checkbox of a popover, with its count on the right.
fn pick_row(store: &Entity<Store>, filter: &SessionFilter, pick: &Pick, p: &Pal) -> Div {
    let mut next = filter.clone();
    next.toggle(&pick.choice);
    let store = store.clone();
    let id = match &pick.choice {
        Choice::Worker(worker) => format!("pick-worker-{worker:?}"),
        Choice::Machine(machine) => format!("pick-machine-{machine}"),
        Choice::Account(account) => format!("pick-account-{account}"),
    };
    div()
        .flex()
        .items_center()
        .gap(px(16.))
        .child(
            div().flex_1().child(
                Checkbox::new(ElementId::Name(id.into()))
                    .label(pick.label.clone())
                    .checked(pick.on)
                    .on_click(move |_, _, cx| set(&store, next.clone(), cx)),
            ),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(p.muted)
                .child(pick.count.to_string()),
        )
}
