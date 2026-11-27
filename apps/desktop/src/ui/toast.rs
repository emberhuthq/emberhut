//! The toast about the last answer: what happened, with Undo while it is held (⌘Z presses it,
//! Ctrl+Z on Linux and Windows), a spinner while it is sent, or why the server refused it with
//! Hide (it goes by itself after a few seconds). It shows at the foot of the page, or under a
//! report that fills the window in the page's place.

use std::time::Duration;

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::Icon;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, App, Div, Entity, FontWeight, Keystroke, Stateful, Subscription,
    Transformation, div, percentage, px,
};

use super::a11y::Accessible as _;
use super::palette::{Pal, pal};
use super::window::MainWindow;
use crate::store::Store;

/// The toast's selector, for the tests (a no-op in a normal build).
pub(super) const TOAST: &str = "answer-toast";

/// The toast, when one shows, to place with [`over_page`] or [`under_report`]: the last
/// answer's or action's, else a note about a task change that didn't go through.
pub(crate) fn toast(
    window: &mut MainWindow,
    cx: &mut Context<'_, MainWindow>,
) -> Option<Stateful<Div>> {
    let p = pal(cx);
    let held = window.store.read(cx);
    let Some(toast) = held.answers().toast().cloned() else {
        let notice = held.tasks().notice()?.to_owned();
        return Some(
            frame(&p, notice.clone())
                .child(
                    Icon::new(AssetIcon::TriangleAlert)
                        .size(px(15.))
                        .text_color(p.warn),
                )
                .child(div().min_w_0().line_clamp(2).child(notice)),
        );
    };
    let (mark, color) = if toast.failed() {
        (AssetIcon::TriangleAlert, p.warn)
    } else if toast.closes {
        (AssetIcon::CircleX, p.faint)
    } else {
        (AssetIcon::CircleCheck, p.success)
    };
    let (undo, hide, serial) = (window.store.clone(), window.store.clone(), toast.serial);
    let trailing = if toast.sending() {
        Some(
            Icon::new(AssetIcon::LoaderCircle)
                .size(px(15.))
                .text_color(p.faint)
                .with_animation(
                    "toast-sending",
                    Animation::new(Duration::from_secs(1)).repeat(),
                    |icon, t| icon.transform(Transformation::rotate(percentage(t))),
                )
                .into_any_element(),
        )
    } else if toast.failed() {
        Some(
            div()
                .id("toast-hide")
                .button_named("Hide")
                .cursor_pointer()
                .child(Icon::new(AssetIcon::X).size(px(14.)).text_color(p.faint))
                .tooltip(|window, cx| Tooltip::new("Hide").build(window, cx))
                .on_click(move |_, _, cx| hide.update(cx, Store::hide_toast))
                .into_any_element(),
        )
    } else if toast.undoable() {
        Some(
            div()
                .id("undo")
                .role(gpui_kit::Role::Button)
                .accessibility_id("undo")
                .cursor_pointer()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.accent)
                .child("Undo")
                .on_click(move |_, _, cx| undo.update(cx, |store, cx| store.undo(serial, cx)))
                // Seen by the headless tests (a no-op outside them).
                .test_support()
                .into_any_element(),
        )
    } else {
        None
    };
    Some(
        frame(&p, toast.text.clone())
            .child(Icon::new(mark).size(px(15.)).text_color(color))
            .child(div().min_w_0().line_clamp(2).child(toast.text))
            .children(trailing),
    )
}

/// The toast's pill, its mark, text and button to add.
fn frame(p: &Pal, said: String) -> Stateful<Div> {
    div()
        // Said by the screen reader as it shows, as the reference client announces it.
        .id("toast")
        .announced(said)
        .accessibility_id("toast")
        .debug_selector(|| TOAST.to_owned())
        .flex()
        .items_center()
        .gap(px(12.))
        .max_w(px(440.))
        .px(px(14.))
        .py(px(8.))
        .rounded(px(18.))
        .bg(p.text)
        .text_color(p.surface)
        .text_size(px(13.))
}

/// The toast at the foot of the page, over it.
pub(crate) fn over_page(toast: Stateful<Div>) -> Div {
    div()
        .absolute()
        .bottom(px(90.))
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        .child(toast)
}

/// The toast under a report that fills the window: a strip of its own, so the report (whose web
/// view covers whatever GPUI draws in its bounds) ends above it.
pub(crate) fn under_report(toast: Stateful<Div>) -> Div {
    div()
        .flex_none()
        .flex()
        .justify_center()
        .py(px(10.))
        .child(toast)
}

/// Whether `keystroke` is Undo's shortcut: ⌘Z on the Mac, Ctrl+Z elsewhere.
#[must_use]
pub(crate) fn is_undo(keystroke: &Keystroke) -> bool {
    let modifiers = &keystroke.modifiers;
    let primary = if cfg!(target_os = "macos") {
        modifiers.platform && !modifiers.control
    } else {
        modifiers.control && !modifiers.platform
    };
    keystroke.key == "z" && primary && !modifiers.shift && !modifiers.alt
}

/// While the toast offers Undo, its shortcut presses it, ahead of a text field's own undo (the
/// toast's Undo is what the user just did). Otherwise the keystroke goes on as usual.
pub(crate) fn undo_on_shortcut(store: &Entity<Store>, cx: &mut App) -> Subscription {
    let store = store.clone();
    cx.intercept_keystrokes(move |event, _, cx| {
        let offered = store
            .read(cx)
            .answers()
            .toast()
            .filter(|toast| toast.undoable())
            .map(|toast| toast.serial);
        if let Some(serial) = offered
            && is_undo(&event.keystroke)
        {
            store.update(cx, |store, cx| store.undo(serial, cx));
            cx.stop_propagation();
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undos_shortcut_is_the_platforms() {
        let [undo, redo, other] = if cfg!(target_os = "macos") {
            ["cmd-z", "cmd-shift-z", "cmd-y"]
        } else {
            ["ctrl-z", "ctrl-shift-z", "ctrl-y"]
        };
        let key = |text: &'static str| Keystroke::parse(text).unwrap();
        assert!(is_undo(&key(undo)));
        assert!(!is_undo(&key(redo)), "redo");
        assert!(!is_undo(&key("z")));
        assert!(!is_undo(&key(other)));
    }
}
