//! iPhone pushes (APNs): an iPhone registering its token (`POST /api/devices`), the replies it
//! is then sent through a fake APNs, and what the real sender puts in front of Apple: its
//! provider token and its request.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure here fails the test that called them"
)]

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use emberhut_api::error::{ErrorBody, OkReply};
use emberhut_api::paths;
use emberhut_core::RoomId;
use emberhut_core::push::{Alert, ApnsDevice, ApnsEnv};
use emberhut_server::config::AuthMode;
use emberhut_server::services::push::Delivery;
use emberhut_server::services::push::apns::{self, PRODUCTION_HOST, SANDBOX_HOST};
use jiff::Timestamp;
use ring::rand::SystemRandom;
use ring::signature::{
    ECDSA_P256_SHA256_FIXED, ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair as _,
    UnparsedPublicKey,
};
use serde_json::{Value, json};

use crate::common::{Call, TestApp};
use crate::fake_push::{FakePush, Sent};

/// A made-up iPhone token: 64 hex digits of `digit`.
fn iphone(digit: char) -> String {
    digit.to_string().repeat(64)
}

/// A new P-256 key as Apple's `.p8` file holds it, and its public key.
fn p8_key() -> (String, Vec<u8>) {
    let random = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &random).unwrap();
    let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &random)
        .unwrap();
    let body = STANDARD.encode(pkcs8.as_ref());
    let lines: Vec<&str> = body
        .as_bytes()
        .chunks(64)
        .map(|line| std::str::from_utf8(line).unwrap())
        .collect();
    let pem = format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
        lines.join("\n")
    );
    (pem, pair.public_key().as_ref().to_vec())
}

fn part(jwt: &str, index: usize) -> Value {
    let part = jwt.split('.').nth(index).unwrap();
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).unwrap()).unwrap()
}

#[tokio::test]
pub(crate) async fn an_iphone_registers_its_token_and_is_sent_the_replies() {
    let (apns, fcm, senders) = FakePush::pair();
    let app = TestApp::with_push(AuthMode::Enforce, senders).await;
    let session = app.sign_in().await;
    let registered = app
        .call(
            Call::post(paths::DEVICES)
                .token(&session)
                .json(&json!({ "token": iphone('a'), "env": "production" })),
        )
        .await;
    assert_eq!(registered.status, 200);
    assert_eq!(registered.read_as::<OkReply>(), OkReply::OK);
    // A development build registers without saying so: the sandbox.
    let sandbox = app
        .call(
            Call::post(paths::DEVICES)
                .token(&session)
                .json(&json!({ "token": iphone('B') })),
        )
        .await;
    assert_eq!(sandbox.status, 200);

    app.assistant.answers(
        &[],
        Some("**Done**: see [the plan](https://example.com/plan)"),
    );
    let said = app
        .call(
            Call::post(paths::TALK)
                .token(&session)
                .json(&json!({ "text": "Plan it" })),
        )
        .await;
    assert_eq!(said.status, 200);
    app.settle().await;

    let sent = apns.sent();
    let mut tokens: Vec<&str> = sent.iter().map(|(token, _)| token.as_str()).collect();
    tokens.sort_unstable();
    assert_eq!(tokens, [iphone('B'), iphone('a')]);
    let alert = &sent[0].1;
    assert_eq!(alert.title(), "Emberhut");
    assert_eq!(alert.body(), "Done: see the plan");
    assert_eq!(alert.link(), "emberhut://chat?room=1");
    assert_eq!(
        fcm.sent(),
        Vec::<Sent>::new(),
        "no Android phone registered"
    );
    let devices = app.state.push();
    assert!(devices.ready().await);
}

#[tokio::test]
pub(crate) async fn a_token_that_is_not_one_is_400_naming_the_token_kind() {
    let (_apns, _fcm, senders) = FakePush::pair();
    let app = TestApp::with_push(AuthMode::Enforce, senders).await;
    let session = app.sign_in().await;
    for (body, error) in [
        (json!({ "token": "not hex" }), "not a device token"),
        (json!({}), "not a device token"),
        (json!({ "token": iphone('a'), "platform": "android" }), ""),
        (
            json!({ "token": "short", "platform": "android" }),
            "not an FCM registration token",
        ),
    ] {
        let answer = app
            .call(Call::post(paths::DEVICES).token(&session).json(&body))
            .await;
        if error.is_empty() {
            // 64 hex digits are a fine FCM token too.
            assert_eq!(answer.status, 200, "{body}");
            continue;
        }
        assert_eq!(answer.status, 400, "{body}");
        assert_eq!(answer.read_as::<ErrorBody>(), ErrorBody::new(error));
    }
    let not_text = app
        .call(
            Call::post(paths::DEVICES)
                .token(&session)
                .json(&json!({ "token": 12 })),
        )
        .await;
    assert_eq!(not_text.status, 400);
    let huge = format!(r#"{{"token":"{}"}}"#, "a".repeat(3 * 1024 * 1024));
    let too_large = app
        .call(Call::post(paths::DEVICES).token(&session).raw(&huge))
        .await;
    assert_eq!(too_large.status, 413);
}

#[tokio::test]
pub(crate) async fn registering_needs_a_session() {
    let app = TestApp::new().await;
    let refused = app
        .call(Call::post(paths::DEVICES).json(&json!({ "token": iphone('a') })))
        .await;
    assert_eq!(refused.status, 401);
    assert!(refused.read_as::<ErrorBody>().sign_in);
    // Report mode's pass for a request without a session is not for this: the replies would
    // go to whoever registered.
    let (apns, _fcm, senders) = FakePush::pair();
    let report = TestApp::with_push(AuthMode::Report, senders).await;
    let refused = report
        .call(Call::post(paths::DEVICES).json(&json!({ "token": iphone('a') })))
        .await;
    assert_eq!(refused.status, 401);
    assert!(refused.read_as::<ErrorBody>().sign_in);
    report
        .state
        .push()
        .push(&Alert::reply(RoomId::MAIN, None, "Hi"))
        .await;
    assert_eq!(apns.sent(), Vec::<Sent>::new(), "nothing was registered");
}

#[tokio::test]
async fn a_signed_out_phone_is_sent_nothing_more() {
    let (apns, fcm, senders) = FakePush::pair();
    let app = TestApp::with_push(AuthMode::Enforce, senders).await;
    let laptop = app.sign_in().await;
    let phone = app
        .state
        .auth()
        .open_session(&crate::common::device("phone-2"))
        .await
        .unwrap();
    for (session, body) in [
        (&laptop, json!({ "token": iphone('a') })),
        (&phone, json!({ "token": iphone('b') })),
        (
            &phone,
            json!({ "token": format!("phone:{}", "x".repeat(40)), "platform": "android" }),
        ),
    ] {
        let registered = app
            .call(Call::post(paths::DEVICES).token(session).json(&body))
            .await;
        assert_eq!(registered.status, 200, "{}", registered.text);
    }

    let revoked = app
        .call(
            Call::post(paths::SETTINGS)
                .token(&laptop)
                .json(&json!({ "authRevoke": phone.device.get() })),
        )
        .await;
    assert_eq!(revoked.status, 200, "{}", revoked.text);

    let report = app
        .state
        .push()
        .push(&Alert::reply(RoomId::MAIN, None, "Hi"))
        .await;
    assert_eq!(report.delivered, 1, "the laptop's iPhone token only");
    let sent: Vec<String> = apns.sent().into_iter().map(|(token, _)| token).collect();
    assert_eq!(sent, [iphone('a')]);
    assert_eq!(fcm.sent(), Vec::<Sent>::new());
}

#[tokio::test]
async fn a_token_apple_says_is_gone_is_forgotten_and_a_failure_is_kept() {
    let (apns, _fcm, senders) = FakePush::pair();
    let app = TestApp::with_push(AuthMode::Enforce, senders).await;
    let session = app.sign_in().await;
    for digit in ['a', 'b', 'c'] {
        let answer = app
            .call(
                Call::post(paths::DEVICES)
                    .token(&session)
                    .json(&json!({ "token": iphone(digit) })),
            )
            .await;
        assert_eq!(answer.status, 200);
    }
    apns.answer(&iphone('a'), Delivery::Gone);
    apns.answer(
        &iphone('b'),
        Delivery::Failed("APNs answered 500".to_owned()),
    );
    let alert = Alert::new("Title", "Body", None);
    let report = app.state.push().push(&alert).await;
    assert_eq!(
        (report.delivered, report.forgotten, report.failed),
        (1, 1, 1)
    );
    let again = app.state.push().push(&alert).await;
    assert_eq!(
        (again.delivered, again.forgotten, again.failed),
        (1, 0, 1),
        "the gone token is not tried again"
    );
    assert_eq!(apns.sent().len(), 5);
}

#[tokio::test]
async fn a_sender_that_is_not_ready_sends_nothing_and_the_state_says_so() {
    let (apns, fcm, senders) = FakePush::pair();
    let app = TestApp::with_push(AuthMode::Enforce, senders).await;
    let session = app.sign_in().await;
    let state = app.call(Call::get(paths::STATE).token(&session)).await;
    assert_eq!(state.json()["push"], true);
    apns.set_ready(false);
    assert!(app.state.push().ready().await, "FCM still is");
    fcm.set_ready(false);
    let state = app.call(Call::get(paths::STATE).token(&session)).await;
    assert_eq!(state.json()["push"], false);
    let answer = app
        .call(
            Call::post(paths::DEVICES)
                .token(&session)
                .json(&json!({ "token": iphone('a') })),
        )
        .await;
    assert_eq!(
        answer.status, 200,
        "a phone may register before the keys are there"
    );
    let report = app.state.push().push(&Alert::new("t", "b", None)).await;
    assert_eq!(report.delivered, 0);
    assert_eq!(apns.sent(), Vec::<Sent>::new());

    // A server with no push services at all.
    let bare = TestApp::new().await;
    let session = bare.sign_in().await;
    let state = bare.call(Call::get(paths::STATE).token(&session)).await;
    assert_eq!(state.json()["push"], false);
}

#[test]
fn the_provider_token_is_an_es256_jwt_naming_the_key_and_the_team() {
    let (pem, public) = p8_key();
    let at = Timestamp::from_second(1_800_000_000).unwrap();
    let token = apns::provider_token(&pem, "KEY1234567", "TEAM123456", at).unwrap();
    assert_eq!(
        part(&token, 0),
        json!({ "alg": "ES256", "kid": "KEY1234567" })
    );
    assert_eq!(
        part(&token, 1),
        json!({ "iss": "TEAM123456", "iat": 1_800_000_000 })
    );
    let (signed, signature) = token.rsplit_once('.').unwrap();
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, &public)
        .verify(
            signed.as_bytes(),
            &URL_SAFE_NO_PAD.decode(signature).unwrap(),
        )
        .unwrap();
    assert!(apns::provider_token("not a key", "k", "t", at).is_err());
}

#[test]
fn apple_gets_the_alert_on_the_phone_s_service() {
    let alert = Alert::reply(RoomId::new(4), Some("Garden"), "Water them");
    let payload = apns::payload(&alert);
    assert_eq!(
        payload,
        json!({
            "aps": {
                "alert": { "title": "Emberhut · Garden", "body": "Water them" },
                "sound": "default",
                "thread-id": "emberhut-4",
            },
            "link": "emberhut://chat?room=4",
            "room": 4,
        })
    );
    let device = ApnsDevice {
        token: iphone('a'),
        env: ApnsEnv::Production,
    };
    let request = apns::request(&device, "com.emberhut.example", "jwt", &payload).unwrap();
    assert_eq!(
        request.uri().to_string(),
        format!("{PRODUCTION_HOST}/3/device/{}", iphone('a'))
    );
    assert_eq!(request.method(), "POST");
    assert_eq!(request.version(), axum::http::Version::HTTP_2);
    let header = |name: &str| request.headers()[name].to_str().unwrap().to_owned();
    assert_eq!(header("authorization"), "bearer jwt");
    assert_eq!(header("apns-topic"), "com.emberhut.example");
    assert_eq!(header("apns-push-type"), "alert");
    assert_eq!(header("apns-priority"), "10");
    assert_eq!(apns::host(ApnsEnv::Sandbox), SANDBOX_HOST);

    // Without a room, the chat as it was, and no room field.
    let plain = apns::payload(&Alert::new("Emberhut", "Hi", None));
    assert_eq!(plain["link"], "emberhut://chat");
    assert!(plain.get("room").is_none());
}
