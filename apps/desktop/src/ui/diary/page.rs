//! The Diary window: one section a day, newest on top (`Today`, `Yesterday`, `Wednesday`,
//! `Saturday 25 October`), the user's notes with their times and a mark on those not sent yet,
//! the assistant's entry for the day set apart with its face. A note the server refused keeps
//! its words, marked with why. A click on a note edits it in a sheet; its right-click menu edits
//! or deletes it, and a swipe to the left shows a Delete button at its end (a swipe to the right,
//! or a click elsewhere, puts it away), so a sideways flick never deletes a note by itself. A
//! date picker jumps to a day (or the nearest before it); ⌘N writes a new note.

use std::cell::RefCell;
use std::rc::Rc;

use emberhut_api::ids::NoteUid;
use emberhut_api::notes::Note;
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::calendar::Matcher;
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Div, ElementId, Entity, FocusHandle, FontWeight, ScrollHandle, Window, div, img, px,
};
use jiff::Timestamp;

use super::NewNote;
use super::editor::{EditorEvent, NoteEditor, Target};
use crate::assets::{FACE, UI_FONT};
use crate::brand::APP_NAME;
use crate::store::Store;
use crate::store::diary::book::landing;
use crate::store::diary::words;
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::controls::{tab_stop, tip};
use crate::ui::palette::{Pal, pal};
use crate::ui::reads;
use crate::ui::tasks::swipe::{Swipe, Swiped};

/// The swipe over the notes: one gesture at a time, on the note it started over.
#[derive(Clone, Debug, Default)]
struct NoteSwipes(Rc<RefCell<(Option<NoteUid>, Swipe)>>);

impl NoteSwipes {
    /// Takes in a scroll event over `uid`'s row; the swipe when it counted.
    fn feed(&self, uid: &NoteUid, event: &gpui_kit::ScrollEvent) -> Option<Swiped> {
        let mut held = self.0.borrow_mut();
        if held.0.as_ref() != Some(uid) {
            *held = (Some(uid.clone()), Swipe::default());
        }
        held.1.feed_event(event)
    }
}

/// The window's view.
pub(crate) struct DiaryView {
    store: Entity<Store>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    jump: Entity<DatePickerState>,
    /// The note being written or changed, over the diary.
    sheet: Option<Entity<NoteEditor>>,
    swipes: NoteSwipes,
    /// The note a swipe to the left showed the Delete button of.
    revealed: Option<NoteUid>,
}

impl DiaryView {
    /// The diary over `store`, read again from the server as it opens.
    pub(crate) fn new(
        store: Entity<Store>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        store.read(cx).sync_diary();
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        let zone = store.read(cx).diary().zone().clone();
        let today = Timestamp::now().to_zoned_datetime(zone).date();
        let jump = cx.new(|cx| {
            let mut picker = DatePickerState::new(window, cx)
                .disabled_matcher(Matcher::interval(None, Some(today)));
            picker.set_date(today, window, cx);
            picker
        });
        cx.subscribe_in(&jump, window, |this, _, event: &DatePickerEvent, _, cx| {
            let DatePickerEvent::Change(value) = event;
            if let Some(date) = value.start() {
                this.go_to(date.date(), cx);
            }
        })
        .detach();
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            store,
            focus,
            scroll: ScrollHandle::new(),
            jump,
            sheet: None,
            swipes: NoteSwipes::default(),
            revealed: None,
        }
    }

    /// A swipe over `uid`'s row: to the left it shows the note's Delete button, to the right it
    /// puts it away. Nothing is deleted until that button is pressed.
    fn swiped(&mut self, uid: &NoteUid, swiped: Swiped, cx: &mut Context<'_, Self>) {
        let revealed = match swiped {
            Swiped::Left => Some(uid.clone()),
            Swiped::Right => None,
        };
        if self.revealed != revealed {
            self.revealed = revealed;
            cx.notify();
        }
    }

    /// Scrolls to `date`'s day, or the nearest one before it.
    fn go_to(&self, date: jiff::civil::Date, cx: &Context<'_, Self>) {
        let pick = emberhut_api::time::Day::new(date.strftime("%Y-%m-%d").to_string());
        let book = self.store.read(cx).diary().book();
        if let Some(at) = landing(&book.days(), &pick) {
            self.scroll.scroll_to_top_of_item(at);
        }
    }

    /// Opens the editor over the diary for `target`.
    fn open_sheet(&mut self, target: Target, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.revealed = None;
        let store = self.store.clone();
        let editor = cx.new(|cx| NoteEditor::new(store, target, window, cx));
        cx.subscribe_in(
            &editor,
            window,
            |this, _, event: &EditorEvent, window, cx| {
                let EditorEvent::Done = event;
                this.sheet = None;
                window.focus(&this.focus, cx);
                cx.notify();
            },
        )
        .detach();
        self.sheet = Some(editor);
        cx.notify();
    }

    /// A new note, from the toolbar or ⌘N.
    fn new_note(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.sheet.is_none() {
            self.open_sheet(Target::New { today: false }, window, cx);
        }
    }

    /// The window's bar: its title, Go to day, and a new note.
    fn bar(&self, p: &Pal, cx: &mut Context<'_, Self>) -> Div {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(52.))
            .px(px(20.))
            .border_b_1()
            .border_color(p.border)
            .child(
                div()
                    .flex_1()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(15.))
                    .child("Diary"),
            )
            .child(
                div()
                    .w(px(150.))
                    .child(DatePicker::new(&self.jump).small().placeholder("Go to day")),
            )
            .child(
                Button::new("diary-new-note")
                    .ghost()
                    .icon(Icon::new(AssetIcon::SquarePen).size(px(15.)))
                    .accessibility_label("New note")
                    .tooltip("New note (⌘N)")
                    .on_click(cx.listener(|this, _, window, cx| this.new_note(window, cx))),
            )
    }

    /// A note of the user's: its time, its words, a mark while it isn't sent or after the
    /// server refused it (`refused`, why), and its Delete button once a swipe showed it.
    fn note_row(
        &self,
        note: &Note,
        unsent: bool,
        refused: Option<&str>,
        p: &Pal,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let zone = self.store.read(cx).diary().zone().clone();
        let (edit, uid) = (note.uid.clone(), note.uid.clone());
        let revealed = self.revealed.as_ref() == Some(&note.uid);
        let store = self.store.clone();
        // One line for the note: its time, its words, and whether it is sent.
        let spoken = a11y::line(&[
            &words::time(note, &zone),
            &note.text,
            if unsent { "not sent yet" } else { "" },
        ]);
        let row = div()
            .id(ElementId::Name(format!("note-{}", note.uid).into()))
            .button_named(spoken)
            .hint("Opens it to edit")
            .flex()
            .items_start()
            .gap(px(12.))
            .px(px(10.))
            .py(px(6.))
            .rounded(px(6.))
            .border_1()
            .cursor_pointer()
            .hover(|d| d.bg(p.hover))
            .child(
                div()
                    .w(px(74.))
                    .flex_none()
                    .text_size(px(12.5))
                    .text_color(p.muted)
                    .child(words::time(note, &zone)),
            )
            .child(note_text(note))
            .when(unsent, |d| {
                d.child(mark(
                    format!("unsent-{}", note.uid),
                    AssetIcon::CloudOff,
                    p.muted,
                    format!("Not sent to {APP_NAME} yet"),
                ))
            })
            .when_some(refused, |d, why| {
                d.child(mark(
                    format!("refused-{}", note.uid),
                    AssetIcon::TriangleAlert,
                    p.danger,
                    format!("Not saved: {why}. Change it to try again."),
                ))
            })
            .when(revealed, |d| {
                let (store, delete) = (store.clone(), uid.clone());
                d.child(
                    Button::new(ElementId::Name(format!("delete-{}", note.uid).into()))
                        .danger()
                        .small()
                        .label("Delete")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.revealed = None;
                            store.update(cx, |store, cx| store.delete_note(&delete, cx));
                        })),
                )
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                if this.revealed.take().is_some() {
                    // A click puts the Delete button away first.
                    cx.notify();
                    return;
                }
                this.open_sheet(Target::Edit(edit.clone()), window, cx);
            }))
            .on_scroll(cx.listener({
                let uid = uid.clone();
                move |this, event, _, cx| {
                    if let Some(swiped) = this.swipes.feed(&uid, event) {
                        this.swiped(&uid, swiped, cx);
                    }
                }
            }));
        let view = cx.entity().downgrade();
        tab_stop(row, p)
            .context_menu(move |menu, _, _| {
                let (view, store, edit, delete) =
                    (view.clone(), store.clone(), uid.clone(), uid.clone());
                menu.item(PopupMenuItem::new("Edit").on_click(move |_, window, cx| {
                    let edit = edit.clone();
                    if let Err(error) = view.update(cx, |this, cx| {
                        this.open_sheet(Target::Edit(edit), window, cx);
                    }) {
                        log::debug!("the diary closed before its menu acted: {error}");
                    }
                }))
                .item(PopupMenuItem::new("Delete").on_click(move |_, _, cx| {
                    store.update(cx, |store, cx| store.delete_note(&delete, cx));
                }))
            })
            .into_any_element()
    }
}

/// A note's words, from the right when they start in Arabic.
fn note_text(note: &Note) -> impl IntoElement {
    div()
        .id(ElementId::Name(format!("note-text-{}", note.uid).into()))
        .flex_1()
        .min_w_0()
        .text_size(px(13.5))
        .when(reads::starts_right(&note.text), Styled::text_right)
        .child(note.text.clone())
        // Seen by the headless tests (a no-op outside them).
        .test_support()
}

/// A note's mark (`icon` in `color`, named `id`), saying `said` under the pointer.
fn mark(id: String, icon: AssetIcon, color: gpui_kit::Hsla, said: String) -> impl IntoElement {
    tip(
        div()
            .id(ElementId::Name(id.into()))
            .pt(px(2.))
            .child(Icon::new(icon).size(px(12.)).text_color(color)),
        said,
    )
    // Seen by the headless tests (a no-op outside them).
    .test_support()
}

/// The assistant's entry for the day: its face and name, set apart from the user's notes.
fn entry(note: &Note, p: &Pal) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .mt(px(4.))
        .px(px(12.))
        .py(px(10.))
        .rounded(px(8.))
        .bg(p.accent_soft)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(img(FACE).size(px(22.)).rounded_full())
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(APP_NAME),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(p.muted)
                        .child("on the day"),
                ),
        )
        .child(
            div()
                .text_size(px(13.))
                .when(reads::starts_right(&note.text), Styled::text_right)
                .child(note.text.clone()),
        )
}

/// The diary with no notes.
fn empty(p: &Pal) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(6.))
        .py(px(60.))
        .px(px(30.))
        .child(
            Icon::new(AssetIcon::SquarePen)
                .size(px(28.))
                .text_color(p.faint),
        )
        .child(
            div()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("No notes yet"),
        )
        .child(
            div()
                .text_size(px(12.5))
                .text_color(p.muted)
                .text_center()
                .child(format!(
                    "Write a quick note any time: a thought, what happened, an idea, a to-do. \
             {APP_NAME} reads them and adds a few lines each evening."
                )),
        )
}

impl Render for DiaryView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        let store = self.store.read(cx);
        let zone = store.diary().zone().clone();
        let now = Timestamp::now().to_zoned_datetime(zone);
        let book = store.diary().book().clone();
        let problem = store.diary().problem().map(str::to_owned);
        let days = book.days();
        let mut list = div()
            .id("diary-days")
            .flex_1()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .px(px(12.))
            .pb(px(24.));
        if days.is_empty() {
            list = list.child(empty(&p));
        }
        for day in &days {
            let mut section = div().flex().flex_col().gap(px(2.)).pt(px(16.)).child(
                div()
                    .px(px(10.))
                    .pb(px(4.))
                    .text_size(px(17.))
                    .font_weight(FontWeight::BOLD)
                    .child(words::title(day.day, &now)),
            );
            for note in &day.notes {
                section = section.child(if note.is_assistants() {
                    entry(note, &p).into_any_element()
                } else {
                    self.note_row(
                        note,
                        book.is_unsent(&note.uid),
                        book.refusal(&note.uid),
                        &p,
                        cx,
                    )
                });
            }
            list = list.child(section);
        }
        let bar = self.bar(&p, cx);
        let problem = problem.map(|problem| {
            div()
                .id("diary-problem")
                .flex_none()
                .px(px(20.))
                .py(px(8.))
                .border_b_1()
                .border_color(p.border)
                .text_size(px(12.5))
                .text_color(p.danger)
                .child(problem)
                // Seen by the headless tests (a no-op outside them).
                .test_support()
        });
        div()
            .key_context("Diary")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &NewNote, window, cx| this.new_note(window, cx)))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.surface)
            .text_color(p.text)
            .font_family(UI_FONT)
            .text_size(px(13.))
            .child(bar)
            .children(problem)
            .child(list)
            .children(self.sheet.clone().map(|editor| sheet(editor, &p)))
    }
}

/// The editor over the diary, on a dimmed backdrop.
fn sheet(editor: Entity<NoteEditor>, p: &Pal) -> impl IntoElement {
    div()
        .id("diary-sheet")
        .occlude()
        .absolute()
        .inset_0()
        .flex()
        .justify_center()
        .items_start()
        .pt(px(60.))
        .bg(gpui_kit::black().opacity(0.25))
        .child(
            div()
                .w(px(440.))
                .max_h(px(520.))
                .rounded(px(12.))
                .border_1()
                .border_color(p.border)
                .bg(p.surface_raised)
                .shadow_lg()
                .child(editor),
        )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{TestAppContext, WindowHandle};

    use super::*;
    use crate::backend::fake::FakeBackend;

    fn open(cx: &mut TestAppContext) -> (Arc<FakeBackend>, Entity<Store>, WindowHandle<DiaryView>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let backend = Arc::new(FakeBackend::new([]));
        let shared = Arc::clone(&backend) as Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::for_tests(shared));
        store.update(cx, |store, cx| {
            store.save_note("Buy the part", None, cx);
        });
        let window = cx.add_window(|window, cx| DiaryView::new(store.clone(), window, cx));
        (backend, store, window)
    }

    #[gpui_kit::test]
    fn a_note_opens_in_the_sheet_and_the_diary_has_its_day(cx: &mut TestAppContext) {
        let (_, store, window) = open(cx);
        let uid = store.read_with(cx, |store, _| store.diary().book().shown()[0].uid.clone());
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("note-editor").is_none());
        })
        .unwrap();
        window
            .update(cx, |view, window, cx| {
                view.open_sheet(Target::Edit(uid.clone()), window, cx);
            })
            .unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("note-editor").is_some(), "the note's sheet");
            assert!(
                window
                    .try_find(ElementId::Name(format!("unsent-{uid}").into()))
                    .is_some(),
                "not sent yet"
            );
        })
        .unwrap();
    }

    /// A two-finger swipe over `uid`'s row, `across` points sideways (negative: to the left),
    /// then the fingers lift.
    fn swipe(window: WindowHandle<DiaryView>, uid: &NoteUid, across: f32, cx: &mut TestAppContext) {
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            let text = ElementId::Name(format!("note-text-{uid}").into());
            let delta = gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(across), px(0.)));
            window.scroll(text.clone(), delta, cx);
            let lifted = gpui_kit::ScrollEvent {
                position: window.find(text).bounds().center(),
                touch_phase: gpui_kit::TouchPhase::Ended,
                ..Default::default()
            };
            window.dispatch_event(gpui_kit::InputEvent::to_platform_input(lifted), cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn a_swipe_alone_never_deletes_a_note(cx: &mut TestAppContext) {
        let (_, store, window) = open(cx);
        let uid = store.read_with(cx, |store, _| store.diary().book().shown()[0].uid.clone());
        let delete = ElementId::Name(format!("delete-{uid}").into());
        swipe(window, &uid, -150., cx);
        store.read_with(cx, |store, _| {
            assert!(store.diary().book().note(&uid).is_some(), "still there");
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find(delete.clone()).is_some(), "Delete shows");
        })
        .unwrap();

        // A swipe back puts it away; another shows it, and only its press deletes.
        swipe(window, &uid, 150., cx);
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find(delete.clone()).is_none(), "put away");
        })
        .unwrap();
        swipe(window, &uid, -150., cx);
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click(delete.clone(), cx);
        })
        .unwrap();
        cx.run_until_parked();
        store.read_with(cx, |store, _| {
            assert!(
                store.diary().book().note(&uid).is_none(),
                "deleted on its press"
            );
        });
    }

    #[gpui_kit::test]
    fn a_note_the_server_refused_is_marked(cx: &mut TestAppContext) {
        let (backend, store, window) = open(cx);
        backend.days.push_save(Err(crate::backend::Error::Refused {
            status: Some(400),
            message: Some("Too long".into()),
        }));
        store.update(cx, Store::start_diary);
        cx.run_until_parked();
        let uid = store.read_with(cx, |store, _| store.diary().book().shown()[0].uid.clone());
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window
                    .try_find(ElementId::Name(format!("refused-{uid}").into()))
                    .is_some(),
                "marked"
            );
            assert!(window.try_find("diary-problem").is_some(), "and said");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn the_new_note_button_opens_an_empty_sheet(cx: &mut TestAppContext) {
        let (_, _, window) = open(cx);
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("diary-new-note", cx);
            window.render_frame(cx);
            window.render_frame(cx);
            assert!(window.try_find("note-editor").is_some());
        })
        .unwrap();
    }
}
