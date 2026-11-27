//! The pages about the person's days: what is scheduled, the calendar, and the diary's notes.

use emberhut_api::calendar::{CalendarItemKind, CalendarPage, CalendarQuery, EventKind};
use emberhut_api::ids::{NoteUid, ProjectId, RoomId, TaskId, WatchId};
use emberhut_api::notes::{NoteAuthor, NoteSave, NoteSaved, NotesPage};
use emberhut_api::scheduled::{ScheduledKind, ScheduledPage};
use emberhut_api::tasks::TaskStatus;
use emberhut_api::time::{Day, Timestamp};
use serde_json::json;

use crate::support::{
    assert_round_trip, assert_writes_shape, fixture, from_value, to_value, with_unknown_fields,
};

#[test]
fn reads_what_is_scheduled() {
    let page: ScheduledPage = assert_writes_shape("scheduled.json", &[]);
    let [checkin, watch, routine, unit] = page.items.as_slice() else {
        panic!("four rows")
    };
    assert_eq!(checkin.kind, ScheduledKind::Checkin);
    assert_eq!(
        checkin.checkin,
        Some(Timestamp::new("2026-11-24T10:30:00.000Z"))
    );
    assert_eq!(checkin.tasks[0].status, TaskStatus::Working);
    assert_eq!(watch.watch, Some(WatchId::new(3)));
    assert_eq!(watch.room, RoomId::new(2));
    assert_eq!(watch.repeat.as_deref(), Some("every 4 h"));
    assert_eq!(routine.kind, ScheduledKind::Routine);
    assert_eq!(unit.next_at, None, "a unit with no next run");
}

#[test]
fn what_is_scheduled_tolerates_unknown_fields_and_kinds() {
    let mut body = fixture("scheduled.json");
    with_unknown_fields(&mut body);
    assert_eq!(
        from_value::<ScheduledPage>(body),
        from_value::<ScheduledPage>(fixture("scheduled.json"))
    );
    let mut body = fixture("scheduled.json");
    body["items"][0]["kind"] = json!("reminder");
    let page: ScheduledPage = from_value(body);
    assert!(!page.items[0].kind.is_known());
}

#[test]
fn reads_the_calendar() {
    let page: CalendarPage = assert_writes_shape("calendar.json", &[]);
    let [meeting, day_off] = page.events.as_slice() else {
        panic!("two events")
    };
    assert_eq!(meeting.kind, EventKind::Meeting);
    assert_eq!(meeting.location.as_deref(), Some("Room 2"));
    assert!(day_off.all_day && day_off.notes.is_none());
    let [decide, deadline, project, today] = page.items.as_slice() else {
        panic!("four items")
    };
    assert_eq!(decide.kind, CalendarItemKind::Decide);
    assert_eq!(decide.action, Some(None), "written as null on an ask's day");
    assert_eq!(decide.action(), None);
    assert!(deadline.hard && deadline.action.is_none());
    assert_eq!(project.task, None);
    assert_eq!(project.project, Some(ProjectId::new(3)));
    assert_eq!(project.count, Some(2));
    assert_eq!(today.task, Some(TaskId::new(15)));
    assert_eq!(today.kind, CalendarItemKind::Today);
}

#[test]
fn the_calendar_tolerates_unknown_fields() {
    let mut body = fixture("calendar.json");
    with_unknown_fields(&mut body);
    assert_eq!(
        from_value::<CalendarPage>(body),
        from_value::<CalendarPage>(fixture("calendar.json"))
    );
}

#[test]
fn an_asks_action_reads_when_it_names_one() {
    let mut body = fixture("calendar.json");
    body["items"][0]["action"] = json!("Pick a name");
    let page: CalendarPage = from_value(body);
    assert_eq!(page.items[0].action(), Some("Pick a name"));
}

#[test]
fn the_calendars_range_is_its_query() {
    let query = CalendarQuery::new(Day::new("2026-11-23"), Day::new("2026-11-29"));
    assert_eq!(query.to_query(), "from=2026-11-23&to=2026-11-29");
}

#[test]
fn reads_the_notes() {
    let page: NotesPage = assert_writes_shape("notes.json", &[]);
    let [entry, note] = page.notes.as_slice() else {
        panic!("two notes")
    };
    assert!(entry.is_assistants());
    assert_eq!(note.who, NoteAuthor::Person);
    assert!(!note.is_assistants());
    assert_eq!(note.day, Day::new("2026-11-24"));
}

#[test]
fn a_note_saved_round_trips() {
    let request: NoteSave = assert_round_trip("note_save.json");
    assert_eq!(
        request.uid,
        NoteUid::new("9d3c2b1a-0000-4000-8000-000000000001")
    );
    let reply: NoteSaved = assert_writes_shape("note_saved.json", &[]);
    assert!(reply.ok && reply.note.is_some());
    let deleted: NoteSaved = assert_writes_shape("note_deleted.json", &[]);
    assert_eq!(deleted.note, None);
}

#[test]
fn a_note_without_a_time_leaves_the_key_out() {
    let request = NoteSave {
        uid: NoteUid::new("abcdefgh"),
        text: String::new(),
        at: None,
    };
    assert_eq!(to_value(&request), json!({ "uid": "abcdefgh", "text": "" }));
}
