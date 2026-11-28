//! The chat's foot: the message sent from here until the chat has it (a failed one sent again
//! with a click), the assistant's step while it works, the ask card, the message being replied
//! to, and the composer.

use std::time::Duration;

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, Div, Entity, Stateful, Transformation, div, percentage, px,
    relative,
};
use jiff::ZonedDateTime;

use super::ChatView;
use super::attachments;
use super::composer::is_working;
use crate::store::{PendingMessage, Store};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::palette::Pal;
use crate::ui::quote::{self, Surface};
use crate::ui::widgets::icon;

impl ChatView {
    pub(super) fn footer(&self, p: &Pal, cx: &mut Context<'_, Self>) -> Div {
        let store = self.store.read(cx);
        let doing = store.live().and_then(|live| live.doing.clone());
        let name = store.ember_name();
        let spoken = a11y::working(name, doing.as_deref());
        let working = is_working(store)
            .then(|| doing.unwrap_or_else(|| format!("{name} is working…")))
            .map(|line| (line, spoken));
        let column =
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .w_full()
                .max_w(px(880.))
                .px(px(24.))
                .pb(px(16.))
                .pt(px(6.))
                .children(
                    store
                        .pending()
                        .map(|pending| pending_bubble(pending, &self.now, store, &self.store, p)),
                )
                .children(working.map(|(line, spoken)| {
                    working_line(line, &spoken, crate::motion::reduced(cx), p)
                }))
                .children(self.ask_area(p, cx))
                .children(self.replying_chip(p, cx))
                .child(self.composer(p, cx));
        div().w_full().flex().justify_center().child(column)
    }
}

/// The message sent from here, until the chat has it, with the message it replies to, its
/// files and how it is going; a failed one's note is a button that sends it again (`entity` is
/// `store`'s). `now` is the chat's clock this frame.
fn pending_bubble(
    pending: &PendingMessage,
    now: &ZonedDateTime,
    store: &Store,
    entity: &Entity<Store>,
    p: &Pal,
) -> Div {
    let files = pending.files.iter().map(|file| {
        div()
            .flex()
            .items_center()
            .gap(px(5.))
            .px(px(9.))
            .py(px(4.))
            .rounded(px(10.))
            .bg(p.card)
            .text_size(px(12.))
            .text_color(p.muted)
            .child(Icon::new(attachments::file_icon(&file.name)).size(px(13.)))
            .child(file.name.clone())
    });
    div()
        .flex()
        .flex_col()
        .items_end()
        .gap(px(3.))
        .children(pending.reply.as_ref().map(|quoted| {
            div()
                .max_w(px(300.))
                .child(quote::view(quoted, Surface::Plain, now, store, p))
        }))
        .when(!pending.text.is_empty(), |d| {
            d.child(
                div()
                    .max_w(relative(0.72))
                    .px(px(13.))
                    .py(px(8.))
                    .rounded(px(17.))
                    .bg(p.queued)
                    .text_color(p.on_queued)
                    .text_size(px(14.))
                    .child(pending.text.clone()),
            )
        })
        .child(
            div()
                .flex()
                .flex_wrap()
                .justify_end()
                .gap(px(6.))
                .children(files),
        )
        .children(if pending.failed() {
            let store = entity.clone();
            Some(
                div().child(
                    div()
                        .id("send-again")
                        .cursor_pointer()
                        .text_size(px(11.5))
                        .text_color(p.danger)
                        .hover(Styled::underline)
                        .child(pending.resend_label())
                        .on_click(move |_, _, cx| store.update(cx, Store::resend))
                        // Seen by the headless tests (a no-op outside them).
                        .test_support(),
                ),
            )
        } else {
            // One the server took without a turn has no note line.
            pending
                .note()
                .map(|note| div().text_size(px(11.5)).text_color(p.muted).child(note))
        })
}

/// The assistant's step while it works, with a spinner (standing still under Reduce motion).
/// A screen reader hears it as one line, `spoken`, said again as the step changes.
fn working_line(line: String, spoken: &str, still: bool, p: &Pal) -> Stateful<Div> {
    let loader = icon(IconName::LoaderCircle, p.info, 14.);
    let spinner = if still {
        loader.into_any_element()
    } else {
        loader
            .with_animation(
                "spin",
                Animation::new(Duration::from_secs(1)).repeat(),
                |icon, t| icon.transform(Transformation::rotate(percentage(t))),
            )
            .into_any_element()
    };
    div()
        .id("working-line")
        .announced(spoken.to_owned())
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(4.))
        .text_size(px(12.5))
        .text_color(p.muted)
        .child(spinner)
        .child(div().truncate().child(line))
}

#[cfg(test)]
mod tests {
    use gpui_kit::base::input::Paste;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{ClipboardItem, Image, ImageFormat, TestAppContext};

    use super::super::tests::open_chat;
    use super::*;
    use crate::backend::fake::Call;

    /// Pastes `item` into the focused composer.
    fn paste(
        cx: &mut TestAppContext,
        window: gpui_kit::WindowHandle<ChatView>,
        item: ClipboardItem,
    ) {
        cx.write_to_clipboard(item);
        window
            .update(cx, |view, window, cx| {
                view.input.update(cx, |input, cx| input.focus(window, cx));
            })
            .unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.dispatch_action(Box::new(Paste), cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn a_pasted_picture_attaches_instead_of_pasting(cx: &mut TestAppContext) {
        let (backend, store, window) = open_chat(cx);
        let picture = Image::from_bytes(ImageFormat::Png, b"\x89PNG".to_vec());
        paste(cx, window, ClipboardItem::new_image(&picture));
        store.read_with(cx, |store, _| {
            let items = store.attachments().items();
            assert_eq!(items.len(), 1);
            assert!(items[0].name.starts_with("pasted-"), "{}", items[0].name);
        });
        assert!(matches!(backend.calls().as_slice(), [Call::Upload(_)]));
        window
            .update(cx, |view, _, cx| {
                assert_eq!(view.input.read(cx).value(), "");
            })
            .unwrap();
    }

    #[gpui_kit::test]
    fn pasted_text_goes_into_the_field(cx: &mut TestAppContext) {
        let (backend, store, window) = open_chat(cx);
        paste(cx, window, ClipboardItem::new_string("Hello there".into()));
        store.read_with(cx, |store, _| {
            assert!(store.attachments().items().is_empty());
        });
        assert_eq!(backend.calls(), []);
        window
            .update(cx, |view, _, cx| {
                assert_eq!(view.input.read(cx).value(), "Hello there");
            })
            .unwrap();
    }

    #[gpui_kit::test]
    fn send_waits_until_the_uploads_are_done(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.pdf");
        std::fs::write(&path, b"pdf").unwrap();
        let (backend, store, window) = open_chat(cx);
        store.update(cx, |store, cx| store.attach_paths(&[path], cx));
        // Return, before the send button shows.
        window.update(cx, ChatView::send).unwrap();
        assert_eq!(backend.calls(), [], "still uploading: nothing sent");
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("send", cx);
        })
        .unwrap();
        cx.run_until_parked();
        let talk = emberhut_api::talk::TalkRequest::text(emberhut_api::ids::RoomId::MAIN, "")
            .with_files(vec![crate::backend::fake::FakeBackend::stored("a.pdf")]);
        assert_eq!(backend.calls().last(), Some(&Call::Talk(talk)));
    }
}
