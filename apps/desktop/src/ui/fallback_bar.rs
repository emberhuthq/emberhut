//! The fallback bar: at the top of the chat while the app is on the fallback server because the
//! server stopped answering. It says so, with Try again, which asks the server and turns back
//! only if it answers (the app never turns back by itself: what is done on the fallback stays
//! there).

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::Button;
use gpui_kit::prelude::*;
use gpui_kit::{App, Div, Entity, FontWeight, div, px};

use super::palette::Pal;
use crate::backend::Serving;
use crate::store::Store;

/// What the bar says, while on the fallback server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Shown {
    /// The headline.
    pub(crate) title: String,
    /// The line under it.
    pub(crate) detail: String,
    /// Whether the server is being asked now.
    pub(crate) trying: bool,
}

/// What the bar says now; none while on the server.
#[must_use]
pub(crate) fn shown(store: &Store) -> Option<Shown> {
    let Serving::Fallback { primary, fallback } = store.serving() else {
        return None;
    };
    let retry = store.primary_retry();
    let detail = if retry.still_silent {
        format!("It still isn't answering. You're on the fallback server, {fallback}.")
    } else {
        format!(
            "You're on the fallback server, {fallback}, until it answers again. What you do \
             now stays there."
        )
    };
    Some(Shown {
        title: format!("The server isn't answering ({primary})"),
        detail,
        trying: retry.trying,
    })
}

/// The bar, while on the fallback server.
pub(crate) fn bar(store: &Entity<Store>, p: &Pal, cx: &App) -> Option<Div> {
    let shown = shown(store.read(cx))?;
    let again = store.clone();
    Some(
        div().px(px(14.)).pt(px(6.)).child(
            div()
                .id("fallback-bar")
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
                    Icon::new(IconName::WifiOff)
                        .size(px(15.))
                        .text_color(p.warn),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(shown.title),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(p.muted)
                                .child(shown.detail),
                        ),
                )
                .child(
                    Button::new("fallback-try-again")
                        .small()
                        .label(if shown.trying {
                            "Checking…"
                        } else {
                            "Try again"
                        })
                        .loading(shown.trying)
                        .on_click(move |_, _, cx| {
                            again.update(cx, Store::try_primary);
                        }),
                ),
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{Context, TestAppContext, Window};

    use super::*;
    use crate::backend::Backend;
    use crate::backend::fake::{Call, FakeBackend};
    use crate::ui::palette::{Pal, pal};

    struct Shows(Entity<Store>);

    impl Render for Shows {
        fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            div().children(bar(&self.0, &pal(cx), cx))
        }
    }

    #[test]
    fn on_the_server_there_is_no_bar() {
        let store = Store::for_tests(Arc::new(FakeBackend::new([])));
        assert_eq!(shown(&store), None);
    }

    #[gpui_kit::test]
    fn the_bar_says_where_the_app_is_and_try_again_turns_back_only_when_the_server_answers(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let fake =
            Arc::new(FakeBackend::new([]).on_fallback("http://new.example", "http://old.example"));
        let store = cx.new(|_| Store::for_tests(Arc::clone(&fake) as Arc<dyn Backend>));
        let window = cx.add_window(|_, cx| {
            cx.observe(&store, |_, _, cx| cx.notify()).detach();
            Shows(store.clone())
        });
        let bar = store.read_with(cx, |store, _| shown(store)).unwrap();
        assert_eq!(bar.title, "The server isn't answering (http://new.example)");
        assert!(
            bar.detail.contains("fallback server, http://old.example"),
            "{}",
            bar.detail
        );
        assert!(!bar.trying);

        let click = |cx: &mut TestAppContext| {
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                window.click("fallback-try-again", cx);
            })
            .unwrap();
            cx.run_until_parked();
        };
        click(cx);
        let bar = store.read_with(cx, |store, _| shown(store)).unwrap();
        assert!(
            bar.detail.starts_with("It still isn't answering."),
            "{}",
            bar.detail
        );

        fake.primary_answers(true);
        click(cx);
        assert_eq!(
            store.read_with(cx, |store, _| shown(store)),
            None,
            "back on the server"
        );
        let tries = fake
            .calls()
            .into_iter()
            .filter(|call| *call == Call::TryPrimary)
            .count();
        assert_eq!(tries, 2);
    }
}
