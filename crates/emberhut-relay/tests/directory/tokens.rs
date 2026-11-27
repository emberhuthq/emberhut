//! The token rules of ADR 0007, section 5, over HTTP: algorithms, claims, the two kinds of
//! token, and the keys' cache (refetch limit, outages, rotation).

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde_json::{Value, json};

use crate::support::{ACCOUNT, APP, Harness, NATIVE, Setup, WEB, current, next, stranger};

/// Whether `token` is taken: 200 on the list, or the refusal's code.
async fn taken(harness: &Harness, token: &str) -> Result<(), String> {
    let answer = harness.list(token).await;
    if answer.status == 200 {
        Ok(())
    } else {
        Err(answer.body["code"].as_str().unwrap_or("none").to_owned())
    }
}

#[tokio::test]
async fn a_session_token_shaped_as_clerk_makes_it_is_taken() {
    let harness = Harness::start().await;
    // RS256, a kid, `typ: JWT`, Clerk's own `cat` header field; aud, exp, iat, iss, jti, nbf,
    // sub; no azp, as from a native app.
    let token = harness.session(ACCOUNT, &json!({}));
    assert_eq!(taken(&harness, &token).await, Ok(()));
    // From the web app's origin, Clerk fills azp.
    let token = harness.session(ACCOUNT, &json!({ "azp": APP }));
    assert_eq!(taken(&harness, &token).await, Ok(()));
    // Fields a later Clerk adds are ignored.
    let token = harness.session(ACCOUNT, &json!({ "v": 2, "fva": [0, -1] }));
    assert_eq!(taken(&harness, &token).await, Ok(()));
}

#[tokio::test]
async fn every_claim_check_refuses_with_sign_in() {
    let harness = Harness::start().await;
    for extra in [
        json!({ "iss": "https://clerk.other" }),
        json!({ "iss": null }),
        json!({ "aud": "another-template" }),
        json!({ "aud": null }),
        json!({ "azp": "https://evil.example" }),
        json!({ "sub": null }),
        json!({ "exp": null }),
    ] {
        let token = harness.session(ACCOUNT, &extra);
        let answer = harness.list(&token).await;
        answer.refused(401, "sign-in");
    }
}

#[tokio::test]
async fn exp_and_nbf_have_thirty_seconds_of_leeway() {
    let harness = Harness::start().await;
    let now = harness.clock.seconds();
    for (extra, ok) in [
        (json!({ "exp": now - 29 }), true),
        (json!({ "exp": now - 30 }), false),
        (json!({ "exp": now - 3_600 }), false),
        (json!({ "nbf": now + 29 }), true),
        (json!({ "nbf": now + 31 }), false),
    ] {
        let token = harness.session(ACCOUNT, &extra);
        let result = taken(&harness, &token).await;
        assert_eq!(result.is_ok(), ok, "{extra}: {result:?}");
        if !ok {
            assert_eq!(result, Err("sign-in".to_owned()));
        }
    }
}

#[tokio::test]
async fn only_rs256_is_taken() {
    let harness = Harness::start().await;
    let claims = harness.session_claims(ACCOUNT, &json!({}));
    // alg none, unsigned.
    let unsigned = format!(
        "{}.{}.",
        URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT"}"#),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    assert_eq!(taken(&harness, &unsigned).await, Err("sign-in".to_owned()));
    // HS256 under a published kid, keyed with the key's public modulus.
    let mut header = Header::new(Algorithm::HS256);
    header.kid = Some(current().kid.clone());
    let modulus = current().jwk()["n"].as_str().unwrap().to_owned();
    let hs256 = jsonwebtoken::encode(
        &header,
        &claims,
        &EncodingKey::from_secret(modulus.as_bytes()).unwrap(),
    )
    .unwrap();
    assert_eq!(taken(&harness, &hs256).await, Err("sign-in".to_owned()));
    // A token whose signature is another's.
    let good = current().sign(Some("JWT"), &claims);
    let forged = stranger().sign(Some("JWT"), &claims);
    let spliced = format!(
        "{}.{}",
        good.rsplit_once('.').unwrap().0,
        forged.rsplit_once('.').unwrap().1
    );
    assert_eq!(taken(&harness, &spliced).await, Err("sign-in".to_owned()));
    // An opaque token.
    assert_eq!(
        taken(&harness, "oat_2example0opaque").await,
        Err("sign-in".to_owned())
    );
    assert_eq!(taken(&harness, &good).await, Ok(()));
}

#[tokio::test]
async fn an_access_token_is_taken_from_either_application() {
    let harness = Harness::start().await;
    let access = |client: &str, typ: Option<&str>, extra: &Value| {
        current().sign(typ, &harness.access_claims(ACCOUNT, client, extra))
    };
    // RFC 9068's form, by client_id, from both applications.
    for client in [NATIVE, WEB] {
        assert_eq!(
            taken(&harness, &access(client, Some("at+jwt"), &json!({}))).await,
            Ok(()),
            "{client}"
        );
    }
    // Marked only by the configured claim.
    assert_eq!(
        taken(&harness, &access(NATIVE, Some("JWT"), &json!({}))).await,
        Ok(())
    );
    assert_eq!(
        taken(&harness, &access(WEB, None, &json!({}))).await,
        Ok(())
    );
    // Named by azp, where there is no client_id.
    let by_azp = access(
        "",
        Some("application/at+jwt"),
        &json!({ "client_id": null, "azp": WEB }),
    );
    assert_eq!(taken(&harness, &by_azp).await, Ok(()));
}

#[tokio::test]
async fn an_access_token_for_another_client_is_refused() {
    let harness = Harness::start().await;
    let refused = |token: String| {
        let harness = &harness;
        async move { taken(harness, &token).await }
    };
    let claims = |client: &str, extra: &Value| harness.access_claims(ACCOUNT, client, extra);
    for token in [
        current().sign(Some("at+jwt"), &claims("another-client", &json!({}))),
        current().sign(Some("JWT"), &claims("another-client", &json!({}))),
        // client_id decides; azp doesn't override it.
        current().sign(
            Some("at+jwt"),
            &claims("another-client", &json!({ "azp": NATIVE })),
        ),
        current().sign(
            Some("at+jwt"),
            &claims("", &json!({ "client_id": null, "azp": "another-client" })),
        ),
        current().sign(Some("at+jwt"), &claims("", &json!({ "client_id": null }))),
        // The access-token form with the session template's audience is still an access token.
        current().sign(
            Some("at+jwt"),
            &claims("another-client", &json!({ "aud": "emberhut-directory" })),
        ),
    ] {
        assert_eq!(refused(token).await, Err("sign-in".to_owned()));
    }
}

#[tokio::test]
async fn an_id_token_from_the_same_keys_is_refused() {
    let harness = Harness::start().await;
    for client in [NATIVE, WEB] {
        let now = harness.clock.seconds();
        let id_token = json!({
            "iss": crate::support::ISSUER,
            "sub": ACCOUNT,
            "aud": client,
            "azp": client,
            "exp": now + 600,
            "iat": now,
            "nonce": "n-0S6_WzA2Mj",
        });
        let token = current().sign(Some("JWT"), &id_token);
        assert_eq!(
            taken(&harness, &token).await,
            Err("sign-in".to_owned()),
            "{client}"
        );
    }
}

/// A wait long enough that requests made right after a fetch are surely within it.
const LONG_WAIT: Duration = Duration::from_secs(3);
/// A wait short enough to sleep past in a test.
const SHORT_WAIT: Duration = Duration::from_millis(200);
/// Sleeping this long more than a wait is surely past it.
const PAST: Duration = Duration::from_millis(500);

#[tokio::test]
async fn an_unknown_kid_makes_at_most_one_fetch_a_minute() {
    let harness = Harness::with(Setup {
        unknown_kid_wait: LONG_WAIT,
        ..Setup::default()
    })
    .await;
    let token = harness.session(ACCOUNT, &json!({}));
    let claims = harness.session_claims(ACCOUNT, &json!({}));
    // Signed first: making a test key takes a moment.
    let unknown = stranger().sign(Some("JWT"), &claims);
    assert_eq!(taken(&harness, &token).await, Ok(()));
    assert_eq!(harness.jwks.fetches(), 1);
    // Right after a fetch, an unknown kid fetches nothing, and is never a sign-out.
    harness.list(&unknown).await.refused(503, "unavailable");
    assert_eq!(harness.jwks.fetches(), 1);
    tokio::time::sleep(LONG_WAIT + PAST).await;
    // Once the wait is over it makes one fetch; still not published, so not Clerk's.
    harness.list(&unknown).await.refused(401, "sign-in");
    assert_eq!(harness.jwks.fetches(), 2);
    for _ in 0..5 {
        let answer = harness.list(&unknown).await;
        answer.refused(503, "unavailable");
        let seconds: u64 = answer.header("retry-after").unwrap().parse().unwrap();
        assert!((1..=3).contains(&seconds), "{seconds}");
    }
    assert_eq!(harness.jwks.fetches(), 2);
    // Known keys are unaffected.
    assert_eq!(taken(&harness, &token).await, Ok(()));
}

#[tokio::test]
async fn the_wait_after_a_fetch_is_a_minute_by_default() {
    let harness = Harness::start().await;
    let token = harness.session(ACCOUNT, &json!({}));
    let claims = harness.session_claims(ACCOUNT, &json!({}));
    let unknown = stranger().sign(Some("JWT"), &claims);
    assert_eq!(taken(&harness, &token).await, Ok(()));
    let answer = harness.list(&unknown).await;
    answer.refused(503, "unavailable");
    let seconds: u64 = answer.header("retry-after").unwrap().parse().unwrap();
    assert!((50..=60).contains(&seconds), "{seconds}");
    assert_eq!(harness.jwks.fetches(), 1);
}

#[tokio::test]
async fn a_rotation_never_fails_a_valid_token() {
    let harness = Harness::with(Setup {
        unknown_kid_wait: LONG_WAIT,
        ..Setup::default()
    })
    .await;
    let claims = harness.session_claims(ACCOUNT, &json!({}));
    let old = current().sign(Some("JWT"), &claims);
    let new = next().sign(Some("JWT"), &claims);
    assert_eq!(taken(&harness, &old).await, Ok(()));
    // Clerk publishes the next key before it signs with it.
    harness.jwks.publish(&[current(), next()]);
    // A token by the new key, within the wait after the last fetch: try again, not sign in.
    harness.list(&new).await.refused(503, "unavailable");
    tokio::time::sleep(LONG_WAIT + PAST).await;
    assert_eq!(taken(&harness, &new).await, Ok(()));
    assert_eq!(taken(&harness, &old).await, Ok(()));
    assert_eq!(harness.jwks.fetches(), 2);
}

#[tokio::test]
async fn while_clerk_is_unreachable_known_keys_work_and_unknown_kids_wait() {
    let harness = Harness::with(Setup {
        unknown_kid_wait: SHORT_WAIT,
        ..Setup::default()
    })
    .await;
    let claims = harness.session_claims(ACCOUNT, &json!({}));
    let known = current().sign(Some("JWT"), &claims);
    let rotated = next().sign(Some("JWT"), &claims);
    assert_eq!(taken(&harness, &known).await, Ok(()));
    harness.jwks.set_down(true);
    harness.jwks.publish(&[current(), next()]);
    tokio::time::sleep(SHORT_WAIT + PAST).await;
    // The fetch fails: 503, never 401.
    harness.list(&rotated).await.refused(503, "unavailable");
    assert_eq!(harness.jwks.fetches(), 2);
    assert_eq!(taken(&harness, &known).await, Ok(()));
    // Back up: once the wait has passed, the new key is fetched.
    harness.jwks.set_down(false);
    tokio::time::sleep(SHORT_WAIT + PAST).await;
    assert_eq!(taken(&harness, &rotated).await, Ok(()));
}

#[tokio::test]
async fn before_the_first_fetch_succeeds_tokens_wait_rather_than_sign_out() {
    let harness = Harness::with(Setup {
        unknown_kid_wait: SHORT_WAIT,
        ..Setup::default()
    })
    .await;
    harness.jwks.set_down(true);
    let token = harness.session(ACCOUNT, &json!({}));
    harness.list(&token).await.refused(503, "unavailable");
    harness.jwks.set_down(false);
    tokio::time::sleep(SHORT_WAIT + PAST).await;
    assert_eq!(taken(&harness, &token).await, Ok(()));
}

#[tokio::test]
async fn the_set_is_refreshed_in_the_background_and_kept_if_that_fails() {
    let refresh = Duration::from_secs(1);
    let harness = Harness::with(Setup {
        refresh,
        unknown_kid_wait: SHORT_WAIT,
        ..Setup::default()
    })
    .await;
    let token = harness.session(ACCOUNT, &json!({}));
    let rotated = next().sign(Some("JWT"), &harness.session_claims(ACCOUNT, &json!({})));
    assert_eq!(taken(&harness, &token).await, Ok(()));
    assert_eq!(harness.jwks.fetches(), 1);
    harness.jwks.set_down(true);
    tokio::time::sleep(refresh + PAST).await;
    // Due: the token is checked with the old set, and the refresh, which fails, runs apart.
    assert_eq!(taken(&harness, &token).await, Ok(()));
    harness.jwks.wait_for_fetches(2).await;
    // Up again, with the old key retired: the new key's token fetches the set.
    harness.jwks.set_down(false);
    harness.jwks.publish(&[next()]);
    tokio::time::sleep(SHORT_WAIT + PAST).await;
    assert_eq!(taken(&harness, &rotated).await, Ok(()));
    assert_eq!(harness.jwks.fetches(), 3);
    // The retired key is gone from that set: once the wait has passed, one more fetch, and
    // its tokens are refused.
    tokio::time::sleep(SHORT_WAIT + PAST).await;
    assert_eq!(taken(&harness, &token).await, Err("sign-in".to_owned()));
    assert_eq!(harness.jwks.fetches(), 4);
}

#[tokio::test]
async fn no_request_waits_on_a_refresh() {
    let refresh = Duration::from_secs(1);
    let harness = Harness::with(Setup {
        refresh,
        unknown_kid_wait: SHORT_WAIT,
        ..Setup::default()
    })
    .await;
    let token = harness.session(ACCOUNT, &json!({}));
    assert_eq!(taken(&harness, &token).await, Ok(()));
    // Clerk becomes slow: each answer takes 4 s.
    harness.jwks.set_delay(Duration::from_secs(4));
    tokio::time::sleep(refresh + PAST).await;
    for _ in 0..3 {
        let started = std::time::Instant::now();
        assert_eq!(taken(&harness, &token).await, Ok(()));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }
    // One refresh is under way, not one per request.
    harness.jwks.wait_for_fetches(2).await;
    tokio::time::sleep(PAST).await;
    assert_eq!(harness.jwks.fetches(), 2);
}
