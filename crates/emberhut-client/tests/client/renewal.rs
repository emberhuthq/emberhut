//! Renewing the access token: once, however many calls need it at the same moment.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Condvar, Mutex};
use std::time::Duration;

use emberhut_api::ids::RoomId;
use emberhut_client::{Error, MemorySessionStore, SessionStore, StatePoll, StateReply};
use serde_json::json;

use crate::support::{Reply, TestServer, access_for, expired, fixture};

/// A server that turns its one valid refresh token over on each renewal, as the reference server
/// does, and lets in only the access token of the current one.
struct Rotating {
    valid: Mutex<String>,
    renewals: AtomicUsize,
    /// Renewals that reached the server, and a way to wait for more.
    arrived: Mutex<usize>,
    arrival: Condvar,
    /// The first renewal waits (a second at most) until this many have arrived.
    hold_first_for: usize,
}

/// How many polls run at once in the race test.
const CALLERS: usize = 8;

impl Rotating {
    fn new(refresh: &str) -> Arc<Self> {
        Self::holding_first(refresh, 0)
    }

    fn holding_first(refresh: &str, callers: usize) -> Arc<Self> {
        Arc::new(Self {
            valid: Mutex::new(refresh.to_owned()),
            renewals: AtomicUsize::new(0),
            arrived: Mutex::new(0),
            arrival: Condvar::new(),
            hold_first_for: callers,
        })
    }

    /// Holds the first renewal until every caller's has arrived. Without single-flight all of
    /// them arrive, every time, and all reach the server; with it the others wait in the client,
    /// so the first one waits out the second and goes on alone.
    fn hold_first(&self) {
        let mut arrived = self.arrived.lock().unwrap();
        *arrived += 1;
        self.arrival.notify_all();
        if *arrived == 1 && self.hold_first_for > 1 {
            let (arrived, _) = self
                .arrival
                .wait_timeout_while(arrived, Duration::from_secs(1), |n| {
                    *n < self.hold_first_for
                })
                .unwrap();
            drop(arrived);
        }
    }

    fn answer(&self, request: &crate::support::Recorded) -> Reply {
        if request.path == "/api/auth/refresh" {
            self.renew(request)
        } else {
            self.state(request)
        }
    }

    fn renew(&self, request: &crate::support::Recorded) -> Reply {
        self.hold_first();
        let sent = request.json()["refresh"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let mut valid = self.valid.lock().unwrap();
        if sent != *valid {
            return Reply::Json(
                401,
                json!({ "ok": false, "status": 401, "error": "Sign in again." }),
            );
        }
        let n = self.renewals.fetch_add(1, Ordering::SeqCst) + 1;
        *valid = format!("r{}", n + 1);
        Reply::Json(
            200,
            json!({ "ok": true, "status": 200, "access": access_for(&valid), "refresh": *valid, "device": 3,
                    "expires": "2099-01-01T00:00:00.000Z" }),
        )
    }

    fn state(&self, request: &crate::support::Recorded) -> Reply {
        let valid = self.valid.lock().unwrap().clone();
        if request.header("authorization")
            == Some(format!("Bearer {}", access_for(&valid)).as_str())
        {
            Reply::Json(200, fixture("state.json"))
        } else {
            Reply::Json(401, fixture("error_sign_in.json"))
        }
    }
}

#[test]
fn concurrent_calls_renew_once_and_keep_the_session() {
    let rotating = Rotating::holding_first("r1", CALLERS);
    let server = {
        let rotating = Arc::clone(&rotating);
        TestServer::start(move |request| rotating.answer(request))
    };
    let store = Arc::new(MemorySessionStore::with_session(expired("r1")));
    let client = Arc::new(server.client_with(Arc::clone(&store)));
    let start = Arc::new(Barrier::new(CALLERS));

    let polls: Vec<_> = (0..CALLERS)
        .map(|_| {
            let (client, start) = (Arc::clone(&client), Arc::clone(&start));
            std::thread::spawn(move || {
                start.wait();
                client.state(&StatePoll::new(RoomId::MAIN))
            })
        })
        .collect();
    for poll in polls {
        assert!(
            matches!(poll.join().unwrap(), Ok(StateReply::Fresh { .. })),
            "every poll got the state"
        );
    }

    assert_eq!(
        rotating.renewals.load(Ordering::SeqCst),
        1,
        "one renewal for all of them"
    );
    assert_eq!(server.requests_to("/api/auth/refresh").len(), 1);
    assert!(client.is_signed_in());
    let tokens = store.load().unwrap().unwrap().tokens.unwrap();
    assert_eq!(tokens.refresh.expose_secret(), "r2");
    let sent = server.requests_to("/api/auth/refresh")[0].json();
    assert_eq!(
        sent,
        json!({ "refresh": "r1", "install": "zr-test-install" })
    );
}

#[test]
fn a_refused_refresh_token_signs_the_device_out() {
    let rotating = Rotating::new("someone-elses");
    let server = {
        let rotating = Arc::clone(&rotating);
        TestServer::start(move |request| rotating.answer(request))
    };
    let store = Arc::new(MemorySessionStore::with_session(expired("r1")));
    let client = server.client_with(Arc::clone(&store));

    assert_eq!(
        client.state(&StatePoll::new(RoomId::MAIN)).unwrap(),
        StateReply::SignInRequired
    );

    assert!(!client.is_signed_in());
    let stored = store.load().unwrap().unwrap();
    assert!(stored.tokens.is_none());
    assert_eq!(
        stored.install.as_str(),
        "zr-test-install",
        "the install stays"
    );
    assert_eq!(
        server.requests_to("/api/state")[0].header("authorization"),
        None,
        "the poll went without a session"
    );
}

#[test]
fn a_gate_refusing_the_renewal_leaves_the_session() {
    let server = TestServer::start(|_| Reply::Json(401, fixture("error_not_let_in.json")));
    let store = Arc::new(MemorySessionStore::with_session(expired("r1")));
    let client = server.client_with(Arc::clone(&store));
    let error = client.state(&StatePoll::new(RoomId::MAIN)).unwrap_err();
    assert!(
        matches!(&error, Error::Status { status: 401, .. }),
        "{error:?}"
    );
    assert!(
        client.is_signed_in(),
        "a 401 that isn't the refresh's own doesn't sign out"
    );
    assert!(store.load().unwrap().unwrap().tokens.is_some());
}

#[test]
fn a_failing_renewal_is_the_servers_status() {
    let server = TestServer::start(|_| Reply::Text(502, "bad gateway"));
    let client = server.client_with(Arc::new(MemorySessionStore::with_session(expired("r1"))));
    let error = client.state(&StatePoll::new(RoomId::MAIN)).unwrap_err();
    assert!(
        matches!(
            &error,
            Error::Status {
                status: 502,
                message: None
            }
        ),
        "{error:?}"
    );
    assert!(client.is_signed_in());
}

#[test]
fn a_renewal_answer_that_doesnt_read_is_a_decode_error() {
    let server = TestServer::start(|_| Reply::Json(200, fixture("ok.json")));
    let client = server.client_with(Arc::new(MemorySessionStore::with_session(expired("r1"))));
    let error = client.state(&StatePoll::new(RoomId::MAIN)).unwrap_err();
    assert!(matches!(error, Error::Decode(_)), "{error:?}");
}

#[test]
fn a_token_the_server_refuses_is_renewed_and_the_call_sent_again() {
    // The server's clock is ahead (or it signed the token with another key): it refuses an
    // access token this device thinks is good for an hour.
    let server = TestServer::start(|request| match request.path.as_str() {
        "/api/auth/refresh" if request.json()["refresh"] == "r1" => Reply::Json(
            200,
            json!({ "ok": true, "status": 200, "access": access_for("r2"), "refresh": "r2", "device": 3,
                    "expires": "2099-01-01T00:00:00.000Z" }),
        ),
        "/api/auth/refresh" => Reply::Json(
            401,
            json!({ "ok": false, "status": 401, "error": "Sign in again." }),
        ),
        _ if request.header("authorization")
            == Some(format!("Bearer {}", access_for("r2")).as_str()) =>
        {
            Reply::Json(200, fixture("state.json"))
        }
        _ => Reply::Json(401, fixture("error_sign_in.json")),
    });
    let client = server.client(crate::support::fresh("r1"));

    assert!(matches!(
        client.state(&StatePoll::new(RoomId::MAIN)),
        Ok(StateReply::Fresh { .. })
    ));

    assert_eq!(server.requests_to("/api/auth/refresh").len(), 1);
    assert_eq!(
        server.requests_to("/api/state").len(),
        2,
        "sent again once, with the new token"
    );
    assert!(client.is_signed_in());
}

#[test]
fn a_refused_token_whose_renewal_is_refused_too_needs_sign_in() {
    let server = TestServer::start(|request| match request.path.as_str() {
        "/api/auth/refresh" => Reply::Json(
            401,
            json!({ "ok": false, "status": 401, "error": "Sign in again." }),
        ),
        _ => Reply::Json(401, fixture("error_sign_in.json")),
    });
    let client = server.client(crate::support::fresh("r1"));
    assert_eq!(
        client.state(&StatePoll::new(RoomId::MAIN)).unwrap(),
        StateReply::SignInRequired
    );
    assert!(!client.is_signed_in());
    assert_eq!(server.requests_to("/api/state").len(), 2);
    assert_eq!(
        server.requests_to("/api/state")[1].header("authorization"),
        None
    );
}
