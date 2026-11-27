//! "Pause until…": the day a project's room comes back by itself, picked on a calendar from
//! tomorrow on (a week out to start with), over the window. Until then the room folds under
//! Paused with no badges.
//!
//! The sheet holds the keyboard while it shows: Esc cancels it, and the window's shortcuts (the
//! rooms, the report, the pages) wait until it closes, so nothing changes under it. The focus
//! goes back where it was when it closes.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::calendar::{Calendar, CalendarState, Matcher};
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, FocusHandle, FontWeight, KeyBinding, Window, actions, div, px};
use jiff::ZonedDateTime;

use super::palette::pal;
use crate::store::Store;
use crate::store::room_actions::{Pausing, RoomAction, pause_days};

actions!(
    pause,
    [
        /// Closes the sheet without pausing.
        CancelPause,
    ]
);

/// The key context the sheet's keys work in.
const CONTEXT: &str = "PauseSheet";

/// The sheet's selector, for the tests (a no-op in a normal build).
pub(super) const SHEET: &str = "pause-sheet";

/// Binds the sheet's keys: Esc cancels it.
pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", CancelPause, Some(CONTEXT))]);
}

/// The sheet.
pub(crate) struct PauseSheet {
    store: Entity<Store>,
    calendar: Entity<CalendarState>,
    /// The room the calendar was last set for, so it starts a week out for each new one.
    set_for: Option<Pausing>,
    /// The sheet's focus, while it shows.
    focus: FocusHandle,
    /// What had the focus when the sheet opened, to give it back.
    focus_before: Option<FocusHandle>,
}

impl PauseSheet {
    /// The sheet over `store`; it shows while the store has a room to pause.
    pub(crate) fn new(
        store: Entity<Store>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        cx.observe_in(&store, window, |this, store, window, cx| {
            let pausing = store.read(cx).pausing().cloned();
            if pausing != this.set_for {
                let opens = this.set_for.is_none();
                this.set_for.clone_from(&pausing);
                if pausing.is_some() {
                    this.start_a_week_out(window, cx);
                    if opens {
                        this.focus_before = window.focused(cx);
                        window.focus(&this.focus, cx);
                    }
                } else if let Some(before) = this.focus_before.take() {
                    window.focus(&before, cx);
                }
                cx.notify();
            }
        })
        .detach();
        let calendar = cx.new(|cx| CalendarState::new(window, cx));
        Self {
            store,
            calendar,
            set_for: None,
            focus: cx.focus_handle(),
            focus_before: None,
        }
    }

    /// The calendar on a week from today, the days before tomorrow out of reach.
    fn start_a_week_out(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let (tomorrow, week) = pause_days(ZonedDateTime::now().date());
        self.calendar.update(cx, |calendar, cx| {
            calendar.set_disabled_matcher(Matcher::interval(Some(tomorrow), None), window, cx);
            calendar.set_date(week, window, cx);
        });
    }

    fn close(&self, cx: &mut Context<'_, Self>) {
        self.store
            .update(cx, |store, cx| store.set_pausing(None, cx));
    }

    fn pause(&self, cx: &mut Context<'_, Self>) {
        let Some(pausing) = self.set_for.clone() else {
            return;
        };
        let (tomorrow, _) = pause_days(ZonedDateTime::now().date());
        let day = self
            .calendar
            .read(cx)
            .date()
            .start()
            .filter(|day| *day >= tomorrow)
            .unwrap_or(tomorrow);
        self.store.update(cx, |store, cx| {
            store.act_on_room(
                pausing.project,
                &pausing.name,
                RoomAction::PauseUntil(day),
                cx,
            );
            store.set_pausing(None, cx);
        });
    }
}

impl Render for PauseSheet {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let Some(pausing) = self.set_for.clone() else {
            return div().into_any_element();
        };
        let p = pal(cx);
        let card = div()
            .id("pause-sheet")
            .debug_selector(|| SHEET.to_owned())
            .flex()
            .flex_col()
            .gap(px(12.))
            .w(px(360.))
            .p(px(20.))
            .rounded(px(12.))
            .bg(p.card)
            .border_1()
            .border_color(p.border)
            .shadow_lg()
            // A click inside the card is the card's, not the dimmed window's behind it.
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("Pause {}", pausing.name)),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(p.muted)
                    .child("Back on"),
            )
            .child(Calendar::new(&self.calendar))
            .child(div().text_size(px(12.)).text_color(p.muted).child(format!(
                "{} folds under Paused with no badges, and comes back by itself on that day.",
                pausing.name
            )))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        Button::new("pause-cancel")
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                    )
                    .child(
                        Button::new("pause-confirm")
                            .primary()
                            .label("Pause")
                            .on_click(cx.listener(|this, _, _, cx| this.pause(cx))),
                    ),
            );
        let backdrop = div()
            .id("pause-backdrop")
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &CancelPause, _, cx| this.close(cx)))
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui_kit::black().opacity(0.25))
            .on_click(cx.listener(|this, _, _, cx| this.close(cx)))
            .child(card);
        super::window::hold_shortcuts(backdrop).into_any_element()
    }
}
