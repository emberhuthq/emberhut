//! `POST /api/auth/pair/new`, `POST /api/auth/pair`, `POST /api/auth/refresh` and
//! `GET /api/auth/status`, over HTTP.

use emberhut_api::auth::{AuthReply, AuthStatus, Device, PairingCode};
use emberhut_api::error::ErrorBody;
use emberhut_api::paths;
use emberhut_server::config::AuthMode;
use serde_json::json;

use crate::common::{Call, TestApp};

fn pair_body(code: &str, install: &str) -> serde_json::Value {
    json!({ "code": code, "install": install, "name": "Room 2 phone", "model": "Example 2" })
}

#[tokio::test]
pub(crate) async fn a_signed_in_device_makes_a_code_that_pairs_a_new_one() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let made = app
        .call(Call::post("/api/auth/pair/new").token(&session))
        .await;
    assert_eq!(made.status, 200);
    let code: PairingCode = made.read_as();
    assert!(code.ok);
    assert_eq!(code.code.len(), 9);

    let typed = code.code.to_lowercase().replace('-', " ");
    let paired = app
        .call(Call::post("/api/auth/pair").json(&pair_body(&typed, "phone-1")))
        .await;
    assert_eq!(paired.status, 200);
    let reply: AuthReply = paired.read_as();
    assert!(reply.ok);
    assert_eq!(reply.status, 200);
    let grant = reply.into_grant().unwrap();
    let device = grant.device.unwrap();
    assert_ne!(device.get(), session.device.get());
    assert!(
        app.state
            .auth()
            .authenticate(grant.access.expose_secret())
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
pub(crate) async fn only_a_signed_in_device_makes_a_code_even_in_report_mode() {
    for mode in [AuthMode::Enforce, AuthMode::Report] {
        let app = TestApp::with_mode(mode).await;
        let refused = app.call(Call::post("/api/auth/pair/new")).await;
        assert_eq!(refused.status, 401, "{mode:?}");
        let body: ErrorBody = refused.read_as();
        assert_eq!(body.ok, Some(false));
        assert!(!body.sign_in);
        let wrong = app
            .call(Call::post("/api/auth/pair/new").header("authorization", "Bearer nope"))
            .await;
        assert_eq!(wrong.status, 401);
    }
}

#[tokio::test]
pub(crate) async fn pairing_refusals_answer_in_the_auth_reply_shape() {
    let app = TestApp::new().await;
    let cases = [
        (pair_body("", "phone-1"), 400, "Type the pairing code."),
        (
            json!({ "install": "phone-1", "name": "", "model": "" }),
            400,
            "Type the pairing code.",
        ),
        (
            pair_body("AAAA-AAAA", "phone-1"),
            401,
            "That code isn't right. Check it, or make a new one.",
        ),
    ];
    for (body, status, error) in cases {
        let answer = app.call(Call::post("/api/auth/pair").json(&body)).await;
        assert_eq!(answer.status, status, "{body}");
        let reply: AuthReply = answer.read_as();
        assert_eq!(reply, AuthReply::refused(status, error), "{body}");
    }

    let long_name =
        json!({ "code": "AAAA-AAAA", "install": "phone-1", "name": "x".repeat(61), "model": "" });
    let answer = app
        .call(Call::post("/api/auth/pair").json(&long_name))
        .await;
    assert_eq!(answer.status, 400);
    let reply: AuthReply = answer.read_as();
    assert!(!reply.ok);
    assert!(reply.error.unwrap().contains("name"));
}

#[tokio::test]
pub(crate) async fn too_many_wrong_codes_answer_429_with_retry_after() {
    let app = TestApp::new().await;
    for _ in 0..5 {
        let wrong = app
            .call(Call::post("/api/auth/pair").json(&pair_body("AAAA-AAAA", "guesser")))
            .await;
        assert_eq!(wrong.status, 401);
    }
    let braked = app
        .call(Call::post("/api/auth/pair").json(&pair_body("AAAA-AAAA", "guesser")))
        .await;
    assert_eq!(braked.status, 429);
    assert_eq!(braked.header("retry-after"), "600");
    let reply: AuthReply = braked.read_as();
    assert_eq!(
        reply,
        AuthReply::refused(429, "Too many wrong codes. Try again in 10 minutes.")
    );
}

#[tokio::test]
pub(crate) async fn a_refresh_token_is_traded_once_for_new_tokens() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let body = json!({ "refresh": session.tokens.refresh.as_str(), "install": "laptop-1" });
    app.advance_minutes(5);
    let renewed = app.call(Call::post("/api/auth/refresh").json(&body)).await;
    assert_eq!(renewed.status, 200);
    let grant = renewed.read_as::<AuthReply>().into_grant().unwrap();
    assert_ne!(
        grant.refresh.expose_secret(),
        session.tokens.refresh.as_str()
    );

    app.advance_minutes(5);
    let again = app.call(Call::post("/api/auth/refresh").json(&body)).await;
    assert_eq!(again.status, 401);
    assert_eq!(
        again.read_as::<AuthReply>(),
        AuthReply::refused(401, "Sign in again.")
    );

    let session = app.sign_in().await;
    let empty_install = app
        .call(Call::post("/api/auth/refresh").json(&json!({
            "refresh": session.tokens.refresh.as_str(), "install": ""
        })))
        .await;
    assert_eq!(
        empty_install.status, 200,
        "an empty install is none, as when pairing"
    );

    for bad in [
        json!({}),
        json!({ "refresh": "" }),
        json!({ "refresh": 42 }),
    ] {
        let answer = app.call(Call::post("/api/auth/refresh").json(&bad)).await;
        assert_eq!(answer.status, 401, "{bad}");
    }
    let long_install = json!({ "refresh": "x", "install": "i".repeat(65) });
    let answer = app
        .call(Call::post("/api/auth/refresh").json(&long_install))
        .await;
    assert_eq!(answer.status, 400);
}

#[tokio::test]
async fn a_pairing_body_is_read_as_the_contract_says() {
    let app = TestApp::new().await;
    // A code that is not text is "type the code", by its type, not by the parser's words.
    let typed = app
        .call(Call::post("/api/auth/pair").json(&json!({
            "code": 5, "install": "p", "name": "n", "model": "m"
        })))
        .await;
    assert_eq!(typed.status, 400);
    assert_eq!(
        typed.read_as::<AuthReply>(),
        AuthReply::refused(400, "Type the pairing code.")
    );

    // Everything but the code may be left out: a wrong code is then just a wrong code.
    let bare = app
        .call(Call::post("/api/auth/pair").json(&json!({ "code": "AAAA-AAAA" })))
        .await;
    assert_eq!(bare.status, 401);
    assert_eq!(
        bare.read_as::<AuthReply>(),
        AuthReply::refused(401, "That code isn't right. Check it, or make a new one.")
    );

    // A code longer than any code is a wrong code (401), counted on the brake.
    let long = app
        .call(Call::post("/api/auth/pair").json(&json!({ "code": "A".repeat(40) })))
        .await;
    assert_eq!(long.status, 401);
    assert_eq!(
        long.read_as::<AuthReply>(),
        AuthReply::refused(401, "That code isn't right. Check it, or make a new one.")
    );

    // Null names are absent names.
    let nulls = app
        .call(Call::post("/api/auth/pair").json(&json!({
            "code": "AAAA-AAAA", "name": null, "model": null, "install": null
        })))
        .await;
    assert_eq!(
        nulls.status, 401,
        "read, and refused only for its wrong code"
    );

    // Left out, the name is "A device"; an empty install is none: a new row each time.
    let mut devices = Vec::new();
    for _ in 0..2 {
        let code = app
            .state
            .pairing()
            .new_code(emberhut_server::services::pairing::CodeMaker::CommandLine)
            .await
            .unwrap()
            .code
            .to_string();
        let paired = app
            .call(Call::post("/api/auth/pair").json(&json!({ "code": code, "install": "" })))
            .await;
        assert_eq!(paired.status, 200);
        devices.push(paired.read_as::<AuthReply>().device.unwrap());
    }
    assert_ne!(devices[0], devices[1]);
    let listed = app.state.auth().devices().await.unwrap();
    assert!(listed.iter().all(|device| device.name == "A device"));
}

#[tokio::test]
pub(crate) async fn a_sign_in_body_over_the_limit_is_413_in_the_auth_reply_shape() {
    let app = TestApp::new().await;
    let huge = format!(r#"{{"code":"{}"}}"#, "A".repeat(3 * 1024 * 1024));
    for path in ["/api/auth/pair", "/api/auth/refresh"] {
        let answer = app.call(Call::post(path).raw(&huge)).await;
        assert_eq!(answer.status, 413, "{path}");
        assert_eq!(
            answer.read_as::<AuthReply>(),
            AuthReply::refused(413, "The request is too large.")
        );
    }
}

#[tokio::test]
async fn while_the_brake_is_on_even_a_missing_code_is_429() {
    let app = TestApp::new().await;
    for _ in 0..5 {
        app.call(Call::post("/api/auth/pair").json(&pair_body("AAAA-AAAA", "guesser")))
            .await;
    }
    for body in [json!({}), json!({ "code": "  " }), json!({ "code": 5 })] {
        let braked = app.call(Call::post("/api/auth/pair").json(&body)).await;
        assert_eq!(braked.status, 429, "{body}");
    }
}

#[tokio::test]
async fn a_code_too_long_to_be_one_counts_on_the_brake() {
    let app = TestApp::new().await;
    for _ in 0..5 {
        let long = app
            .call(Call::post("/api/auth/pair").json(&json!({ "code": "B".repeat(33) })))
            .await;
        assert_eq!(long.status, 401);
    }
    let braked = app
        .call(Call::post("/api/auth/pair").json(&pair_body("AAAA-AAAA", "guesser")))
        .await;
    assert_eq!(braked.status, 429);
}

#[tokio::test]
pub(crate) async fn the_status_says_whether_this_device_is_signed_in() {
    for mode in [AuthMode::Enforce, AuthMode::Report] {
        let app = TestApp::with_mode(mode).await;
        let fresh = app.call(Call::get(paths::AUTH_STATUS)).await;
        assert_eq!(fresh.status, 200, "{mode:?}: answered without a session");
        let fresh: AuthStatus = fresh.read_as();
        assert!(!fresh.signed_in && !fresh.info.bound);
        assert_eq!(fresh.info.mode, mode.to_api());

        let session = app.sign_in().await;
        let mine: AuthStatus = app
            .call(Call::get(paths::AUTH_STATUS).token(&session))
            .await
            .read_as();
        assert!(mine.signed_in && mine.info.bound);
        assert_eq!(mine.info.devices.len(), 1);
        assert_eq!(mine.info.devices[0].id.get(), session.device.get());

        // Without a session (or with a token that isn't current): the mode and whether the
        // server is in use, but not the devices.
        let stranger: AuthStatus = app
            .call(Call::get(paths::AUTH_STATUS).header("authorization", "Bearer nope"))
            .await
            .read_as();
        assert!(!stranger.signed_in && stranger.info.bound);
        assert_eq!(stranger.info.devices, Vec::<Device>::new());
    }
}

#[tokio::test]
async fn a_device_signed_out_is_signed_out_at_once_and_leaves_the_list() {
    // authRevoke's logic: `AuthService::sign_out`.
    let app = TestApp::new().await;
    let kept = app.sign_in().await;
    let gone = app
        .state
        .auth()
        .open_session(&crate::common::device("phone-2"))
        .await
        .unwrap();
    app.state.auth().sign_out(gone.device).await.unwrap();
    let status: AuthStatus = app
        .call(Call::get(paths::AUTH_STATUS).token(&kept))
        .await
        .read_as();
    let ids: Vec<i64> = status.info.devices.iter().map(|d| d.id.get()).collect();
    assert_eq!(ids, [kept.device.get()]);
    let refused: AuthStatus = app
        .call(Call::get(paths::AUTH_STATUS).token(&gone))
        .await
        .read_as();
    assert!(!refused.signed_in);
    let state = app.call(Call::get(paths::STATE).token(&gone)).await;
    assert_eq!(state.status, 401);
}
