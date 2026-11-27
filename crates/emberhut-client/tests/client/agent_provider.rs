//! The agent's model provider: read, checked, saved and removed, a refusal in the server's words.

use emberhut_api::agent_provider::{ProviderKind, ProviderRequest};
use emberhut_client::Error;
use serde_json::json;

use crate::support::{Reply, TestServer, fresh};

/// An example request with a key.
fn request(model: Option<&str>) -> ProviderRequest {
    ProviderRequest {
        provider: ProviderKind::DeepSeek,
        api_key: Some("sk-example-key-0000".to_owned()),
        base_url: None,
        model: model.map(str::to_owned),
        fast_model: None,
    }
}

#[test]
fn the_provider_is_read_checked_saved_and_forgotten() {
    let saved = json!({ "provider": "deepseek", "keySet": true, "keyHint": "0000",
        "model": "example-model", "usable": true, "inUse": true });
    let server = TestServer::start(move |request| match request.method.as_str() {
        "POST" => Reply::Json(
            200,
            json!({ "checked": true, "models": [{ "id": "example-model", "name": "Example" }] }),
        ),
        "DELETE" => Reply::Json(
            200,
            json!({ "keySet": false, "usable": false, "inUse": false }),
        ),
        _ => Reply::Json(200, saved.clone()),
    });
    let client = server.client(fresh("r1"));
    assert_eq!(
        client.agent_provider().unwrap().key_hint.as_deref(),
        Some("0000")
    );
    let test = client.test_agent_provider(&request(None)).unwrap();
    assert_eq!(test.models[0].name.as_deref(), Some("Example"));
    let after = client
        .save_agent_provider(&request(Some("example-model")))
        .unwrap();
    assert!(after.in_use);
    assert_eq!(client.forget_agent_provider().unwrap().provider, None);

    let tested = server.requests_to("/api/agent/provider/test");
    assert_eq!(
        tested[0].json(),
        json!({ "provider": "deepseek", "apiKey": "sk-example-key-0000" })
    );
    let methods: Vec<_> = server
        .requests_to("/api/agent/provider")
        .into_iter()
        .map(|request| request.method)
        .collect();
    assert_eq!(methods, ["GET", "PUT", "DELETE"]);
    assert_eq!(
        server.requests_to("/api/agent/provider")[1].json(),
        json!({ "provider": "deepseek", "apiKey": "sk-example-key-0000",
            "model": "example-model" })
    );
}

#[test]
fn a_refused_key_comes_back_in_the_servers_words() {
    let server = TestServer::start(|_| {
        Reply::Json(
            422,
            json!({ "error": "DeepSeek didn't accept that key. Check that it was copied whole." }),
        )
    });
    let error = server
        .client(fresh("r1"))
        .test_agent_provider(&request(None))
        .unwrap_err();
    assert!(
        matches!(&error, Error::Status { status: 422, message: Some(m) } if m.contains("didn't accept")),
        "{error:?}"
    );
}
