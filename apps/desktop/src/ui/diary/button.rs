//! The note button in the chat's bar: a click writes a quick note; its right-click menu opens
//! the diary too.

use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::Icon;
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::prelude::*;
use gpui_kit::{div, px};

use super::OpenQuickNote;
use crate::shell::{WindowKind, windows};
use crate::ui::a11y::Accessible as _;
use crate::ui::controls::{tab_stop, tip_for};
use crate::ui::palette::Pal;

/// The button.
pub(crate) fn note_button(p: &Pal) -> impl IntoElement {
    let button = div()
        .id("quick-note-button")
        .button_named("Quick note")
        .hint("Right-click for your diary")
        .accessibility_id("quick note")
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(28.))
        .rounded(px(6.))
        .border_2()
        .cursor_pointer()
        .hover(|d| d.bg(p.hover))
        .child(
            Icon::new(AssetIcon::SquarePen)
                .size(px(15.))
                .text_color(p.muted),
        )
        .on_click(|_, _, cx| {
            windows::open_single(WindowKind::QuickNote, cx);
        });
    tip_for(
        tab_stop(button, p),
        "Quick note; right-click for your diary",
        &OpenQuickNote,
    )
    .context_menu(|menu, _, _| {
        menu.item(PopupMenuItem::new("Quick note").on_click(|_, _, cx| {
            windows::open_single(WindowKind::QuickNote, cx);
        }))
        .item(PopupMenuItem::new("Diary").on_click(|_, _, cx| {
            windows::open_single(WindowKind::Diary, cx);
        }))
    })
}
