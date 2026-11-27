//! `POST` [`emberhut_api::paths::MCP_ROOM`]: the MCP endpoint a room's agent session calls, with
//! its room's token.

use axum::body::Body;
use axum::http::{Method, Request};
use emberhut_api::error::ErrorBody;
use emberhut_api::mcp::{InitializeResult, RpcResponse, ToolsList};
use emberhut_api::paths;
use emberhut_api::theme::ThemePage;
use emberhut_core::RoomId;
use http_body_util::BodyExt as _;
use serde_json::{Value, json};

use crate::common::{Call, TestApp, text};

/// The endpoint's path.
const MCP: &str = paths::MCP_ROOM;

#[tokio::test]
pub(crate) async fn initialize_then_list_then_call_a_tool() {
    let app = TestApp::new().await;
    let answer = app
        .rpc(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "an agent", "version": "1" }
            }),
        )
        .await;
    assert_eq!(answer.status, 200);
    assert!(
        answer.text.starts_with(r#"{"result":"#),
        "`result` comes first: {}",
        answer.text
    );
    let reply: RpcResponse = answer.read_as();
    assert_eq!((reply.jsonrpc.as_str(), &reply.id), ("2.0", &json!(7)));
    let hello: InitializeResult = serde_json::from_value(reply.result.unwrap()).unwrap();
    assert_eq!(
        hello.protocol_version, "2025-06-18",
        "the client's revision"
    );
    assert_eq!(hello.server_info.name, "emberhut");
    assert_eq!(hello.server_info.version, "1");
    assert!(hello.capabilities.tools.list_changed);

    let newest: InitializeResult = serde_json::from_value(
        app.rpc("initialize", json!({ "protocolVersion": "1999-01-01" }))
            .await
            .read_as::<RpcResponse>()
            .result
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        newest.protocol_version,
        emberhut_api::mcp::PROTOCOL_VERSIONS[0],
        "an unknown revision gets the newest"
    );

    let list: ToolsList = serde_json::from_value(
        app.rpc("tools/list", json!({}))
            .await
            .read_as::<RpcResponse>()
            .result
            .unwrap(),
    )
    .unwrap();
    let theme = list
        .tools
        .iter()
        .find(|tool| tool.name == "theme")
        .expect("the theme tool is listed");
    assert_ne!(theme.description, "");
    assert_eq!(theme.input_schema["type"], "object");
    for property in ["preset", "appearance", "accent"] {
        assert!(
            theme.input_schema["properties"][property].is_object(),
            "{property}: {}",
            theme.input_schema
        );
    }

    let read = app.call_tool("theme", json!({})).await;
    assert_eq!(read.is_error, None);
    let read: Value = serde_json::from_str(text(&read)).unwrap();
    assert_eq!(read["choice"]["preset"], "paper");
    assert_eq!(read["aa"], true);

    let changed = app
        .call_tool("theme", json!({ "preset": "green", "appearance": "light" }))
        .await;
    assert_eq!(changed.is_error, None, "{changed:?}");
    let changed: Value = serde_json::from_str(text(&changed)).unwrap();
    assert_eq!(changed["choice"]["preset"], "green");
    assert_eq!(changed["accent"]["light"], "#15803D");

    let session = app.sign_in().await;
    let page: ThemePage = app
        .call(Call::get(paths::THEME).token(&session))
        .await
        .read_as();
    assert_eq!(page.choice.preset, "green", "the tool changed the theme");
}

#[tokio::test]
pub(crate) async fn a_tool_that_isnt_there_or_bad_arguments_are_error_results() {
    let app = TestApp::new().await;
    let missing = app.call_tool("nope", json!({})).await;
    assert_eq!(missing.is_error, Some(true));
    assert_eq!(text(&missing), "MCP error -32602: Tool nope not found");

    for arguments in [json!({ "appearance": "dim" }), json!({ "preset": 5 })] {
        let bad = app.call_tool("theme", arguments.clone()).await;
        assert_eq!(bad.is_error, Some(true), "{arguments}");
        assert!(
            text(&bad).starts_with(
                "MCP error -32602: Input validation error: Invalid arguments for tool theme: "
            ),
            "{arguments}: {}",
            text(&bad)
        );
    }

    let refused = app.call_tool("theme", json!({ "preset": "pink" })).await;
    assert_eq!(refused.is_error, Some(true));
    assert_eq!(
        text(&refused),
        "preset: one of paper, graphite, blue, teal, green, orange, rose, purple"
    );

    // tools/call without a name is not a call.
    let reply: RpcResponse = app.rpc("tools/call", json!({})).await.read_as();
    assert_eq!(reply.error.unwrap().code, -32602);
}

#[tokio::test]
pub(crate) async fn other_methods_notifications_and_bodies() {
    let app = TestApp::new().await;
    let ping: RpcResponse = app.rpc("ping", json!({})).await.read_as();
    assert_eq!(ping.result, Some(json!({})));

    let unknown = app.rpc("resources/list", json!({})).await;
    assert_eq!(unknown.status, 200);
    assert!(
        unknown
            .text
            .starts_with(r#"{"jsonrpc":"2.0","id":7,"error":"#),
        "{}",
        unknown.text
    );
    let error = unknown.read_as::<RpcResponse>().error.unwrap();
    assert_eq!(
        (error.code, error.message.as_str()),
        (-32601, "Method not found")
    );

    let token = app.state.agent_tokens().for_room(RoomId::MAIN).unwrap();
    let bearer = format!("Bearer {}", token.as_str());
    let notified = app
        .call(
            Call::post(MCP)
                .header("authorization", &bearer)
                .json(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })),
        )
        .await;
    assert_eq!(notified.status, 202);
    assert_eq!(notified.text, "");

    for (body, code) in [
        ("{not json", -32700),
        (r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#, -32600),
        (r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#, -32600),
        (r#"{"jsonrpc":"2.0","id":1}"#, -32600),
    ] {
        let answer = app
            .call(Call::post(MCP).header("authorization", &bearer).raw(body))
            .await;
        assert_eq!(answer.status, 400, "{body}");
        let reply: RpcResponse = answer.read_as();
        assert_eq!(reply.error.unwrap().code, code, "{body}");
    }

    let huge = json!({ "jsonrpc": "2.0", "id": 1, "method": "x".repeat(3 * 1024 * 1024) });
    let too_large = app
        .call(Call::post(MCP).header("authorization", &bearer).json(&huge))
        .await;
    assert_eq!(too_large.status, 413);
    assert_eq!(
        too_large.read_as::<RpcResponse>().error.unwrap().code,
        -32600
    );
}

#[tokio::test]
pub(crate) async fn only_a_rooms_agent_token_is_let_in() {
    let app = TestApp::new().await;
    let ping = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" });
    let session = app.sign_in().await;
    let main = app.state.agent_tokens().for_room(RoomId::MAIN).unwrap();
    app.state.agent_tokens().revoke(RoomId::MAIN);
    for call in [
        Call::post(MCP).json(&ping),
        Call::post(MCP)
            .header("authorization", "Bearer not-a-token")
            .json(&ping),
        // A device's session is not an agent's.
        Call::post(MCP).token(&session).json(&ping),
        // A token stopped is refused.
        Call::post(MCP)
            .header("authorization", &format!("Bearer {}", main.as_str()))
            .json(&ping),
    ] {
        let refused = app.call(call).await;
        assert_eq!(refused.status, 401);
        assert_eq!(
            refused.read_as::<ErrorBody>(),
            ErrorBody::new("unknown caller")
        );
    }
}

#[tokio::test]
async fn any_other_method_is_405_post_only() {
    let app = TestApp::new().await;
    for method in [Method::GET, Method::PUT, Method::DELETE] {
        // Before the caller is looked at: no token is needed for the refusal.
        let request = Request::builder()
            .method(method.clone())
            .uri(MCP)
            .body(Body::empty())
            .unwrap();
        let response = app.send(request).await;
        assert_eq!(response.status(), 405, "{method}");
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({ "error": "POST only" }),
            "{method}"
        );
    }
}
