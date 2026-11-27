//! Following the live stream.

use std::sync::atomic::{AtomicUsize, Ordering};

use emberhut_api::approvals::{ApprovalState, ApprovalsEvent};
use emberhut_api::ids::ApprovalId;
use emberhut_api::ids::RoomId;
use emberhut_api::live::{Live, LiveEvent};
use emberhut_client::{Error, LiveEnd, RoomEvent};

use crate::support::{Reply, TestServer, fixture, fresh, signed_out};

const STREAM: [&str; 4] = [
    "data: {\"text\":\"\",\"doing\":\"Reading your new messages\"}\n\n",
    ": ping\n\n",
    "data: {\"text\":\"\",\"doing\":\"Checking\"}\n",
    "\ndata: null\n\n",
];

#[test]
fn hands_on_each_event_until_the_server_closes() {
    let server = TestServer::start(|_| Reply::Stream(STREAM.to_vec()));
    let client = server.client(fresh("r1"));
    let mut events = Vec::new();
    let end = client
        .follow_live(RoomId::new(2), || false, |e| events.push(e))
        .unwrap();
    assert_eq!(end, LiveEnd::Closed);
    let working = |doing: &str| {
        LiveEvent::Working(Live {
            text: String::new(),
            doing: Some(doing.to_owned()),
        })
    };
    assert_eq!(
        events,
        [
            working("Reading your new messages"),
            working("Checking"),
            LiveEvent::Idle
        ]
    );
    let sent = &server.requests_to("/api/live")[0];
    assert_eq!(sent.query, "room=2");
    assert_eq!(sent.header("accept"), Some("text/event-stream"));
    assert!(sent.header("authorization").is_some());
}

#[test]
fn a_rooms_stream_hands_on_the_approvals_changes_too() {
    let stream = [
        "event: approvals\ndata: {\"id\":7,\"state\":\"approved\",\"pending\":0}\n\n",
        "data: null\n\n",
    ];
    let server = TestServer::start(move |_| Reply::Stream(stream.to_vec()));
    let client = server.client(fresh("r1"));
    let mut events = Vec::new();
    client
        .follow_room(RoomId::new(2), || false, |e| events.push(e))
        .unwrap();
    let changed = ApprovalsEvent {
        id: ApprovalId::new(7),
        state: ApprovalState::Approved,
        pending: 0,
    };
    assert_eq!(
        events,
        [
            RoomEvent::Approvals(changed),
            RoomEvent::Live(LiveEvent::Idle)
        ]
    );
    assert_eq!(server.requests_to("/api/live")[0].query, "room=2");
}

#[test]
fn stops_when_asked() {
    let server = TestServer::start(|_| Reply::Stream(STREAM.to_vec()));
    let items = AtomicUsize::new(0);
    let mut events = Vec::new();
    let end = server
        .client(fresh("r1"))
        .follow_live(
            RoomId::MAIN,
            || items.fetch_add(1, Ordering::SeqCst) >= 1,
            |e| events.push(e),
        )
        .unwrap();
    assert_eq!(end, LiveEnd::Stopped);
    assert_eq!(events.len(), 1, "stopped at the ping after the first event");
}

#[test]
fn can_need_sign_in() {
    let server = TestServer::start(|_| Reply::Json(401, fixture("error_sign_in.json")));
    let error = server
        .client(signed_out())
        .follow_live(RoomId::MAIN, || false, |_| {})
        .unwrap_err();
    assert!(matches!(error, Error::SignInRequired), "{error:?}");
}

#[test]
fn a_stopped_client_ends_its_stream_and_sends_nothing_more() {
    let server = TestServer::start(|_| Reply::Stream(STREAM.to_vec()));
    let client = server.client(fresh("r1"));
    let mut events = Vec::new();
    let end = client
        .follow_live(
            RoomId::MAIN,
            || false,
            |event| {
                events.push(event);
                client.stop().unwrap();
            },
        )
        .unwrap();
    assert_eq!(end, LiveEnd::Stopped, "ended at the item after the stop");
    assert_eq!(events.len(), 1);
    assert!(client.is_stopped());
    assert!(matches!(
        client.follow_live(RoomId::MAIN, || false, |_| {}),
        Err(Error::Stopped)
    ));
    assert!(matches!(client.pair("123456"), Err(Error::Stopped)));
    assert_eq!(
        server.requests_to("/api/live").len(),
        1,
        "nothing sent after"
    );
    assert!(server.requests_to("/api/auth/pair").is_empty());
}
