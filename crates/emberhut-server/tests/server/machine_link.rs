//! The machine's identity in pairing (its id and key in the answer, the device's key on its
//! row, the pairing link with a new code) and `POST /api/connect/link-statement`, served only
//! straight to a loopback TCP peer.

use std::time::Duration;

use emberhut_api::auth::{AuthReply, PairingCode};
use emberhut_api::connect::SignedLink;
use emberhut_api::error::ErrorBody;
use emberhut_identity::{AccountId, PairingLink, RelayDomain, SecretKey};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::common::{Call, PAIRING_ROUTE, REQUEST_TIMEOUT, TestApp, test_machine};

const LINK_STATEMENT: &str = emberhut_api::connect::LINK_STATEMENT;

fn link_body() -> serde_json::Value {
    json!({ "account": "user_example", "relayDomain": "relay.example" })
}

#[tokio::test]
async fn pairing_names_the_machine_and_ignores_an_unproven_device_key() {
    let app = TestApp::with_machine().await;
    let machine = test_machine();
    let session = app.sign_in().await;
    let made = app
        .call(Call::post("/api/auth/pair/new").token(&session))
        .await;
    assert_eq!(made.status, 200);
    let code: PairingCode = made.read_as();
    let link = PairingLink::parse(code.link.as_deref().unwrap()).unwrap();
    assert_eq!(link.code(), code.code, "the link carries the same code");
    assert_eq!(*link.id(), machine.id());
    assert_eq!(link.routes(), [PAIRING_ROUTE]);

    // The device pairs from the link, sending its own key, which isn't kept without its proof.
    let device_key = SecretKey::from_bytes(&[3; 32]).public_key();
    let body = json!({
        "code": link.code(),
        "install": "phone-1",
        "name": "Phone",
        "publicKey": device_key.to_string(),
    });
    let paired = app.call(Call::post("/api/auth/pair").json(&body)).await;
    assert_eq!(paired.status, 200);
    let reply: AuthReply = paired.read_as();
    let named = reply.machine.unwrap();
    assert_eq!(named, machine.identity(), "the key the device pins");
    assert_eq!(named.id(), link.id(), "and the one the link named");

    // Whatever the field holds, it is ignored: a client that sends one pairs as without it.
    for key in [json!("not-a-key"), json!(5), json!(null)] {
        let again = app
            .call(Call::post("/api/auth/pair/new").token(&session))
            .await
            .read_as::<PairingCode>();
        let body = json!({ "code": again.code, "publicKey": key });
        let paired = app.call(Call::post("/api/auth/pair").json(&body)).await;
        assert_eq!(paired.status, 200, "{key}");
    }
}

#[tokio::test]
async fn a_server_without_an_identity_answers_as_a_legacy_host() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let code: PairingCode = app
        .call(Call::post("/api/auth/pair/new").token(&session))
        .await
        .read_as();
    assert_eq!(code.link, None);
    let paired: AuthReply = app
        .call(Call::post("/api/auth/pair").json(&json!({ "code": code.code })))
        .await
        .read_as();
    assert!(paired.ok);
    assert_eq!(paired.machine, None);
}

#[tokio::test]
pub(crate) async fn a_device_on_this_computer_gets_a_signed_link() {
    let app = TestApp::with_machine().await;
    let session = app.sign_in().await;
    for peer in ["127.0.0.1:50000", "[::1]:50000", "[::ffff:127.0.0.1]:50000"] {
        let answer = app
            .call(
                Call::post(LINK_STATEMENT)
                    .peer(peer)
                    .token(&session)
                    .json(&link_body()),
            )
            .await;
        assert_eq!(answer.status, 200, "{peer}: {}", answer.text);
        let signed: SignedLink = answer.read_as();
        let account = AccountId::new("user_example").unwrap();
        let domain: RelayDomain = "relay.example".parse().unwrap();
        assert_eq!(
            signed.verify(&account, &domain),
            Ok(test_machine().id()),
            "signed by the machine's key, for that account and relay"
        );
        assert_eq!(signed.name.as_str(), "Test machine");
        let now = u64::try_from(app.clock_now().as_millisecond()).unwrap();
        assert_eq!(u64::from(signed.time), now);
    }
}

#[tokio::test]
pub(crate) async fn any_other_peer_is_refused_a_link_statement() {
    let app = TestApp::with_machine().await;
    let session = app.sign_in().await;
    let refused = |call: Call| {
        let app = &app;
        async move {
            let answer = app.call(call).await;
            assert_eq!(answer.status, 403, "{}", answer.text);
            let body: ErrorBody = answer.read_as();
            assert!(body.error.contains("this computer"), "{}", body.error);
        }
    };
    let call = || {
        Call::post(LINK_STATEMENT)
            .token(&session)
            .json(&link_body())
    };
    // Another address, on the local network or anywhere.
    refused(call().peer("192.0.2.20:50000")).await;
    refused(call().peer("[2001:db8::20]:50000")).await;
    // No TCP peer at all (a transport that names none).
    refused(call()).await;
    // A loopback peer that another listener names (a relayed or sealed one, which doesn't
    // mark its requests as direct TCP).
    refused(call().peer_not_direct("127.0.0.1:50000")).await;
    // A loopback peer passing on someone else's request: a proxy, a forward or a relay.
    for header in [
        "forwarded",
        "x-forwarded-for",
        "x-forwarded-host",
        "x-forwarded-proto",
        "x-forwarded-port",
        "x-real-ip",
        "x-client-ip",
        "true-client-ip",
        "cf-connecting-ip",
        "via",
    ] {
        refused(call().peer("127.0.0.1:50000").header(header, "192.0.2.20")).await;
    }
    // Before the session or the body is even looked at: no session, a body that isn't JSON, a
    // body over the limit.
    refused(
        Call::post(LINK_STATEMENT)
            .peer("192.0.2.20:50000")
            .json(&link_body()),
    )
    .await;
    refused(call().peer("192.0.2.20:50000").raw("not json")).await;
    let huge = "a".repeat(3 * 1024 * 1024);
    refused(call().peer("192.0.2.20:50000").raw(&huge)).await;
}

#[tokio::test]
pub(crate) async fn a_link_statement_needs_a_session_a_good_body_and_an_identity() {
    let app = TestApp::with_machine().await;
    let session = app.sign_in().await;
    let local = || Call::post(LINK_STATEMENT).peer("127.0.0.1:50000");

    let unsigned = app.call(local().json(&link_body())).await;
    assert_eq!(unsigned.status, 401);
    assert!(unsigned.read_as::<ErrorBody>().sign_in);

    for body in [
        json!({ "account": "", "relayDomain": "relay.example" }),
        json!({ "account": "user example", "relayDomain": "relay.example" }),
        json!({ "account": "user_example", "relayDomain": "relay.example:443" }),
        json!({ "account": "user_example" }),
    ] {
        let answer = app.call(local().token(&session).json(&body)).await;
        assert_eq!(answer.status, 400, "{body}");
    }

    let huge = format!(
        r#"{{"account":"{}","relayDomain":"relay.example"}}"#,
        "a".repeat(3 * 1024 * 1024)
    );
    let answer = app.call(local().token(&session).raw(&huge)).await;
    assert_eq!(answer.status, 413);

    let legacy = TestApp::new().await;
    let session = legacy.sign_in().await;
    let answer = legacy
        .call(local().token(&session).json(&link_body()))
        .await;
    assert_eq!(answer.status, 404);
}

/// Over a real socket: the peer the server sees is the socket's.
#[tokio::test]
async fn a_loopback_socket_is_served_and_a_forwarded_request_on_it_is_not() {
    let app = TestApp::with_machine().await;
    let session = app.sign_in().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(emberhut_server::run(
        listener,
        app.state.clone(),
        REQUEST_TIMEOUT,
        Duration::from_secs(5),
        async {
            stopped.await.ok();
        },
    ));
    let client = crate::common::http_client();
    let post = || {
        client
            .post(format!("http://{address}{LINK_STATEMENT}"))
            .bearer_auth(session.tokens.access.as_str())
            .header("content-type", "application/json")
            .body(link_body().to_string())
    };
    let answer = post().send().await.unwrap();
    assert_eq!(answer.status(), 200);
    let signed: SignedLink = serde_json::from_str(&answer.text().await.unwrap()).unwrap();
    assert_eq!(
        signed.public_key,
        test_machine().identity().public_key().to_owned()
    );
    let forwarded = post()
        .header("x-forwarded-for", "192.0.2.20")
        .send()
        .await
        .unwrap();
    assert_eq!(forwarded.status(), 403);
    stop.send(()).unwrap();
    server.await.unwrap().unwrap();
}
