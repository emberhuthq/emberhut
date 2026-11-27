//! The calendar's chips: a work item's chip (its colour, icon and words; a click opens its
//! task) and an all-day event's chip. Each kind of thing's colours are the palette's
//! ([`calendar`]).

use emberhut_api::calendar::{CalendarEvent, CalendarItem};
use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::Icon;
use gpui_kit::prelude::*;
use gpui_kit::{ElementId, FontWeight, Stateful, div, px};

use crate::prose::direction;
use crate::store::calendar::words::{self, Tone};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::palette::{Pal, calendar};

/// A work item's icon: a hand for an ask's day, the sun for today's plan, a flag (filled when
/// hard) for a deadline.
fn item_icon(tone: Tone) -> AssetIcon {
    match tone {
        Tone::Decide => AssetIcon::Hand,
        Tone::Today => AssetIcon::Sun,
        Tone::Hard => AssetIcon::FlagTriangleRight,
        _ => AssetIcon::Flag,
    }
}

/// A work item's chip, `compact` in a week's strip or a month's day; `id` keeps it apart from
/// its neighbours.
pub(super) fn item_chip(
    item: &CalendarItem,
    compact: bool,
    id: ElementId,
    p: &Pal,
) -> Stateful<gpui_kit::Div> {
    let tone = Tone::of_item(item);
    let colors = calendar(tone, p);
    let (background, text) = if tone == Tone::Today {
        (colors.tint, colors.on_tint)
    } else {
        (colors.fill, colors.on_fill)
    };
    div()
        .id(id)
        // Read as its kind and words in full ("Deadline, K-12 Send it"), however short it shows.
        .button_named(a11y::line(&[tone.name(), &words::chip_text(item, false)]))
        .flex()
        .items_center()
        .gap(px(3.))
        .w_full()
        .min_w_0()
        .px(px(4.))
        .py(px(if compact { 1. } else { 2. }))
        .rounded(px(4.))
        .bg(background)
        .text_color(text)
        .text_size(px(if compact { 10. } else { 11. }))
        .font_weight(FontWeight::SEMIBOLD)
        .cursor_pointer()
        .child(
            Icon::new(item_icon(tone))
                .size(px(if compact { 8. } else { 9. }))
                .text_color(text),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .child(direction::marked(&words::chip_text(item, compact))),
        )
}

/// An event as a screen reader says it, however little of it shows: its kind, title and time
/// ("Meeting, Stand-up, 9:00 AM – 9:15 AM").
pub(super) fn event_words(event: &CalendarEvent, zone: &jiff::tz::TimeZone) -> String {
    a11y::line(&[
        words::kind_name(event),
        &event.title,
        &words::event_time(event, zone),
    ])
}

/// An all-day event's chip, filled with its colour.
pub(super) fn all_day_chip(
    event: &CalendarEvent,
    compact: bool,
    id: ElementId,
    p: &Pal,
) -> Stateful<gpui_kit::Div> {
    let colors = calendar(Tone::of_event(event), p);
    div()
        .id(id)
        .button_named(a11y::line(&[
            words::kind_name(event),
            &event.title,
            "all day",
        ]))
        .w_full()
        .min_w_0()
        .px(px(4.))
        .py(px(if compact { 1. } else { 2. }))
        .rounded(px(4.))
        .bg(colors.fill)
        .text_color(colors.on_fill)
        .text_size(px(if compact { 10. } else { 11. }))
        .font_weight(FontWeight::MEDIUM)
        .truncate()
        .cursor_pointer()
        .child(direction::marked(&event.title))
}
