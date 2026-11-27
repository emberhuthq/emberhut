//! The Scheduled page's filters: a room, a kind, and how soon it runs ([`Runs`]). Each is off
//! or one value; a row shows when it matches every one that is on.

use emberhut_api::ids::RoomId;
use emberhut_api::scheduled::{ScheduledItem, ScheduledKind};
use jiff::ZonedDateTime;

use super::when::{NextRun, Runs};
use crate::store::tasks::clock::instant;

/// The kinds the Kind filter offers, in its order.
pub(crate) const KINDS: [ScheduledKind; 5] = [
    ScheduledKind::Watch,
    ScheduledKind::Checkin,
    ScheduledKind::Routine,
    ScheduledKind::Prep,
    ScheduledKind::Unit,
];

/// A kind's name, as the rows and the filter say it.
#[must_use]
pub(crate) fn kind_name(kind: &ScheduledKind) -> String {
    match kind {
        ScheduledKind::Watch => "Watch".to_owned(),
        ScheduledKind::Checkin => "Check-in".to_owned(),
        ScheduledKind::Routine => "Routine".to_owned(),
        ScheduledKind::Prep => "Meeting prep".to_owned(),
        ScheduledKind::Unit => "Timer unit".to_owned(),
        other => crate::format::capitalized(other.as_str()),
    }
}

/// The filters on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScheduledFilter {
    /// Only this room's.
    pub(crate) room: Option<RoomId>,
    /// Only this kind.
    pub(crate) kind: Option<ScheduledKind>,
    /// Only what runs this soon.
    pub(crate) runs: Option<Runs>,
}

impl ScheduledFilter {
    /// Whether no filter is on.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.count() == 0
    }

    /// How many filters are on.
    #[must_use]
    pub(crate) fn count(&self) -> usize {
        usize::from(self.room.is_some())
            + usize::from(self.kind.is_some())
            + usize::from(self.runs.is_some())
    }

    /// Whether `item` shows under these filters, judged at `now`.
    #[must_use]
    pub(crate) fn takes(&self, item: &ScheduledItem, now: &ZonedDateTime) -> bool {
        self.room.is_none_or(|room| item.room == room)
            && self.kind.as_ref().is_none_or(|kind| item.kind == *kind)
            && self
                .runs
                .is_none_or(|runs| runs.takes(item.next_at.as_ref().and_then(instant), now))
    }
}

/// What the page shows under a filter: its groups, soonest first (empty ones left out), and how
/// many rows the filter hides.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Shown<'a> {
    /// Each group with its rows, in the list's order.
    pub(crate) groups: Vec<(NextRun, Vec<&'a ScheduledItem>)>,
    /// How many rows the filter hides.
    pub(crate) hidden: usize,
}

impl Shown<'_> {
    /// Whether nothing shows.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }
}

/// What `items` show under `filter` at `now`.
#[must_use]
pub(crate) fn shown<'a>(
    items: &'a [ScheduledItem],
    filter: &ScheduledFilter,
    now: &ZonedDateTime,
) -> Shown<'a> {
    let taken: Vec<&ScheduledItem> = items
        .iter()
        .filter(|item| filter.takes(item, now))
        .collect();
    let hidden = items.len() - taken.len();
    let groups = NextRun::ALL
        .into_iter()
        .filter_map(|group| {
            let rows: Vec<&ScheduledItem> = taken
                .iter()
                .copied()
                .filter(|item| NextRun::of(item.next_at.as_ref().and_then(instant), now) == group)
                .collect();
            (!rows.is_empty()).then_some((group, rows))
        })
        .collect();
    Shown { groups, hidden }
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use jiff::tz::TimeZone;
    use serde_json::json;

    use super::*;

    fn item(kind: &str, room: i64, next: Option<&str>) -> ScheduledItem {
        serde_json::from_value(json!({
            "id": format!("{kind}:1"), "kind": kind, "title": "Example", "nextAt": next,
            "repeat": null, "room": room, "note": null, "noteAt": null, "tasks": [],
            "watch": null, "checkin": null
        }))
        .unwrap()
    }

    #[test]
    fn a_row_shows_when_it_matches_every_filter_on() {
        let now = "2026-11-24T09:00:00Z"
            .parse::<Timestamp>()
            .unwrap()
            .to_zoned_datetime(TimeZone::UTC);
        let watch = item("watch", 2, Some("2026-11-24T12:00:00.000Z"));
        let unit = item("unit", 1, None);
        let none = ScheduledFilter::default();
        assert!(none.is_empty() && none.takes(&watch, &now) && none.takes(&unit, &now));
        let room = ScheduledFilter {
            room: Some(RoomId::new(2)),
            ..ScheduledFilter::default()
        };
        assert!(room.takes(&watch, &now) && !room.takes(&unit, &now));
        let today = ScheduledFilter {
            kind: Some(ScheduledKind::Watch),
            runs: Some(Runs::Today),
            ..ScheduledFilter::default()
        };
        assert_eq!(today.count(), 2);
        assert!(today.takes(&watch, &now));
        assert!(!today.takes(&unit, &now), "neither a watch nor run today");
    }

    #[test]
    fn the_page_shows_its_groups_soonest_first_and_counts_what_is_hidden() {
        let now = "2026-11-24T09:00:00Z"
            .parse::<Timestamp>()
            .unwrap()
            .to_zoned_datetime(TimeZone::UTC);
        let items = vec![
            item("watch", 2, Some("2026-11-24T09:30:00.000Z")),
            item("routine", 1, Some("2026-11-25T05:30:00.000Z")),
            item("unit", 1, None),
            item("checkin", 1, Some("2026-11-24T09:45:00.000Z")),
        ];
        let all = shown(&items, &ScheduledFilter::default(), &now);
        let groups: Vec<(NextRun, usize)> = all
            .groups
            .iter()
            .map(|(group, rows)| (*group, rows.len()))
            .collect();
        assert_eq!(
            groups,
            [
                (NextRun::Hour, 2),
                (NextRun::Tomorrow, 1),
                (NextRun::None, 1)
            ]
        );
        assert_eq!(all.hidden, 0);
        let room = ScheduledFilter {
            room: Some(RoomId::new(2)),
            ..ScheduledFilter::default()
        };
        let one = shown(&items, &room, &now);
        assert_eq!(one.groups.len(), 1);
        assert_eq!(one.hidden, 3);
        assert!(shown(&[], &room, &now).is_empty());
    }

    #[test]
    fn kinds_have_the_pages_names() {
        let names: Vec<String> = KINDS.iter().map(kind_name).collect();
        assert_eq!(
            names,
            ["Watch", "Check-in", "Routine", "Meeting prep", "Timer unit"]
        );
        assert_eq!(kind_name(&ScheduledKind::from("reminder")), "Reminder");
    }
}
