//! The app lock's cover over a window: "Emberhut is locked" with Unlock while it is locked, the
//! same without the button while the app is only out of sight.

use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{App, Div, FontWeight, div, img, px};

use super::palette::Pal;
use crate::assets::FACE;
use crate::brand::APP_NAME;
use crate::lock;

/// The cover, while the windows are covered.
pub(crate) fn cover(p: &Pal, cx: &App) -> Option<Div> {
    let state = lock::state(cx);
    state.covered().then(|| {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(p.surface)
            .text_color(p.text)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(16.))
                    .child(img(FACE).size(px(72.)).rounded_full())
                    .child(
                        div()
                            .text_size(px(22.))
                            .font_weight(FontWeight::BOLD)
                            .child(format!("{APP_NAME} is locked")),
                    )
                    .when(state.couldnt_ask(), |d| {
                        d.child(
                            div()
                                .text_size(px(13.))
                                .text_color(p.muted)
                                .child("Touch ID and the password can't be asked for now."),
                        )
                    })
                    .when(state.offers_unlock(), |d| {
                        d.child(
                            Button::new("unlock")
                                .accessibility_id("unlock")
                                .primary()
                                .large()
                                .label("Unlock")
                                .on_click(|_, _, cx| lock::unlock(cx)),
                        )
                    }),
            )
    })
}
