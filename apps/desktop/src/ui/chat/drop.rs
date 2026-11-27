//! Files dragged from the file manager attach when they are dropped anywhere on the chat, not
//! only on the composer, several at once; a "Drop to attach" frame shows while they are over
//! it. GPUI takes file drops on macOS, Windows, and Linux (X11 and Wayland), as paths: a
//! picture dragged out of a browser or Photos on macOS comes as a file promise, not a path, so
//! GPUI doesn't see it and nothing attaches (save it to disk first).

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::prelude::*;
use gpui_kit::{Div, DragMoveEvent, ExternalPaths, FontWeight, Stateful, div, px};

use super::ChatView;
use crate::ui::palette::Pal;

impl ChatView {
    /// `chat` as the drop target: it follows a drag of files over it, attaches them when they
    /// are dropped, and shows the frame meanwhile.
    pub(super) fn drop_target(
        &self,
        chat: Div,
        p: &Pal,
        cx: &mut Context<'_, Self>,
    ) -> Stateful<Div> {
        let dropping = self.dropping && cx.has_active_drag();
        div()
            .id("chat")
            .relative()
            .size_full()
            .child(chat)
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<ExternalPaths>, _, cx| {
                    let over = event.bounds.contains(&event.event.position);
                    if this.dropping != over {
                        this.dropping = over;
                        cx.notify();
                    }
                }),
            )
            .on_file_drop_exit(cx.listener(|this, _, _, cx| {
                this.dropping = false;
                cx.notify();
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.dropping = false;
                this.attach_dropped(paths, window, cx);
            }))
            .when(dropping, |d| d.child(frame(p)))
    }

    /// Attaches dropped files, and puts the cursor in the field to ask about them.
    fn attach_dropped(
        &mut self,
        paths: &ExternalPaths,
        window: &mut gpui_kit::Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.store
            .update(cx, |store, cx| store.attach_paths(paths.paths(), cx));
        self.input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }
}

/// The frame over the chat while files are dragged over it.
fn frame(p: &Pal) -> Div {
    div()
        .absolute()
        .inset(px(8.))
        .rounded(px(16.))
        .border_2()
        .border_color(p.accent)
        .bg(p.accent.opacity(0.08))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .text_color(p.accent)
        .text_size(px(17.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(
            Icon::new(IconName::Paperclip)
                .size(px(18.))
                .text_color(p.accent),
        )
        .child("Drop to attach")
}

#[cfg(test)]
mod tests {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{FileDropEvent, PlatformInput, TestAppContext, point};

    use super::super::tests::open_chat;
    use super::*;
    use crate::backend::fake::Call;

    #[gpui_kit::test]
    fn files_dropped_on_the_chat_attach_with_a_frame_meanwhile(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.pdf"), dir.path().join("b.txt"));
        std::fs::write(&a, b"pdf").unwrap();
        std::fs::write(&b, b"text").unwrap();
        let (backend, store, window) = open_chat(cx);
        let position = point(px(200.), px(200.));
        let paths = ExternalPaths(vec![a, b].into());

        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.dispatch_event(
                PlatformInput::FileDrop(FileDropEvent::Entered { position, paths }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::FileDrop(FileDropEvent::Pending { position }),
                cx,
            );
        })
        .unwrap();
        window
            .update(cx, |view, _, _| assert!(view.dropping, "the frame shows"))
            .unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.dispatch_event(
                PlatformInput::FileDrop(FileDropEvent::Submit { position }),
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();

        window
            .update(cx, |view, _, _| assert!(!view.dropping, "the frame goes"))
            .unwrap();
        store.read_with(cx, |store, _| {
            assert_eq!(store.attachments().items().len(), 2);
        });
        let mut calls = backend.calls();
        calls.sort_by_key(|call| format!("{call:?}"));
        assert_eq!(
            calls,
            [Call::Upload("a.pdf".into()), Call::Upload("b.txt".into())]
        );
    }

    #[gpui_kit::test]
    fn files_dragged_away_leave_nothing(cx: &mut TestAppContext) {
        let (backend, store, window) = open_chat(cx);
        let position = point(px(200.), px(200.));
        let paths = ExternalPaths(vec!["/files/a.pdf".into()].into());
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.dispatch_event(
                PlatformInput::FileDrop(FileDropEvent::Entered { position, paths }),
                cx,
            );
            window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Exited), cx);
        })
        .unwrap();
        window
            .update(cx, |view, _, _| assert!(!view.dropping))
            .unwrap();
        store.read_with(cx, |store, _| {
            assert!(store.attachments().items().is_empty());
        });
        assert_eq!(backend.calls(), []);
    }
}
