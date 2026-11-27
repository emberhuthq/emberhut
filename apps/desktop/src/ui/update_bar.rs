//! The update bar: one quiet line at the top of the chat once a newer build is downloaded and
//! verified ("Emberhut 0.3.0 is ready", or "Build 15 is ready" when only the build changes), with
//! Restart to Update and an X that puts it away until a newer build. Kept across launches.
//!
//! The chat mounts it with [`bar`], and tells the store whether its composer holds words with
//! [`note_writing`], so a ready build doesn't install by itself under a message being written.

use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{App, Div, Entity, EntityId, FontWeight, div, px};

use super::palette::Pal;
use crate::store::Store;

/// The bar, when a build is ready and not put away.
pub(crate) fn bar(store: &Entity<Store>, p: &Pal, cx: &App) -> Option<Div> {
    let shown = store.read(cx).updates().bar()?;
    let (restart, away) = (store.clone(), store.clone());
    Some(
        div().px(px(14.)).pt(px(6.)).child(
            div()
                .id("update-bar")
                .flex()
                .items_center()
                .gap(px(10.))
                .pl(px(14.))
                .pr(px(10.))
                .py(px(6.))
                .rounded_full()
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .child(
                    Icon::new(AssetIcon::CircleArrowDown)
                        .size(px(15.))
                        .text_color(p.accent),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(shown.title),
                )
                .child(
                    Button::new("update-restart")
                        .primary()
                        .small()
                        .label("Restart to Update")
                        .on_click(move |_, _, cx| {
                            restart.update(cx, Store::restart_to_update);
                        }),
                )
                .child(
                    Button::new("update-not-now")
                        .ghost()
                        .small()
                        .icon(IconName::Close)
                        .tooltip("Not now")
                        .on_click(move |_, _, cx| {
                            away.update(cx, Store::dismiss_update_bar);
                        }),
                ),
        ),
    )
}

/// The chat view with id `view` has words in its composer (or not any more).
pub(crate) fn note_writing(store: &Entity<Store>, view: EntityId, writing: bool, cx: &mut App) {
    store.update(cx, |store, _| store.note_writing(view, writing));
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{Context, TestAppContext, Window};

    use super::*;
    use crate::backend::fake::FakeBackend;
    use crate::ui::palette::{Pal, pal};

    struct Shown(Entity<Store>);

    impl Render for Shown {
        fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            div().children(bar(&self.0, &pal(cx), cx))
        }
    }

    #[gpui_kit::test]
    fn the_bar_says_whats_ready_and_its_x_puts_it_away(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let backend = Arc::new(FakeBackend::new([])) as Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::for_tests(backend));
        let window = cx.add_window(|_, cx| {
            cx.observe(&store, |_, _, cx| cx.notify()).detach();
            Shown(store.clone())
        });
        assert_eq!(store.read_with(cx, |store, _| store.updates().bar()), None);
        store.update(cx, |store, _| store.put_ready_update_for_tests("0.3.0", 15));
        let title = store.read_with(cx, |store, _| store.updates().bar().map(|bar| bar.title));
        assert_eq!(title.as_deref(), Some("Emberhut 0.3.0 is ready"));
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("update-not-now", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(store.read_with(cx, |store, _| store.updates().bar()), None);
    }
}
