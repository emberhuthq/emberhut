//! The note editor: a quick note (a field with the keyboard in it, "What's on your mind?", and
//! today's notes under it) or a note being changed. Save (⌘↩) keeps it and closes; Close (Esc)
//! closes, and a quick note closed unsaved keeps what was written for the next one. A text
//! longer than the server takes can't be saved: Save is off and the editor says by how much. Its owner
//! (the Quick Note window, the diary's sheet) hears [`EditorEvent::Done`] and closes it.

use emberhut_api::ids::NoteUid;
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::{Disableable as _, Icon};
use gpui_kit::prelude::*;
use gpui_kit::{App, Div, Entity, EventEmitter, FocusHandle, FontWeight, Window, div, px};
use jiff::{Timestamp, ZonedDateTime};

use super::{CloseNote, SaveNote};
use crate::store::Store;
use crate::store::diary::book::over_limit;
use crate::store::diary::words;
use crate::ui::a11y::Accessible as _;
use crate::ui::palette::{Pal, pal};
use crate::ui::reads;

/// What the editor writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// A new note; with `today`, today's notes show under the field.
    New {
        /// Whether today's notes show under the field.
        today: bool,
    },
    /// The note with this uid.
    Edit(NoteUid),
}

/// What the editor tells its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditorEvent {
    /// Saved or closed: it goes.
    Done,
}

/// The editor.
pub(crate) struct NoteEditor {
    store: Entity<Store>,
    target: Target,
    field: Entity<TextareaState>,
    focus: FocusHandle,
    /// Saved: nothing is kept as a draft, and a second ⌘↩ does nothing.
    saved: bool,
}

impl EventEmitter<EditorEvent> for NoteEditor {}

impl NoteEditor {
    /// An editor for `target`, the keyboard in its field.
    pub(crate) fn new(
        store: Entity<Store>,
        target: Target,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let placeholder = match target {
            Target::New { .. } => "What's on your mind?",
            Target::Edit(_) => "Note",
        };
        let text = match &target {
            Target::New { .. } => store.read(cx).diary().book().draft().to_owned(),
            Target::Edit(uid) => store
                .read(cx)
                .diary()
                .book()
                .note(uid)
                .map_or_default(|note| note.text.clone()),
        };
        let field = cx.new(|cx| {
            let mut field = TextareaState::new(window, cx)
                .placeholder(placeholder)
                .auto_grow(3, 14);
            field.set_value(text, window, cx);
            field
        });
        cx.subscribe_in(
            &field,
            window,
            |this, _, event: &InputEvent, _, cx| match event {
                InputEvent::PressEnter {
                    secondary: true, ..
                } => this.save(cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            },
        )
        .detach();
        field.update(cx, |field, cx| field.focus(window, cx));
        // A quick note closed without saving, however it closed, keeps what was written.
        cx.on_release(|this, cx| this.keep_draft(cx)).detach();
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        Self {
            store,
            target,
            field,
            focus: cx.focus_handle(),
            saved: false,
        }
    }

    /// The text written.
    fn text(&self, cx: &App) -> String {
        self.field.read(cx).value().to_string()
    }

    /// Whether Save does anything: a new note needs words; an edit may empty a note (deleting it);
    /// neither may be longer than the server takes.
    fn can_save(&self, cx: &App) -> bool {
        let text = self.text(cx);
        over_limit(&text).is_none()
            && (matches!(self.target, Target::Edit(_)) || !text.trim().is_empty())
    }

    /// Saves and closes.
    pub(crate) fn save(&mut self, cx: &mut Context<'_, Self>) {
        if self.saved || !self.can_save(cx) {
            return;
        }
        self.saved = true;
        let text = self.text(cx);
        let editing = match &self.target {
            Target::New { .. } => None,
            Target::Edit(uid) => Some(uid.clone()),
        };
        self.store.update(cx, |store, cx| {
            store.save_note(&text, editing.as_ref(), cx);
            if editing.is_none() {
                store.keep_note_draft(String::new(), cx);
            }
        });
        cx.emit(EditorEvent::Done);
    }

    /// Closes without saving.
    pub(crate) fn close(cx: &mut Context<'_, Self>) {
        cx.emit(EditorEvent::Done);
    }

    /// A new note not saved keeps its words as the draft.
    fn keep_draft(&self, cx: &mut App) {
        if self.saved || !matches!(self.target, Target::New { .. }) {
            return;
        }
        let text = self.text(cx);
        self.store
            .update(cx, |store, cx| store.keep_note_draft(text, cx));
    }

    /// The title: Quick note, or the edited note's day.
    fn title(&self, cx: &App) -> String {
        match &self.target {
            Target::New { .. } => "Quick note".to_owned(),
            Target::Edit(uid) => {
                let store = self.store.read(cx);
                let zone = store.diary().zone().clone();
                let now = Timestamp::now().to_zoned_datetime(zone);
                store
                    .diary()
                    .book()
                    .note(uid)
                    .map_or_else(|| "Note".to_owned(), |note| words::title(&note.day, &now))
            }
        }
    }
}

/// Today's own notes, newest first, under a new note's field.
fn today(store: &Store, p: &Pal) -> Option<Div> {
    let zone = store.diary().zone();
    let now: ZonedDateTime = Timestamp::now().to_zoned_datetime(zone.clone());
    let day = words::today(now.timestamp(), zone);
    let notes = store.diary().book().own_notes_of(&day);
    if notes.is_empty() {
        return None;
    }
    let mut list = div().flex().flex_col().gap(px(8.)).child(
        div()
            .text_size(px(11.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(p.muted)
            .child("Today"),
    );
    for note in notes {
        list = list.child(
            div()
                .flex()
                .items_start()
                .gap(px(10.))
                .child(
                    div()
                        .w(px(64.))
                        .flex_none()
                        .text_size(px(11.5))
                        .text_color(p.faint)
                        .child(words::time(note, zone)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(12.5))
                        .text_color(p.muted)
                        .line_clamp(2)
                        .when(reads::starts_right(&note.text), Styled::text_right)
                        .child(note.text.clone()),
                ),
        );
    }
    Some(list)
}

/// What the editor says of a text `over` characters longer than the server takes.
fn too_long(over: usize, p: &Pal) -> impl IntoElement {
    let words = if over == 1 {
        "1 character".to_owned()
    } else {
        format!("{over} characters")
    };
    div()
        .id(TOO_LONG)
        .text_size(px(12.))
        .text_color(p.danger)
        .child(format!(
            "Too long to save: take out {words} (a note holds {} at most).",
            emberhut_api::notes::MAX_NOTE_LEN
        ))
        // Seen by the headless tests (a no-op outside them).
        .test_support()
}

/// The element id of what the editor says of a text too long to save.
const TOO_LONG: &str = "note-too-long";

impl Render for NoteEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        let can_save = self.can_save(cx);
        let over = over_limit(&self.text(cx)).map(|over| too_long(over, &p));
        let title = self.title(cx);
        let named = title.clone();
        let list = match self.target {
            Target::New { today: true } => today(self.store.read(cx), &p),
            Target::New { today: false } | Target::Edit(_) => None,
        };
        let bar = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                Button::new("note-close")
                    .ghost()
                    .icon(Icon::new(AssetIcon::X).size(px(14.)))
                    .accessibility_label("Close")
                    .tooltip("Close (Esc)")
                    .on_click(cx.listener(|_, _, _, cx| Self::close(cx))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(14.))
                    .child(title),
            )
            .child(
                Button::new("note-save")
                    .primary()
                    .label("Save")
                    .disabled(!can_save)
                    .tooltip("Save (⌘↩)")
                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
            );
        div()
            .id("note-editor")
            .read_as(gpui_kit::Role::Group, named)
            .key_context("NoteEditor")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &SaveNote, _, cx| this.save(cx)))
            .on_action(cx.listener(|_, _: &CloseNote, _, cx| Self::close(cx)))
            .flex()
            .flex_col()
            .gap(px(16.))
            .p(px(18.))
            .size_full()
            .overflow_y_scroll()
            .child(bar)
            .child(div().text_size(px(16.)).child(Textarea::new(&self.field)))
            .children(over)
            .children(list)
            // Seen by the headless tests (a no-op outside them).
            .test_support()
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::sync::Arc;

    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{TestAppContext, WindowHandle};

    use super::*;
    use crate::backend::fake::FakeBackend;

    fn open(cx: &mut TestAppContext) -> (Entity<Store>, WindowHandle<NoteEditor>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let backend = Arc::new(FakeBackend::new([])) as Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::for_tests(backend));
        let window = cx.add_window(|window, cx| {
            NoteEditor::new(store.clone(), Target::New { today: true }, window, cx)
        });
        (store, window)
    }

    fn write(window: WindowHandle<NoteEditor>, text: &'static str, cx: &mut TestAppContext) {
        window
            .update(cx, |editor, window, cx| {
                editor
                    .field
                    .update(cx, |field, cx| field.set_value(text, window, cx));
            })
            .unwrap();
    }

    #[gpui_kit::test]
    fn save_keeps_the_note_and_says_done(cx: &mut TestAppContext) {
        let (store, window) = open(cx);
        let done = Rc::new(std::cell::Cell::new(false));
        let heard = Rc::clone(&done);
        window
            .update(cx, |_, window, cx| {
                let editor = cx.entity();
                cx.subscribe_in(&editor, window, move |_, _, _: &EditorEvent, _, _| {
                    heard.set(true);
                })
                .detach();
            })
            .unwrap();
        write(window, "  Buy the part  ", cx);
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("note-save", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(done.get());
        store.read_with(cx, |store, _| {
            let book = store.diary().book();
            assert_eq!(book.shown()[0].text, "Buy the part");
            assert_eq!(book.draft(), "");
        });
    }

    #[gpui_kit::test]
    fn a_note_too_long_for_the_server_cant_be_saved_and_says_why(cx: &mut TestAppContext) {
        let (store, window) = open(cx);
        let long: &'static str = "x".repeat(emberhut_api::notes::MAX_NOTE_LEN + 3).leak();
        write(window, long, cx);
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find(TOO_LONG).is_some(), "it says why");
            window.click("note-save", cx);
        })
        .unwrap();
        window
            .update(cx, |editor, _, cx| {
                editor.save(cx);
                assert!(!editor.saved);
            })
            .unwrap();
        cx.run_until_parked();
        store.read_with(cx, |store, _| {
            assert!(store.diary().book().shown().is_empty(), "nothing saved");
        });
        cx.update_window(window.into(), |_, window, _| window.remove_window())
            .unwrap();
        cx.run_until_parked();
        store.read_with(cx, |store, _| {
            assert_eq!(
                store.diary().book().draft().len(),
                long.len(),
                "its words wait as the draft"
            );
        });
    }

    #[gpui_kit::test]
    fn a_quick_note_closed_unsaved_keeps_its_words(cx: &mut TestAppContext) {
        let (store, window) = open(cx);
        write(window, "Half a thought", cx);
        cx.update_window(window.into(), |_, window, _| window.remove_window())
            .unwrap();
        cx.run_until_parked();
        store.read_with(cx, |store, _| {
            assert_eq!(store.diary().book().draft(), "Half a thought");
            assert!(store.diary().book().shown().is_empty(), "nothing saved");
        });
        let again = cx.add_window(|window, cx| {
            NoteEditor::new(store.clone(), Target::New { today: true }, window, cx)
        });
        again
            .update(cx, |editor, _, cx| {
                assert_eq!(
                    editor.text(cx),
                    "Half a thought",
                    "the next one starts with it"
                );
            })
            .unwrap();
    }
}
