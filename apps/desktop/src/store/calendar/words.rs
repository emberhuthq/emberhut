//! What the calendar's chips and details say: an event's time (`9:00 AM – 10:30 AM`, `All day`)
//! and kind, a work item's words and what it is (`Deadline`, `Hard deadline`, `Decide by`,
//! `Planned today`), and the legend's names. The colours are the view's; what each thing is
//! ([`Tone`]) is decided here.

use emberhut_api::calendar::{CalendarEvent, CalendarItem, CalendarItemKind, EventKind};
use jiff::tz::TimeZone;

use super::layout::span;
use crate::store::tasks::clock::clock_minutes;

/// What a chip or block is, for its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Tone {
    /// A meeting: someone else is in it.
    Meeting,
    /// An event of the user's alone.
    Event,
    /// Time blocked to work.
    Focus,
    /// A deadline.
    Deadline,
    /// A hard deadline.
    Hard,
    /// The day an ask has to be decided by.
    Decide,
    /// Planned for today.
    Today,
}

impl Tone {
    /// The legend, in its order.
    pub(crate) const LEGEND: [Self; 7] = [
        Self::Meeting,
        Self::Event,
        Self::Focus,
        Self::Deadline,
        Self::Hard,
        Self::Decide,
        Self::Today,
    ];

    /// Its name in the legend.
    #[must_use]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Meeting => "Meeting",
            Self::Event => "Event",
            Self::Focus => "Focus",
            Self::Deadline => "Deadline",
            Self::Hard => "Hard deadline",
            Self::Decide => "Decide by",
            Self::Today => "Planned today",
        }
    }

    /// An event's tone.
    #[must_use]
    pub(crate) fn of_event(event: &CalendarEvent) -> Self {
        match event.kind {
            EventKind::Focus => Self::Focus,
            EventKind::Meeting => Self::Meeting,
            _ => Self::Event,
        }
    }

    /// A work item's tone.
    #[must_use]
    pub(crate) fn of_item(item: &CalendarItem) -> Self {
        match item.kind {
            CalendarItemKind::Decide => Self::Decide,
            CalendarItemKind::Today => Self::Today,
            _ if item.hard => Self::Hard,
            _ => Self::Deadline,
        }
    }
}

/// The line under the legend.
pub(crate) const LEGEND_NOTE: &str = "Events keep the names they have on your calendar.";

/// An event's kind, in its details.
#[must_use]
pub(crate) fn kind_name(event: &CalendarEvent) -> &'static str {
    match event.kind {
        EventKind::Focus => "Focus time",
        EventKind::Meeting => "Meeting",
        _ => "Event",
    }
}

/// An event's time in `zone`: `9:00 AM – 10:30 AM`, or `All day`.
#[must_use]
pub(crate) fn event_time(event: &CalendarEvent, zone: &TimeZone) -> String {
    if event.all_day {
        return "All day".to_owned();
    }
    let (starts, ends) = span(event);
    format!(
        "{} – {}",
        clock_minutes(&starts.to_zoned_datetime(zone.clone())),
        clock_minutes(&ends.to_zoned_datetime(zone.clone()))
    )
}

/// When an event starts, on its block: `9:00 AM`.
#[must_use]
pub(crate) fn start_time(event: &CalendarEvent, zone: &TimeZone) -> String {
    clock_minutes(&span(event).0.to_zoned_datetime(zone.clone()))
}

/// The day an event starts, in its details: `Tuesday 24 November`.
#[must_use]
pub(crate) fn event_day(event: &CalendarEvent, zone: &TimeZone) -> String {
    span(event)
        .0
        .to_zoned_datetime(zone.clone())
        .strftime("%A %-d %B")
        .to_string()
}

/// A work item's words on its chip: an ask's action (else its title), `Launch due · 2 tasks`
/// for a project's deadline, `Send it due` for a task's.
#[must_use]
pub(crate) fn item_label(item: &CalendarItem) -> String {
    match item.kind {
        CalendarItemKind::Decide => item.action().unwrap_or(&item.title).to_owned(),
        CalendarItemKind::Deadline if item.task.is_none() => {
            let tasks = item
                .count
                .filter(|count| *count > 0)
                .map_or_default(|count| {
                    format!(" · {count} task{}", if count == 1 { "" } else { "s" })
                });
            format!("{} due{tasks}", item.title)
        }
        CalendarItemKind::Deadline => format!("{} due", item.title),
        _ => item.title.clone(),
    }
}

/// A work item's chip text: in a week or month, its label; on a day, an ask's reads
/// `Decide: …`.
#[must_use]
pub(crate) fn chip_text(item: &CalendarItem, compact: bool) -> String {
    let label = item_label(item);
    if compact || item.kind != CalendarItemKind::Decide {
        label
    } else {
        format!("Decide: {label}")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn item(value: &serde_json::Value) -> CalendarItem {
        let mut base = json!({
            "kind": "deadline", "day": "2026-11-24", "task": 12, "key": "K-12", "project": null,
            "title": "Send it", "hard": false, "status": "todo"
        });
        if let (Some(base), Some(extra)) = (base.as_object_mut(), value.as_object()) {
            base.extend(extra.clone());
        }
        serde_json::from_value(base).unwrap()
    }

    #[test]
    fn work_items_say_what_they_are() {
        let deadline = item(&json!({}));
        assert_eq!(item_label(&deadline), "Send it due");
        assert_eq!(Tone::of_item(&deadline), Tone::Deadline);
        let hard = item(&json!({"hard": true}));
        assert_eq!(Tone::of_item(&hard), Tone::Hard);
        let project = item(&json!({"task": null, "project": 3, "title": "Launch", "count": 2}));
        assert_eq!(item_label(&project), "Launch due · 2 tasks");
        let one = item(&json!({"task": null, "project": 3, "title": "Launch", "count": 1}));
        assert_eq!(item_label(&one), "Launch due · 1 task");
        let ask = item(&json!({"kind": "decide", "action": "Pick a name"}));
        assert_eq!(item_label(&ask), "Pick a name");
        assert_eq!(chip_text(&ask, false), "Decide: Pick a name");
        assert_eq!(chip_text(&ask, true), "Pick a name");
        let unnamed = item(&json!({"kind": "decide", "action": null}));
        assert_eq!(item_label(&unnamed), "Send it");
        let today = item(&json!({"kind": "today", "hard": true}));
        assert_eq!(Tone::of_item(&today), Tone::Today);
        assert_eq!(item_label(&today), "Send it");
    }

    #[test]
    fn events_read_with_their_times() {
        let event: CalendarEvent = serde_json::from_value(json!({
            "id": "e", "title": "Review", "start": "2026-11-24T09:00:00.000Z",
            "end": "2026-11-24T10:30:00.000Z", "allDay": false, "calendar": "Work",
            "people": 3, "day": "2026-11-24", "kind": "meeting"
        }))
        .unwrap();
        let zone = TimeZone::UTC;
        assert_eq!(event_time(&event, &zone), "9:00 AM – 10:30 AM");
        assert_eq!(start_time(&event, &zone), "9:00 AM");
        assert_eq!(event_day(&event, &zone), "Tuesday 24 November");
        assert_eq!(kind_name(&event), "Meeting");
        assert_eq!(Tone::of_event(&event), Tone::Meeting);
        let all_day = CalendarEvent {
            all_day: true,
            kind: EventKind::Focus,
            ..event
        };
        assert_eq!(event_time(&all_day, &zone), "All day");
        assert_eq!(kind_name(&all_day), "Focus time");
        assert_eq!(Tone::LEGEND.map(Tone::name)[4], "Hard deadline");
    }
}
