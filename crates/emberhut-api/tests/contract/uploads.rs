//! Uploads, and the talk that sends what they stored.

use emberhut_api::chat::FileKind;
use emberhut_api::error::ErrorBody;
use emberhut_api::ids::RoomId;
use emberhut_api::talk::TalkRequest;
use emberhut_api::uploads::{UploadQuery, UploadReply};
use serde_json::json;

use crate::support::{assert_round_trip, fixture, to_value};

#[test]
fn an_upload_reply_round_trips() {
    let reply: UploadReply = assert_round_trip("upload_reply.json");
    assert!(reply.ok);
    assert_eq!(reply.file.name, "Week 3 example.png");
    assert_eq!(reply.file.kind, FileKind::Image);
    assert_eq!(
        reply.file.path.as_deref(),
        Some("/data/uploads/1a2b3c4d-Week 3 example.png")
    );
}

#[test]
fn a_refused_upload_is_an_error_body() {
    let refused: ErrorBody = assert_round_trip("upload_refused.json");
    assert_eq!(refused.ok, Some(false));
    assert_eq!(refused.error, "file is larger than 500 MB");
}

#[test]
fn the_stored_file_goes_back_whole_with_the_talk() {
    let reply: UploadReply = assert_round_trip("upload_reply.json");
    let request = TalkRequest::text(RoomId::new(2), "").with_files(vec![reply.file]);
    assert_eq!(
        to_value(&request),
        json!({ "text": "", "files": [fixture("upload_reply.json")["file"]], "room": 2 })
    );
}

#[test]
fn the_query_carries_the_name_escaped() {
    assert_eq!(
        UploadQuery::new("Week 3 example.png").to_query(),
        "name=Week%203%20example.png"
    );
}
