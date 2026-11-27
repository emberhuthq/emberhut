//! Where the assistant works and what it uses: environments, usage and jobs.

use emberhut_api::environments::{EnvActionKind, EnvActionRequest};
use emberhut_api::ids::{EnvironmentId, MachineJobId};
use emberhut_api::jobs::{JobAction, JobActionRequest};
use emberhut_api::usage::UseAccountRequest;
use emberhut_client::Error;
use serde_json::json;

use crate::support::{Reply, TestServer, fixture, fresh};

#[test]
fn the_environments_are_read_and_a_rows_action_posted() {
    let server = TestServer::start(|request| match request.method.as_str() {
        "GET" => Reply::Json(200, fixture("environments.json")),
        _ if request.json()["kind"] == "start" => Reply::Json(
            400,
            json!({ "error": "Only the browser can be started here" }),
        ),
        _ => Reply::Json(200, fixture("environments_check_reply.json")),
    });
    let client = server.client(fresh("r1"));
    assert_eq!(client.environments().unwrap().groups.len(), 4);
    let link = EnvActionRequest {
        id: EnvironmentId::new("gmail:work"),
        kind: EnvActionKind::Link,
    };
    let page = client.environment_action(&link).unwrap();
    assert!(page.link.is_some());
    let start = EnvActionRequest {
        id: EnvironmentId::new("runner"),
        kind: EnvActionKind::Start,
    };
    let error = client.environment_action(&start).unwrap_err();
    assert!(
        matches!(&error, Error::Status { status: 400, message: Some(m) } if m.contains("started")),
        "{error:?}"
    );
    let sent = server.requests_to("/api/environments/check");
    assert_eq!(
        sent[0].json(),
        json!({ "id": "gmail:work", "kind": "link" })
    );
}

#[test]
fn usage_is_checked_and_an_account_pinned() {
    let server = TestServer::start(|request| match request.path.as_str() {
        "/api/usage/check" => Reply::Json(200, json!({ "ok": true, "asked": true })),
        _ => Reply::Json(200, fixture("use_account_reply.json")),
    });
    let client = server.client(fresh("r1"));
    assert!(client.check_usage().unwrap());
    let reply = client.use_account(&UseAccountRequest::auto()).unwrap();
    assert_eq!(reply.usage.len(), 1);
    assert_eq!(server.requests_to("/api/usage/check")[0].method, "POST");
    assert_eq!(
        server.requests_to("/api/accounts/use")[0].json(),
        json!({ "account": "auto" })
    );
}

#[test]
fn the_jobs_are_read_fresh_and_one_stopped() {
    let server = TestServer::start(|request| match request.method.as_str() {
        "GET" => Reply::Json(200, fixture("jobs.json")),
        _ if request.path == "/api/jobs/clear" => Reply::Json(
            400,
            json!({ "ok": false, "error": "only a failed job can be cleared" }),
        ),
        _ => Reply::Json(200, json!({ "ok": true })),
    });
    let client = server.client(fresh("r1"));
    assert_eq!(client.jobs(true).unwrap().jobs.len(), 3);
    let id = JobActionRequest {
        id: MachineJobId::new("unit:ex12-pictures"),
    };
    client.act_on_job(JobAction::Stop, &id).unwrap();
    let error = client.act_on_job(JobAction::Clear, &id).unwrap_err();
    assert!(
        matches!(&error, Error::Status { status: 400, message: Some(m) } if m.contains("failed")),
        "{error:?}"
    );
    let read = server.requests_to("/api/jobs");
    assert_eq!(read[0].query, "fresh=1");
    assert_eq!(
        server.requests_to("/api/jobs/stop")[0].json(),
        json!({ "id": "unit:ex12-pictures" })
    );
}
