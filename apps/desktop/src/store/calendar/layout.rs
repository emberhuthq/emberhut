//! Where things go on the calendar: each day's timed events, all-day events and work items;
//! timed events side by side where they overlap (each takes the first free column of its
//! cluster), at their height for the hour; and how many chips a strip or a month's day has room
//! for before `+n`. Pure, so it is tested without a window.

use emberhut_api::calendar::{CalendarEvent, CalendarItem, CalendarPage};
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan as _};

use crate::store::tasks::clock::instant;

/// When an event starts and ends (an end before its start is its start).
#[must_use]
pub(crate) fn span(event: &CalendarEvent) -> (Timestamp, Timestamp) {
    let starts = instant(&event.start).unwrap_or_default();
    let ends = instant(&event.end).unwrap_or(starts).max(starts);
    (starts, ends)
}

/// The instants `day` starts and ends at in `zone`.
#[must_use]
pub(crate) fn bounds(day: Date, zone: &TimeZone) -> (Timestamp, Timestamp) {
    let start = day
        .to_zoned_datetime(zone.clone())
        .map_or_default(|at| at.timestamp());
    let end = day
        .checked_add(1.day())
        .ok()
        .and_then(|next| next.to_zoned_datetime(zone.clone()).ok())
        .map_or(start, |at| at.timestamp());
    (start, end)
}

/// Whether `event` is on `day` at all.
fn overlaps(event: &CalendarEvent, day: Date, zone: &TimeZone) -> bool {
    let ((starts, ends), (start, end)) = (span(event), bounds(day, zone));
    starts < end && ends > start
}

/// `day`'s timed events.
#[must_use]
pub(crate) fn timed_on<'a>(
    page: Option<&'a CalendarPage>,
    day: Date,
    zone: &TimeZone,
) -> Vec<&'a CalendarEvent> {
    page.map_or_default(|page| {
        page.events
            .iter()
            .filter(|event| !event.all_day && overlaps(event, day, zone))
            .collect()
    })
}

/// `day`'s all-day events.
#[must_use]
pub(crate) fn all_day_on<'a>(
    page: Option<&'a CalendarPage>,
    day: Date,
    zone: &TimeZone,
) -> Vec<&'a CalendarEvent> {
    page.map_or_default(|page| {
        page.events
            .iter()
            .filter(|event| event.all_day && overlaps(event, day, zone))
            .collect()
    })
}

/// Every event on `day`, the all-day ones first, then by start (a month's day).
#[must_use]
pub(crate) fn events_on<'a>(
    page: Option<&'a CalendarPage>,
    day: Date,
    zone: &TimeZone,
) -> Vec<&'a CalendarEvent> {
    let mut events: Vec<&CalendarEvent> = page.map_or_default(|page| {
        page.events
            .iter()
            .filter(|event| overlaps(event, day, zone))
            .collect()
    });
    events.sort_by_key(|event| (!event.all_day, span(event).0));
    events
}

/// `day`'s work items, as the server listed them.
#[must_use]
pub(crate) fn items_on(page: Option<&CalendarPage>, day: Date) -> Vec<&CalendarItem> {
    let key = day.strftime("%Y-%m-%d").to_string();
    page.map_or_default(|page| {
        page.items
            .iter()
            .filter(|item| item.day.as_str() == key)
            .collect()
    })
}

/// A timed event placed in its day: its column of how many.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Laid<'a> {
    /// The event.
    pub(crate) event: &'a CalendarEvent,
    /// Its column, from 0.
    pub(crate) col: usize,
    /// How many columns its cluster has.
    pub(crate) cols: usize,
}

/// `events` side by side where they overlap: in each cluster of overlapping events, each takes
/// the first column free when it starts, and the cluster is as wide as its most columns.
#[must_use]
pub(crate) fn lay<'a>(events: &[&'a CalendarEvent]) -> Vec<Laid<'a>> {
    let mut sorted: Vec<&CalendarEvent> = events.to_vec();
    sorted.sort_by_key(|event| {
        let (starts, ends) = span(event);
        (starts, std::cmp::Reverse(ends))
    });
    let mut out = Vec::with_capacity(sorted.len());
    let mut cluster: Vec<(&CalendarEvent, usize)> = Vec::new();
    let mut ends: Vec<Timestamp> = Vec::new();
    let mut cluster_end: Option<Timestamp> = None;
    let flush = |cluster: &mut Vec<(&'a CalendarEvent, usize)>,
                 ends: &mut Vec<Timestamp>,
                 out: &mut Vec<Laid<'a>>| {
        let cols = ends.len().max(1);
        out.extend(
            cluster
                .drain(..)
                .map(|(event, col)| Laid { event, col, cols }),
        );
        ends.clear();
    };
    for event in sorted {
        let (starts, finishes) = span(event);
        if cluster_end.is_some_and(|end| starts >= end) {
            flush(&mut cluster, &mut ends, &mut out);
        }
        if let Some(free) = ends.iter().position(|end| *end <= starts) {
            ends[free] = finishes;
            cluster.push((event, free));
        } else {
            ends.push(finishes);
            cluster.push((event, ends.len() - 1));
        }
        cluster_end = Some(cluster_end.map_or(finishes, |end| end.max(finishes)));
    }
    flush(&mut cluster, &mut ends, &mut out);
    out
}

/// Where a timed event's block goes in its day, `hour` pixels an hour: its top and height
/// (never under 18 px, so a short one can be clicked). Both are the wall clock's ([`wall_hours`]),
/// as the grid's 24 hour lines are: on a day the clocks change, a 10:00 event still sits on the
/// 10 AM line.
#[must_use]
pub(crate) fn block(event: &CalendarEvent, day: Date, zone: &TimeZone, hour: f32) -> (f32, f32) {
    let (starts, ends) = span(event);
    let (from, to) = (wall_hours(starts, day, zone), wall_hours(ends, day, zone));
    let top = from * hour;
    let height = ((to - from) * hour).max(18.);
    (top, height)
}

/// Where `now` is on `day`'s hours, `hour` pixels an hour (by the wall clock, as [`block`]).
#[must_use]
pub(crate) fn now_at(now: Timestamp, day: Date, zone: &TimeZone, hour: f32) -> f32 {
    wall_hours(now, day, zone) * hour
}

/// The wall-clock time of `at` on `day` in `zone`, in hours from 0 to 24: the local time of
/// that instant, with the zone's offset at that instant (not the day's first), so a day the
/// clocks change on keeps its 24 hour lines. Before the day it is 0, from the day's end on 24.
#[must_use]
pub(crate) fn wall_hours(at: Timestamp, day: Date, zone: &TimeZone) -> f32 {
    let (start, end) = bounds(day, zone);
    if at <= start {
        return 0.;
    }
    if at >= end {
        return 24.;
    }
    let time = at.to_zoned_datetime(zone.clone()).time();
    let seconds = i32::from(time.hour()) * 3600 + i32::from(time.minute()) * 60;
    let seconds = seconds + i32::from(time.second());
    #[expect(
        clippy::cast_precision_loss,
        reason = "seconds within a day fit an f32 well enough for pixels"
    )]
    let hours = seconds as f32 / 3600.;
    hours.clamp(0., 24.)
}

/// How a day's chips fold: how many show and what the fold says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fold {
    /// How many work items show.
    pub(crate) items: usize,
    /// How many events show after them.
    pub(crate) events: usize,
    /// How many more there are, when some are folded away.
    pub(crate) more: Option<usize>,
    /// Whether "Show less" folds an opened day again.
    pub(crate) less: bool,
}

/// A strip day's fold: `items` and `events`, `room` chips at most.
#[must_use]
pub(crate) fn fold(items: usize, events: usize, room: usize) -> Fold {
    let all = items + events;
    let shown_items = items.min(room);
    Fold {
        items: shown_items,
        events: events.min(room - shown_items),
        more: (all > room).then(|| all - room),
        less: false,
    }
}

/// The strip's room: Week shows five a day (a tap on `+n` opens the day); Day four, or all of
/// them while opened, with "Show less" once there are more than four.
#[must_use]
pub(crate) fn strip_fold(items: usize, events: usize, one_day: bool, opened: bool) -> Fold {
    const WEEK: usize = 5;
    const DAY: usize = 4;
    if !one_day {
        return fold(items, events, WEEK);
    }
    if opened {
        let mut all = fold(items, events, usize::MAX / 2);
        all.less = items + events > DAY;
        return all;
    }
    fold(items, events, DAY)
}

/// A month's day `height` pixels tall: how many lines fit under its date (each 15 px), one
/// kept for `+n more` when there are more.
#[must_use]
pub(crate) fn month_fold(items: usize, events: usize, height: f32) -> Fold {
    let room = lines_in(height);
    let all = items + events;
    let shown = if all > room { room - 1 } else { room };
    let shown_items = items.min(shown);
    Fold {
        items: shown_items,
        events: events.min(shown - shown_items),
        more: (all > shown).then(|| all - shown),
        less: false,
    }
}

/// How many 15 px lines fit in a month's day `height` tall, under its 24 px date (one at least).
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a day's height in pixels is small and positive once floored at 0"
)]
fn lines_in(height: f32) -> usize {
    (((height - 24.) / 15.).floor().max(1.)) as usize
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn event(id: &str, start: &str, end: &str, all_day: bool) -> CalendarEvent {
        serde_json::from_value(json!({
            "id": id, "title": id, "start": start, "end": end, "allDay": all_day,
            "calendar": "Work", "people": 0, "day": &start[..10], "kind": "event"
        }))
        .unwrap()
    }

    fn page(events: &[CalendarEvent]) -> CalendarPage {
        serde_json::from_value(json!({
            "from": "2026-11-23", "to": "2026-11-29", "events": events, "items": [
                {"kind": "today", "day": "2026-11-24", "task": 1, "key": "K-1", "project": null,
                 "title": "Write", "hard": false, "status": "todo"}
            ], "note": null, "at": "2026-11-24T00:00:00.000Z"
        }))
        .unwrap()
    }

    #[test]
    fn each_day_has_its_own_events_and_items() {
        let zone = TimeZone::UTC;
        let page = page(&[
            event(
                "meet",
                "2026-11-24T09:00:00.000Z",
                "2026-11-24T10:00:00.000Z",
                false,
            ),
            event(
                "night",
                "2026-11-24T23:00:00.000Z",
                "2026-11-25T01:00:00.000Z",
                false,
            ),
            event(
                "off",
                "2026-11-25T00:00:00.000Z",
                "2026-11-27T00:00:00.000Z",
                true,
            ),
        ]);
        let tuesday = jiff::civil::date(2026, 11, 24);
        let wednesday = jiff::civil::date(2026, 11, 25);
        let ids = |events: Vec<&CalendarEvent>| -> Vec<String> {
            events.iter().map(|event| event.id.clone()).collect()
        };
        assert_eq!(
            ids(timed_on(Some(&page), tuesday, &zone)),
            ["meet", "night"]
        );
        assert_eq!(
            ids(timed_on(Some(&page), wednesday, &zone)),
            ["night"],
            "past midnight"
        );
        assert_eq!(ids(all_day_on(Some(&page), wednesday, &zone)), ["off"]);
        assert_eq!(
            ids(all_day_on(
                Some(&page),
                jiff::civil::date(2026, 11, 26),
                &zone
            )),
            ["off"]
        );
        assert_eq!(
            all_day_on(Some(&page), jiff::civil::date(2026, 11, 27), &zone),
            Vec::<&CalendarEvent>::new()
        );
        assert_eq!(
            ids(events_on(Some(&page), wednesday, &zone)),
            ["off", "night"]
        );
        assert_eq!(items_on(Some(&page), tuesday).len(), 1);
        assert_eq!(items_on(None, tuesday), Vec::<&CalendarItem>::new());
    }

    #[test]
    fn overlapping_events_share_the_width() {
        let a = event(
            "a",
            "2026-11-24T09:00:00.000Z",
            "2026-11-24T11:00:00.000Z",
            false,
        );
        let b = event(
            "b",
            "2026-11-24T09:30:00.000Z",
            "2026-11-24T10:00:00.000Z",
            false,
        );
        let c = event(
            "c",
            "2026-11-24T10:00:00.000Z",
            "2026-11-24T10:30:00.000Z",
            false,
        );
        let d = event(
            "d",
            "2026-11-24T12:00:00.000Z",
            "2026-11-24T13:00:00.000Z",
            false,
        );
        let laid = lay(&[&d, &c, &b, &a]);
        let at = |id: &str| {
            let laid = laid.iter().find(|laid| laid.event.id == id).unwrap();
            (laid.col, laid.cols)
        };
        assert_eq!(at("a"), (0, 2));
        assert_eq!(at("b"), (1, 2));
        assert_eq!(at("c"), (1, 2), "takes b's column once b is over");
        assert_eq!(at("d"), (0, 1), "a cluster of its own");
    }

    #[test]
    fn a_block_sits_at_its_hour_and_is_never_too_small() {
        let zone = TimeZone::UTC;
        let day = jiff::civil::date(2026, 11, 24);
        let meet = event(
            "m",
            "2026-11-24T09:30:00.000Z",
            "2026-11-24T11:00:00.000Z",
            false,
        );
        assert_eq!(block(&meet, day, &zone, 52.), (494., 78.));
        let blink = event(
            "b",
            "2026-11-24T09:00:00.000Z",
            "2026-11-24T09:05:00.000Z",
            false,
        );
        assert_eq!(block(&blink, day, &zone, 52.).1, 18.);
        let late = event(
            "l",
            "2026-11-24T23:00:00.000Z",
            "2026-11-25T02:00:00.000Z",
            false,
        );
        assert_eq!(
            block(&late, day, &zone, 52.),
            (1196., 52.),
            "cut at midnight"
        );
        let now: Timestamp = "2026-11-24T12:00:00Z".parse().unwrap();
        assert!((now_at(now, day, &zone, 52.) - 624.).abs() < f32::EPSILON);
    }

    /// Central European time, with its summer time: the clocks go forward an hour at 02:00 on
    /// the last Sunday of March (29 March 2026) and back at 03:00 on the last Sunday of
    /// October (25 October 2026). Spelled as a POSIX rule, so no system zone file is needed.
    fn central_europe() -> TimeZone {
        TimeZone::posix("CET-1CEST,M3.5.0,M10.5.0/3").unwrap()
    }

    #[test]
    fn a_block_keeps_its_wall_clock_hour_when_the_clocks_go_forward() {
        let (zone, day) = (central_europe(), jiff::civil::date(2026, 3, 29));
        assert_eq!(
            bounds(day, &zone).1.as_second() - bounds(day, &zone).0.as_second(),
            23 * 3600
        );
        // 10:00 to 11:30 summer time (UTC+2).
        let meet = event(
            "m",
            "2026-03-29T08:00:00.000Z",
            "2026-03-29T09:30:00.000Z",
            false,
        );
        assert_eq!(
            block(&meet, day, &zone, 50.),
            (500., 75.),
            "on the 10 AM line"
        );
        // 01:00 winter time to 04:00 summer time: two hours on the clock's face, three lines.
        let across = event(
            "a",
            "2026-03-29T00:00:00.000Z",
            "2026-03-29T02:00:00.000Z",
            false,
        );
        assert_eq!(block(&across, day, &zone, 50.), (50., 150.));
        let late = event(
            "l",
            "2026-03-29T21:00:00.000Z",
            "2026-03-29T23:30:00.000Z",
            false,
        );
        assert_eq!(
            block(&late, day, &zone, 50.),
            (1150., 50.),
            "cut at midnight"
        );
        let now: Timestamp = "2026-03-29T13:15:00Z".parse().unwrap();
        assert!((now_at(now, day, &zone, 50.) - 15.25 * 50.).abs() < f32::EPSILON);
    }

    #[test]
    fn a_block_keeps_its_wall_clock_hour_when_the_clocks_go_back() {
        let (zone, day) = (central_europe(), jiff::civil::date(2026, 10, 25));
        assert_eq!(
            bounds(day, &zone).1.as_second() - bounds(day, &zone).0.as_second(),
            25 * 3600
        );
        // 10:00 to 11:00 winter time (UTC+1).
        let meet = event(
            "m",
            "2026-10-25T09:00:00.000Z",
            "2026-10-25T10:00:00.000Z",
            false,
        );
        assert_eq!(
            block(&meet, day, &zone, 50.),
            (500., 50.),
            "on the 10 AM line"
        );
        // 23:00 to midnight: the day's last hour, inside its 24 lines.
        let last = event(
            "l",
            "2026-10-25T22:00:00.000Z",
            "2026-10-25T23:00:00.000Z",
            false,
        );
        assert_eq!(block(&last, day, &zone, 50.), (1150., 50.));
        let now: Timestamp = "2026-10-25T21:30:00Z".parse().unwrap();
        assert!((now_at(now, day, &zone, 50.) - 22.5 * 50.).abs() < f32::EPSILON);
        let before: Timestamp = "2026-10-24T12:00:00Z".parse().unwrap();
        assert!(
            now_at(before, day, &zone, 50.).abs() < f32::EPSILON,
            "before the day"
        );
    }

    #[test]
    fn chips_fold_into_plus_n() {
        assert_eq!(
            strip_fold(3, 4, false, false),
            Fold {
                items: 3,
                events: 2,
                more: Some(2),
                less: false
            }
        );
        assert_eq!(
            strip_fold(1, 2, true, false),
            Fold {
                items: 1,
                events: 2,
                more: None,
                less: false
            }
        );
        let opened = strip_fold(3, 4, true, true);
        assert_eq!(
            (opened.items, opened.events, opened.more, opened.less),
            (3, 4, None, true)
        );
        assert_eq!(strip_fold(2, 3, true, false).more, Some(1));
        let month = month_fold(2, 3, 84.);
        assert_eq!(
            month,
            Fold {
                items: 2,
                events: 1,
                more: Some(2),
                less: false
            }
        );
        assert_eq!(month_fold(0, 1, 30.).events, 1, "one line at least");
    }
}
