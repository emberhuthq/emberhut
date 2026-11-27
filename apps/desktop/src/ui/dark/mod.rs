//! The Dark room's window (the last row of the sidebar's Rooms, `emberhut://dark`): a plain chat
//! with a model, off the record, always dark (its own controls, [`parts`], whatever the app's
//! appearance). The answer streams in as it is written; answers are markdown with a
//! right-click Copy, the user's own lines plain text, selectable; the composer sends on Return
//! and grows to 8 lines; Clear history deletes everything said, once confirmed (Esc cancels).
//! The data is the store's (`store::dark`).

mod parts;

use std::rc::Rc;
use std::time::Duration;

use emberhut_api::theme::Appearance;
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::text::TextView;
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, ClipboardItem, Div, ElementId, Entity,
    FontWeight, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Point, ScrollHandle, Window,
    anchored, deferred, div, pulsating_between, px,
};

use crate::brand::APP_NAME;
use crate::motion;
use crate::shell::{WindowKind, windows};
use crate::store::Store;
use crate::store::dark::DarkData;
use crate::ui::a11y::Accessible as _;
use crate::ui::palette::Pal;
use crate::ui::prose::follow_web_link;
use crate::ui::reads;

/// How wide the talk and the composer grow at most.
const MOST_WIDE: f32 = 760.;

/// Registers the Dark room's window.
pub(crate) fn register(cx: &mut App) {
    windows::register_view(
        WindowKind::Dark,
        Rc::new(|store, window, cx| cx.new(|cx| DarkView::new(store, window, cx)).into()),
        cx,
    );
}

/// The Dark room's window.
pub(crate) struct DarkView {
    store: Entity<Store>,
    draft: Entity<TextareaState>,
    scroll: ScrollHandle,
    /// What the room held when last drawn (its messages, the answer's length), so it follows
    /// the answer down.
    seen: Option<(usize, Option<usize>)>,
    /// The window's title as last set.
    title: String,
    /// The right-click menu of an answer, while open: where, and the answer it copies.
    menu: Option<(Point<Pixels>, String)>,
    /// Clear history asks first: the question shows.
    confirming: bool,
}

impl DarkView {
    fn new(store: Entity<Store>, window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let draft = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 8)
                .submit_on_enter(true)
                .placeholder("Message")
        });
        cx.subscribe_in(
            &draft,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { shift: false, .. } => this.send(window, cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            },
        )
        .detach();
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        store.update(cx, Store::load_dark);
        draft.update(cx, |input, cx| input.focus(window, cx));
        Self {
            store,
            draft,
            scroll: ScrollHandle::new(),
            seen: None,
            title: String::new(),
            menu: None,
            confirming: false,
        }
    }

    /// Sends what the composer holds.
    fn send(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let text = self.draft.read(cx).value().to_string();
        if !self.store.read(cx).dark().can_send(&text) {
            return;
        }
        self.draft
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.store
            .update(cx, |store, cx| store.send_dark(&text, cx));
    }

    /// Asks before clearing the room (in the room's own question, which stays dark).
    fn ask_to_clear(&mut self, cx: &mut Context<'_, Self>) {
        self.menu = None;
        self.confirming = true;
        cx.notify();
    }

    /// The question's answer: clear the room, or not.
    fn answer_clear(&mut self, clear: bool, cx: &mut Context<'_, Self>) {
        self.confirming = false;
        if clear {
            self.store.update(cx, Store::clear_dark);
        }
        cx.notify();
    }

    /// Esc cancels the question, else closes the menu.
    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<'_, Self>) {
        if event.keystroke.key != "escape" {
            return;
        }
        if self.confirming {
            self.answer_clear(false, cx);
            cx.stop_propagation();
        } else if self.menu.take().is_some() {
            cx.notify();
            cx.stop_propagation();
        }
    }

    /// The question over the room.
    fn question(p: &Pal, cx: &mut Context<'_, Self>) -> Div {
        parts::confirm(
            "Clear the Dark room?",
            "Everything said here is deleted from the server. This can't be undone.",
            [
                parts::dialog_button("dark-clear-yes", "Clear history", Some(p.danger), p)
                    .on_click(cx.listener(|this, _, _, cx| this.answer_clear(true, cx))),
                parts::dialog_button("dark-clear-no", "Cancel", None, p)
                    .on_click(cx.listener(|this, _, _, cx| this.answer_clear(false, cx))),
            ],
            p,
        )
    }

    /// The open answer's menu: Copy.
    fn menu_view(
        at: Point<Pixels>,
        text: String,
        p: &Pal,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let item = parts::menu_item("dark-copy", IconName::Copy, "Copy", p).on_click(cx.listener(
            move |this, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                this.menu = None;
                cx.notify();
            },
        ));
        deferred(anchored().position(at).child(
            parts::menu_panel(p, [item.into_any_element()]).on_mouse_down_out(cx.listener(
                |this, _, _, cx| {
                    this.menu = None;
                    cx.notify();
                },
            )),
        ))
        .with_priority(1)
        .into_any_element()
    }
}

/// The palette the room draws with: the theme's dark one, whatever the app's appearance.
fn dark_palette(store: &Store) -> Pal {
    let mut page = store.theme().page().clone();
    page.choice.appearance = Appearance::Dark;
    Pal::new(&page, true)
}

impl Render for DarkView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let reduced = motion::reduced(cx);
        let store = self.store.read(cx);
        let p = dark_palette(store);
        let dark = store.dark();
        let seen = (dark.chat().len(), dark.writing().map(str::len));
        if self.seen != Some(seen) {
            self.seen = Some(seen);
            self.scroll.scroll_to_bottom();
        }
        let talk = talk(dark, &self.store, &p, reduced, cx);
        let ready = dark.can_send(&self.draft.read(cx).value());
        let (model, clearable) = (
            dark.model().to_owned(),
            !dark.chat().is_empty() && !dark.is_busy(),
        );
        let title = format!("{} · Off the record · {model}", WindowKind::Dark.title());
        if title != self.title {
            window.set_window_title(&title);
            self.title = title;
        }
        let head = head(&model, clearable, &p, cx);
        let composer = composer(&self.draft, ready, &p, cx);
        let menu = self
            .menu
            .clone()
            .map(|(at, text)| Self::menu_view(at, text, &p, cx));
        let question = self.confirming.then(|| Self::question(&p, cx));
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.background)
            .text_color(p.text)
            .on_key_down(cx.listener(Self::key_down))
            .child(head)
            .child(
                div()
                    .id("dark-talk")
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(div().flex().justify_center().child(talk)),
            )
            .child(div().flex().justify_center().child(composer))
            .children(menu)
            .children(question)
    }
}

/// The bar: the lock, Dark, "Off the record · the model", and Clear history.
fn head(model: &str, clearable: bool, p: &Pal, cx: &mut Context<'_, DarkView>) -> Div {
    div()
        .flex()
        .items_center()
        .h(px(48.))
        .px(px(12.))
        .border_b_1()
        .border_color(p.border)
        .child(div().w(px(32.)))
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(Icon::new(IconName::Lock).size(px(13.)).text_color(p.text))
                        .child("Dark"),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(p.muted)
                        .child(format!("Off the record · {model}")),
                ),
        )
        .child(
            parts::icon_button("dark-clear", IconName::Trash, "Clear history", clearable, p)
                .when(clearable, |d| {
                    d.on_click(cx.listener(|this, _, _, cx| this.ask_to_clear(cx)))
                }),
        )
}

/// The talk: the empty room's note, the messages, the answer being written, a problem.
fn talk(
    dark: &DarkData,
    store: &Entity<Store>,
    p: &Pal,
    reduced: bool,
    cx: &Context<'_, DarkView>,
) -> Div {
    let mut talk = div()
        .w_full()
        .max_w(px(MOST_WIDE))
        .flex()
        .flex_col()
        .gap(px(14.))
        .px(px(16.))
        .py(px(12.));
    if dark.is_empty() {
        talk = talk.child(empty(dark.place(), p));
    }
    for (i, said) in dark.chat().iter().enumerate() {
        // A message that starts in Arabic reads from the right edge.
        let from_right = reads::starts_right(&said.text);
        talk = talk.child(
            div()
                .when(from_right, Styled::text_right)
                .child(if said.mine {
                    mine(i, &said.text, store, p)
                } else {
                    theirs(&format!("dark-{i}"), &said.text, store, p, cx)
                }),
        );
    }
    match dark.writing() {
        Some("") => {
            talk = talk.child(
                div()
                    .id("dark-thinking-line")
                    .announced("Thinking…")
                    .child(thinking(p, reduced)),
            );
        }
        Some(writing) => {
            talk = talk.child(
                div()
                    .when(reads::starts_right(writing), Styled::text_right)
                    .child(theirs("dark-writing", writing, store, p, cx)),
            );
        }
        None => {}
    }
    if let Some(error) = dark.error() {
        talk = talk.child(
            div()
                .id("dark-problem")
                .announced(error.to_owned())
                .flex()
                .items_start()
                .gap(px(6.))
                .text_size(px(12.))
                .text_color(p.warn)
                .child(
                    Icon::new(IconName::TriangleAlert)
                        .size(px(13.))
                        .text_color(p.warn),
                )
                .child(error.to_owned()),
        );
    }
    talk
}

/// The empty room: what it is and where the user's words go.
fn empty(place: &str, p: &Pal) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(10.))
        .px(px(24.))
        .pt(px(80.))
        .child(Icon::new(IconName::Lock).size(px(34.)).text_color(p.faint))
        .child(
            div()
                .text_size(px(17.))
                .font_weight(FontWeight::BOLD)
                .child("Off the record"),
        )
        .child(
            div()
                .text_size(px(13.))
                .text_color(p.muted)
                .text_center()
                .child(format!(
                    "{APP_NAME} can't see this room, and the model here has none of your data \
                     or {APP_NAME}'s tools. What you write goes to {place} and stays on your \
                     server until you clear it."
                )),
        )
}

/// The user's line: on the right, as typed (not markdown), selectable. A link in it opens only
/// as a web or mail link ([`follow_web_link`]).
fn mine(index: usize, text: &str, store: &Entity<Store>, p: &Pal) -> AnyElement {
    let store = store.clone();
    div()
        .flex()
        .justify_end()
        .pl(px(48.))
        .child(
            div()
                .px(px(13.))
                .py(px(8.))
                .rounded(px(18.))
                .bg(p.surface_raised)
                .child(
                    TextView::markdown(
                        ElementId::Name(format!("dark-mine-{index}").into()),
                        crate::prose::literal::literal(text),
                    )
                    .selectable(true)
                    .on_link_click(move |url, _, _, cx| follow_web_link(&store, url, cx)),
                ),
        )
        .into_any_element()
}

/// The model's answer: markdown, with a right-click Copy. A link in it opens only as a web or
/// mail link ([`follow_web_link`]): the model writes it, and what it writes can be steered.
fn theirs(
    key: &str,
    text: &str,
    store: &Entity<Store>,
    p: &Pal,
    cx: &Context<'_, DarkView>,
) -> AnyElement {
    let copied = text.to_owned();
    let store = store.clone();
    div()
        .id(ElementId::Name(format!("{key}-row").into()))
        // Seen by the headless tests (a no-op outside them).
        .test_support()
        .w_full()
        .child(
            TextView::markdown(ElementId::Name(key.to_owned().into()), text.to_owned())
                .style(parts::answer_style(p))
                .on_link_click(move |url, _, _, cx| follow_web_link(&store, url, cx)),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                this.menu = Some((event.position, copied.clone()));
                cx.notify();
            }),
        )
        .into_any_element()
}

/// "Thinking…", pulsing (still when the system asks for less motion).
fn thinking(p: &Pal, reduced: bool) -> AnyElement {
    let line = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(13.))
        .text_color(p.muted)
        .child(
            Icon::new(IconName::Ellipsis)
                .size(px(14.))
                .text_color(p.muted),
        )
        .child("Thinking…");
    if reduced {
        line.into_any_element()
    } else {
        line.with_animation(
            "dark-thinking",
            Animation::new(Duration::from_millis(1400))
                .repeat()
                .with_easing(pulsating_between(0.35, 1.)),
            Styled::opacity,
        )
        .into_any_element()
    }
}

/// The composer: the field, and Send.
fn composer(
    draft: &Entity<TextareaState>,
    ready: bool,
    p: &Pal,
    cx: &mut Context<'_, DarkView>,
) -> Div {
    div()
        .w_full()
        .max_w(px(MOST_WIDE))
        .flex()
        .items_end()
        .gap(px(8.))
        .px(px(12.))
        .py(px(8.))
        .child(
            div()
                .relative()
                .flex_1()
                .min_w_0()
                .px(px(14.))
                .py(px(6.))
                .rounded(px(20.))
                .bg(p.surface_raised)
                .child(parts::colour_field(draft, p))
                .child(Textarea::new(draft).appearance(false).text_color(p.text)),
        )
        .child(
            div()
                .id("dark-send")
                .button_named("Send")
                .accessibility_id("dark send")
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(36.))
                .rounded_full()
                .bg(if ready { p.text } else { p.faint })
                .when(ready, Styled::cursor_pointer)
                .child(
                    Icon::new(IconName::ArrowUp)
                        .size(px(17.))
                        .text_color(p.background),
                )
                .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
        )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{TestAppContext, point};
    use serde_json::json;

    use super::*;
    use crate::backend::fake::FakeBackend;

    #[gpui_kit::test]
    fn the_room_opens_titled_with_its_model_and_says_where_words_go(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let backend = FakeBackend::new([]);
        backend.party.push_dark(Ok(serde_json::from_value(json!({
            "model": "Example 7B", "where": "Example 7B on a test machine", "busy": false,
            "chat": []
        }))
        .unwrap()));
        let store = cx.new(|_| Store::for_tests(Arc::new(backend)));
        let (view, cx) = cx.add_window_view(|window, cx| DarkView::new(store.clone(), window, cx));
        cx.run_until_parked();
        cx.update(TestWindowExt::render_frame);
        assert_eq!(
            view.read_with(cx, |view, _| view.title.clone()),
            "Dark · Off the record · Example 7B"
        );
        let place = cx.read(|cx| store.read(cx).dark().place().to_owned());
        assert_eq!(place, "Example 7B on a test machine");
        assert!(cx.read(|cx| store.read(cx).dark().is_empty()));
    }

    #[gpui_kit::test]
    fn clearing_asks_in_the_room_and_esc_says_no(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let backend = FakeBackend::new([]);
        backend.party.push_dark(Ok(serde_json::from_value(json!({
            "model": "Example 7B", "where": "a test machine", "busy": false,
            "chat": [{"id": 1, "role": "user", "text": "*hi*", "at": "2026-11-25T10:00:00.000Z"}]
        }))
        .unwrap()));
        let store = cx.new(|_| Store::for_tests(Arc::new(backend)));
        let (view, cx) = cx.add_window_view(|window, cx| DarkView::new(store.clone(), window, cx));
        cx.run_until_parked();
        view.update(cx, DarkView::ask_to_clear);
        cx.update(TestWindowExt::render_frame);
        assert!(view.read_with(cx, |view, _| view.confirming));
        cx.simulate_keystrokes("escape");
        assert!(
            !view.read_with(cx, |view, _| view.confirming),
            "Esc is Cancel"
        );
        view.update(cx, |view, cx| {
            view.menu = Some((Point::default(), "an answer".to_owned()));
            cx.notify();
        });
        cx.update(TestWindowExt::render_frame);
        cx.simulate_keystrokes("escape");
        assert!(
            view.read_with(cx, |view, _| view.menu.is_none()),
            "Esc closes the menu"
        );
    }

    #[gpui_kit::test]
    fn a_link_in_an_answer_opens_only_on_the_web(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let answer = |id: i64, link: &str| {
            json!({"id": id, "role": "assistant", "text": format!("[Read the notes]({link})"),
                   "at": "2026-11-25T10:00:00.000Z"})
        };
        let backend = FakeBackend::new([]);
        backend.party.push_dark(Ok(serde_json::from_value(json!({
            "model": "Example 7B", "where": "a test machine", "busy": false,
            "chat": [
                answer(1, "file:///etc/passwd"),
                answer(2, "x-example-app://open"),
                answer(3, "https://example.com/notes"),
            ]
        }))
        .unwrap()));
        let store = cx.new(|_| Store::for_tests(Arc::new(backend)));
        let (_view, cx) = cx.add_window_view(|window, cx| DarkView::new(store.clone(), window, cx));
        cx.run_until_parked();
        cx.update(TestWindowExt::render_frame);
        let on_the_link = point(px(6.), px(8.));
        for shut in ["dark-0-row", "dark-1-row"] {
            cx.update(|window, cx| window.click_at(shut, on_the_link, cx));
            assert_eq!(cx.opened_url(), None, "{shut}");
        }
        cx.update(|window, cx| window.click_at("dark-2-row", on_the_link, cx));
        assert_eq!(
            cx.opened_url().as_deref(),
            Some("https://example.com/notes")
        );
    }
}
