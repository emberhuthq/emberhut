//! `POST` [`crate::paths::MCP_ROOM`]: the MCP endpoint an agent session calls for the server's
//! own tools.
//!
//! The Model Context Protocol over its Streamable HTTP transport, stateless: every request is
//! one JSON-RPC 2.0 message in the body, and its answer is one JSON-RPC message (or 202 with no
//! body for a notification). The methods served are `initialize`, `ping`, `tools/list` and
//! `tools/call`; any other is the JSON-RPC error "Method not found".
//!
//! A call to a tool always answers with a [`CallToolResult`]: a tool that isn't there, arguments
//! its schema refuses, and a tool that failed are each a result with `isError: true` and the
//! reason as text, which the agent reads, rather than a JSON-RPC error.
//!
//! The caller is a room's agent session, with the room's own token as its bearer; a request
//! without one the server issued is 401 with an [`crate::error::ErrorBody`].

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The server's name in `initialize`'s answer, which agent configurations name the server by
/// ([`crate::wire`]).
pub const SERVER_NAME: &str = crate::wire::mcp::SERVER_NAME;

/// The server's version in `initialize`'s answer.
pub const SERVER_VERSION: &str = "1";

/// The MCP revisions the server speaks, newest first. A client asking for one of them gets it;
/// any other gets the newest, and decides itself whether it can go on.
pub const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// The only JSON-RPC version.
pub const JSONRPC: &str = "2.0";

/// JSON-RPC's error codes the endpoint answers with.
pub mod codes {
    /// The body is not JSON.
    pub const PARSE_ERROR: i64 = -32700;
    /// The body is JSON but not a JSON-RPC request.
    pub const INVALID_REQUEST: i64 = -32600;
    /// No such method.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// The method's parameters are not ones it takes.
    pub const INVALID_PARAMS: i64 = -32602;
}

/// The methods the endpoint serves.
pub mod methods {
    /// The client says hello and which revision it speaks.
    pub const INITIALIZE: &str = "initialize";
    /// The client is ready (a notification).
    pub const INITIALIZED: &str = "notifications/initialized";
    /// Is the server there.
    pub const PING: &str = "ping";
    /// Every tool, with its input schema.
    pub const TOOLS_LIST: &str = "tools/list";
    /// Runs one tool.
    pub const TOOLS_CALL: &str = "tools/call";
}

/// One JSON-RPC message from the client: a request (with an `id`) or a notification (without).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RpcRequest {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// The request's id, a number or a string, which its answer repeats; none for a
    /// notification, which has no answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Value>))]
    pub id: Option<Value>,
    /// The method.
    pub method: String,
    /// Its parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Value>))]
    pub params: Option<Value>,
}

/// The answer to a request: its `result`, or its `error`.
///
/// Fields are in a fixed order: a result reads `{ result, jsonrpc, id }` and an error
/// `{ jsonrpc, id, error }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RpcResponse {
    /// What the method answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Value>))]
    pub result: Option<Value>,
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// The request's id; `null` when the request could not be read.
    #[cfg_attr(feature = "openapi", schema(value_type = Value))]
    pub id: Value,
    /// Why the method has no result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl RpcResponse {
    /// The answer `result` to the request `id`.
    #[must_use]
    pub fn result(id: Value, result: Value) -> Self {
        Self {
            result: Some(result),
            jsonrpc: JSONRPC.to_owned(),
            id,
            error: None,
        }
    }

    /// The error `code` with `message`, for the request `id`.
    #[must_use]
    pub fn error(id: Value, code: i64, message: impl Into<String>) -> Self {
        Self {
            result: None,
            jsonrpc: JSONRPC.to_owned(),
            id,
            error: Some(RpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

/// A JSON-RPC error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RpcError {
    /// One of [`codes`].
    pub code: i64,
    /// What went wrong.
    pub message: String,
    /// More about it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Value>))]
    pub data: Option<Value>,
}

/// `initialize`'s parameters, as far as the server reads them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    /// The revision the client speaks.
    #[serde(default)]
    pub protocol_version: Option<String>,
}

/// `initialize`'s result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    /// The revision the server speaks with this client.
    pub protocol_version: String,
    /// What it offers.
    pub capabilities: ServerCapabilities,
    /// Its name and version.
    pub server_info: Implementation,
}

/// What the server offers: tools.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerCapabilities {
    /// Its tools.
    pub tools: ToolsCapability,
}

/// The tools capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCapability {
    /// The list can change (the server announces it).
    pub list_changed: bool,
}

impl Default for ToolsCapability {
    fn default() -> Self {
        Self { list_changed: true }
    }
}

/// A name and a version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Implementation {
    /// The name.
    pub name: String,
    /// The version.
    pub version: String,
}

/// `tools/list`'s result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolsList {
    /// Every tool.
    pub tools: Vec<ToolInfo>,
}

/// One tool, as `tools/list` describes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    /// Its name, which `tools/call` names.
    pub name: String,
    /// What the agent reads about it.
    pub description: String,
    /// The JSON Schema of its arguments (an object).
    pub input_schema: Value,
}

/// `tools/call`'s parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallToolParams {
    /// The tool.
    pub name: String,
    /// Its arguments; none is `{}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Map<String, Value>>,
}

/// `tools/call`'s result: what the tool said, and whether it is an error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallToolResult {
    /// What it said.
    pub content: Vec<Content>,
    /// `true` when the call failed; left out when it did not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

/// One piece of a tool's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Content {
    /// Text.
    Text {
        /// The text.
        text: String,
    },
    /// A picture.
    #[serde(rename_all = "camelCase")]
    Image {
        /// Its bytes, in base64.
        data: String,
        /// Its type (`image/png`).
        mime_type: String,
    },
}

impl Content {
    /// Text content.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    /// The text, when this is text.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
            Self::Image { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_keep_their_key_order() {
        let ok = RpcResponse::result(Value::from(1), Value::Object(Map::new()));
        assert_eq!(
            serde_json::to_string(&ok).unwrap(),
            r#"{"result":{},"jsonrpc":"2.0","id":1}"#
        );
        let error = RpcResponse::error(Value::from("a"), codes::METHOD_NOT_FOUND, "nope");
        assert_eq!(
            serde_json::to_string(&error).unwrap(),
            r#"{"jsonrpc":"2.0","id":"a","error":{"code":-32601,"message":"nope"}}"#
        );
    }

    #[test]
    fn content_is_tagged_by_its_type() {
        let text = serde_json::to_value(Content::text("hi")).unwrap();
        assert_eq!(text, serde_json::json!({ "type": "text", "text": "hi" }));
        let image = Content::Image {
            data: "AA==".to_owned(),
            mime_type: "image/png".to_owned(),
        };
        assert_eq!(
            serde_json::to_value(&image).unwrap(),
            serde_json::json!({ "type": "image", "data": "AA==", "mimeType": "image/png" })
        );
        assert_eq!(image.as_text(), None);
    }
}
