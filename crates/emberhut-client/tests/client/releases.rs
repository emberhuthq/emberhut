//! The release feed and its payloads: the newest format first and the lower ones on a 404, a
//! payload streamed under a hard cap and hashed on the way.

use emberhut_api::releases::{Build, Platform};
use emberhut_client::Error;
use serde_json::json;
use sha2::{Digest as _, Sha256};

use crate::support::{Reply, TestServer, access_for, fixture, fresh};

#[test]
fn the_feed_is_asked_for_in_the_newest_format_first() {
    let server = TestServer::start(|request| match request.path.as_str() {
        "/api/releases/v1/macos" => Reply::Json(200, fixture("release_feed.json")),
        _ => Reply::Json(404, json!({ "error": "not found" })),
    });
    let client = server.client(fresh("r1"));

    let feed = client.release_feed(&Platform::Macos, 3).unwrap().unwrap();

    assert_eq!(feed.newest().unwrap().build, Build::new(15));
    let asked: Vec<usize> = (1..=3)
        .map(|format| {
            server
                .requests_to(&format!("/api/releases/v{format}/macos"))
                .len()
        })
        .collect();
    assert_eq!(asked, [1, 1, 1], "v3, then v2, then v1");
    let [request] = server
        .requests_to("/api/releases/v1/macos")
        .try_into()
        .unwrap();
    assert_eq!(request.method, "GET");
    assert_eq!(
        request.header("authorization"),
        Some(format!("Bearer {}", access_for("r1")).as_str())
    );
}

#[test]
fn nothing_published_is_none_and_a_failure_is_an_error() {
    let nothing = TestServer::start(|_| Reply::Json(404, json!({ "error": "not found" })));
    assert_eq!(
        nothing
            .client(fresh("r1"))
            .release_feed(&Platform::Macos, 2)
            .unwrap(),
        None
    );
    let failing = TestServer::start(|_| Reply::Json(500, json!({ "error": "disk" })));
    let error = failing
        .client(fresh("r1"))
        .release_feed(&Platform::Macos, 2)
        .unwrap_err();
    assert_eq!(error.status(), Some(500));
    assert_eq!(
        failing.requests_to("/api/releases/v1/macos").len(),
        0,
        "a failure isn't a missing format"
    );
}

#[test]
fn a_payload_streams_to_disk_with_its_hash() {
    let payload: Vec<u8> = (0..200_000_u32).map(|n| n.to_le_bytes()[1]).collect();
    let body = payload.clone();
    let server = TestServer::start(move |_| Reply::Bytes(200, body.clone()));
    let client = server.client(fresh("r1"));
    let mut kept = Vec::new();

    let got = client
        .download_release(
            "/api/releases/files/Emberhut_0.3.0_15_macos_arm64.dmg",
            &mut kept,
            payload.len() as u64,
        )
        .unwrap();

    assert_eq!(kept, payload);
    assert_eq!(got.size, payload.len() as u64);
    assert_eq!(got.sha256, <[u8; 32]>::from(Sha256::digest(&payload)));
    let [request] = server
        .requests_to("/api/releases/files/Emberhut_0.3.0_15_macos_arm64.dmg")
        .try_into()
        .unwrap();
    assert_eq!(request.header("accept-encoding"), Some("identity"));
    assert_eq!(
        request.header("authorization"),
        Some(format!("Bearer {}", access_for("r1")).as_str())
    );
}

#[test]
fn a_payload_over_its_cap_is_refused_by_its_size_or_cut_off() {
    let server = TestServer::start(|request| match request.path.as_str() {
        "/sized" => Reply::Bytes(200, vec![7; 1000]),
        _ => Reply::Stream(vec!["0123456789"; 100]),
    });
    let client = server.client(fresh("r1"));
    let mut kept = Vec::new();
    let refused = client
        .download_release("/sized", &mut kept, 999)
        .unwrap_err();
    assert!(
        matches!(
            refused,
            Error::TooLarge {
                size: 1000,
                limit: 999
            }
        ),
        "{refused:?}"
    );
    assert_eq!(kept, Vec::<u8>::new(), "nothing read");
    let cut = client
        .download_release("/unsized", &mut kept, 500)
        .unwrap_err();
    assert!(
        matches!(cut, Error::TooLarge { limit: 500, size } if size > 500),
        "{cut:?}"
    );
    assert!(kept.len() <= 500, "never more than the cap on disk");
}

#[test]
fn only_a_path_on_the_server_is_fetched() {
    let server = TestServer::start(|_| Reply::Bytes(200, vec![1; 10]));
    let client = server.client(fresh("r1"));
    let error = client
        .download_release("https://example.com/Emberhut.dmg", &mut Vec::new(), 100)
        .unwrap_err();
    assert!(matches!(error, Error::InvalidPath(_)), "{error:?}");
    let gone = TestServer::start(|_| Reply::Json(404, json!({ "error": "not found" })));
    let error = gone
        .client(fresh("r1"))
        .download_release("/api/releases/files/x.dmg", &mut Vec::new(), 100)
        .unwrap_err();
    assert_eq!(error.status(), Some(404));
}
