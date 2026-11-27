//! History pages and joining them into a held chat.

use emberhut_api::chat::{ChatPage, FileKind, join};
use emberhut_api::ids::{MessageId, TaskId};

use crate::support::{assert_round_trip, fixture, from_value, read, with_unknown_fields};

#[test]
fn a_history_page_round_trips() {
    let page: ChatPage = assert_round_trip("history.json");
    assert_eq!(page.chat.len(), 2);
    assert!(!page.is_full());
    let report = &page.chat[1];
    assert!(report.files[0].is_html());
    assert_eq!(report.files[0].kind, FileKind::File);
    assert_eq!(report.refs[0].task, Some(TaskId::new(12)));
    assert_eq!(report.refs[0].name.as_deref(), Some("report.html"));
}

#[test]
fn a_history_page_tolerates_unknown_fields() {
    let mut body = fixture("history.json");
    with_unknown_fields(&mut body);
    assert_eq!(
        from_value::<ChatPage>(body),
        read::<ChatPage>("history.json")
    );
}

#[test]
fn an_earlier_page_joins_before_what_is_held() {
    let held = read::<emberhut_api::state::State>("state.json").chat;
    let page: ChatPage = read("history.json");
    let joined = join(&held, &page.chat);
    let ids: Vec<MessageId> = joined.iter().map(|m| m.id).collect();
    assert_eq!(ids, [38, 39, 40, 41, 42].map(MessageId::new));
}
