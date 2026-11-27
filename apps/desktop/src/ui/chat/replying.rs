//! The message being replied to, quoted above the composer with an X to drop it.

use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::Icon;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::{Div, div, px};

use super::ChatView;
use crate::store::Store;
use crate::ui::palette::Pal;
use crate::ui::quote::{self, Surface};

impl ChatView {
    /// The quote waiting above the composer, if a reply is being written.
    pub(super) fn replying_chip(&self, p: &Pal, cx: &Context<'_, Self>) -> Option<Div> {
        let store = self.store.read(cx);
        let quoted = store.replying()?;
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(div().flex_1().min_w_0().child(quote::view(
                    quoted,
                    Surface::Plain,
                    &self.now,
                    store,
                    p,
                )))
                .child(
                    div()
                        .id("stop-replying")
                        .flex_none()
                        .size(px(26.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(|d| d.bg(p.hover))
                        .child(Icon::new(AssetIcon::X).size(px(14.)).text_color(p.muted))
                        .tooltip(|window, cx| Tooltip::new("Don't reply").build(window, cx))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.store.update(cx, Store::stop_replying);
                        })),
                ),
        )
    }
}
