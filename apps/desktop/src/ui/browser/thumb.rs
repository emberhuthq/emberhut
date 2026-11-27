//! The browser's page, live, on the card of the task it browses for: its latest picture, a
//! few seconds old at most, with a globe in its corner. A click on the card still opens the
//! task.

use emberhut_api::work::WorkRow;
use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::Icon;
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, ImageSource, ObjectFit, Window, div, img, px};

use crate::store::Store;
use crate::store::browser::shown;
use crate::ui::a11y::Accessible as _;
use crate::ui::palette::pal;

/// The picture for a card's row: nothing unless the row's task is the one in the browser now.
#[derive(IntoElement)]
pub(crate) struct LiveThumb {
    store: Entity<Store>,
    row: WorkRow,
    size: f32,
}

impl LiveThumb {
    /// The picture for `row`, `size` points square.
    #[must_use]
    pub(crate) fn new(store: &Entity<Store>, row: &WorkRow, size: f32) -> Self {
        Self {
            store: store.clone(),
            row: row.clone(),
            size,
        }
    }
}

impl RenderOnce for LiveThumb {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let store = self.store.read(cx);
        if !shown::is_browsing(&self.row, store.browser()) {
            return div().into_any_element();
        }
        let p = pal(cx);
        let size = px(self.size);
        let picture = store.browser_frame().map_or_else(
            || div().size_full().bg(p.hover).into_any_element(),
            |frame| {
                img(ImageSource::Render(frame))
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .into_any_element()
            },
        );
        div()
            .id("browser-live-thumb")
            .read_as(
                gpui_kit::Role::Image,
                format!("{}'s browser, live", crate::brand::APP_NAME),
            )
            .relative()
            .flex_none()
            .size(size)
            .rounded(px(8.))
            .overflow_hidden()
            .border_1()
            .border_color(p.border)
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(format!(
                    "{}'s browser, live",
                    crate::brand::APP_NAME
                ))
                .build(window, cx)
            })
            .child(picture)
            .child(
                div()
                    .absolute()
                    .right(px(2.))
                    .bottom(px(2.))
                    .p(px(2.))
                    .rounded_full()
                    .bg(p.surface_raised)
                    .child(
                        Icon::new(AssetIcon::Globe)
                            .size(px(10.))
                            .text_color(p.muted),
                    ),
            )
            .into_any_element()
    }
}
