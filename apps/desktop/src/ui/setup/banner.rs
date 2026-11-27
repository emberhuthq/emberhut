//! The quiet line at the top of the chat while the ember isn't lit after the setup ended:
//! what is missing, and Light it, which opens the picker in Settings.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::Button;
use gpui_kit::prelude::*;
use gpui_kit::{App, Div, Entity, FontWeight, TestSupportExt as _, div, px};

use crate::store::Store;
use crate::store::settings::SettingsPane;
use crate::store::setup::NOTES;
use crate::store::setup::SetupStep;
use crate::ui::palette::Pal;
use crate::ui::routes;

/// The line, while the ember isn't lit.
pub(crate) fn bar(store: &Entity<Store>, p: &Pal, cx: &App) -> Option<Div> {
    if !store.read(cx).shows_unlit_line() {
        return None;
    }
    Some(
        div().px(px(14.)).pt(px(6.)).child(
            div()
                .id("unlit-bar")
                .flex()
                .items_center()
                .gap(px(10.))
                .pl(px(14.))
                .pr(px(10.))
                .py(px(8.))
                .rounded(px(16.))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .child(
                    Icon::new(IconName::Sparkles)
                        .size(px(15.))
                        .text_color(p.muted),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(NOTES[0]),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(p.muted)
                                .child(SetupStep::Light.line()),
                        ),
                )
                .child(
                    Button::new("unlit-light-it")
                        .small()
                        .label(SetupStep::Light.name())
                        .on_click(|_, window, cx| {
                            routes::open_settings(Some(SettingsPane::Credentials), window, cx);
                        }),
                )
                .test_support(),
        ),
    )
}
