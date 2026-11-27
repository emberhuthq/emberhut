//! The calendar's colours and chips: a work item's chip (its colour, icon and words; a click
//! opens its task), an all-day event's chip, and the colour of each kind of thing.

use emberhut_api::calendar::{CalendarEvent, CalendarItem};
use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::Icon;
use gpui_kit::prelude::*;
use gpui_kit::{ElementId, FontWeight, Hsla, Stateful, div, px, rgb, white};

use crate::prose::direction;
use crate::store::calendar::words::{self, Tone};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::palette::Pal;

/// A tone's colour: the system's blue, teal and green for events; orange, red, the accent and
/// indigo for work.
#[must_use]
pub(super) fn color(tone: Tone, p: &Pal) -> Hsla {
    match tone {
        Tone::Meeting => rgb(0x000A_84FF).into(),
        Tone::Event => rgb(0x0030_B0C7).into(),
        Tone::Focus => rgb(0x0034_C759).into(),
        Tone::Deadline => rgb(0x00FF_9500).into(),
        Tone::Hard => rgb(0x00FF_3B30).into(),
        Tone::Decide => p.accent,
        Tone::Today => rgb(0x0058_56D6).into(),
    }
}

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
    let fill = color(tone, p);
    let (background, text) = if tone == Tone::Today {
        (fill.opacity(0.16), fill)
    } else {
        (fill, white())
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
    let fill = color(Tone::of_event(event), p);
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
        .bg(fill.opacity(0.85))
        .text_color(white())
        .text_size(px(if compact { 10. } else { 11. }))
        .font_weight(FontWeight::MEDIUM)
        .truncate()
        .cursor_pointer()
        .child(direction::marked(&event.title))
}
