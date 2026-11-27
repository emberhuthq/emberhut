//! The Quick Note window: the editor alone, with today's notes under it. Saving or closing it
//! closes the window; the diary is read again as it opens, so today's notes are fresh.

use gpui_kit::prelude::*;
use gpui_kit::{Entity, Window, div, px};

use super::editor::{EditorEvent, NoteEditor, Target};
use crate::assets::UI_FONT;
use crate::store::Store;
use crate::ui::palette::pal;

/// The window's view.
pub(crate) struct QuickNoteView {
    editor: Entity<NoteEditor>,
}

impl QuickNoteView {
    /// A quick note over `store`.
    pub(crate) fn new(
        store: Entity<Store>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        store.read(cx).sync_diary();
        let editor = cx.new(|cx| NoteEditor::new(store, Target::New { today: true }, window, cx));
        cx.subscribe_in(&editor, window, |_, _, event: &EditorEvent, window, _| {
            let EditorEvent::Done = event;
            window.remove_window();
        })
        .detach();
        Self { editor }
    }
}

impl Render for QuickNoteView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        div()
            .size_full()
            .bg(p.surface)
            .text_color(p.text)
            .font_family(UI_FONT)
            .text_size(px(13.))
            .child(self.editor.clone())
    }
}
