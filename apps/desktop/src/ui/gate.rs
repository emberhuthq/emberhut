//! The screens before the app proper: connecting (before the first state), and "not your
//! device" (the server doesn't let this device in). Sign-in is [`super::sign_in`].

use gpui_kit::component::spinner::Spinner;
use gpui_kit::prelude::*;
use gpui_kit::{Div, FontWeight, div, img, px};

use super::palette::Pal;
use crate::assets::FACE;
use crate::brand::APP_NAME;

/// What the "not your device" screen says the user can do.
pub(crate) const NOT_YOURS_HINT: &str = "The server only talks to your own devices, on your \
    own network: for example signed in to Tailscale with your account. Join it, and this app \
    connects by itself.";

/// Before the first state: a spinner, who it is connecting to and where, and why it can't yet
/// once polls have failed for a while.
pub(crate) fn connecting(server: &str, offline: Option<&str>, p: &Pal) -> Div {
    centered(p).child(
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(10.))
            .child(Spinner::new().color(p.muted))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.muted)
                    .child(format!("Connecting to {APP_NAME}…")),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child(server.to_owned()),
            )
            .when_some(offline.map(str::to_owned), |d, offline| {
                d.child(div().text_size(px(12.)).text_color(p.muted).child(offline))
            }),
    )
}

/// The server refused this device: what to do, and the server's address. The poll keeps
/// checking, and the app opens by itself once the device is let in.
pub(crate) fn not_your_device(server: &str, p: &Pal) -> Div {
    centered(p).child(
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(14.))
            .w(px(400.))
            .child(img(FACE).size(px(72.)).rounded_full())
            .child(
                div()
                    .text_size(px(20.))
                    .font_weight(FontWeight::BOLD)
                    .child(format!("{APP_NAME} doesn't know this device")),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(p.muted)
                    .text_center()
                    .child(NOT_YOURS_HINT),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(p.faint)
                    .child(server.to_owned()),
            ),
    )
}

fn centered(p: &Pal) -> Div {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(p.surface)
}
