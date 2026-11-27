//! A room's chat: the messages in a virtualised list that sticks to the bottom, the assistant's
//! live step while it works, the ask card over the composer, and the composer. Files dropped
//! anywhere on it attach to the composer. The composer keeps each room's draft, and takes words
//! put in from elsewhere (the starters of an empty chat among them).

pub(crate) mod ask;
pub(crate) mod attachments;
mod composer;
#[cfg(test)]
mod composer_tests;
mod dictation;
mod drop;
mod empty;
mod focus;
mod footer;
mod header;
mod jump;
mod paste;
mod paused;
pub(super) mod reading;
mod replying;
mod row;

use std::collections::HashSet;
use std::ops::Range;

use emberhut_api::chat::FileKind;
use emberhut_api::ids::{MessageId, RoomId};
use gpui_kit::Focusable as _;
use gpui_kit::component::input::{InputEvent, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Div, Entity, ListAlignment, ListState, Window, div, list, px};
use jiff::ZonedDateTime;

use super::message;
use super::palette::{Pal, pal};
use crate::dictation::Dictation;
use crate::prose::parsed::ParsedCache;
use crate::store::Store;
use crate::store::composer::placeholder;

pub(crate) use self::composer::bind_keys;

/// The most lines the field grows to before it scrolls.
const FIELD_LINES: usize = 6;

/// The chat view.
pub(crate) struct ChatView {
    store: Entity<Store>,
    list: ListState,
    input: Entity<TextareaState>,
    /// What the list holds: the room, how many messages, the first and last ids.
    shown: Shown,
    /// Event runs opened by a click, by their first message.
    open_events: HashSet<MessageId>,
    /// The pictures, by message and path, that had come when their rows were last measured.
    /// The list keeps the heights of the rows it measured out of sight (above or below the
    /// view), so a row whose picture comes is measured again, at the picture's size.
    sized: HashSet<(MessageId, String)>,
    /// The store's count of pictures come when [`Self::sized`] was last brought up to date;
    /// `None` to look again whatever it says.
    pictures_seen: Option<u64>,
    /// The clock for this frame's time stamps (read once a frame, not once a row).
    now: ZonedDateTime,
    /// Files are being dragged over the chat.
    dropping: bool,
    /// The composer holds more than blanks.
    has_text: bool,
    /// Where the user was reading in each room left.
    places: reading::Places,
    /// Whether new messages came below while the user reads further up.
    new_below: reading::NewBelow,
    /// Whether the sidebar is hidden: the bar then counts what needs the user.
    sidebar_hidden: bool,
    /// The room whose draft the field holds.
    composing_in: RoomId,
    /// The field's prompt as last set.
    prompt: String,
    /// Dictation into the field.
    dictation: Dictation,
    /// The messages of the room as they were read, each read again only when its text changes.
    parsed: ParsedCache,
    /// The messages shown whole, as plain text ("Show all").
    whole: HashSet<MessageId>,
    /// What the ask card and Main's line show, worked out when the store changes.
    ask: ask::AskArea,
    /// The message just jumped to from a quote of it, lit for a moment.
    lit: Option<MessageId>,
    /// The quote whose Copy was just pressed (its message and its place among its quotes).
    copied: Option<(MessageId, usize)>,
    /// A jump to a quoted message that waits for earlier pages.
    jump: Option<jump::Jump>,
    /// How wide an answer's text may be this frame, in pixels (from the list's last width: the
    /// list can't be asked while it lays its rows out).
    text_width: f32,
    /// The automation hooks' view of the chat.
    #[cfg(feature = "automation")]
    pub(crate) probe: crate::automation::ChatProbe,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Shown {
    room: Option<RoomId>,
    count: usize,
    first: Option<MessageId>,
    last: Option<MessageId>,
}

impl ChatView {
    /// The chat over `store`.
    pub(crate) fn new(
        store: Entity<Store>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        // The list follows the chat as it changes, not as it is drawn; the field follows the
        // room.
        cx.observe_in(&store, window, |this, _, window, cx| {
            this.sync(cx);
            this.follow_room(window, cx);
            this.continue_jump(cx);
            cx.notify();
        })
        .detach();
        let room = store.read(cx).room();
        let prompt = placeholder(store.read(cx).state(), room);
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, FIELD_LINES)
                .submit_on_enter(true)
                .placeholder(prompt.clone())
        });
        cx.subscribe_in(
            &input,
            window,
            |this, input, event: &InputEvent, window, cx| {
                match event {
                    // Return and Cmd-Return send; Shift-Return has started a new line.
                    InputEvent::PressEnter { shift: false, .. } => this.send(window, cx),
                    // The send button shows once there is text, and goes again without it: a
                    // redraw then, not on every key. A field emptied drops a dictation.
                    InputEvent::Change => {
                        let has_text = !input.read(cx).value().trim().is_empty();
                        if input.read(cx).value().is_empty() {
                            this.discard_dictation(cx);
                        }
                        if has_text != this.has_text {
                            this.has_text = has_text;
                            cx.notify();
                        }
                    }
                    _ => {}
                }
            },
        )
        .detach();
        // Back to the front: the newest, even if the user was reading above.
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.scroll_to_newest(cx);
            }
        })
        .detach();
        // Talk about a task starts the composer with the task named: taken once, here.
        cx.observe_in(&store, window, |this, store, window, cx| {
            if let Some(start) = store.update(cx, |store, _| store.take_composer_start()) {
                this.input
                    .update(cx, |input, cx| input.set_value(start, window, cx));
                let focus = this.input.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }
        })
        .detach();
        // A chat that goes away with words in its field isn't being written in any more.
        let view_id = cx.entity_id();
        cx.on_release(move |this: &mut Self, cx| {
            super::update_bar::note_writing(&this.store, view_id, false, cx);
        })
        .detach();
        let mut view = Self {
            store,
            list: ListState::new(0, ListAlignment::Bottom, px(600.)),
            input,
            shown: Shown::default(),
            open_events: HashSet::new(),
            sized: HashSet::new(),
            pictures_seen: None,
            now: ZonedDateTime::now(),
            dropping: false,
            has_text: false,
            places: reading::Places::default(),
            new_below: reading::NewBelow::default(),
            sidebar_hidden: false,
            composing_in: room,
            prompt,
            dictation: Dictation::system(),
            parsed: ParsedCache::default(),
            whole: HashSet::new(),
            ask: ask::AskArea::default(),
            lit: None,
            copied: None,
            jump: None,
            text_width: row::TEXT_WIDTH_AT_FIRST,
            #[cfg(feature = "automation")]
            probe: crate::automation::ChatProbe::default(),
        };
        view.sync(cx);
        view
    }

    /// Sends what the composer holds (Enter, or the send button), and empties it; nothing
    /// happens while an upload runs or there is nothing to send.
    fn send(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let text = self.input.read(cx).value().to_string();
        if self.store.update(cx, |store, cx| store.talk(&text, cx)) {
            self.input
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.has_text = false;
            self.discard_dictation(cx);
            self.scroll_to_newest(cx);
        }
    }

    /// Gives the keyboard focus to the composer.
    pub(crate) fn focus_composer(&self, window: &mut Window, cx: &mut App) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    /// Whether the composer has the keyboard focus.
    pub(crate) fn composer_focused(&self, window: &Window, cx: &App) -> bool {
        use gpui_kit::Focusable as _;

        self.input.read(cx).focus_handle(cx).is_focused(window)
    }

    /// The sidebar was shown or hidden.
    pub(crate) fn set_sidebar_hidden(&mut self, hidden: bool, cx: &mut Context<'_, Self>) {
        self.sidebar_hidden = hidden;
        cx.notify();
    }

    /// Scrolls to the newest message (after sending, back to the front, the pill).
    fn scroll_to_newest(&mut self, cx: &mut Context<'_, Self>) {
        self.list.scroll_to_end();
        self.new_below.seen();
        cx.notify();
    }

    /// Scrolls the list by `by` pixels (negative: up).
    #[cfg(feature = "automation")]
    pub(crate) fn scroll(&mut self, by: f32) {
        self.list.scroll_by(px(by));
    }

    /// Keeps the list in step with the room's chat: its rows ([`Self::sync_rows`]) and their
    /// pictures ([`Self::sync_pictures`]).
    fn sync(&mut self, cx: &mut Context<'_, Self>) {
        self.sync_rows(cx);
        self.sync_pictures(cx);
        self.ask = ask::AskArea::of(self.store.read(cx));
    }

    /// Messages appended splice in at the end (the list stays at the bottom if it was),
    /// earlier ones at the top; anything else starts over at the bottom, every row measured
    /// afresh. Row 0 is the "Show earlier" row.
    fn sync_rows(&mut self, cx: &mut Context<'_, Self>) {
        let store = self.store.read(cx);
        let messages = store.chats().messages(store.room());
        let now = Shown {
            room: Some(store.room()),
            count: messages.len(),
            first: messages.first().map(|m| m.id),
            last: messages.last().map(|m| m.id),
        };
        let was = self.shown;
        if now == was {
            return;
        }
        let grew = now.room == was.room && now.count > was.count && was.count > 0;
        if grew && now.first == was.first {
            let mine = messages.last().is_some_and(|message| message.mine);
            self.new_below.added(self.list.is_scrolled_to_end(), mine);
            self.list
                .splice(was.count + 1..was.count + 1, now.count - was.count);
        } else if grew && now.last == was.last {
            self.list.splice(1..1, now.count - was.count);
        } else if now.room != was.room {
            self.places
                .switch(was.room, now.room, now.count + 1, &self.list);
            self.new_below.seen();
        } else {
            self.list.reset(now.count + 1);
            self.list.scroll_to_end();
            self.sized.clear();
            self.pictures_seen = None;
        }
        self.forget_parsed(cx);
        self.shown = now;
    }

    /// Measures again the rows whose pictures have come since they were measured, keeping
    /// what is in view where it is. Nothing to do unless more pictures have come.
    fn sync_pictures(&mut self, cx: &mut Context<'_, Self>) {
        let store = self.store.read(cx);
        let arrived = store.pictures_come();
        if self.pictures_seen == Some(arrived) {
            return;
        }
        self.pictures_seen = Some(arrived);
        let mut rows = Vec::new();
        for (at, message) in store.chats().messages(store.room()).iter().enumerate() {
            let came: Vec<(MessageId, String)> = message
                .files
                .iter()
                .filter(|file| file.kind == FileKind::Image)
                .map(|file| (message.id, file.url.clone()))
                .filter(|key| !self.sized.contains(key) && store.picture(&key.1).is_some())
                .collect();
            if !came.is_empty() {
                self.sized.extend(came);
                rows.push(at + 1);
            }
        }
        for range in runs(&rows) {
            self.list.remeasure_items(range);
        }
    }

    fn render_item(&mut self, ix: usize, cx: &mut Context<'_, Self>) -> AnyElement {
        let p = pal(cx);
        if ix == 0 {
            return self.earlier_row(&p, cx).into_any_element();
        }
        let store = self.store.read(cx);
        let messages = store.chats().messages(store.room());
        let Some(at) = ix.checked_sub(1).filter(|&at| at < messages.len()) else {
            return div().into_any_element();
        };
        let item = message::Item::at(messages, at, &self.now);
        match item {
            message::Item::Folded => div().h(px(0.)).into_any_element(),
            message::Item::Message { message, stamp } => {
                let row = self.row_state(message, cx);
                let body = message::bubble(message, &row, &self.store, &p, cx).into_any_element();
                message::row(stamp, body, &p).into_any_element()
            }
            message::Item::Events { run, stamp } => {
                let Some(first) = run.first().map(|m| m.id) else {
                    return div().into_any_element();
                };
                let open = self.open_events.contains(&first);
                let parsed = self.parsed(run);
                let body = message::event_line(
                    run,
                    &parsed,
                    open,
                    &p,
                    cx.listener(move |this, _, _, cx| {
                        if !this.open_events.remove(&first) {
                            this.open_events.insert(first);
                        }
                        this.list.remeasure();
                        cx.notify();
                    }),
                );
                message::row(stamp, body.into_any_element(), &p).into_any_element()
            }
        }
    }

    fn earlier_row(&self, p: &Pal, cx: &mut Context<'_, Self>) -> Div {
        let store = self.store.read(cx);
        let room = store.room();
        let earlier = store.state().is_some_and(|state| state.earlier);
        let more = store.chats().has_earlier(room, earlier);
        let loading = store.chats().is_loading_earlier(room);
        div().flex().justify_center().py(px(10.)).when(more, |d| {
            d.child(
                div()
                    .id("earlier")
                    .cursor_pointer()
                    .px(px(10.))
                    .py(px(3.))
                    .rounded(px(10.))
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .hover(|d| d.bg(p.hover))
                    .child(if loading {
                        "Loading…"
                    } else {
                        "Show earlier"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.store.update(cx, Store::load_earlier);
                    })),
            )
        })
    }
}

impl Render for ChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.now = ZonedDateTime::now();
        super::update_bar::note_writing(&self.store, cx.entity_id(), self.has_text, cx);
        let laid_out = self.list.viewport_bounds().size.width;
        self.text_width = row::text_width(f32::from(laid_out));
        let p = pal(cx);
        let store = self.store.read(cx);
        let count = store.chats().messages(store.room()).len();
        #[cfg(feature = "automation")]
        {
            let room = store.room();
            self.probe.painted(room, count);
            if let Some(by) = self.probe.bench_frame() {
                self.list.scroll_by(px(by));
                window.request_animation_frame();
            }
        }
        if self.new_below.shows() && self.list.is_scrolled_to_end() == Some(true) {
            self.new_below.seen();
        }
        let body = if count == 0 {
            self.empty_body(&p, cx)
        } else {
            list(
                self.list.clone(),
                cx.processor(|this, ix, _window, cx| this.render_item(ix, cx)),
            )
            .flex_1()
            .w_full()
            .into_any_element()
        };
        let chat = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.surface)
            .child(self.header(&p, window, cx))
            .children(super::fallback_bar::bar(&self.store, &p, cx))
            .children(super::update_bar::bar(&self.store, &p, cx))
            .child(body)
            .children((count > 0).then(|| row::follow_width(self.list.clone(), laid_out)))
            .children(self.new_below.shows().then(|| {
                div().relative().child(
                    div()
                        .absolute()
                        .bottom(px(8.))
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(
                            reading::pill(&p)
                                .on_click(cx.listener(|this, _, _, cx| this.scroll_to_newest(cx))),
                        ),
                )
            }))
            .children(reading::banners(&self.store, &p, cx))
            .children(paused::banner(&self.store, &p, cx))
            .child(self.footer(&p, cx));
        self.drop_target(chat, &p, cx)
    }
}

/// `rows`, ascending, as runs of consecutive rows: `[1, 2, 3, 7]` is `1..4` and `7..8`.
fn runs(rows: &[usize]) -> Vec<Range<usize>> {
    let mut runs: Vec<Range<usize>> = Vec::new();
    for &row in rows {
        match runs.last_mut() {
            Some(run) if run.end == row => run.end = row + 1,
            _ => runs.push(row..row + 1),
        }
    }
    runs
}

#[cfg(test)]
mod ask_width;
#[cfg(test)]
mod picture_layout;
#[cfg(test)]
mod reflow;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{TestAppContext, WindowHandle};

    use super::*;
    use emberhut_api::settings::SettingsAction;
    use emberhut_api::talk::TalkRequest;

    use crate::backend::fake::{Call, FakeBackend};
    use crate::ui::palette::Pal;

    /// A chat view over a fake backend, in a headless window.
    pub(super) fn open_chat(
        cx: &mut TestAppContext,
    ) -> (Arc<FakeBackend>, Entity<Store>, WindowHandle<ChatView>) {
        open_chat_over(FakeBackend::new([]), cx)
    }

    /// A chat view over `backend`, in a headless window.
    pub(super) fn open_chat_over(
        backend: FakeBackend,
        cx: &mut TestAppContext,
    ) -> (Arc<FakeBackend>, Entity<Store>, WindowHandle<ChatView>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let backend = Arc::new(backend);
        let shared = Arc::clone(&backend) as Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::for_tests(shared));
        let window = cx.add_window(|window, cx| {
            // The window's column, where the chat opens pages.
            crate::ui::column::Column::new(store.clone(), window, cx);
            ChatView::new(store.clone(), window, cx)
        });
        (backend, store, window)
    }

    #[test]
    fn rows_are_measured_again_in_runs() {
        assert_eq!(runs(&[]), Vec::<Range<usize>>::new());
        assert_eq!(runs(&[4, 6]), [4..5, 6..7]);
        assert_eq!(runs(&[1, 2, 3, 7, 9, 10]), [1..4, 7..8, 9..11]);
    }

    #[gpui_kit::test]
    fn resume_switches_a_paused_assistant_back_on(cx: &mut TestAppContext) {
        let (backend, store, window) = open_chat(cx);
        store.update(cx, |store, cx| store.set_on_duty(false, cx));
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("resume", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(
            backend.calls(),
            [
                Call::Settings(SettingsAction::SetOn(false)),
                Call::Settings(SettingsAction::SetOn(true))
            ]
        );
        assert!(store.read_with(cx, |store, _| store.is_on_duty()));
    }

    #[gpui_kit::test]
    fn the_send_button_sends_what_the_composer_holds(cx: &mut TestAppContext) {
        let (backend, _store, window) = open_chat(cx);
        window
            .update(cx, |view, window, cx| {
                view.input
                    .update(cx, |input, cx| input.set_value("Hello", window, cx));
            })
            .unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("send", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(
            backend.calls(),
            [Call::Talk(TalkRequest::text(RoomId::MAIN, "Hello"))]
        );
        window
            .update(cx, |view, _, cx| {
                assert_eq!(view.input.read(cx).value(), "");
            })
            .unwrap();
    }
}
