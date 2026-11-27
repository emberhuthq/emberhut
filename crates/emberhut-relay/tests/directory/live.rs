//! Against a real Clerk instance, run by hand (`cargo test -p emberhut-relay --test directory --
//! --ignored live`), never in CI. It reads the instance's settings from the JSON file named by
//! `EMBERHUT_RELAY_LIVE_SETTINGS` (`issuer`, `jwks_url`, `audience`, `native_client_id`), and, if
//! `EMBERHUT_RELAY_LIVE_TOKEN` names a file holding a token the instance issued, checks it. Nothing
//! it reads is printed.
#![expect(
    clippy::unwrap_used,
    reason = "a test helper: a failure here fails the test that called it"
)]

use std::sync::Arc;

use emberhut_relay::directory::{
    AccountRefusal, AccountVerifier as _, ClerkSettings, ClerkVerifier, KeyTiming, SystemClock,
};
use serde::Deserialize;
use serde_json::json;

use crate::support::stranger;

#[derive(Deserialize)]
struct LiveSettings {
    issuer: String,
    jwks_url: String,
    audience: String,
    native_client_id: Option<String>,
}

fn read(variable: &str) -> Option<String> {
    let path = std::env::var_os(variable)?;
    Some(std::fs::read_to_string(path).unwrap().trim().to_owned())
}

#[tokio::test]
#[ignore = "reaches a real Clerk instance; run by hand with EMBERHUT_RELAY_LIVE_SETTINGS set"]
async fn live_clerk_keys_and_tokens_are_read() {
    let settings: LiveSettings = serde_json::from_str(
        &read("EMBERHUT_RELAY_LIVE_SETTINGS").expect("EMBERHUT_RELAY_LIVE_SETTINGS"),
    )
    .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let verifier = ClerkVerifier::new(
        ClerkSettings {
            issuer: settings.issuer.clone(),
            jwks_url: settings.jwks_url.parse().unwrap(),
            audience: settings.audience,
            client_ids: settings.native_client_id.into_iter().collect(),
            allowed_origins: Vec::new(),
            access_token_claim: "client_id".to_owned(),
            key_timing: KeyTiming::default(),
        },
        Arc::new(SystemClock),
    )
    .unwrap();
    // A token under a kid the instance doesn't publish: the keys are fetched (else this would
    // be Unavailable), and it is refused.
    let foreign = stranger().sign(
        Some("JWT"),
        &json!({ "iss": settings.issuer, "sub": "user_x", "aud": "emberhut-directory", "exp": now + 60 }),
    );
    assert_eq!(
        verifier.verify(&foreign).await,
        Err(AccountRefusal::SignIn),
        "the instance's keys couldn't be fetched"
    );
    if let Some(token) = read("EMBERHUT_RELAY_LIVE_TOKEN") {
        assert!(
            verifier.verify(&token).await.is_ok(),
            "the instance's token was refused"
        );
    }
}
