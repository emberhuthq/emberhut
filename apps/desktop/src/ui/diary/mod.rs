//! Quick notes and the diary: the Quick Note window (⇧⌘N, File › Quick Note, the chat's note
//! button, `emberhut://note`), the Diary window (⇧⌘D, File › Diary, the sidebar's More), and the
//! note editor both use. The notes themselves, kept on this device and sent when the server can
//! be reached, are the store's (`store::diary`).

mod button;
mod editor;
mod page;
mod quick_note;

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::{App, KeyBinding, actions};

pub(crate) use self::button::note_button;
use self::page::DiaryView;
use self::quick_note::QuickNoteView;
use crate::shell::{WindowKind, windows};

actions!(
    diary,
    [
        /// Opens the Quick Note window, from any window.
        OpenQuickNote,
        /// Opens the Diary window, from any window.
        OpenDiary,
        /// Saves the note being written.
        SaveNote,
        /// Closes the note being written.
        CloseNote,
        /// A new note, from the diary.
        NewNote,
    ]
);

/// Registers the Quick Note and Diary windows, their menu actions and their keys.
pub(crate) fn register(cx: &mut App) {
    windows::register_view(
        WindowKind::QuickNote,
        Rc::new(|store, window, cx| cx.new(|cx| QuickNoteView::new(store, window, cx)).into()),
        cx,
    );
    windows::register_view(
        WindowKind::Diary,
        Rc::new(|store, window, cx| cx.new(|cx| DiaryView::new(store, window, cx)).into()),
        cx,
    );
    cx.on_action(|_: &OpenQuickNote, cx| {
        windows::open_single(WindowKind::QuickNote, cx);
    });
    cx.on_action(|_: &OpenDiary, cx| {
        windows::open_single(WindowKind::Diary, cx);
    });
    cx.bind_keys([
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-shift-n", OpenQuickNote, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-n", OpenQuickNote, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-shift-d", OpenDiary, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-d", OpenDiary, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-enter", SaveNote, Some("NoteEditor")),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-enter", SaveNote, Some("NoteEditor")),
        KeyBinding::new("escape", CloseNote, Some("NoteEditor")),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-n", NewNote, Some("Diary")),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-n", NewNote, Some("Diary")),
    ]);
}
