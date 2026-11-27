//! Talk, seen and answers.

use emberhut_api::answers::AnswerRequest;
use emberhut_api::error::OkReply;
use emberhut_api::ids::{MessageId, RoomId, TaskId};
use emberhut_api::seen::SeenRequest;
use emberhut_api::talk::{TalkReply, TalkRequest};

use crate::support::{assert_round_trip, read, to_value};

#[test]
fn talk_round_trips() {
    let request: TalkRequest = assert_round_trip("talk_request.json");
    assert_eq!(request.reply_to, Some(MessageId::new(39)));
    assert_eq!(request.room, RoomId::new(2));
    let reply: TalkReply = assert_round_trip("talk_reply.json");
    assert!(reply.ok && reply.started && reply.id.is_none());
}

#[test]
fn talk_without_a_room_is_about_main() {
    let request: TalkRequest = serde_json::from_str(r#"{"text":"Hello"}"#).unwrap();
    assert_eq!(request, TalkRequest::text(RoomId::MAIN, "Hello"));
    assert_eq!(
        to_value(&request),
        serde_json::json!({ "text": "Hello", "files": [], "room": 1 })
    );
}

#[test]
fn seen_round_trips() {
    let request: SeenRequest = assert_round_trip("seen_request.json");
    assert_eq!(request.room, RoomId::new(2));
    assert_eq!(
        serde_json::from_str::<SeenRequest>("{}").unwrap().room,
        RoomId::MAIN
    );
}

#[test]
fn an_answer_round_trips_and_its_reply_reads_as_ok() {
    let request: AnswerRequest = assert_round_trip("answer_request.json");
    assert_eq!(request, AnswerRequest::new(TaskId::new(12), "Report A"));
    assert!(!request.cancels());
    let reply: OkReply = read("answer_reply.json");
    assert!(reply.ok, "the state spread into the answer is skipped");
}

#[test]
fn a_project_change_round_trips_with_only_what_changes() {
    use emberhut_api::ids::ProjectId;
    use emberhut_api::projects::ProjectSetRequest;
    use emberhut_api::time::Day;

    let request: ProjectSetRequest = assert_round_trip("project_set_request.json");
    assert_eq!(
        request,
        ProjectSetRequest::pause_until(ProjectId::new(2), Day::new("2026-12-01"))
    );
    assert_eq!(
        to_value(&ProjectSetRequest::archive(ProjectId::new(2))),
        serde_json::json!({ "projectSet": { "id": 2, "status": "archived" } }),
        "a field that doesn't change is left out: the server touches only what is given"
    );
    assert_eq!(
        to_value(&ProjectSetRequest::activate(ProjectId::new(4))),
        serde_json::json!({ "projectSet": { "id": 4, "status": "active" } })
    );
}

#[test]
fn a_device_sign_out_round_trips() {
    use emberhut_api::auth::RevokeRequest;
    use emberhut_api::ids::DeviceId;

    let request: RevokeRequest = assert_round_trip("auth_revoke_request.json");
    assert_eq!(request.auth_revoke, DeviceId::new(3));
}
