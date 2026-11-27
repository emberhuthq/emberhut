//! How Emberhut here stays lit, from step 1's done line: Change opens a small sheet with its two
//! switches, while the window is closed and from login. Each switch is the hut's own setting
//! (`crate::hut::KeepLit`), changed at once.

use gpui_kit::component::Disableable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::switch::Switch;
use gpui_kit::prelude::*;
use gpui_kit::{App, Div, Entity, FocusHandle, Role, TestSupportExt as _, div, px};

use crate::hut::KeepLit;
use crate::store::Store;
use crate::ui::palette::Pal;
use crate::ui::settings::parts::{note, sheet};

/// The sheet's title.
pub(crate) const TITLE: &str = "How your ember stays lit";

/// The first switch's name.
pub(crate) const WHEN_CLOSED: &str = "Stays lit when the window is closed";

/// The second switch's name.
pub(crate) const AT_LOGIN: &str = "Lights up when you log in";

/// What the sheet says under its switches.
pub(crate) const BOTH_OFF: &str = "With both off, it runs only while Emberhut is open.";

/// Change, at the end of step 1's done line.
pub(crate) fn change_button(store: &Entity<Store>) -> Button {
    let store = store.clone();
    Button::new("setup-keep-change")
        .ghost()
        .small()
        .label("Change")
        .on_click(move |_, _, cx| store.update(cx, Store::open_keep_sheet))
}

/// The sheet, holding `focus`, over the setup while it shows.
pub(crate) fn keep_sheet(store: &Entity<Store>, focus: &FocusHandle, p: &Pal, cx: &App) -> Div {
    let setup = store.read(cx).setup();
    let keep = setup.keep().unwrap_or_default();
    let busy = setup.keeping();
    let problem = setup.keep_problem().map(str::to_owned);
    let switch =
        |id: &'static str, label: &'static str, on: bool, turn: fn(KeepLit, bool) -> KeepLit| {
            let store = store.clone();
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .py(px(8.))
                .child(div().text_size(px(13.5)).child(label))
                .child(
                    Switch::new(id)
                        .accessibility_label(label)
                        .checked(on)
                        .disabled(busy)
                        .on_click(move |on: &bool, _, cx| {
                            let wanted = turn(keep, *on);
                            store.update(cx, |store, cx| store.change_keep_lit(wanted, cx));
                        }),
                )
        };
    let body = div()
        .flex()
        .flex_col()
        .child(switch(
            "setup-keep-when-closed",
            WHEN_CLOSED,
            keep.when_closed,
            |keep, on| KeepLit {
                when_closed: on,
                ..keep
            },
        ))
        .child(switch(
            "setup-keep-at-login",
            AT_LOGIN,
            keep.at_login,
            |keep, on| KeepLit {
                at_login: on,
                ..keep
            },
        ))
        .child(note(p, BOTH_OFF).pt(px(4.)))
        .children(problem.map(|why| {
            div()
                .id("setup-keep-problem")
                .role(Role::Status)
                .aria_label(why.clone())
                .pt(px(8.))
                .text_size(px(12.5))
                .text_color(p.warn)
                .child(why)
                .test_support()
        }));
    let close = store.downgrade();
    // Its backdrop takes the pointer: a click on the sheet never reaches the step under it.
    sheet(
        p,
        focus,
        TITLE,
        body,
        ("Done", move |_, cx| {
            if let Err(error) = close.update(cx, Store::close_keep_sheet) {
                log::debug!("the store went with its sheet: {error}");
            }
        }),
    )
    .occlude()
}
