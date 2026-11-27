//! Day and Week: the days across (Week: their names and dates, today's in a red circle, a click
//! opens the day), a strip for each day's work and all-day events, then the hours, with each
//! timed event where it falls and side by side when they overlap, and now as a red line.

use emberhut_api::calendar::{CalendarEvent, CalendarPage};
use gpui_kit::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, ElementId, FontWeight, div, px, relative, rgb, white};
use jiff::Timestamp;
use jiff::civil::Date;

use super::chips::{all_day_chip, color, event_words, item_chip};
use super::{CalendarView, HOUR, open_item};
use crate::prose::direction;
use crate::store::calendar::layout::{self, Laid};
use crate::store::calendar::words::{self, Tone};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::palette::Pal;

/// The hours' gutter.
const GUTTER: f32 = 52.;
/// The red of today and of now.
const NOW_RED: u32 = 0x00FF_3B30;

impl CalendarView {
    /// The grid.
    pub(super) fn grid(
        &self,
        page: Option<&CalendarPage>,
        p: &Pal,
        cx: &mut Context<'_, Self>,
    ) -> Div {
        let days = self.days();
        let one = days.len() == 1;
        let today = self.today();
        let mut grid = div().flex().flex_col().flex_1().min_h_0();
        if !one {
            let mut heads = div()
                .flex()
                .pb(px(4.))
                .child(div().w(px(GUTTER)).flex_none());
            for (n, day) in (0_u64..).zip(&days) {
                let day = *day;
                heads = heads.child(
                    div()
                        .id(("calendar-head", n))
                        .button_named(a11y::day(day, day == today))
                        .hint("Shows the day")
                        .flex_1()
                        .cursor_pointer()
                        .child(day_head(day, day == today, p))
                        .on_click(cx.listener(move |this, _, _, cx| this.pick(day, cx))),
                );
            }
            grid = grid.child(heads);
        }
        let mut hours = div().relative().h(px(HOUR * 24. + 8.)).child(hour_lines(p));
        let mut columns = div()
            .absolute()
            .top(px(8.))
            .left(px(GUTTER))
            .right_0()
            .h(px(HOUR * 24.))
            .flex();
        for day in &days {
            columns = columns.child(self.column(page, *day, *day == today, p, cx));
        }
        hours = hours.child(columns);
        grid.child(self.strip(page, &days, p, cx))
            .child(div().h(px(1.)).bg(p.border))
            .child(
                div()
                    .id("calendar-hours")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.hours)
                    .child(hours)
                    // Seen by the headless tests (a no-op outside them).
                    .test_support(),
            )
    }

    /// The strip: each day's work items, then its all-day events, folded after the first few.
    fn strip(
        &self,
        page: Option<&CalendarPage>,
        days: &[Date],
        p: &Pal,
        cx: &mut Context<'_, Self>,
    ) -> Div {
        let one = days.len() == 1;
        let mut strip = div()
            .flex()
            .items_start()
            .pr(px(2.))
            .child(div().w(px(GUTTER)).flex_none());
        let mut busy = false;
        for (n, day) in (0_usize..).zip(days) {
            let day = *day;
            let items = layout::items_on(page, day);
            let events = layout::all_day_on(page, day, &self.zone);
            busy |= !items.is_empty() || !events.is_empty();
            let fold = layout::strip_fold(items.len(), events.len(), one, self.strip_open);
            let mut cell = div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .flex_1()
                .min_w_0()
                .px(px(1.));
            for (i, item) in items.into_iter().take(fold.items).enumerate() {
                let open = item.clone();
                cell = cell.child(
                    item_chip(
                        item,
                        !one,
                        ElementId::Name(format!("strip-item-{n}-{i}").into()),
                        p,
                    )
                    .on_click(move |_, _, cx| open_item(&open, cx)),
                );
            }
            for (i, event) in events.into_iter().take(fold.events).enumerate() {
                let open = event.clone();
                cell = cell.child(
                    all_day_chip(
                        event,
                        !one,
                        ElementId::Name(format!("strip-event-{n}-{i}").into()),
                        p,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.open_event(open.clone(), cx))),
                );
            }
            if let Some(more) = fold.more {
                let label = if one {
                    format!("+{more} more")
                } else {
                    format!("+{more}")
                };
                cell = cell.child(
                    div()
                        .id(("strip-more", n))
                        .button_named(format!("{more} more"))
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.muted)
                        .when(!one, Styled::text_center)
                        .cursor_pointer()
                        .child(label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if one {
                                this.strip_open = true;
                                cx.notify();
                            } else {
                                this.pick(day, cx);
                            }
                        })),
                );
            } else if fold.less {
                cell = cell.child(
                    div()
                        .id(("strip-less", n))
                        .role(gpui_kit::Role::Button)
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.muted)
                        .cursor_pointer()
                        .child("Show less")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.strip_open = false;
                            cx.notify();
                        })),
                );
            }
            strip = strip.child(cell);
        }
        strip.when(busy, |d| d.py(px(4.)))
    }

    /// One day's column of timed events, and now on today.
    fn column(
        &self,
        page: Option<&CalendarPage>,
        day: Date,
        is_today: bool,
        p: &Pal,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let timed = layout::timed_on(page, day, &self.zone);
        let laid = layout::lay(&timed);
        let narrow = self.days().len() > 1;
        let mut column = div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .border_l_1()
            .border_color(p.border);
        for Laid { event, col, cols } in laid {
            let (top, height) = layout::block(event, day, &self.zone, HOUR);
            let open = event.clone();
            #[expect(
                clippy::cast_precision_loss,
                reason = "a handful of columns side by side"
            )]
            let (left, width) = (col as f32 / cols as f32, 1. / cols as f32);
            column = column.child(
                div()
                    .absolute()
                    .top(px(top))
                    .left(relative(left))
                    .w(relative(width))
                    .h(px(height - 1.))
                    .px(px(1.))
                    .child(self.block(event, height >= 34., narrow, p).on_click(
                        cx.listener(move |this, _, _, cx| this.open_event(open.clone(), cx)),
                    )),
            );
        }
        if is_today {
            let top = layout::now_at(Timestamp::now(), day, &self.zone, HOUR);
            column = column.child(
                div()
                    .absolute()
                    .top(px(top - 3.5))
                    .left(px(-3.))
                    .right_0()
                    .flex()
                    .items_center()
                    .child(div().size(px(7.)).rounded_full().bg(rgb(NOW_RED)))
                    .child(div().flex_1().h(px(1.5)).bg(rgb(NOW_RED))),
            );
        }
        column.into_any_element()
    }

    /// A timed event's block: its colour, title and (when there's room) its start.
    fn block(
        &self,
        event: &CalendarEvent,
        tall: bool,
        narrow: bool,
        p: &Pal,
    ) -> gpui_kit::Stateful<Div> {
        let fill = color(Tone::of_event(event), p);
        let ink = if p.dark {
            fill.opacity(1.)
        } else {
            fill.blend(p.text.opacity(0.45))
        };
        div()
            .id(ElementId::Name(
                format!("event-{}-{}", event.id, event.day).into(),
            ))
            .button_named(event_words(event, &self.zone))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(1.))
            .pl(px(if narrow { 5. } else { 7. }))
            .pr(px(2.))
            .py(px(2.))
            .rounded(px(5.))
            .overflow_hidden()
            .bg(fill.opacity(0.22))
            .text_color(ink)
            .cursor_pointer()
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(px(3.))
                    .bg(fill),
            )
            .child(
                div()
                    .text_size(px(if narrow { 10. } else { 11. }))
                    .font_weight(FontWeight::SEMIBOLD)
                    .line_clamp(if tall { 3 } else { 1 })
                    .child(direction::marked(&event.title)),
            )
            .when(tall, |d| {
                d.child(
                    div()
                        .text_size(px(10.))
                        .opacity(0.8)
                        .child(words::start_time(event, &self.zone)),
                )
            })
    }
}

/// A day's name and date over its column; today's date in a red circle.
fn day_head(day: Date, today: bool, p: &Pal) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(2.))
        .child(
            div()
                .text_size(px(10.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if today { rgb(NOW_RED).into() } else { p.muted })
                .child(day.strftime("%a").to_string()),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(28.))
                .rounded_full()
                .text_size(px(14.))
                .when(today, |d| {
                    d.bg(rgb(NOW_RED))
                        .text_color(white())
                        .font_weight(FontWeight::BOLD)
                })
                .child(day.day().to_string()),
        )
}

/// The hours' lines and their names in the gutter.
fn hour_lines(p: &Pal) -> Div {
    let mut lines = div()
        .absolute()
        .top(px(8.))
        .left_0()
        .right_0()
        .flex()
        .flex_col();
    for hour in 0_i8..24 {
        let name = match hour {
            0 => String::new(),
            12 => "12 PM".to_owned(),
            1..12 => format!("{hour} AM"),
            _ => format!("{} PM", hour - 12),
        };
        lines = lines.child(
            div()
                .h(px(HOUR))
                .flex()
                .items_start()
                .gap(px(4.))
                .child(
                    div()
                        .w(px(GUTTER - 6.))
                        .flex_none()
                        .mt(px(-7.))
                        .text_size(px(10.))
                        .text_color(p.muted)
                        .text_right()
                        .child(name),
                )
                .child(div().flex_1().h(px(1.)).bg(p.border)),
        );
    }
    lines
}
