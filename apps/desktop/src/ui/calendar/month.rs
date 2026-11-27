//! Month: the month's whole weeks as rows, each day with its work items first, then its events
//! as many as fit and `+n more`; the other months' days shaded, today circled; a click opens
//! the day.

use emberhut_api::calendar::CalendarPage;
use gpui_kit::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{ElementId, FontWeight, Window, div, px, rgb, white};
use jiff::civil::Date;

use super::chips::{all_day_chip, color, event_words, item_chip};
use super::{CalendarView, open_item};
use crate::prose::direction::marked;
use crate::store::calendar::layout;
use crate::store::calendar::range::WEEKDAYS;
use crate::store::calendar::words::Tone;
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::palette::Pal;

/// What the window keeps above the weeks: the bar, the weekday names and the margins.
const CHROME: f32 = 110.;
/// The least a week's row is tall.
const SHORTEST: f32 = 64.;

impl CalendarView {
    /// The month.
    pub(super) fn month(
        &self,
        page: Option<&CalendarPage>,
        window: &Window,
        p: &Pal,
        cx: &mut Context<'_, Self>,
    ) -> impl IntoElement {
        let days = self.days();
        let weeks = (days.len() / 7).max(1);
        #[expect(clippy::cast_precision_loss, reason = "a month has at most six weeks")]
        let height =
            ((f32::from(window.viewport_size().height) - CHROME) / weeks as f32).max(SHORTEST);
        let mut names = div().flex().pb(px(4.));
        for name in WEEKDAYS {
            names = names.child(
                div()
                    .flex_1()
                    .text_center()
                    .text_size(px(10.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.muted)
                    .child(name),
            );
        }
        let mut rows = div()
            .id("calendar-month")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col();
        for week in days.chunks(7) {
            let mut row = div().flex().border_b_1().border_color(p.border);
            for day in week {
                row = row.child(self.cell(page, *day, height, p, cx));
            }
            rows = rows.child(row);
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(names)
            .child(div().h(px(1.)).bg(p.border))
            // Seen by the headless tests (a no-op outside them).
            .child(rows.test_support())
    }

    /// One day of the month.
    fn cell(
        &self,
        page: Option<&CalendarPage>,
        day: Date,
        height: f32,
        p: &Pal,
        cx: &mut Context<'_, Self>,
    ) -> impl IntoElement {
        let items = layout::items_on(page, day);
        let events = layout::events_on(page, day, &self.zone);
        let fold = layout::month_fold(items.len(), events.len(), height);
        let in_month = (day.year(), day.month()) == (self.anchor.year(), self.anchor.month());
        let today = day == self.today();
        let key = day.strftime("%Y%m%d").to_string();
        let mut cell = div()
            .id(ElementId::Name(format!("month-{key}").into()))
            .button_named(a11y::day_with(day, today, items.len() + events.len()))
            .hint("Shows the day")
            .flex()
            .flex_col()
            .gap(px(1.))
            .flex_1()
            .min_w_0()
            .h(px(height))
            .p(px(2.))
            .overflow_hidden()
            .border_l_1()
            .border_color(p.border)
            .when(!in_month, |d| d.bg(p.muted.opacity(0.06)))
            .cursor_pointer()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(22.))
                    .rounded_full()
                    .text_size(px(12.))
                    .text_color(if in_month { p.text } else { p.muted })
                    .when(today, |d| {
                        d.bg(rgb(0x00FF_3B30))
                            .text_color(white())
                            .font_weight(FontWeight::BOLD)
                    })
                    .child(day.day().to_string()),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.pick(day, cx)));
        for (i, item) in items.into_iter().take(fold.items).enumerate() {
            let open = item.clone();
            cell = cell.child(
                item_chip(
                    item,
                    true,
                    ElementId::Name(format!("month-{key}-item-{i}").into()),
                    p,
                )
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    open_item(&open, cx);
                }),
            );
        }
        for (i, event) in events.into_iter().take(fold.events).enumerate() {
            let open = event.clone();
            let id = ElementId::Name(format!("month-{key}-event-{i}").into());
            let chip = if event.all_day {
                all_day_chip(event, true, id, p)
            } else {
                div()
                    .id(id)
                    .button_named(event_words(event, &self.zone))
                    .flex()
                    .items_center()
                    .gap(px(3.))
                    .min_w_0()
                    .text_size(px(10.))
                    .child(
                        div()
                            .size(px(6.))
                            .flex_none()
                            .rounded_full()
                            .bg(color(Tone::of_event(event), p)),
                    )
                    .child(div().min_w_0().truncate().child(marked(&event.title)))
            };
            cell = cell.child(chip.on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.open_event(open.clone(), cx);
            })));
        }
        if let Some(more) = fold.more {
            cell = cell.child(
                div()
                    .text_size(px(10.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.muted)
                    .child(format!("+{more} more")),
            );
        }
        cell.test_support()
    }
}
