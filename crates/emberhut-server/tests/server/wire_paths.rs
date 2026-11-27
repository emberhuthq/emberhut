//! The MCP endpoint answers at its wire path and names itself and its account tool by the wire
//! words (`emberhut_api::wire::mcp`).

use emberhut_api::error::ErrorBody;
use emberhut_api::mcp::{InitializeResult, RpcResponse, ToolsList};
use emberhut_api::paths;
use emberhut_api::wire;
use emberhut_core::RoomId;
use serde_json::json;

use crate::common::{self, Call, TestApp};

/// A JSON-RPC request with `id` 1.
fn rpc(method: &str) -> serde_json::Value {
    json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": {} })
}

#[tokio::test]
async fn the_mcp_endpoint_answers_at_its_wire_path() {
    let app = TestApp::new().await;
    let token = app.state.agent_tokens().for_room(RoomId::MAIN).unwrap();
    let bearer = format!("Bearer {}", token.as_str());

    let hello = app
        .call(
            Call::post(paths::MCP_ROOM)
                .header("authorization", &bearer)
                .json(&rpc("initialize")),
        )
        .await;
    assert_eq!(hello.status, 200, "{}", hello.text);
    let hello: InitializeResult =
        serde_json::from_value(hello.read_as::<RpcResponse>().result.unwrap()).unwrap();
    assert_eq!(hello.server_info.name, wire::mcp::SERVER_NAME);

    let list = app
        .call(
            Call::post(paths::MCP_ROOM)
                .header("authorization", &bearer)
                .json(&rpc("tools/list")),
        )
        .await;
    let list: ToolsList =
        serde_json::from_value(list.read_as::<RpcResponse>().result.unwrap()).unwrap();
    assert!(
        list.tools
            .iter()
            .any(|tool| tool.name == wire::mcp::ACCOUNT_TOOL),
        "the assistant's account tool is listed under its wire name"
    );

    let stranger = app
        .call(Call::post(paths::MCP_ROOM).json(&rpc("initialize")))
        .await;
    assert_eq!(stranger.status, 401);
    assert_eq!(stranger.read_as::<ErrorBody>().error, "unknown caller");

    // Not in the document (only `POST` is), so sent around the document's check.
    let get = app.send(common::get(paths::MCP_ROOM)).await;
    assert_eq!(get.status(), 405);
    assert_eq!(get.headers()["allow"], "POST");
}
