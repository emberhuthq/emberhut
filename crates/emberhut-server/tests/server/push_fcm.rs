//! Android pushes (FCM): an Android phone registering its token (`POST /api/devices`), and the
//! real sender against a stand-in for Google's token endpoint and FCM on loopback: the signed
//! assertion, the access token kept and renewed, the message, and a gone token forgotten.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure here fails the test that called them"
)]

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::IntoResponse as _;
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use emberhut_api::paths;
use emberhut_core::push::Alert;
use emberhut_core::{ManualClock, RoomId};
use emberhut_server::config::AuthMode;
use emberhut_server::services::push::fcm::{self, FcmEndpoints, HttpFcm, ServiceAccount};
use emberhut_server::services::push::{Delivery, FcmSender as _, PushSenders};
use jiff::{SignedDuration, Timestamp};
use ring::signature::{RSA_PKCS1_2048_8192_SHA256, RsaKeyPair, UnparsedPublicKey};
use serde_json::{Value, json};

use crate::common::{Call, TestApp};
use crate::fake_push::{FakePush, Sent};

/// A made-up RSA key for these tests only (PKCS #8, DER), never used anywhere else.
const TEST_KEY: &[u8] = include_bytes!("../fixtures/rsa-2048-test-key.pk8");

/// A made-up Android registration token.
fn android(name: &str) -> String {
    format!("{name}:APA91b{}", "x".repeat(40))
}

fn test_key_pem() -> String {
    let body = STANDARD.encode(TEST_KEY);
    let lines: Vec<&str> = body
        .as_bytes()
        .chunks(64)
        .map(|line| std::str::from_utf8(line).unwrap())
        .collect();
    format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
        lines.join("\n")
    )
}

fn account() -> ServiceAccount {
    ServiceAccount::parse(&account_file()).unwrap()
}

fn account_file() -> String {
    json!({
        "type": "service_account",
        "project_id": "example-project",
        "private_key_id": "key-1",
        "private_key": test_key_pem(),
        "client_email": "pusher@example.com",
    })
    .to_string()
}

fn part(jwt: &str, index: usize) -> Value {
    let part = jwt.split('.').nth(index).unwrap();
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).unwrap()).unwrap()
}

#[tokio::test]
pub(crate) async fn an_android_phone_registers_its_token_and_is_sent_the_replies() {
    let (apns, fcm, senders) = FakePush::pair();
    let app = TestApp::with_push(AuthMode::Enforce, senders).await;
    let session = app.sign_in().await;
    let registered = app
        .call(
            Call::post(paths::DEVICES)
                .token(&session)
                .json(&json!({ "token": android("one"), "platform": "android" })),
        )
        .await;
    assert_eq!(registered.status, 200);
    app.assistant.answers(&[], Some("All set."));
    app.call(
        Call::post(paths::TALK)
            .token(&session)
            .json(&json!({ "text": "Set it up" })),
    )
    .await;
    app.settle().await;
    let sent = fcm.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, android("one"));
    assert_eq!(sent[0].1.body(), "All set.");
    assert_eq!(apns.sent(), Vec::<Sent>::new());
}

#[test]
fn a_service_account_needs_a_project_an_address_and_a_key() {
    assert_eq!(account().project_id, "example-project");
    assert!(ServiceAccount::parse("not json").is_none());
    assert!(ServiceAccount::parse(r#"{"project_id":"p","client_email":"e"}"#).is_none());
    assert!(
        ServiceAccount::parse(r#"{"project_id":"","client_email":"e","private_key":"k"}"#)
            .is_none()
    );
    assert!(!format!("{:?}", account()).contains("PRIVATE KEY"));
}

#[test]
fn the_assertion_is_an_rs256_jwt_for_fcm_s_scope() {
    let at = Timestamp::from_second(1_800_000_000).unwrap();
    let jwt = fcm::assertion(&account(), "https://oauth2.example.com/token", at).unwrap();
    assert_eq!(
        part(&jwt, 0),
        json!({ "alg": "RS256", "typ": "JWT", "kid": "key-1" })
    );
    assert_eq!(
        part(&jwt, 1),
        json!({
            "iss": "pusher@example.com",
            "scope": fcm::MESSAGING_SCOPE,
            "aud": "https://oauth2.example.com/token",
            "iat": 1_800_000_000,
            "exp": 1_800_003_600,
        })
    );
    let public = RsaKeyPair::from_pkcs8(TEST_KEY).unwrap();
    let (signed, signature) = jwt.rsplit_once('.').unwrap();
    UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, public.public().as_ref())
        .verify(
            signed.as_bytes(),
            &URL_SAFE_NO_PAD.decode(signature).unwrap(),
        )
        .unwrap();
}

#[test]
fn the_message_carries_what_a_tap_opens_as_strings() {
    let alert = Alert::reply(RoomId::new(3), Some("Trips"), "Booked");
    assert_eq!(
        fcm::message("tok", &alert),
        json!({
            "message": {
                "token": "tok",
                "notification": { "title": "Emberhut · Trips", "body": "Booked" },
                "data": { "link": "emberhut://chat?room=3", "thread": "emberhut-3", "room": "3" },
                "android": {
                    "priority": "high",
                    "notification": { "channel_id": "emberhut", "sound": "default" },
                },
            },
        })
    );
}

/// What the stand-in saw, and how it answers.
#[derive(Debug, Default)]
struct Seen {
    token_requests: Vec<String>,
    sends: Vec<(String, String, Value)>,
    gone: Vec<String>,
    refuse_next_send: bool,
}

type Shared = Arc<Mutex<Seen>>;

async fn stand_in(
    State(seen): State<Shared>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let mut seen = seen.lock().unwrap();
    if uri.path() == "/token" {
        let form = String::from_utf8(body.to_vec()).unwrap();
        let n = seen.token_requests.len() + 1;
        seen.token_requests.push(form);
        let granted = json!({ "access_token": format!("access-{n}"), "expires_in": 3600 });
        return axum::Json(granted).into_response();
    }
    let auth = headers["authorization"].to_str().unwrap().to_owned();
    let message: Value = serde_json::from_slice(&body).unwrap();
    let token = message["message"]["token"].as_str().unwrap().to_owned();
    seen.sends
        .push((uri.path().to_owned(), auth, message.clone()));
    if std::mem::take(&mut seen.refuse_next_send) {
        return (StatusCode::UNAUTHORIZED, "token expired").into_response();
    }
    if seen.gone.contains(&token) {
        let error = json!({ "error": { "status": "NOT_FOUND", "details": [
            { "errorCode": "UNREGISTERED" }
        ] } });
        return (StatusCode::NOT_FOUND, axum::Json(error)).into_response();
    }
    axum::Json(json!({ "name": "projects/example-project/messages/1" })).into_response()
}

/// A stand-in for Google's token endpoint and FCM, on loopback.
async fn google() -> (FcmEndpoints, Shared) {
    let seen = Shared::default();
    let app = Router::new()
        .fallback(stand_in)
        .with_state(Arc::clone(&seen));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let endpoints = FcmEndpoints {
        token_url: format!("http://{address}/token"),
        send_base: format!("http://{address}"),
    };
    (endpoints, seen)
}

#[tokio::test]
async fn the_real_sender_trades_an_assertion_keeps_the_token_and_sends_the_message() {
    let (endpoints, seen) = google().await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(fcm::ACCOUNT_FILE);
    let clock = Arc::new(ManualClock::new(
        Timestamp::from_second(1_800_000_000).unwrap(),
    ));
    let sender = HttpFcm::new(file.clone(), endpoints.clone(), Arc::clone(&clock) as _).unwrap();
    assert!(!sender.ready().await, "no service account yet");
    let alert = Alert::new("Emberhut", "Hello", Some(RoomId::MAIN));
    assert!(matches!(
        sender.send("tok", &alert).await,
        Delivery::Failed(_)
    ));
    std::fs::write(&file, account_file()).unwrap();
    assert!(sender.ready().await, "read when needed: no restart");

    assert_eq!(
        sender.send(&android("a"), &alert).await,
        Delivery::Delivered
    );
    assert_eq!(
        sender.send(&android("b"), &alert).await,
        Delivery::Delivered
    );
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.token_requests.len(), 1, "the access token is kept");
        let form: Vec<(String, String)> = form_urlencoded::parse(seen.token_requests[0].as_bytes())
            .into_owned()
            .collect();
        assert_eq!(form[0].1, "urn:ietf:params:oauth:grant-type:jwt-bearer");
        assert_eq!(part(&form[1].1, 1)["aud"], endpoints.token_url);
        let (path, auth, message) = &seen.sends[0];
        assert_eq!(path, "/v1/projects/example-project/messages:send");
        assert_eq!(auth, "Bearer access-1");
        assert_eq!(message, &fcm::message(&android("a"), &alert));
    }

    // Five minutes before it expires, a new one.
    clock.advance(SignedDuration::from_mins(55));
    assert_eq!(
        sender.send(&android("a"), &alert).await,
        Delivery::Delivered
    );
    assert_eq!(seen.lock().unwrap().token_requests.len(), 2);

    // Refused: the token is dropped, and the next push gets another.
    seen.lock().unwrap().refuse_next_send = true;
    assert!(matches!(
        sender.send(&android("a"), &alert).await,
        Delivery::Failed(reason) if reason.contains("401")
    ));
    assert_eq!(
        sender.send(&android("a"), &alert).await,
        Delivery::Delivered
    );
    assert_eq!(seen.lock().unwrap().token_requests.len(), 3);

    // A token FCM says is gone.
    seen.lock().unwrap().gone.push(android("b"));
    assert_eq!(sender.send(&android("b"), &alert).await, Delivery::Gone);
}

#[tokio::test]
async fn a_token_fcm_says_is_gone_is_forgotten() {
    let (endpoints, seen) = google().await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(fcm::ACCOUNT_FILE);
    std::fs::write(&file, account_file()).unwrap();
    let clock = Arc::new(ManualClock::new(Timestamp::now()));
    let sender = HttpFcm::new(file, endpoints, clock as _).unwrap();
    let senders = PushSenders {
        apns: None,
        fcm: Some(Arc::new(sender)),
    };
    let app = TestApp::with_push(AuthMode::Enforce, senders).await;
    let session = app.sign_in().await;
    for name in ["kept", "gone"] {
        let answer = app
            .call(
                Call::post(paths::DEVICES)
                    .token(&session)
                    .json(&json!({ "token": android(name), "platform": "android" })),
            )
            .await;
        assert_eq!(answer.status, 200);
    }
    seen.lock().unwrap().gone.push(android("gone"));
    let alert = Alert::new("Emberhut", "Hi", None);
    let first = app.state.push().push(&alert).await;
    assert_eq!((first.delivered, first.forgotten), (1, 1));
    let second = app.state.push().push(&alert).await;
    assert_eq!((second.delivered, second.forgotten), (1, 0));
    assert_eq!(seen.lock().unwrap().sends.len(), 3);
}
