//! "Add a hut": the sheet that adds another computer to the ember as a hut. It shows the line
//! to run there (`emberhut-runner install --server … --pair …`), the lines that first download
//! the runner (macOS and Linux, Windows), each with Copy, and waits for the new hut to join.
//! It says plainly when the hut's address can't be reached from another computer, and that a
//! Mac may ask once to allow the runner (someone has to be at it). Shared by the setup and the
//! Huts page ([`crate::store::huts_add`]).

use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, ClipboardItem, Div, Entity, FocusHandle, Role, SharedString, TestSupportExt as _, div, px,
};

use crate::store::Store;
use crate::store::huts_add::AddHut;
use crate::ui::palette::Pal;
use crate::ui::settings::parts::{note, sheet};

/// The sheet's title.
pub(crate) const TITLE: &str = "Add a hut";

/// What the sheet says when the hut's address only reaches this computer.
pub(crate) const UNREACHABLE: &str = "Right now only this computer can reach your ember's hut, so \
     another computer can't join it yet. Put both on the same network or tailnet, or give the \
     hut a public address.";

/// What the sheet says about a Mac.
pub(crate) const ON_A_MAC: &str = "On a Mac, macOS may ask once to allow it: someone needs to be \
     at that Mac, unlocked.";

/// The sheet while adding a hut is open; `None` while it is closed.
#[must_use]
pub(crate) fn add_hut_sheet(
    store: &Entity<Store>,
    focus: &FocusHandle,
    p: &Pal,
    cx: &App,
) -> Option<Div> {
    let state = store.read(cx).add_hut().clone();
    let (body, done): (Div, &'static str) = match state {
        AddHut::Closed => return None,
        AddHut::Making => (working(p, "Making a code…"), "Cancel"),
        AddHut::Waiting(code) => {
            let body = div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .when(!code.reachable, |d| {
                    d.child(
                        div()
                            .id("add-hut-unreachable")
                            .role(Role::Status)
                            .text_size(px(12.5))
                            .text_color(p.warn)
                            .child(UNREACHABLE)
                            .test_support(),
                    )
                })
                .child(
                    div()
                        .text_size(px(13.))
                        .child("On the computer you're adding, run:"),
                )
                .child(line("add-hut-command", &code.install.command, p))
                .child(note(
                    p,
                    "No Emberhut there yet? This downloads it first, then runs the same:",
                ))
                .child(line("add-hut-shell", &code.install.shell, p))
                .child(note(p, "On Windows, in PowerShell:"))
                .child(line("add-hut-powershell", &code.install.powershell, p))
                .child(note(
                    p,
                    format!(
                        "The code {} works once, for ten minutes. {ON_A_MAC}",
                        code.code
                    ),
                ))
                .child(working(p, "Waiting for the new hut…"));
            (body, "Cancel")
        }
        AddHut::Joined(name) => (
            div().child(
                div()
                    .id("add-hut-joined")
                    .role(Role::Status)
                    .text_size(px(13.5))
                    .text_color(p.success)
                    .child(SharedString::from(format!(
                        "{name} joined. It's one of your huts now."
                    )))
                    .test_support(),
            ),
            "Done",
        ),
        AddHut::Expired => (
            again(store, p, "The code ran out before a hut joined."),
            "Close",
        ),
        AddHut::Failed(why) => (again(store, p, &why), "Close"),
    };
    let close = store.downgrade();
    Some(
        sheet(
            p,
            focus,
            TITLE,
            body,
            (done, move |_, cx| {
                if let Err(error) = close.update(cx, Store::close_add_hut) {
                    log::debug!("the store went with its sheet: {error}");
                }
            }),
        )
        .occlude(),
    )
}

/// A line to run, in a monospace box, with Copy.
fn line(id: &'static str, text: &str, p: &Pal) -> impl IntoElement {
    let copied = text.to_owned();
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(10.))
        .py(px(8.))
        .rounded(px(8.))
        .bg(p.surface_raised)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .font_family("Menlo")
                .text_size(px(12.))
                .child(SharedString::from(text.to_owned())),
        )
        .child(
            Button::new(SharedString::from(format!("{id}-copy")))
                .ghost()
                .small()
                .label("Copy")
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()));
                }),
        )
        .test_support()
}

/// A line with a spinner: something under way.
fn working(p: &Pal, text: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .text_size(px(12.5))
        .text_color(p.muted)
        .child(Spinner::new().small())
        .child(text)
}

/// Why it stopped, and a new code.
fn again(store: &Entity<Store>, p: &Pal, why: &str) -> Div {
    let store = store.clone();
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(
            div()
                .id("add-hut-problem")
                .role(Role::Status)
                .text_size(px(13.))
                .text_color(p.warn)
                .child(SharedString::from(why.to_owned()))
                .test_support(),
        )
        .child(
            div().flex().child(
                Button::new("add-hut-again")
                    .primary()
                    .label("Make a new code")
                    .on_click(move |_, _, cx| store.update(cx, Store::open_add_hut)),
            ),
        )
}
