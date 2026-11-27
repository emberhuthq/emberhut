//! A message's bubble put together: its words (read once, [`Parsed`]) with its menu, the
//! message it replies to, its Reply button (shown while the pointer is over the message, by
//! the row's hover style alone, so moving the pointer redraws nothing else), its pictures and
//! its chips, each side's way.

use std::rc::Rc;

use emberhut_api::chat::ChatMessage;
use emberhut_api::ids::MessageId;
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Div, ElementId, Entity, Role, SharedString, Window, div, px, relative,
};
use jiff::ZonedDateTime;

use super::picture::picture;
use super::video::video;
use super::{file_chip, is_video, shown_files, shows_as_picture};
use crate::prose::parsed::Parsed;
use crate::store::Store;
use crate::ui::a11y::Accessible as _;
use crate::ui::chat::ChatView;
use crate::ui::message_menu::{self, OnReply};
use crate::ui::palette::Pal;
use crate::ui::prose::{self, Place};
use crate::ui::quote::{self, Surface};
use crate::ui::refs;

/// What the chat view knows of a message's row beyond the message: its text as read, whether
/// it is lit (a quote of it was just clicked), where its text goes, and what Reply and a click
/// on its quote do.
#[derive(Clone)]
pub(crate) struct RowState {
    /// Its text, read once.
    pub(crate) parsed: Rc<Parsed>,
    /// It was just jumped to.
    pub(crate) lit: bool,
    /// Where its text goes.
    pub(crate) place: Place,
    /// The chat's clock this frame, for the time in a quote.
    pub(crate) now: ZonedDateTime,
    /// Reply to it.
    pub(crate) on_reply: OnReply,
    /// A click on its quote: go to the message quoted.
    pub(crate) on_jump: OnJump,
}

/// What a click on a quote does: go to the message it quotes.
pub(crate) type OnJump = Rc<dyn Fn(MessageId, &mut Window, &mut App)>;

/// A message's parts, built once and laid out by its side.
struct Parts {
    /// Its words in their bubble (with its menu), if it has any.
    words: Option<AnyElement>,
    /// The message it replies to, when it isn't inside the bubble.
    quote: Option<AnyElement>,
    reply: AnyElement,
    pictures: Vec<AnyElement>,
    chips: Vec<AnyElement>,
    /// The tint while it is lit.
    lit: gpui_kit::Hsla,
}

/// The hover group of message `id`'s row.
fn group(id: MessageId) -> SharedString {
    SharedString::from(format!("message-row-{id}"))
}

/// A message: the user's as a bubble on the right, the assistant's on the left, each with the
/// message it replies to on top, its menu on a right-click and its Reply button beside it;
/// pictures and chips under it.
pub(crate) fn bubble(
    message: &ChatMessage,
    row: &RowState,
    store: &Entity<Store>,
    p: &Pal,
    cx: &Context<'_, ChatView>,
) -> Div {
    let parts = parts(message, row, store, p, cx);
    let column = div()
        .debug_selector(|| format!("message-{}", message.id))
        .group(group(message.id))
        .flex()
        .flex_col()
        .gap(px(if message.mine { 4. } else { 6. }))
        .w_full()
        .rounded(px(12.))
        .bg(parts.lit)
        .children(parts.quote);
    let line = div().flex().items_center().gap(px(6.)).w_full();
    if message.mine {
        column
            .items_end()
            .when_some(parts.words, |d, words| {
                d.child(line.justify_end().child(parts.reply).child(words))
            })
            .children(parts.pictures)
            .when(!parts.chips.is_empty(), |d| {
                d.child(
                    div()
                        .debug_selector(|| format!("chips-{}", message.id))
                        .flex()
                        .flex_wrap()
                        .justify_end()
                        .gap(px(6.))
                        .children(parts.chips),
                )
            })
    } else {
        column
            .items_start()
            .when_some(parts.words, |d, words| {
                d.child(line.child(words).child(parts.reply))
            })
            .children(parts.pictures)
            .when(!parts.chips.is_empty(), |d| {
                d.child(
                    div()
                        .debug_selector(|| format!("chips-{}", message.id))
                        .flex()
                        .flex_wrap()
                        .gap(px(6.))
                        .max_w(relative(0.82))
                        .children(parts.chips),
                )
            })
    }
}

fn parts(
    message: &ChatMessage,
    row: &RowState,
    store: &Entity<Store>,
    p: &Pal,
    cx: &Context<'_, ChatView>,
) -> Parts {
    let mut pictures = Vec::new();
    let mut chips = Vec::new();
    for (i, file) in shown_files(&message.files) {
        if is_video(file) {
            pictures.push(video(message, i, file, store, cx).into_any_element());
        } else if shows_as_picture(file) {
            pictures.push(picture(message, i, file, store, p, cx).into_any_element());
        } else {
            chips.push(file_chip(message, i, file, store, p, cx).into_any_element());
        }
    }
    let state = store.read(cx).state();
    for (i, reference) in message.refs.iter().enumerate() {
        chips.push(refs::chip(message, i, reference, store, p, state).into_any_element());
    }
    let has_words = !row.parsed.text().is_empty();
    let quote = message.reply.as_ref().map(|quoted| {
        let (on_jump, id) = (Rc::clone(&row.on_jump), quoted.id);
        let surface = if message.mine && has_words {
            Surface::OnMine
        } else {
            Surface::Plain
        };
        div()
            .id(ElementId::Name(format!("quote-{}", message.id).into()))
            .role(Role::Button)
            .hint("Shows the message it replies to")
            .cursor_pointer()
            .max_w(px(300.))
            .child(quote::view(quoted, surface, &row.now, store.read(cx), p))
            .on_click(move |_, window, cx| on_jump(id, window, cx))
    });
    // The user's bubble holds its quote; anywhere else it sits on top.
    let (inside, quote) = if message.mine && has_words {
        (quote, None)
    } else {
        (None, quote)
    };
    let menu = message_menu::menu(
        Rc::clone(row.parsed.text()),
        message.mine,
        store,
        Rc::clone(&row.on_reply),
    );
    let bubble = div()
        .id(ElementId::Name(format!("bubble-{}", message.id).into()))
        .flex()
        .flex_col()
        .gap(px(6.))
        .rounded(px(17.))
        .text_size(px(14.));
    let words = has_words.then(|| {
        if message.mine {
            bubble
                .max_w(relative(0.72))
                .px(px(13.))
                .py(px(8.))
                .bg(p.mine)
                .text_color(p.on_mine)
                .line_height(px(20.))
                .children(inside)
                .child(prose::mine(&row.parsed, &row.place, store, p))
                .context_menu(menu)
                .into_any_element()
        } else {
            bubble
                .max_w(relative(0.82))
                .px(px(14.))
                .py(px(9.))
                .bg(p.surface_raised)
                .line_height(px(21.))
                .child(prose::answer(&row.parsed, &row.place, store, p, cx))
                .context_menu(menu)
                .into_any_element()
        }
    });
    Parts {
        words,
        quote: quote.map(IntoElement::into_any_element),
        reply: message_menu::reply_button(
            ElementId::Name(format!("reply-{}", message.id).into()),
            group(message.id),
            p,
            Rc::clone(&row.on_reply),
        )
        .into_any_element(),
        pictures,
        chips,
        lit: if row.lit {
            p.warn.opacity(0.22)
        } else {
            gpui_kit::transparent_black()
        },
    }
}
