//! The Party window (⇧⌘P, File › Party, the sidebar's More, `emberhut://party`): the user's
//! parties ([`list`]), a new one ([`new_party`]), and one party live ([`talk`]), where the
//! personas talk word by word and the user jumps in, steers, pauses, resumes and stops them.
//! The data is the store's (`store::party`); this window keeps its fields and scroll places.

mod list;
mod look;
mod new_party;
mod talk;
#[cfg(test)]
mod tests;

use std::rc::Rc;

use emberhut_api::ids::PartyId;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, FocusHandle, KeyBinding, ScrollHandle, Window, actions, div};

use crate::shell::{WindowKind, windows};
use crate::store::Store;
use crate::ui::palette::pal;

/// The key context of the Party window.
const CONTEXT: &str = "PartyWindow";

actions!(
    party,
    [
        /// Opens the Party window, from any window.
        OpenParty,
        /// Opens the New party form, in the Party window.
        NewParty,
    ]
);

/// Registers the Party window, its menu action and its keys.
pub(crate) fn register(cx: &mut App) {
    windows::register_view(
        WindowKind::Party,
        Rc::new(|store, window, cx| cx.new(|cx| PartyWindow::new(store, window, cx)).into()),
        cx,
    );
    cx.on_action(|_: &OpenParty, cx| {
        windows::open_single(WindowKind::Party, cx);
    });
    cx.bind_keys([
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-shift-p", OpenParty, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-p", OpenParty, None),
        #[cfg(target_os = "macos")]
        KeyBinding::new("alt-cmd-n", NewParty, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("alt-ctrl-n", NewParty, Some(CONTEXT)),
    ]);
}

/// Opens party `id` in the Party window (a link, the command line).
pub(crate) fn open_party(id: PartyId, cx: &mut App) {
    if let Some(store) = windows::store(cx) {
        store.update(cx, |store, cx| store.open_party(Some(id), cx));
    }
    windows::open_single(WindowKind::Party, cx);
}

/// The Party window's root: the list, or the party open, with the forms over them.
pub(crate) struct PartyWindow {
    store: Entity<Store>,
    focus: FocusHandle,
    /// The new party's topic.
    topic: Entity<TextareaState>,
    /// The persona being edited: its name and its role.
    name: Entity<InputState>,
    role: Entity<TextareaState>,
    /// The user's line in the party open.
    line: Entity<TextareaState>,
    /// The line steers (else it jumps in).
    steer: bool,
    list_scroll: ScrollHandle,
    form_scroll: ScrollHandle,
    talk_scroll: ScrollHandle,
    /// What the talk held when last drawn (the party, its lines, the live words), so it
    /// sticks to the bottom as it grows.
    talk_seen: Option<(PartyId, usize, usize)>,
    /// The window's title as last set.
    title: String,
}

impl PartyWindow {
    fn new(store: Entity<Store>, window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let topic = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 6)
                .placeholder("What should they talk about?")
        });
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Name"));
        let role = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 8)
                .placeholder("Who they are and how they talk")
        });
        let line = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 5)
                .submit_on_enter(true)
        });
        // Return and ⌘Return send.
        cx.subscribe_in(
            &line,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { shift: false, .. } => this.send_line(window, cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            },
        )
        .detach();
        // Start, Done and the editor's title follow what is typed.
        for field in [&topic, &role] {
            cx.subscribe(field, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            })
            .detach();
        }
        cx.subscribe(&name, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        // The list is read again whenever the window comes back to the front showing it.
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() && this.store.read(cx).party().room().is_none() {
                this.store.update(cx, Store::load_parties);
            }
        })
        .detach();
        // Closing the window stops following the party open.
        cx.on_release(|this, cx| {
            this.store.update(cx, Store::leave_parties);
        })
        .detach();
        store.update(cx, Store::load_parties);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut this = Self {
            store,
            focus,
            topic,
            name,
            role,
            line,
            steer: false,
            list_scroll: ScrollHandle::new(),
            form_scroll: ScrollHandle::new(),
            talk_scroll: ScrollHandle::new(),
            talk_seen: None,
            title: String::new(),
        };
        this.set_steer(false, window, cx);
        this
    }

    /// Opens the New party form, its topic empty.
    fn new_party(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.topic
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.store
            .update(cx, |store, cx| store.show_new_party(true, cx));
        self.topic.update(cx, |input, cx| input.focus(window, cx));
    }

    /// The line's mode: steer, or jump in; its placeholder says which.
    fn set_steer(&mut self, steer: bool, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.steer = steer;
        let placeholder = if steer {
            "Steer them, e.g. now argue the other side"
        } else {
            "Jump in, or name someone to ask them"
        };
        self.line.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
        });
        cx.notify();
    }

    /// Sends the user's line to the party open.
    fn send_line(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let text = self.line.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        self.line
            .update(cx, |input, cx| input.set_value("", window, cx));
        let steer = self.steer;
        self.store
            .update(cx, |store, cx| store.say_in_party(&text, steer, cx));
    }

    /// Esc is the sheet's Cancel: the persona's editor closes (back to the topic), else the
    /// New party sheet.
    fn key_down(
        &mut self,
        event: &gpui_kit::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if event.keystroke.key != "escape" {
            return;
        }
        let editing = match self.store.read(cx).party().draft() {
            None => return,
            Some(draft) => draft.editing.is_some(),
        };
        self.store.update(cx, |store, cx| {
            if editing {
                store.change_draft(|draft| draft.editing = None, cx);
            } else {
                store.show_new_party(false, cx);
            }
        });
        if editing {
            self.topic.update(cx, |topic, cx| topic.focus(window, cx));
        } else {
            window.focus(&self.focus, cx);
        }
        cx.stop_propagation();
    }

    /// Sets the window's title, when it changed: the party's, else Party.
    fn title_window(&mut self, window: &mut Window, cx: &App) {
        let title = self
            .store
            .read(cx)
            .party()
            .room()
            .and_then(|room| room.party())
            .map_or_else(
                || WindowKind::Party.title().to_owned(),
                |party| party.title.clone(),
            );
        if title != self.title {
            window.set_window_title(&title);
            self.title = title;
        }
    }
}

impl Render for PartyWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.title_window(window, cx);
        let p = pal(cx);
        let (open, form) = {
            let party = self.store.read(cx).party();
            (party.room().is_some(), party.draft().is_some())
        };
        let page = if open {
            talk::page(self, &p, cx).into_any_element()
        } else {
            self.talk_seen = None;
            list::page(self, &p, cx).into_any_element()
        };
        div()
            .id("party-window")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &NewParty, window, cx| this.new_party(window, cx)))
            .on_key_down(cx.listener(Self::key_down))
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(p.background)
            .text_color(p.text)
            .child(page)
            .children(form.then(|| new_party::sheet(self, &p, cx)).flatten())
    }
}
