//! The status line about files, at the top of the window: a spinner while a file comes down,
//! then a mark and how its use went, for a moment ([`crate::store::files::status`]). It takes
//! no clicks: the window under it works as before.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::prelude::*;
use gpui_kit::{App, Div, Entity, FontWeight, div, px};

use crate::store::Store;
use crate::store::files::status::Note;
use crate::ui::a11y::Accessible as _;
use crate::ui::palette::Pal;

/// The line, while it says something.
pub(crate) fn view(store: &Entity<Store>, p: &Pal, cx: &App) -> Option<Div> {
    let note = store.read(cx).file_status()?.clone();
    let (mark, color) = match &note {
        Note::Busy { .. } => (IconName::LoaderCircle, p.muted),
        Note::Done { failed: true, .. } => (IconName::CircleAlert, p.danger),
        Note::Done { failed: false, .. } => (IconName::CircleCheck, p.success),
    };
    Some(
        div()
            .absolute()
            .top(px(8.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    // Said by the screen reader as it changes: coming down, then how it went
                    // (without the percent, which the line shows).
                    .id("file-status")
                    .announced(note.spoken())
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .max_w(px(520.))
                    .px(px(16.))
                    .py(px(8.))
                    .rounded_full()
                    .bg(p.card)
                    .border_1()
                    .border_color(p.border)
                    .shadow_md()
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(Icon::new(mark).size(px(15.)).text_color(color))
                    .child(div().truncate().child(note.text())),
            ),
    )
}
