//! CORS: the web app's origin only, with the headers clients send, and no credentials.
#![expect(
    clippy::unwrap_used,
    reason = "a test helper: a failure here fails the test that called it"
)]

use serde_json::json;

use crate::support::{ACCOUNT, APP, Answer, Harness, VECTOR_MACHINE};

async fn preflight(harness: &Harness, path: &str, origin: &str, method: &str) -> Answer {
    Answer::of(
        harness
            .http
            .request(reqwest::Method::OPTIONS, harness.url(path))
            .header("origin", origin)
            .header("access-control-request-method", method)
            .header(
                "access-control-request-headers",
                "authorization,content-type",
            )
            .send()
            .await
            .unwrap(),
    )
    .await
}

#[tokio::test]
async fn the_web_app_is_allowed_every_route_with_the_headers_it_sends() {
    let harness = Harness::start().await;
    for (path, method) in [
        ("/v1/machines", "GET"),
        ("/v1/machines", "POST"),
        (format!("/v1/machines/{VECTOR_MACHINE}").as_str(), "DELETE"),
        (
            format!("/v1/machines/{VECTOR_MACHINE}/unlink").as_str(),
            "POST",
        ),
    ] {
        let answer = preflight(&harness, path, APP, method).await;
        assert_eq!(answer.status, 200, "{path} {method}");
        assert_eq!(answer.header("access-control-allow-origin"), Some(APP));
        let methods = answer.header("access-control-allow-methods").unwrap();
        assert!(methods.contains(method), "{methods}");
        let headers = answer
            .header("access-control-allow-headers")
            .unwrap()
            .to_ascii_lowercase();
        assert!(headers.contains("authorization") && headers.contains("content-type"));
        assert_eq!(answer.header("access-control-allow-credentials"), None);
        assert!(
            answer
                .header("vary")
                .unwrap()
                .to_ascii_lowercase()
                .contains("origin")
        );
    }
}

#[tokio::test]
async fn other_origins_are_not_allowed() {
    let harness = Harness::start().await;
    for origin in [
        "https://emberhut.com",
        "https://evil.example",
        "http://app.emberhut.com",
        "null",
    ] {
        let answer = preflight(&harness, "/v1/machines", origin, "GET").await;
        assert_eq!(
            answer.header("access-control-allow-origin"),
            None,
            "{origin}"
        );
    }
}

#[tokio::test]
async fn answers_and_refusals_carry_the_origin_and_show_retry_after() {
    let harness = Harness::start().await;
    let token = harness.session(ACCOUNT, &json!({}));
    for token in [token.as_str(), "opaque"] {
        let answer = Answer::of(
            harness
                .http
                .get(harness.url("/v1/machines"))
                .header("origin", APP)
                .bearer_auth(token)
                .send()
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(answer.header("access-control-allow-origin"), Some(APP));
        assert!(
            answer
                .header("access-control-expose-headers")
                .unwrap()
                .to_ascii_lowercase()
                .contains("retry-after")
        );
        assert_eq!(answer.header("access-control-allow-credentials"), None);
    }
}
