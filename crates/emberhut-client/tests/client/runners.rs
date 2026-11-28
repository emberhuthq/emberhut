//! The runners: a pairing code with its install lines, the list, and forgetting one.

use emberhut_api::runner::pairing::RunnerSource;
use emberhut_client::Error;
use serde_json::json;

use crate::support::{Reply, TestServer, access_for, fixture, fresh};

#[test]
fn a_pairing_code_carries_the_app_s_address_when_it_has_one() {
    let server = TestServer::start(|_| Reply::Json(200, fixture("runner_code.json")));
    let client = server.client(fresh("r1"));

    let code = client
        .runner_pair_code(Some("https://emberhut.example.com/"))
        .unwrap();
    assert_eq!(code.code, "ABCD-EFGH");
    assert!(code.reachable);
    assert!(code.install.command.ends_with("--pair ABCD-EFGH"));
    client.runner_pair_code(None).unwrap();

    let [with, without] = server.requests_to("/api/runners/codes").try_into().unwrap();
    assert_eq!(with.method, "POST");
    assert_eq!(
        with.json(),
        json!({ "server": "https://emberhut.example.com" }),
        "the final / dropped"
    );
    assert_eq!(without.json(), json!({}));
    assert_eq!(
        with.header("authorization"),
        Some(format!("Bearer {}", access_for("r1")).as_str())
    );
}

#[test]
fn the_list_reads_where_each_runner_comes_from() {
    let server = TestServer::start(|_| Reply::Json(200, fixture("runners.json")));
    let client = server.client(fresh("r1"));
    let page = client.runners().unwrap();
    assert_eq!(page.runners.len(), 2);
    assert_eq!(page.runner("build").unwrap().source, RunnerSource::Paired);
    let [sent] = server.requests_to("/api/runners").try_into().unwrap();
    assert_eq!((sent.method.as_str(), sent.query.as_str()), ("GET", ""));
}

#[test]
fn forgetting_a_runner_names_it_in_the_path() {
    let server = TestServer::start(|request| {
        if request.path == "/api/runners/build" {
            Reply::Json(200, json!({ "ok": true }))
        } else {
            Reply::Json(
                404,
                json!({ "error": "No paired runner is called laptop." }),
            )
        }
    });
    let client = server.client(fresh("r1"));
    client.forget_runner("build").unwrap();
    let [sent] = server.requests_to("/api/runners/build").try_into().unwrap();
    assert_eq!(sent.method, "DELETE");

    let error = client.forget_runner("laptop").unwrap_err();
    assert!(
        matches!(&error, Error::Status { status: 404, message: Some(m) } if m.contains("laptop")),
        "{error:?}"
    );
}

#[test]
fn a_name_no_runner_can_have_is_never_sent() {
    let server = TestServer::start(|_| Reply::Json(200, json!({ "ok": true })));
    let client = server.client(fresh("r1"));
    for name in ["../auth/pair", "build box", "codes", ""] {
        assert!(
            matches!(client.forget_runner(name), Err(Error::InvalidPath(_))),
            "{name}"
        );
    }
    assert!(server.requests_to("/api/runners/codes").is_empty());
}
