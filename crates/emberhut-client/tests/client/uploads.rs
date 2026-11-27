//! Uploads and downloads: the raw file as the body, its name in the query, its progress, the
//! server's refusals, and a stop asked for mid-way.

use std::ops::ControlFlow;
use std::sync::atomic::{AtomicUsize, Ordering};

use emberhut_api::chat::FileKind;
use emberhut_api::uploads::MAX_SIZE;
use emberhut_client::{Error, Progress, Upload};
use serde_json::json;

use crate::support::{Reply, TestServer, access_for, fixture, fresh};

fn stored() -> TestServer {
    TestServer::start(|_| Reply::Json(200, fixture("upload_reply.json")))
}

/// A progress callback that never stops, keeping what it was told.
fn recording(seen: &mut Vec<Progress>) -> impl FnMut(Progress) -> ControlFlow<()> + '_ {
    |progress| {
        seen.push(progress);
        ControlFlow::Continue(())
    }
}

#[test]
fn bytes_go_as_the_raw_body_with_the_name_in_the_query() {
    let server = stored();
    let client = server.client(fresh("r1"));
    let picture = b"\x89PNG not really a picture".to_vec();
    let mut seen = Vec::new();

    let file = client
        .upload(
            &Upload::bytes("Week 3 example.png", picture.clone()),
            recording(&mut seen),
        )
        .unwrap();

    assert_eq!(file.kind, FileKind::Image);
    assert_eq!(file.url, "/uploads/1a2b3c4d-Week%203%20example.png");
    let [request] = server.requests_to("/api/uploads").try_into().unwrap();
    assert_eq!(request.method, "POST");
    assert_eq!(request.query, "name=Week%203%20example.png");
    assert_eq!(request.body, picture);
    assert_eq!(
        request.header("content-type"),
        Some("application/octet-stream")
    );
    let length = picture.len().to_string();
    assert_eq!(request.header("content-length"), Some(length.as_str()));
    assert_eq!(request.header("transfer-encoding"), None);
    assert_eq!(
        request.header("authorization"),
        Some(format!("Bearer {}", access_for("r1")).as_str())
    );
    let total = picture.len() as u64;
    assert_eq!(
        seen.last(),
        Some(&Progress {
            done: total,
            total: Some(total)
        })
    );
    assert!(seen.windows(2).all(|w| w[0].done <= w[1].done), "{seen:?}");
}

#[test]
fn a_file_is_streamed_from_disk_under_its_own_name() {
    let server = stored();
    let client = server.client(fresh("r1"));
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let reports = AtomicUsize::new(0);

    client
        .upload(&Upload::file(path), |_| {
            reports.fetch_add(1, Ordering::Relaxed);
            ControlFlow::Continue(())
        })
        .unwrap();

    let [request] = server.requests_to("/api/uploads").try_into().unwrap();
    assert_eq!(request.query, "name=Cargo.toml");
    assert_eq!(request.body, std::fs::read(path).unwrap());
    assert!(reports.load(Ordering::Relaxed) > 0);
}

#[test]
fn a_blank_name_goes_as_file() {
    let server = stored();
    let client = server.client(fresh("r1"));
    client
        .upload(&Upload::bytes(" ", vec![1]), |_| ControlFlow::Continue(()))
        .unwrap();
    assert_eq!(server.requests_to("/api/uploads")[0].query, "name=file");
}

#[test]
fn a_refusal_is_the_servers_status_and_words() {
    let server = TestServer::start(|_| Reply::Json(400, fixture("upload_refused.json")));
    let client = server.client(fresh("r1"));
    let error = client
        .upload(&Upload::bytes("a.mov", vec![0; 16]), |_| {
            ControlFlow::Continue(())
        })
        .unwrap_err();
    assert!(
        matches!(error, Error::Status { status: 400, message: Some(ref why) } if why == "file is larger than 500 MB"),
        "{error:?}"
    );
}

#[test]
fn a_file_over_the_limit_or_missing_is_never_sent() {
    let server = stored();
    let client = server.client(fresh("r1"));
    let dir = std::env::temp_dir().join(format!("emberhut-client-uploads-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let large = dir.join("large.bin");
    // Sparse: it takes no room on disk.
    std::fs::File::create(&large)
        .unwrap()
        .set_len(MAX_SIZE + 1)
        .unwrap();
    let go = |_| ControlFlow::Continue(());

    let too_large = client.upload(&Upload::file(&large), go).unwrap_err();
    let missing = client
        .upload(&Upload::file(dir.join("missing.bin")), go)
        .unwrap_err();
    std::fs::remove_dir_all(&dir).unwrap();

    assert!(
        matches!(too_large, Error::TooLarge { size, limit } if size == MAX_SIZE + 1 && limit == MAX_SIZE),
        "{too_large:?}"
    );
    assert!(matches!(missing, Error::Read(_)), "{missing:?}");
    assert!(server.requests_to("/api/uploads").is_empty());
}

#[test]
fn an_upload_stops_when_asked() {
    let server = stored();
    let client = server.client(fresh("r1"));
    let error = client
        .upload(&Upload::bytes("a.bin", vec![7; 300_000]), |_| {
            ControlFlow::Break(())
        })
        .unwrap_err();
    assert!(matches!(error, Error::Cancelled), "{error:?}");
}

#[test]
fn a_refused_token_is_renewed_and_the_file_sent_again_from_its_start() {
    let server = TestServer::start(|request| match request.path.as_str() {
        "/api/auth/refresh" => Reply::Json(
            200,
            json!({ "ok": true, "status": 200, "access": access_for("r2"), "refresh": "r2", "device": 3,
                    "expires": "2099-01-01T00:00:00.000Z" }),
        ),
        _ if request.header("authorization")
            == Some(format!("Bearer {}", access_for("r2")).as_str()) =>
        {
            Reply::Json(200, fixture("upload_reply.json"))
        }
        _ => Reply::Json(401, fixture("error_sign_in.json")),
    });
    let client = server.client(fresh("r1"));
    let body = vec![9; 200_000];
    let mut seen = Vec::new();

    client
        .upload(
            &Upload::bytes("notes.txt", body.clone()),
            recording(&mut seen),
        )
        .unwrap();

    let requests = server.requests_to("/api/uploads");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].body, body);
    let total = body.len() as u64;
    let restarts = seen.windows(2).filter(|w| w[1].done < w[0].done).count();
    assert_eq!(restarts, 1, "the second sending starts over: {seen:?}");
    assert_eq!(seen.iter().filter(|p| p.done == total).count(), 2);
}

#[test]
fn a_download_streams_the_file_with_its_progress() {
    let file: Vec<u8> = (0..300_000_u32).map(|n| n.to_le_bytes()[0]).collect();
    let body = file.clone();
    let server = TestServer::start(move |_| Reply::Bytes(200, body.clone()));
    let client = server.client(fresh("r1"));
    let mut kept = Vec::new();
    let mut seen = Vec::new();

    let size = client
        .download("/uploads/1-a.mov", &mut kept, recording(&mut seen))
        .unwrap();

    assert_eq!(size, file.len() as u64);
    assert_eq!(kept, file);
    let total = Some(file.len() as u64);
    assert!(seen.len() > 1, "told chunk by chunk: {seen:?}");
    assert!(seen.iter().all(|p| p.total == total));
    assert_eq!(seen.last().map(|p| p.done), Some(file.len() as u64));
    let [request] = server.requests_to("/uploads/1-a.mov").try_into().unwrap();
    assert_eq!(request.header("accept-encoding"), Some("identity"));
    assert_eq!(
        request.header("authorization"),
        Some(format!("Bearer {}", access_for("r1")).as_str())
    );
}

#[test]
fn a_download_stops_when_asked_or_on_a_refusal() {
    let server = TestServer::start(|request| match request.path.as_str() {
        "/uploads/gone" => Reply::Json(404, json!({ "error": "not found" })),
        _ => Reply::Bytes(200, vec![1; 300_000]),
    });
    let client = server.client(fresh("r1"));
    let mut kept = Vec::new();
    let stopped = client
        .download("/uploads/a.bin", &mut kept, |_| ControlFlow::Break(()))
        .unwrap_err();
    assert!(matches!(stopped, Error::Cancelled), "{stopped:?}");
    let gone = client
        .download("/uploads/gone", &mut kept, |_| ControlFlow::Continue(()))
        .unwrap_err();
    assert_eq!(gone.status(), Some(404));
    let elsewhere = client
        .download("https://example.com/x", &mut kept, |_| {
            ControlFlow::Continue(())
        })
        .unwrap_err();
    assert!(matches!(elsewhere, Error::InvalidPath(_)));
}

#[test]
fn a_download_that_cant_be_written_says_so() {
    struct Full;
    impl std::io::Write for Full {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("disk full"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let server = TestServer::start(|_| Reply::Bytes(200, vec![1; 10]));
    let client = server.client(fresh("r1"));
    let error = client
        .download("/uploads/a.bin", &mut Full, |_| ControlFlow::Continue(()))
        .unwrap_err();
    assert!(matches!(error, Error::Write(_)), "{error:?}");
}

/// A hundred bytes of a body sent without a size.
const CHUNK: &str = "0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789";

#[test]
fn a_download_over_its_limit_is_refused_by_its_size_or_stopped_there() {
    let server = TestServer::start(|request| match request.path.as_str() {
        "/uploads/sized.mov" => Reply::Bytes(200, vec![1; 5000]),
        _ => Reply::Stream(vec![CHUNK; 15]),
    });
    let client = server.client_limited(fresh("r1"), 1000);
    let mut kept = Vec::new();
    let sized = client
        .download("/uploads/sized.mov", &mut kept, |_| {
            ControlFlow::Continue(())
        })
        .unwrap_err();
    assert!(
        matches!(
            sized,
            Error::TooLarge {
                size: 5000,
                limit: 1000
            }
        ),
        "{sized:?}"
    );
    assert!(kept.is_empty(), "refused before a byte is written");
    let streamed = client
        .download("/uploads/unsized.mov", &mut kept, |_| {
            ControlFlow::Continue(())
        })
        .unwrap_err();
    assert!(
        matches!(streamed, Error::TooLarge { size, limit: 1000 } if size > 1000),
        "{streamed:?}"
    );
    assert!(kept.len() <= 1000, "stopped at the limit: {}", kept.len());
}
