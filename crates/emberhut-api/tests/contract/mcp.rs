//! The MCP endpoint's messages: requests, answers, tool calls and their results.

use emberhut_api::mcp::{
    CallToolParams, CallToolResult, Content, InitializeResult, RpcRequest, RpcResponse, ToolsList,
};
use emberhut_api::settings::{DisplayUiChange, SettingsRequest};
use serde_json::json;

use crate::support::{from_value, to_value};

#[test]
fn a_tool_call_and_its_answer_read_as_an_agent_sends_and_takes_them() {
    let request: RpcRequest = from_value(json!({
        "jsonrpc": "2.0", "id": "call-1", "method": "tools/call",
        "params": { "name": "theme", "arguments": { "preset": "teal" } }
    }));
    assert_eq!(request.id, Some(json!("call-1")));
    let params: CallToolParams = from_value(request.params.unwrap());
    assert_eq!(params.name, "theme");
    assert_eq!(params.arguments.unwrap()["preset"], "teal");

    let answer: RpcResponse = from_value(json!({
        "result": { "content": [{ "type": "text", "text": "done" }], "isError": true },
        "jsonrpc": "2.0", "id": "call-1"
    }));
    let result: CallToolResult = from_value(answer.result.unwrap());
    assert_eq!(result.content, [Content::text("done")]);
    assert_eq!(result.is_error, Some(true));
    let ok = CallToolResult {
        content: vec![Content::text("done")],
        is_error: None,
    };
    assert_eq!(
        to_value(&ok),
        json!({ "content": [{ "type": "text", "text": "done" }] }),
        "isError is left out when the call did not fail"
    );
}

#[test]
fn initialize_and_the_tool_list_read_from_the_wire() {
    let hello: InitializeResult = from_value(json!({
        "protocolVersion": "2025-06-18",
        "capabilities": { "tools": { "listChanged": true } },
        "serverInfo": { "name": "emberhut", "version": "1" }
    }));
    assert!(hello.capabilities.tools.list_changed);
    assert_eq!(hello.server_info.name, emberhut_api::mcp::SERVER_NAME);
    let list: ToolsList = from_value(json!({
        "tools": [{
            "name": "theme",
            "description": "The colours.",
            "inputSchema": { "type": "object", "properties": {} }
        }]
    }));
    assert_eq!(list.tools[0].input_schema["type"], "object");
}

#[test]
fn a_display_change_writes_only_the_switches_given() {
    let request = SettingsRequest {
        ui: Some(DisplayUiChange {
            hide_goals: Some(false),
            hide_projects: None,
        }),
        ..SettingsRequest::default()
    };
    assert_eq!(to_value(&request), json!({ "ui": { "hideGoals": false } }));
}
