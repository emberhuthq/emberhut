//! Each route: what the client sends and how it reads the answer.

use emberhut_api::answers::AnswerRequest;
use emberhut_api::chat::HistoryQuery;
use emberhut_api::headers::{self, ETag};
use emberhut_api::ids::{MessageId, RoomId, TaskId};
use emberhut_api::talk::TalkRequest;
use emberhut_api::task_actions::TaskAction;
use emberhut_api::task_message::{MessageTo, TaskMessageRequest};
use emberhut_api::task_search::TaskQuery;
use emberhut_api::theme::{AccentChange, Appearance, ThemeChange};
use emberhut_client::{ClientConfig, Error, MemorySessionStore, StatePoll, StateReply};
use serde_json::json;

use crate::support::{Reply, TestServer, access_for, fixture, fresh, signed_out};

const TAG: &str = "\"tag-1\"";

fn state_server() -> TestServer {
    TestServer::start(|request| {
        if request.header("if-none-match") == Some(TAG) {
            return Reply::Empty(304, vec![("ETag", TAG.to_owned())]);
        }
        Reply::JsonWith(200, fixture("state.json"), vec![("ETag", TAG.to_owned())])
    })
}

#[test]
fn a_state_poll_is_fresh_then_unchanged() {
    let server = state_server();
    let client = server.client(fresh("r1"));
    let room = RoomId::new(2);

    let StateReply::Fresh { state, etag } = client.state(&StatePoll::new(room)).unwrap() else {
        panic!("a fresh state")
    };
    assert_eq!(state.room, room);
    assert_eq!(etag, Some(ETag::new(TAG)));
    assert_eq!(
        client
            .state(&StatePoll::new(room).held(etag).looking(true))
            .unwrap(),
        StateReply::Unchanged
    );

    let [first, second] = server.requests_to("/api/state").try_into().unwrap();
    assert_eq!(first.query, "room=2");
    assert_eq!(
        first.header("authorization"),
        Some(format!("Bearer {}", access_for("r1")).as_str())
    );
    assert_eq!(
        first.header(headers::LOOKING),
        None,
        "only while the room is on screen"
    );
    assert_eq!(first.header("if-none-match"), None);
    assert_eq!(second.header(headers::LOOKING), Some("2"));
    assert_eq!(second.header("if-none-match"), Some(TAG));
}

#[test]
fn a_device_without_a_session_sends_no_token() {
    let server = state_server();
    let client = server.client(signed_out());
    assert!(matches!(
        client.state(&StatePoll::new(RoomId::MAIN)).unwrap(),
        StateReply::Fresh { .. }
    ));
    assert_eq!(
        server.requests_to("/api/state")[0].header("authorization"),
        None
    );
}

#[test]
fn a_state_poll_can_need_sign_in() {
    let server = TestServer::start(|_| Reply::Json(401, fixture("error_sign_in.json")));
    let client = server.client(signed_out());
    assert_eq!(
        client.state(&StatePoll::new(RoomId::MAIN)).unwrap(),
        StateReply::SignInRequired
    );
}

#[test]
fn a_device_that_isnt_let_in_is_a_refusal() {
    let server = TestServer::start(|_| Reply::Json(401, fixture("error_not_let_in.json")));
    let client = server.client(signed_out());
    let error = client.state(&StatePoll::new(RoomId::MAIN)).unwrap_err();
    assert!(
        matches!(&error, Error::Status { status: 401, message: Some(m) } if m == "not one of the owner's devices"),
        "{error:?}"
    );
}

#[test]
fn a_state_that_doesnt_read_is_a_decode_error() {
    let server = TestServer::start(|_| Reply::Json(200, json!({ "room": "not a room" })));
    let error = server
        .client(signed_out())
        .state(&StatePoll::new(RoomId::MAIN))
        .unwrap_err();
    assert!(matches!(error, Error::Decode(_)), "{error:?}");
}

#[test]
fn a_server_failure_is_transient() {
    let server = TestServer::start(|_| Reply::Text(500, "boom"));
    let error = server
        .client(signed_out())
        .state(&StatePoll::new(RoomId::MAIN))
        .unwrap_err();
    assert!(
        matches!(
            error,
            Error::Status {
                status: 500,
                message: None
            }
        ),
        "{error:?}"
    );
    assert!(error.is_transient());
}

#[test]
fn history_asks_for_a_page() {
    let server = TestServer::start(|_| Reply::Json(200, fixture("history.json")));
    let client = server.client(fresh("r1"));
    let page = client
        .history(&HistoryQuery::before(RoomId::new(2), MessageId::new(40)))
        .unwrap();
    assert_eq!(page.chat.len(), 2);
    assert_eq!(
        server.requests_to("/api/history")[0].query,
        "room=2&before=40"
    );
}

#[test]
fn talk_posts_the_message() {
    let server = TestServer::start(|_| Reply::Json(200, fixture("talk_reply.json")));
    let client = server.client(fresh("r1"));
    let request = TalkRequest::text(RoomId::new(2), "Hello");
    let reply = client.talk(&request).unwrap();
    assert!(reply.started);
    let sent = &server.requests_to("/api/talk")[0];
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.header("content-type"), Some("application/json"));
    assert_eq!(
        sent.json(),
        json!({ "text": "Hello", "files": [], "room": 2 })
    );
}

#[test]
fn talk_can_need_sign_in() {
    let server = TestServer::start(|_| Reply::Json(401, fixture("error_sign_in.json")));
    let error = server
        .client(signed_out())
        .talk(&TalkRequest::text(RoomId::MAIN, "Hello"))
        .unwrap_err();
    assert!(matches!(error, Error::SignInRequired), "{error:?}");
}

#[test]
fn seen_posts_the_room() {
    let server = TestServer::start(|_| Reply::Json(200, fixture("ok.json")));
    server.client(fresh("r1")).seen(RoomId::new(2)).unwrap();
    assert_eq!(
        server.requests_to("/api/seen")[0].json(),
        json!({ "room": 2 })
    );
}

#[test]
fn an_answer_goes_to_settings() {
    let server = TestServer::start(|request| match request.json()["taskDecide"].as_i64() {
        Some(12) => Reply::Json(200, fixture("answer_reply.json")),
        _ => Reply::Json(400, fixture("error_refused.json")),
    });
    let client = server.client(fresh("r1"));
    client
        .answer(&AnswerRequest::new(TaskId::new(12), "Report A"))
        .unwrap();
    assert_eq!(
        server.requests_to("/api/settings")[0].json(),
        json!({ "taskDecide": 12, "choice": "Report A" })
    );
    let error = client
        .answer(&AnswerRequest::new(TaskId::new(999), "Done"))
        .unwrap_err();
    assert!(
        matches!(&error, Error::Status { status: 400, message: Some(m) } if m == "no task T-999"),
        "{error:?}"
    );
}

#[test]
fn machines_reads_the_page() {
    let server = TestServer::start(|_| Reply::Json(200, fixture("machines.json")));
    let page = server.client(fresh("r1")).machines().unwrap();
    assert_eq!(page.machines.len(), 2);
    assert_eq!(server.requests_to("/api/machines")[0].query, "");
}

#[test]
fn the_theme_is_read_and_changed() {
    let server = TestServer::start(|request| match request.method.as_str() {
        "GET" => Reply::Json(200, fixture("theme_page.json")),
        _ if request.json()["accent"] == "blue" => {
            Reply::Json(400, json!({ "ok": false, "error": "accent: a hex colour" }))
        }
        _ => Reply::Json(200, fixture("theme_set_reply.json")),
    });
    let client = server.client(fresh("r1"));
    assert_eq!(client.theme().unwrap().choice.appearance, Appearance::Dark);
    let change = ThemeChange {
        appearance: Some(Appearance::Dark),
        accent: AccentChange::Reset,
        ..ThemeChange::default()
    };
    assert_eq!(client.set_theme(&change).unwrap().version, 1);
    assert_eq!(
        server.requests_to("/api/theme")[1].json(),
        json!({ "appearance": "dark", "accent": null })
    );
    let bad = ThemeChange {
        accent: AccentChange::Set("blue".to_owned()),
        ..ThemeChange::default()
    };
    let error = client.set_theme(&bad).unwrap_err();
    assert!(
        matches!(&error, Error::Status { status: 400, message: Some(m) } if m.starts_with("accent")),
        "{error:?}"
    );
}

#[test]
fn a_pairing_code_is_made_on_a_signed_in_device() {
    let server = TestServer::start(|_| Reply::Json(200, fixture("pair_code.json")));
    let code = server.client(fresh("r1")).new_pairing_code().unwrap();
    assert_eq!(code.code, "ABCD-EFGH");
    let sent = &server.requests_to("/api/auth/pair/new")[0];
    assert_eq!(
        sent.header("authorization"),
        Some(format!("Bearer {}", access_for("r1")).as_str())
    );
}

#[test]
fn a_server_that_isnt_there_is_a_network_error() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let config = ClientConfig::new(&format!("http://127.0.0.1:{port}")).unwrap();
    let client = emberhut_client::Client::new(&config, MemorySessionStore::new()).unwrap();
    let error = client.state(&StatePoll::new(RoomId::MAIN)).unwrap_err();
    assert!(matches!(error, Error::Network(_)), "{error:?}");
    assert!(error.is_transient());
    assert_eq!(client.base_url(), format!("http://127.0.0.1:{port}"));
}

#[test]
fn a_file_comes_back_as_its_bytes() {
    let server = TestServer::start(|request| match request.path.as_str() {
        "/uploads/a.png" => Reply::Text(200, "PNG bytes"),
        _ => Reply::Json(404, json!({ "error": "No such file" })),
    });
    let client = server.client(fresh("r1"));
    assert_eq!(client.file("/uploads/a.png").unwrap(), b"PNG bytes");
    let missing = client.file("/uploads/b.png").unwrap_err();
    assert!(
        matches!(missing, Error::Status { status: 404, ref message } if message.as_deref() == Some("No such file")),
        "{missing:?}"
    );
    let sent = &server.requests_to("/uploads/a.png")[0];
    assert_eq!(sent.method, "GET");
    assert!(
        sent.header("authorization").is_some(),
        "files are behind the sign-in"
    );
}

#[test]
fn a_file_with_an_at_sign_in_its_name_comes_from_the_same_server() {
    let server = TestServer::start(|request| match request.path.as_str() {
        "/uploads/shot@2x.png" => Reply::Text(200, "PNG bytes"),
        _ => Reply::Json(404, json!({ "error": "No such file" })),
    });
    let client = server.client(fresh("r1"));
    assert_eq!(client.file("/uploads/shot@2x.png").unwrap(), b"PNG bytes");
    assert_eq!(server.requests_to("/uploads/shot@2x.png").len(), 1);
}

#[test]
fn a_file_outside_the_server_is_never_requested() {
    let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = std::sync::Arc::clone(&asked);
    let server = TestServer::start(move |_| {
        counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Reply::Text(200, "should not be asked")
    });
    let client = server.client(fresh("r1"));
    for path in [
        "@evil.example/x",
        ".evil.net/x",
        "//evil/x",
        "https://evil/x",
    ] {
        let error = client.file(path).unwrap_err();
        assert!(matches!(error, Error::InvalidPath(_)), "{path}: {error:?}");
    }
    assert_eq!(
        asked.load(std::sync::atomic::Ordering::Relaxed),
        0,
        "nothing reached the server"
    );
}

#[test]
fn a_project_change_goes_to_settings() {
    use emberhut_api::ids::ProjectId;
    use emberhut_api::projects::ProjectSetRequest;
    use emberhut_api::time::Day;

    let server = TestServer::start(
        |request| match request.json()["projectSet"]["id"].as_i64() {
            Some(2) => Reply::Json(200, fixture("answer_reply.json")),
            _ => Reply::Json(400, fixture("error_refused.json")),
        },
    );
    let client = server.client(fresh("r1"));
    client
        .set_project(&ProjectSetRequest::pause_until(
            ProjectId::new(2),
            Day::new("2026-12-01"),
        ))
        .unwrap();
    assert_eq!(
        server.requests_to("/api/settings")[0].json(),
        json!({ "projectSet": { "id": 2, "status": "paused", "pausedUntil": "2026-12-01" } })
    );
    let error = client
        .set_project(&ProjectSetRequest::archive(ProjectId::new(9)))
        .unwrap_err();
    assert!(
        matches!(&error, Error::Status { status: 400, .. }),
        "{error:?}"
    );
}

#[test]
fn a_device_sign_out_goes_to_settings() {
    use emberhut_api::ids::DeviceId;

    let server = TestServer::start(|_| Reply::Json(200, fixture("answer_reply.json")));
    server
        .client(fresh("r1"))
        .revoke_device(DeviceId::new(3))
        .unwrap();
    assert_eq!(
        server.requests_to("/api/settings")[0].json(),
        json!({ "authRevoke": 3 })
    );
}

#[test]
fn a_task_page_is_read_by_its_id() {
    let server = TestServer::start(|request| match request.path.as_str() {
        "/api/tasks/12" => Reply::Json(200, fixture("task_page.json")),
        _ => Reply::Json(404, json!({ "error": "no such task" })),
    });
    let client = server.client(fresh("r1"));
    let page = client.task(TaskId::new(12)).unwrap();
    assert_eq!(page.task.key, "T-12");
    assert_eq!(page.history.len(), 4);
    let error = client.task(TaskId::new(99)).unwrap_err();
    assert!(
        matches!(&error, Error::Status { status: 404, message: Some(m) } if m == "no such task"),
        "{error:?}"
    );
}

#[test]
fn a_task_search_sends_its_narrowings() {
    let server = TestServer::start(|_| Reply::Json(200, fixture("tasks_found.json")));
    let query = TaskQuery {
        limit: Some(200),
        archived: true,
        ..TaskQuery::search("the report")
    };
    let found = server.client(fresh("r1")).find_tasks(&query).unwrap();
    assert_eq!(found.tasks.len(), 2);
    assert_eq!(
        server.requests_to("/api/tasks")[0].query,
        "q=the%20report&limit=200&archived=1"
    );
}

#[test]
fn a_message_on_a_task_goes_to_its_page() {
    let server = TestServer::start(|_| Reply::Json(200, fixture("task_message_reply.json")));
    let reply = server
        .client(fresh("r1"))
        .message_task(
            TaskId::new(12),
            &TaskMessageRequest {
                text: "Make it shorter".to_owned(),
                files: Vec::new(),
            },
        )
        .unwrap();
    assert_eq!(reply.to, MessageTo::Worker);
    assert_eq!(
        server.requests_to("/api/tasks/12/message")[0].json(),
        json!({ "text": "Make it shorter", "files": [] })
    );
}

#[test]
fn a_task_action_goes_to_settings() {
    let server = TestServer::start(|request| match request.json()["taskArchive"].as_i64() {
        Some(12) => Reply::Json(200, fixture("answer_reply.json")),
        _ => Reply::Json(400, fixture("error_refused.json")),
    });
    let client = server.client(fresh("r1"));
    client
        .act_on_tasks(&TaskAction::archive(TaskId::new(12)))
        .unwrap();
    assert_eq!(
        server.requests_to("/api/settings")[0].json(),
        json!({ "taskArchive": 12 })
    );
    assert!(matches!(
        client.act_on_tasks(&TaskAction::archive(TaskId::new(9))),
        Err(Error::Status { status: 400, .. })
    ));
}

#[test]
fn what_is_scheduled_is_one_list() {
    let server = TestServer::start(|_| Reply::Json(200, fixture("scheduled.json")));
    let page = server.client(fresh("r1")).scheduled().unwrap();
    assert_eq!(page.items.len(), 4);
    assert_eq!(server.requests_to("/api/scheduled")[0].query, "");
}

#[test]
fn the_calendar_asks_for_its_days() {
    use emberhut_api::calendar::CalendarQuery;
    use emberhut_api::time::Day;

    let server = TestServer::start(|_| Reply::Json(200, fixture("calendar.json")));
    let query = CalendarQuery::new(Day::new("2026-11-23"), Day::new("2026-11-29"));
    let page = server.client(fresh("r1")).calendar(&query).unwrap();
    assert_eq!(page.events.len(), 2);
    assert_eq!(
        server.requests_to("/api/calendar")[0].query,
        "from=2026-11-23&to=2026-11-29"
    );
}

#[test]
fn a_note_is_saved_by_its_uid_and_the_diary_read() {
    use emberhut_api::ids::NoteUid;
    use emberhut_api::notes::NoteSave;
    use emberhut_api::time::Timestamp;

    let server = TestServer::start(|request| {
        if request.method == "POST" {
            Reply::Json(200, fixture("note_saved.json"))
        } else {
            Reply::Json(200, fixture("notes.json"))
        }
    });
    let client = server.client(fresh("r1"));
    let saved = client
        .save_note(&NoteSave {
            uid: NoteUid::new("9d3c2b1a-0000-4000-8000-000000000001"),
            text: "Buy the example part.".to_owned(),
            at: Some(Timestamp::new("2026-11-24T07:15:00.000Z")),
        })
        .unwrap();
    assert!(saved.note.is_some());
    assert_eq!(client.notes().unwrap().notes.len(), 2);
    let [save, list] = server.requests_to("/api/notes").try_into().unwrap();
    assert_eq!(save.json(), fixture("note_save.json"));
    assert_eq!(list.method, "GET");
}

#[test]
fn a_note_the_server_wont_take_is_its_refusal() {
    use emberhut_api::ids::NoteUid;
    use emberhut_api::notes::NoteSave;

    let server = TestServer::start(|_| {
        Reply::Json(400, json!({ "ok": false, "error": "a note needs its uid" }))
    });
    let error = server
        .client(fresh("r1"))
        .save_note(&NoteSave {
            uid: NoteUid::new("short"),
            text: "Hello".to_owned(),
            at: None,
        })
        .unwrap_err();
    assert!(
        matches!(&error, Error::Status { status: 400, message: Some(m) } if m == "a note needs its uid"),
        "{error:?}"
    );
}
