//! The calendar's data: each range of days as last fetched, so moving back to one shows it at
//! once while it is asked for again; a range that never came is marked failed (the window says
//! it can't reach the server and keeps retrying). The window asks when its range changes and
//! every minute while it is in sight.
//!
//! The days shown, where things go and what they say are pure ([`range`], [`layout`],
//! [`words`]), tested on their own.

pub(crate) mod layout;
pub(crate) mod range;
pub(crate) mod words;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use emberhut_api::calendar::{CalendarPage, CalendarQuery};
use emberhut_api::time::Day;
use gpui_kit::AppContext as _;
use gpui_kit::Context;
use jiff::civil::Date;

use super::{Store, update_or_log};

/// How many ranges are kept.
const KEPT: usize = 24;

/// A range of days, first and last.
type Range = (Date, Date);

/// The ranges fetched.
#[derive(Debug, Default)]
pub(crate) struct CalendarFeed {
    pages: HashMap<Range, CalendarPage>,
    /// The ranges kept, the one used last at the back.
    order: VecDeque<Range>,
    /// Ranges asked for that never came.
    failed: HashSet<Range>,
    /// Ranges being asked for now.
    asking: HashSet<Range>,
}

impl CalendarFeed {
    /// Keeps `page` for `range`, dropping the range used longest ago past [`KEPT`].
    fn keep(&mut self, range: Range, page: CalendarPage) {
        self.failed.remove(&range);
        self.order.retain(|kept| *kept != range);
        self.order.push_back(range);
        self.pages.insert(range, page);
        while self.order.len() > KEPT {
            if let Some(old) = self.order.pop_front() {
                self.pages.remove(&old);
            }
        }
    }
}

/// A date as the wire writes days.
fn day(date: Date) -> Day {
    Day::new(date.strftime("%Y-%m-%d").to_string())
}

impl Store {
    /// The calendar for `from..=to`, as last fetched.
    #[must_use]
    pub(crate) fn calendar_page(&self, from: Date, to: Date) -> Option<&CalendarPage> {
        self.calendar.pages.get(&(from, to))
    }

    /// Whether `from..=to` was asked for and never came.
    #[must_use]
    pub(crate) fn calendar_failed(&self, from: Date, to: Date) -> bool {
        self.calendar.failed.contains(&(from, to))
    }

    /// Asks for the calendar of `from..=to` (once at a time per range).
    pub(crate) fn load_calendar(&mut self, from: Date, to: Date, cx: &mut Context<'_, Self>) {
        let range = (from, to);
        if !self.calendar.asking.insert(range) {
            return;
        }
        let backend = Arc::clone(&self.backend);
        let query = CalendarQuery::new(day(from), day(to));
        cx.spawn(async move |this, cx| {
            let page = cx
                .background_spawn(async move { backend.calendar(&query) })
                .await;
            update_or_log(&this, cx, |store, cx| {
                let feed = &mut store.calendar;
                feed.asking.remove(&range);
                match page {
                    Ok(page) => {
                        if feed.pages.get(&range) != Some(&page) || feed.failed.contains(&range) {
                            feed.keep(range, page);
                            cx.notify();
                        }
                    }
                    Err(error) => {
                        log::info!("the calendar for {from}..{to} didn't come: {error}");
                        if !feed.pages.contains_key(&range) && feed.failed.insert(range) {
                            cx.notify();
                        }
                    }
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::TestAppContext;
    use jiff::civil::date;

    use super::*;
    use crate::backend::Error;
    use crate::backend::fake::{Call, FakeBackend};

    #[gpui_kit::test]
    fn a_range_is_kept_and_one_that_never_came_is_marked(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        backend
            .days
            .push_calendar(Err(Error::Unreachable("down".into())));
        let store = cx.new(|_| Store::for_tests(backend.clone()));
        let (from, to) = (date(2026, 11, 23), date(2026, 11, 29));
        store.update(cx, |store, cx| store.load_calendar(from, to, cx));
        cx.run_until_parked();
        store.read_with(cx, |store, _| {
            assert!(store.calendar_failed(from, to));
            assert!(store.calendar_page(from, to).is_none());
        });
        store.update(cx, |store, cx| store.load_calendar(from, to, cx));
        cx.run_until_parked();
        store.read_with(cx, |store, _| {
            assert!(!store.calendar_failed(from, to));
            assert_eq!(
                store.calendar_page(from, to).map(|page| page.from.as_str()),
                Some("2026-11-23")
            );
        });
        assert!(
            backend
                .calls()
                .contains(&Call::Calendar("from=2026-11-23&to=2026-11-29".to_owned()))
        );
    }

    #[test]
    fn only_so_many_ranges_are_kept() {
        let mut feed = CalendarFeed::default();
        let page: CalendarPage = serde_json::from_value(serde_json::json!({
            "from": "2026-01-01", "to": "2026-01-01", "events": [], "items": [], "note": null,
            "at": "2026-01-01T00:00:00.000Z"
        }))
        .unwrap();
        for n in 0..=KEPT {
            let first = date(2026, 1, 1)
                .checked_add(jiff::ToSpan::days(i64::try_from(n).unwrap()))
                .unwrap();
            feed.keep((first, first), page.clone());
        }
        assert_eq!(feed.pages.len(), KEPT);
        assert!(
            !feed
                .pages
                .contains_key(&(date(2026, 1, 1), date(2026, 1, 1)))
        );
    }
}
