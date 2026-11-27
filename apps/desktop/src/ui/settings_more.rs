//! Settings' groups about where the app stands: the Machines row's words, the server (its
//! address, phone push) and the diagnostics (sign-in on the server, how this device is let
//! in), Devices (each signed-in device with Sign out, confirmed before it is sent), a pairing
//! code for another device, and Archived rooms (their count, opening the list: a click opens
//! the room's chat, right-click unarchives it).

use emberhut_api::ids::DeviceId;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{App, Div, ElementId, Entity, FontWeight, ImageSource, div, img, px};
use jiff::{Timestamp, ZonedDateTime};

use super::controls::tab_stop;
use super::machines::summary::{RowState, Summary};
use super::palette::{Pal, room_color};
use super::qr::Qr;
use super::rooms::with_room_menu;
use super::widgets::{dot, group};
use crate::store::devices::{NewCode, device_name, last_seen, sign_in_word};
use crate::store::settings::chrome::Health;
use crate::store::{Store, lookup, room_line};

/// The Machines row's words: their state, how well, and their names.
pub(crate) fn machines_row(store: &Store) -> ((String, Health), String) {
    let feed = store.machines();
    let summary = Summary::of(feed.page(), feed.problem());
    let health = match summary.state {
        RowState::Off => Health::Off,
        RowState::Fine => Health::Fine,
        RowState::Warn => Health::Warn,
        RowState::Down => Health::Down,
    };
    ((summary.status, health), summary.detail)
}

/// The server's facts: its address and phone push.
pub(crate) fn server_rows(store: &Store, p: &Pal) -> Vec<Div> {
    let push = if store.state().is_some_and(|state| state.push) {
        "On"
    } else {
        "Off until the push key is on the server"
    };
    vec![
        fact("Address", store.server().to_string(), p),
        fact("Phone push", push.to_owned(), p),
    ]
}

/// The diagnostics' facts: how sign-in stands on the server, and how this device is let in.
pub(crate) fn sign_in_rows(store: &Store, p: &Pal) -> Vec<Div> {
    let let_in = if store.is_signed_in() {
        "This device's own session"
    } else {
        "This device's network (no session of its own)"
    };
    let mut rows = Vec::new();
    if let Some(auth) = store.state().and_then(|state| state.auth.as_ref()) {
        rows.push(fact("Sign-in mode", sign_in_word(auth).to_owned(), p));
    }
    rows.push(fact("This device", let_in.to_owned(), p));
    rows
}

/// The Devices group: each signed-in device, with Sign out.
pub(crate) fn devices(store: &Entity<Store>, p: &Pal, cx: &App) -> Div {
    let read = store.read(cx);
    let mut group = group(p, "DEVICES");
    let Some(auth) = read.state().and_then(|state| state.auth.as_ref()) else {
        return group.child(
            div()
                .text_size(px(12.5))
                .text_color(p.muted)
                .child("The server doesn't say which devices are signed in."),
        );
    };
    let (this_one, now, asked) = (read.this_device(), Timestamp::now(), read.sign_out_asked());
    for device in &auth.devices {
        let name = device_name(device, this_one);
        let seen = last_seen(device, now);
        let plain = device
            .name
            .clone()
            .unwrap_or_else(|| "the device".to_owned());
        group = group.child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_size(px(13.)).child(name))
                        .when_some(seen, |d, seen| {
                            d.child(div().text_size(px(11.5)).text_color(p.muted).child(seen))
                        }),
                )
                .child(sign_out(
                    store,
                    device.id,
                    plain,
                    asked == Some(device.id),
                    p,
                )),
        );
    }
    group
}

/// "Add a phone or computer": a pairing code for another device, with its QR code; only for
/// a device with a session of its own (none otherwise).
pub(crate) fn pair_card(store: &Entity<Store>, qr: Option<Qr>, p: &Pal, cx: &App) -> Option<Div> {
    let read = store.read(cx);
    read.is_signed_in().then(|| {
        group(p, "ADD A PHONE OR COMPUTER").child(pair_another(store, read.new_code(), qr, p))
    })
}

/// A device's "Sign out", and once pressed its confirmation: signed out, a device has to pair
/// again.
fn sign_out(store: &Entity<Store>, id: DeviceId, name: String, asked: bool, p: &Pal) -> Div {
    let store = store.clone();
    let row = div().flex().items_center().gap(px(6.));
    if !asked {
        return row.child(
            Button::new(ElementId::Name(format!("sign-out-{id}").into()))
                .ghost()
                .small()
                .label("Sign out")
                .on_click(move |_, _, cx| {
                    store.update(cx, |store, cx| store.ask_to_sign_out(id, cx));
                }),
        );
    }
    let cancel = store.clone();
    row.child(
        div()
            .text_size(px(12.))
            .text_color(p.muted)
            .child(format!("Sign out {name}? It has to pair again.")),
    )
    .child(
        Button::new(ElementId::Name(format!("sign-out-cancel-{id}").into()))
            .ghost()
            .small()
            .label("Cancel")
            .on_click(move |_, _, cx| cancel.update(cx, Store::cancel_sign_out)),
    )
    .child(
        Button::new(ElementId::Name(format!("sign-out-confirm-{id}").into()))
            .danger()
            .small()
            .label("Sign out")
            .on_click(move |_, _, cx| {
                store.update(cx, |store, cx| store.sign_out_device(id, &name, cx));
            }),
    )
}

/// "Pair another device": a one-time code to type on it, and until when it works; with the
/// code's pairing link as a QR code, when the server gave one, the code under it.
fn pair_another(store: &Entity<Store>, code: Option<&NewCode>, qr: Option<Qr>, p: &Pal) -> Div {
    let shown = match code {
        Some(NewCode::Made(code)) => Some(code.code.clone()),
        _ => None,
    };
    let row = code_row(store, code);
    match (qr, shown) {
        (Some(qr), Some(shown)) => div().flex().flex_col().gap(px(8.)).child(row).child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(6.))
                .w(px(200.))
                .child(
                    img(ImageSource::Render(qr.picture))
                        .id("pairing-qr")
                        .size(px(qr.side))
                        .rounded(px(8.)),
                )
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::BOLD)
                        .child(shown),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(p.muted)
                        .child("Scan it with Emberhut on the other device, or type the code."),
                ),
        ),
        _ => row,
    }
}

/// The button and the line with the code and until when it works.
fn code_row(store: &Entity<Store>, code: Option<&NewCode>) -> Div {
    let store = store.clone();
    let line = match code {
        None => None,
        Some(NewCode::Making) => Some("Making a code…".to_owned()),
        Some(NewCode::Made(code)) => {
            let until = code
                .expires
                .as_str()
                .parse::<Timestamp>()
                .ok()
                .map(|at| at.to_zoned_datetime(ZonedDateTime::now().time_zone().clone()));
            Some(match until {
                Some(until) => format!(
                    "Type {} on the other device, by {}. It works once.",
                    code.code,
                    until.strftime("%H:%M")
                ),
                None => format!("Type {} on the other device. It works once.", code.code),
            })
        }
    };
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .pt(px(4.))
        .child(
            Button::new("pair-another")
                .small()
                .label("Pair another device")
                .on_click(move |_, _, cx| store.update(cx, Store::make_pairing_code)),
        )
        .when_some(line, |d, line| {
            d.child(
                div()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .child(line),
            )
        })
}

/// The Archived rooms group: their count (or None), opening to the list.
pub(crate) fn archived(store: &Entity<Store>, open: bool, p: &Pal, cx: &App) -> Div {
    let read = store.read(cx);
    let switcher = read.switcher();
    let rooms = switcher.archived();
    let count = if rooms.is_empty() {
        "None".to_owned()
    } else {
        rooms.len().to_string()
    };
    let mut group = group(p, "ROOMS").child(tab_stop(
        div()
            .id("archived-rooms")
            .border_1()
            .flex()
            .items_center()
            .cursor_pointer()
            .rounded(px(6.))
            .hover(|d| d.bg(p.hover))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(13.))
                    .child("Archived rooms"),
            )
            .child(div().text_size(px(12.5)).text_color(p.muted).child(count))
            .on_click({
                let store = store.clone();
                move |_, _, cx| store.update(cx, Store::toggle_archived)
            }),
        p,
    ));
    if !open {
        return group;
    }
    if rooms.is_empty() {
        return group.child(div().text_size(px(12.5)).text_color(p.muted).child(
            "No archived rooms. Right-click a room in the sidebar to archive one you're no \
             longer working on.",
        ));
    }
    let now = ZonedDateTime::now();
    for &room in rooms {
        let rank = read
            .state()
            .and_then(|state| lookup::goal_rank(state, room.goal));
        let last = room_line::preview(room);
        let ago = room_line::ago(
            room.last_at
                .as_ref()
                .map(emberhut_api::time::Timestamp::as_str),
            &now,
        );
        let id = room.id;
        let opener = store.clone();
        let row = div()
            .id(ElementId::Name(format!("archived-{id}").into()))
            .border_1()
            .flex()
            .items_center()
            .gap(px(10.))
            .py(px(4.))
            .cursor_pointer()
            .rounded(px(6.))
            .hover(|d| d.bg(p.hover))
            .child(dot(room_color(rank, p), 10.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_size(px(13.)).child(room.name.clone()))
                    .when_some(last, |d, last| {
                        d.child(
                            div()
                                .truncate()
                                .text_size(px(11.5))
                                .text_color(p.muted)
                                .child(last),
                        )
                    }),
            )
            .when_some(ago, |d, ago| {
                d.child(div().text_size(px(11.)).text_color(p.faint).child(ago))
            })
            .on_click(move |_, _, cx| {
                opener.update(cx, |store, cx| store.switch_room(id, cx));
            });
        group = group.child(with_room_menu(tab_stop(row, p), room, store));
    }
    group
}

/// A labelled fact: its name, then its value.
fn fact(label: &'static str, value: String, p: &Pal) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .text_size(px(13.))
        .child(div().w(px(110.)).child(label))
        .child(div().text_color(p.muted).child(value))
}
