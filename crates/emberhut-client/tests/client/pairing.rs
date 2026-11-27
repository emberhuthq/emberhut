//! Pairing a device with a one-time code.

use std::sync::Arc;

use emberhut_api::auth::AuthRefusal;
use emberhut_api::ids::DeviceId;
use emberhut_client::{Error, MemorySessionStore, SessionStore};
use serde_json::json;

use crate::support::{Reply, TestServer, fixture, signed_out};

#[test]
fn a_right_code_signs_the_device_in() {
    let server = TestServer::start(|request| match request.path.as_str() {
        "/api/auth/pair" => {
            let mut accepted = fixture("auth_accepted.json");
            accepted["expires"] = json!("2099-01-01T00:00:00.000Z"); // no renewal before the next call
            Reply::Json(200, accepted)
        }
        _ => Reply::Json(200, fixture("ok.json")),
    });
    let store = Arc::new(MemorySessionStore::with_session(signed_out()));
    let client = server.client_with(Arc::clone(&store));
    assert!(!client.is_signed_in());

    client.pair("  abcd efgh ").unwrap();

    assert!(client.is_signed_in());
    assert_eq!(client.device(), Some(DeviceId::new(3)));
    let sent = server.requests_to("/api/auth/pair")[0].json();
    assert_eq!(
        sent,
        json!({ "code": "abcd efgh", "install": "zr-test-install", "name": "Emberhut on a test machine", "model": "linux" })
    );
    assert!(
        server.requests_to("/api/auth/pair")[0]
            .header("authorization")
            .is_none()
    );
    let tokens = store.load().unwrap().unwrap().tokens.unwrap();
    assert_eq!(tokens.refresh.expose_secret(), "refresh-token-for-tests");

    client.seen(emberhut_api::ids::RoomId::MAIN).unwrap();
    assert_eq!(
        server.requests_to("/api/seen")[0].header("authorization"),
        Some("Bearer access-token-for-tests")
    );
}

#[test]
fn a_wrong_code_is_refused_with_the_servers_words() {
    let server = TestServer::start(|_| Reply::Json(401, fixture("auth_refused.json")));
    let client = server.client(signed_out());
    let error = client.pair("AAAA-AAAA").unwrap_err();
    let Error::Refused(AuthRefusal { status, error }) = error else {
        panic!("a refusal: {error:?}")
    };
    assert_eq!(
        (status, error.as_str()),
        (401, "That code isn't right. Check it, or make a new one.")
    );
    assert!(!client.is_signed_in());
}

#[test]
fn too_many_wrong_codes_wait() {
    let server = TestServer::start(|_| Reply::Json(429, fixture("auth_too_many.json")));
    let error = server.client(signed_out()).pair("AAAA-AAAA").unwrap_err();
    assert_eq!(error.status(), Some(429));
}

#[test]
fn a_gate_in_front_of_the_server_is_a_status() {
    let server = TestServer::start(|_| Reply::Json(401, fixture("error_not_let_in.json")));
    let error = server.client(signed_out()).pair("AAAA-AAAA").unwrap_err();
    assert!(
        matches!(
            &error,
            Error::Status {
                status: 401,
                message: Some(_)
            }
        ),
        "{error:?}"
    );
}

#[test]
fn an_accepted_answer_that_doesnt_read_is_a_decode_error() {
    let server = TestServer::start(|_| Reply::Text(200, "not json"));
    let error = server.client(signed_out()).pair("ABCD-EFGH").unwrap_err();
    assert!(matches!(error, Error::Decode(_)), "{error:?}");
}

#[test]
fn signing_out_here_keeps_the_install() {
    let store = Arc::new(MemorySessionStore::with_session(crate::support::fresh(
        "r1",
    )));
    let server = TestServer::start(|_| Reply::Json(200, fixture("ok.json")));
    let client = server.client_with(Arc::clone(&store));
    let install = client.install();
    client.sign_out_here().unwrap();
    assert!(!client.is_signed_in());
    let stored = store.load().unwrap().unwrap();
    assert!(stored.tokens.is_none());
    assert_eq!(stored.install, install);
}

/// An accepted pairing answer naming the machine whose key is made from `seed`.
fn accepted_from(seed: u8) -> serde_json::Value {
    let key = emberhut_identity::SecretKey::from_bytes(&[seed; 32]);
    let mut accepted = fixture("auth_accepted.json");
    accepted["expires"] = json!("2099-01-01T00:00:00.000Z");
    accepted["machine"] = json!({
        "id": key.machine_id().to_string(),
        "publicKey": key.public_key().to_string(),
    });
    accepted
}

#[test]
fn a_pairing_from_a_link_keeps_the_session_of_the_machine_it_names() {
    let server = TestServer::start(|_| Reply::Json(200, accepted_from(7)));
    let client = server.client(signed_out());
    let meant = emberhut_identity::SecretKey::from_bytes(&[7; 32]).machine_id();

    let machine = client.pair_with_machine("ABCD-EFGH", &meant).unwrap();

    assert_eq!(machine.id(), &meant);
    assert!(client.is_signed_in());
}

#[test]
fn a_pairing_from_a_link_refuses_another_machine_and_keeps_nothing() {
    let server = TestServer::start(|_| Reply::Json(200, accepted_from(8)));
    let client = server.client(signed_out());
    let meant = emberhut_identity::SecretKey::from_bytes(&[7; 32]).machine_id();

    let error = client.pair_with_machine("ABCD-EFGH", &meant).unwrap_err();

    assert!(matches!(error, Error::WrongMachine), "{error:?}");
    assert_eq!(
        error.to_string(),
        "this server didn't prove it is the machine you meant"
    );
    assert!(!client.is_signed_in());
}

#[test]
fn a_pairing_from_a_link_refuses_an_answer_without_a_machine() {
    let server = TestServer::start(|_| {
        let mut accepted = fixture("auth_accepted.json");
        accepted["expires"] = json!("2099-01-01T00:00:00.000Z");
        Reply::Json(200, accepted)
    });
    let client = server.client(signed_out());
    let meant = emberhut_identity::SecretKey::from_bytes(&[7; 32]).machine_id();

    let error = client.pair_with_machine("ABCD-EFGH", &meant).unwrap_err();

    assert!(matches!(error, Error::WrongMachine), "{error:?}");
    assert!(!client.is_signed_in());
}

#[test]
fn a_link_statement_is_asked_with_this_devices_session() {
    let key = emberhut_identity::SecretKey::from_bytes(&[5; 32]);
    let account = emberhut_identity::AccountId::new("user_example").unwrap();
    let domain: emberhut_identity::RelayDomain = "relay.example".parse().unwrap();
    let signed = emberhut_api::connect::SignedLink::sign(
        &key,
        emberhut_identity::MachineName::cut("Main computer").unwrap(),
        account.clone(),
        domain.clone(),
        emberhut_identity::UnixMillis::new(1_795_561_073_000).unwrap(),
    );
    let answer = serde_json::to_value(&signed).unwrap();
    let server = TestServer::start(move |_| Reply::Json(200, answer.clone()));
    let client = server.client(crate::support::fresh("r1"));

    let request = emberhut_api::connect::LinkStatementRequest {
        account,
        relay_domain: domain,
    };
    assert_eq!(client.link_statement(&request).unwrap(), signed);

    let sent = &server.requests_to("/api/connect/link-statement")[0];
    assert_eq!(
        sent.json(),
        json!({ "account": "user_example", "relayDomain": "relay.example" })
    );
    assert!(sent.header("authorization").is_some());
}

#[test]
fn a_pairing_from_a_link_refuses_the_links_id_with_another_key() {
    let meant = emberhut_identity::SecretKey::from_bytes(&[7; 32]).machine_id();
    let other = emberhut_identity::SecretKey::from_bytes(&[8; 32]);
    let server = TestServer::start(move |_| {
        let mut accepted = fixture("auth_accepted.json");
        accepted["expires"] = json!("2099-01-01T00:00:00.000Z");
        accepted["machine"] = json!({
            "id": meant.to_string(),
            "publicKey": other.public_key().to_string(),
        });
        Reply::Json(200, accepted)
    });
    let client = server.client(signed_out());

    let error = client.pair_with_machine("ABCD-EFGH", &meant).unwrap_err();

    assert!(matches!(error, Error::WrongMachine), "{error:?}");
    assert!(!client.is_signed_in());
    let typed = server.client(signed_out()).pair("ABCD-EFGH").unwrap_err();
    assert!(
        matches!(typed, Error::Decode(_)),
        "a typed code still refuses it: {typed:?}"
    );
}
