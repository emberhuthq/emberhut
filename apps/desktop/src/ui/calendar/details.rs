//! What opens over the calendar: an event's details (its title, day and time, its calendar,
//! kind, people, place and notes, the words selectable), the colours' legend, and the line
//! saying some days show what was last seen.

use emberhut_api::calendar::CalendarEvent;
use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{Div, FontWeight, div, px};
use jiff::tz::TimeZone;

use super::CalendarView;
use crate::store::calendar::words::{self, LEGEND_NOTE, Tone};
use crate::ui::a11y::Accessible as _;
use crate::ui::palette::{Pal, calendar};
use crate::ui::settings::parts::selectable;

/// The server's note over the days (the calendar's source didn't answer), with a moon.
pub(super) fn note_line(note: &str, p: &Pal) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(5.))
        .py(px(4.))
        .text_size(px(11.5))
        .text_color(p.muted)
        .child(
            Icon::new(AssetIcon::MoonStar)
                .size(px(12.))
                .text_color(p.muted),
        )
        .child(note.to_owned())
}

/// The legend, under the bar's (i).
pub(super) fn legend(p: &Pal, cx: &mut Context<'_, CalendarView>) -> impl IntoElement {
    let mut card = div()
        .id("calendar-legend-card")
        .read_as(gpui_kit::Role::Group, "What the colours mean")
        .occlude()
        .absolute()
        .top(px(46.))
        .right(px(12.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(16.))
        .rounded(px(10.))
        .border_1()
        .border_color(p.border)
        .bg(p.surface_raised)
        .shadow_lg()
        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
            this.legend = false;
            cx.notify();
        }));
    for tone in Tone::LEGEND {
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .size(px(14.))
                        .rounded(px(3.))
                        .bg(calendar(tone, p).fill),
                )
                .child(tone.name()),
        );
    }
    card.child(
        div()
            .pt(px(4.))
            .text_size(px(11.5))
            .text_color(p.muted)
            .child(LEGEND_NOTE),
    )
}

/// A line of the details: its name and its value.
fn line(name: &'static str, value: impl IntoElement, p: &Pal) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(12.))
        .py(px(6.))
        .border_b_1()
        .border_color(p.border)
        .child(div().w(px(90.)).flex_none().text_color(p.muted).child(name))
        .child(div().flex_1().min_w_0().child(value))
}

/// An event's head (its colour, title, day and time) and its lines (calendar, kind, people,
/// place).
fn facts(event: &CalendarEvent, zone: &TimeZone, p: &Pal) -> (Div, Div) {
    let fill = calendar(Tone::of_event(event), p).fill;
    let place = event
        .location
        .clone()
        .filter(|place| !place.trim().is_empty());
    let head = div()
        .flex()
        .items_start()
        .gap(px(10.))
        .child(div().w(px(6.)).h(px(40.)).rounded(px(3.)).bg(fill))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(17.))
                        .font_weight(FontWeight::BOLD)
                        .child(selectable("event-title", &event.title)),
                )
                .child(
                    div()
                        .text_color(p.muted)
                        .child(words::event_day(event, zone)),
                )
                .child(
                    div()
                        .text_color(p.muted)
                        .child(words::event_time(event, zone)),
                ),
        );
    let mut lines = div()
        .flex()
        .flex_col()
        .child(line("Calendar", event.calendar.clone(), p))
        .child(line("Kind", words::kind_name(event), p));
    if event.people > 0 {
        lines = lines.child(line("People", event.people.to_string(), p));
    }
    if let Some(place) = place {
        lines = lines.child(line("Where", selectable("event-where", &place), p));
    }
    (head, lines)
}

/// An event's details, over the calendar.
pub(super) fn event_sheet(
    event: &CalendarEvent,
    zone: &TimeZone,
    p: &Pal,
    cx: &mut Context<'_, CalendarView>,
) -> impl IntoElement {
    let notes = event.notes.clone().filter(|notes| !notes.trim().is_empty());
    let (head, lines) = facts(event, zone, p);
    let card = div()
        .id("calendar-event")
        .read_as(gpui_kit::Role::Dialog, event.title.clone())
        .w(px(440.))
        .max_h(px(560.))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(14.))
        .p(px(18.))
        .rounded(px(12.))
        .border_1()
        .border_color(p.border)
        .bg(p.surface_raised)
        .shadow_lg()
        .child(
            div()
                .flex()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Event"),
                )
                .child(
                    Button::new("calendar-event-done")
                        .small()
                        .primary()
                        .label("Done")
                        .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx))),
                ),
        )
        .child(head)
        .child(lines)
        .when_some(notes, |d, notes| {
            d.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.muted)
                            .child("Notes"),
                    )
                    .child(selectable("event-notes", &notes)),
            )
        });
    div()
        .id("calendar-event-backdrop")
        .occlude()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(gpui_kit::black().opacity(0.2))
        .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx)))
        .child(card.on_click(|_, _, cx| cx.stop_propagation()))
}
