//! Where the user reads in each room's chat: the place kept per room (back to it on return,
//! or to the newest if they were at the bottom), the "New messages" pill when an answer comes
//! while they read further up, and the lines over the composer (offline, the last action the
//! server refused). Also the empty chat's welcome, with three things to say.

use std::collections::HashMap;

use emberhut_api::ids::RoomId;
use gpui_kit::component::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Div, Entity, FontWeight, Hsla, ListOffset, ListState, SharedString, div, img, px,
};

use crate::assets::FACE;
use crate::brand::APP_NAME;
use crate::store::Store;
use crate::store::status::Tone;
use crate::ui::palette::Pal;
use crate::ui::widgets::icon;

/// What an empty chat offers to say: a click puts it in the composer.
pub(crate) const STARTERS: [&str; 3] = [
    "What should I look at today?",
    "What did you do while I was away?",
    "Check on the build in an hour",
];

/// Where the user was reading in each room they left, unless they were at its newest.
#[derive(Debug, Default)]
pub(crate) struct Places {
    at: HashMap<RoomId, ListOffset>,
}

impl Places {
    /// The user leaves `room`, at `top` in it, `at_end` when the list says they were at its
    /// newest (`None` when it can't tell: a short chat, or one not laid out yet).
    pub(crate) fn leave(&mut self, room: RoomId, at_end: Option<bool>, top: ListOffset) {
        if at_end == Some(false) {
            self.at.insert(room, top);
        } else {
            self.at.remove(&room);
        }
    }

    /// Where to show `room` on return: the place kept, else its newest (`None`).
    #[must_use]
    pub(crate) fn place_in(&self, room: RoomId) -> Option<ListOffset> {
        self.at.get(&room).copied()
    }

    /// Shows `room` in `list` where the user left it, or at its newest.
    pub(crate) fn return_to(&self, room: RoomId, list: &ListState) {
        match self.place_in(room) {
            Some(place) => list.scroll_to(place),
            None => list.scroll_to_end(),
        }
    }

    /// The chat in `list` switches from room `left` to room `to`, `rows` long: where the user
    /// left the first is kept, and the other opens where they left it (its newest if they were
    /// at the bottom).
    pub(crate) fn switch(
        &mut self,
        left: Option<RoomId>,
        to: Option<RoomId>,
        rows: usize,
        list: &ListState,
    ) {
        if let Some(left) = left {
            self.leave(left, list.is_scrolled_to_end(), list.logical_scroll_top());
        }
        list.reset(rows);
        if let Some(to) = to {
            self.return_to(to, list);
        }
    }
}

/// Whether new messages came below while the user reads further up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct NewBelow(bool);

impl NewBelow {
    /// Messages were added at the end; `at_end` whether the user was at the newest before,
    /// `mine` whether the newest is the user's own (they know about it).
    pub(crate) fn added(&mut self, at_end: Option<bool>, mine: bool) {
        if at_end == Some(false) && !mine {
            self.0 = true;
        }
    }

    /// The user reached the newest (scrolled down, clicked the pill, sent, switched rooms).
    pub(crate) fn seen(&mut self) {
        self.0 = false;
    }

    /// Whether the pill shows.
    #[must_use]
    pub(crate) fn shows(self) -> bool {
        self.0
    }
}

/// The "New messages" pill at the chat's foot; `on_click` scrolls to the newest.
pub(crate) fn pill(p: &Pal) -> gpui_kit::Stateful<Div> {
    div()
        .id("new-messages")
        .flex()
        .items_center()
        .gap(px(5.))
        .px(px(12.))
        .py(px(5.))
        .rounded_full()
        .bg(p.accent)
        .text_color(p.on_accent)
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .shadow_md()
        .cursor_pointer()
        .child(icon(IconName::ArrowDown, p.on_accent, 12.))
        .child("New messages")
}

/// A status line's colour.
#[must_use]
pub(crate) fn status_color(tone: Tone, p: &Pal) -> Hsla {
    match tone {
        Tone::Quiet => p.muted,
        Tone::Working => p.info,
        Tone::Warn => p.warn,
        Tone::Danger => p.danger,
    }
}

/// What needs the user, counted (no number at none), in the window bar while the sidebar
/// (which shows it) is hidden; lit while Needs you is `open` in the column, which a click opens
/// or shuts.
pub(crate) fn needs_counter(count: usize, open: bool, p: &Pal) -> gpui_kit::Stateful<Div> {
    div()
        .id("bar-needs")
        .when(open, |d| d.bg(p.accent_soft))
        .flex()
        .items_center()
        .gap(px(5.))
        .px(px(9.))
        .py(px(4.))
        .mr(px(6.))
        .rounded(px(12.))
        .cursor_pointer()
        .text_color(p.accent)
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .hover(|d| d.bg(p.hover))
        .child(icon(IconName::Inbox, p.accent, 13.))
        .when(count > 0, |d| d.child(crate::format::badge(count)))
}

/// The lines over the composer: offline once polls have failed a while, and the last action
/// the server refused (a click hides it).
pub(crate) fn banners(store: &Entity<Store>, p: &Pal, cx: &App) -> Option<Div> {
    let read = store.read(cx);
    let offline = read.offline().map(str::to_owned);
    let refused = read.action_problem().map(str::to_owned);
    if offline.is_none() && refused.is_none() {
        return None;
    }
    let line = |text: String| {
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(12.))
            .text_color(p.muted)
            .child(icon(IconName::TriangleAlert, p.warn, 12.))
            .child(text)
    };
    let store = store.clone();
    Some(
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .px(px(20.))
            .pt(px(6.))
            .children(offline.map(line))
            .children(refused.map(|refused| {
                line(format!("{refused} · click to hide"))
                    .id("action-problem")
                    .cursor_pointer()
                    .on_click(move |_, _, cx| store.update(cx, Store::hide_action_problem))
            })),
    )
}

/// An empty room's welcome: the face, one line, and the starters; `pick` puts one in the
/// composer.
pub(crate) fn welcome(
    p: &Pal,
    pick: impl Fn(SharedString, &mut gpui_kit::Window, &mut App) + Clone + 'static,
) -> Div {
    let starters = STARTERS.iter().enumerate().map(|(i, starter)| {
        let pick = pick.clone();
        Button::new(("starter", i))
            .outline()
            .label(*starter)
            .on_click(move |_, window, cx| pick(SharedString::new_static(starter), window, cx))
    });
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(16.))
        .child(img(FACE).size(px(88.)).rounded_full())
        .child(
            div()
                .max_w(px(300.))
                .text_center()
                .text_size(px(13.))
                .text_color(p.muted)
                .child(format!(
                    "{APP_NAME} watches your work and speaks up when something matters."
                )),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(10.))
                .pt(px(6.))
                .children(starters),
        )
}

#[cfg(test)]
mod tests {
    use gpui_kit::px;

    use super::*;

    fn at(item_ix: usize) -> ListOffset {
        ListOffset {
            item_ix,
            offset_in_item: px(4.),
        }
    }

    #[test]
    fn a_room_left_reading_above_comes_back_there() {
        let (one, two) = (RoomId::MAIN, RoomId::new(2));
        let mut places = Places::default();
        places.leave(one, Some(false), at(12));
        places.leave(two, Some(true), at(40));
        assert_eq!(places.place_in(one), Some(at(12)));
        assert_eq!(
            places.place_in(two),
            None,
            "it was at the newest: the newest"
        );
        places.leave(one, None, at(3));
        assert_eq!(places.place_in(one), None, "a short chat: the newest");
    }

    #[test]
    fn the_pill_shows_for_an_answer_below_and_goes_once_seen() {
        let mut below = NewBelow::default();
        below.added(Some(true), false);
        assert!(!below.shows(), "at the newest: it shows itself");
        below.added(Some(false), true);
        assert!(!below.shows(), "the user's own message");
        below.added(Some(false), false);
        assert!(below.shows());
        below.seen();
        assert!(!below.shows());
    }

    #[test]
    fn three_starters_for_an_empty_chat() {
        assert_eq!(STARTERS.len(), 3);
        assert!(STARTERS.iter().all(|starter| !starter.is_empty()));
    }
}
