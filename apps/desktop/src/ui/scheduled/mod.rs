//! Scheduled: everything the assistant has with a next run, every room's, grouped by when it
//! runs next (Next hour, Today, Tomorrow, This week, This month, Later, No next run) with a
//! count each, filtered by room, kind and how soon. A row opens its task; its right-click menu
//! stops a watch or cancels a check-in.

mod filter_bar;
mod row;

use gpui_kit::component::spinner::Spinner;
use gpui_kit::prelude::*;
use gpui_kit::{Div, FontWeight, Hsla, ScrollHandle, div, px};
use jiff::ZonedDateTime;

use super::a11y::Accessible as _;
use super::palette::{Pal, pal};
use super::widgets::{page_head, scroller};
use super::window::MainWindow;
use crate::brand::APP_NAME;
use crate::store::scheduled::filter::{ScheduledFilter, shown};
use crate::store::scheduled::when::NextRun;

/// The page.
pub(crate) fn page(
    window: &mut MainWindow,
    scroll: &ScrollHandle,
    cx: &mut Context<'_, MainWindow>,
) -> Div {
    let p = pal(cx);
    let now = ZonedDateTime::now();
    let store = window.store.clone();
    let read = store.read(cx);
    let feed = read.scheduled();
    let filter = feed.filter().clone();
    let mut body = div().flex().flex_col().px(px(12.)).pb(px(18.));
    match feed.items() {
        None => {
            let problem = feed.problem().map(str::to_owned);
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .py(px(14.))
                    .text_color(p.muted)
                    .when(problem.is_none(), |d| {
                        d.child(Spinner::new().color(p.muted))
                    })
                    .when_some(problem, Div::child),
            );
        }
        Some(items) => {
            let shown = shown(items, &filter, &now);
            if shown.is_empty() {
                body = body.child(empty(&p, filter.is_empty()));
            }
            for (group, rows) in &shown.groups {
                body = body.child(header(&p, *group, rows.len()));
                for item in rows {
                    body = body.child(row::row(&store, read, item, &now, &p));
                }
            }
            if !filter.is_empty() && shown.hidden > 0 {
                body = body.child(filter_bar::filtered_out(&store, shown.hidden, &p));
            }
            if let Some(problem) = feed.problem() {
                body = body.child(
                    div()
                        .id("scheduled-problem")
                        .announced(problem.to_owned())
                        .px(px(12.))
                        .pt(px(10.))
                        .text_size(px(12.))
                        .text_color(p.danger)
                        .child(problem.to_owned()),
                );
            }
        }
    }
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(page_head(
            &p,
            "Scheduled",
            Some("every room's, soonest first".into()),
        ))
        .child(filter_bar::bar(&store, read, &p))
        .child(scroller("scheduled-scroll", scroll).child(body))
}

/// A group's header: its name and how many rows it has (Next hour in the accent).
fn header(p: &Pal, group: NextRun, count: usize) -> Div {
    let color: Hsla = if group == NextRun::Hour {
        p.info
    } else {
        p.faint
    };
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(12.))
        .pt(px(14.))
        .pb(px(4.))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child(group.name())
        .child(div().text_color(p.faint).child(count.to_string()))
}

/// Nothing to show: nothing scheduled at all, or nothing under the filter.
fn empty(p: &Pal, unfiltered: bool) -> Div {
    let (title, line) = if unfiltered {
        (
            "Nothing scheduled".to_owned(),
            format!("{APP_NAME}'s watches, check-ins and routines show here, soonest first."),
        )
    } else {
        (
            "Nothing here for this filter".to_owned(),
            "Take a filter off to see the rest.".to_owned(),
        )
    };
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(6.))
        .py(px(40.))
        .px(px(20.))
        .child(
            gpui_kit::component::Icon::new(gpui_kit::assets::IconName::CalendarClock)
                .size(px(28.))
                .text_color(p.faint),
        )
        .child(
            div()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(div().text_size(px(12.5)).text_color(p.muted).child(line))
}

/// The filters off: every row shows.
fn cleared() -> ScheduledFilter {
    ScheduledFilter::default()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use emberhut_api::scheduled::ScheduledPage;
    use gpui_kit::TestAppContext;
    use gpui_kit::test::TestWindowExt as _;
    use serde_json::json;

    use super::*;
    use crate::backend::fake::FakeBackend;
    use crate::store::{Page, Store};
    use crate::ui::report::ReportSize;

    #[gpui_kit::test]
    fn the_page_lists_what_is_scheduled_by_when(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let backend = Arc::new(FakeBackend::new([]));
        let soon = jiff::Timestamp::now()
            .checked_add(jiff::SignedDuration::from_mins(20))
            .unwrap();
        backend
            .days
            .push_scheduled(Ok(serde_json::from_value::<ScheduledPage>(json!({
            "items": [{
                "id": "watch:3", "kind": "watch", "title": "A reply", "nextAt": soon.to_string(),
                "repeat": "every 4 h", "room": 1, "note": "No reply yet.", "noteAt": null,
                "tasks": [], "watch": 3, "checkin": null
            }]
        }))
        .unwrap()));
        let shared = Arc::clone(&backend) as Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::for_tests(shared));
        store.update(cx, |store, cx| {
            store.take_state_for_tests(crate::fixtures::state(json!({})), cx);
            store.start_scheduled(cx);
        });
        cx.run_until_parked();
        let window = cx.add_window(|window, cx| {
            MainWindow::new(store.clone(), ReportSize::default(), window, cx)
        });
        window
            .update(cx, |main, window, cx| {
                main.open(Page::Scheduled, window, cx);
            })
            .unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.try_find("scheduled-when-watch:3").is_some(),
                "its row"
            );
        })
        .unwrap();
    }
}
