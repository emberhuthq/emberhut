//! The server's MCP endpoint (`POST` [`emberhut_api::paths::MCP_ROOM`]): the tools a room's agent
//! session calls.
//!
//! The Model Context Protocol on its Streamable HTTP transport, stateless, as the reference
//! server serves it: each request is one JSON-RPC message and gets its answer in the same HTTP
//! response, as `application/json` (a notification, or a client's answer, gets 202 and no
//! body). There is no session to open first, and no stream: `initialize` is answered for the
//! client's sake, and every request stands alone.
//!
//! - [`tokens`]: the room tokens a session calls with; the route checks them.
//! - `tools`: the `Tool` trait and the one list of the room's tools.
//! - here: reading a message and answering it.

use emberhut_api::mcp::{
    CallToolParams, CallToolResult, Implementation, InitializeParams, InitializeResult,
    PROTOCOL_VERSIONS, RpcRequest, RpcResponse, SERVER_NAME, SERVER_VERSION, ServerCapabilities,
    ToolInfo, ToolsList, codes, methods,
};
use serde_json::{Map, Value};

pub mod tokens;
pub(crate) mod tools;
pub mod worker_tokens;

use crate::error::chain;
use tools::{ToolContext, ToolError, ToolOutput, ToolSet};

/// The answer to one HTTP request to the endpoint.
#[derive(Debug)]
pub(crate) enum Answer {
    /// The request's answer (200).
    Reply(RpcResponse),
    /// The body is not a request the endpoint can read (400).
    Unreadable(RpcResponse),
    /// A notification or a client's answer: taken, nothing to say (202).
    Accepted,
}

/// The answer to `body`, one JSON-RPC message, from `room`'s session, with `tools`.
pub(crate) async fn answer(body: &[u8], tools: &ToolSet, ctx: &ToolContext) -> Answer {
    let message: Value = match serde_json::from_slice(body) {
        Ok(message) => message,
        Err(error) => {
            return Answer::Unreadable(RpcResponse::error(
                Value::Null,
                codes::PARSE_ERROR,
                format!("Parse error: {error}"),
            ));
        }
    };
    let Value::Object(object) = &message else {
        return Answer::Unreadable(RpcResponse::error(
            Value::Null,
            codes::INVALID_REQUEST,
            "Invalid Request: one JSON-RPC message, an object (batches are not taken)",
        ));
    };
    if !object.contains_key("method")
        && (object.contains_key("result") || object.contains_key("error"))
    {
        // The client answering something: the server never asks, so there is nothing to match.
        return Answer::Accepted;
    }
    let id = object.get("id").cloned().unwrap_or(Value::Null);
    let request = match serde_json::from_value::<RpcRequest>(message) {
        Ok(request) if request.jsonrpc == emberhut_api::mcp::JSONRPC => request,
        Ok(_) | Err(_) => {
            return Answer::Unreadable(RpcResponse::error(
                id,
                codes::INVALID_REQUEST,
                "Invalid Request: a JSON-RPC 2.0 message with a method",
            ));
        }
    };
    let Some(id) = request.id else {
        return Answer::Accepted;
    };
    Answer::Reply(respond(id, &request.method, request.params, tools, ctx).await)
}

async fn respond(
    id: Value,
    method: &str,
    params: Option<Value>,
    tools: &ToolSet,
    ctx: &ToolContext,
) -> RpcResponse {
    let result = match method {
        methods::INITIALIZE => to_value(&initialize(params)),
        methods::PING => Value::Object(Map::new()),
        methods::TOOLS_LIST => to_value(&list(tools)),
        methods::TOOLS_CALL => {
            let Some(params) = params.and_then(|params| serde_json::from_value(params).ok()) else {
                return RpcResponse::error(
                    id,
                    codes::INVALID_PARAMS,
                    "Invalid params: tools/call takes { name, arguments? }",
                );
            };
            to_value(&call(params, tools, ctx).await)
        }
        _ => {
            return RpcResponse::error(id, codes::METHOD_NOT_FOUND, "Method not found");
        }
    };
    RpcResponse::result(id, result)
}

/// `initialize`: the client's revision when the server speaks it, else the newest.
fn initialize(params: Option<Value>) -> InitializeResult {
    let asked = params
        .and_then(|params| serde_json::from_value::<InitializeParams>(params).ok())
        .and_then(|params| params.protocol_version);
    let version = asked
        .filter(|asked| PROTOCOL_VERSIONS.contains(&asked.as_str()))
        .unwrap_or_else(|| PROTOCOL_VERSIONS[0].to_owned());
    InitializeResult {
        protocol_version: version,
        capabilities: ServerCapabilities::default(),
        server_info: Implementation {
            name: SERVER_NAME.to_owned(),
            version: SERVER_VERSION.to_owned(),
        },
    }
}

fn list(tools: &ToolSet) -> ToolsList {
    ToolsList {
        tools: tools
            .iter()
            .map(|tool| ToolInfo {
                name: tool.name().to_owned(),
                description: tool.description().to_owned(),
                input_schema: tool.input_schema(),
            })
            .collect(),
    }
}

/// `tools/call`: every outcome is a result; an error is one with `isError`, worded the way
/// MCP libraries word it where the agent may match on it.
async fn call(params: CallToolParams, tools: &ToolSet, ctx: &ToolContext) -> CallToolResult {
    let name = params.name;
    let Some(tool) = tools.get(&name) else {
        return result(ToolOutput::error(format!(
            "MCP error -32602: Tool {name} not found"
        )));
    };
    let args = Value::Object(params.arguments.unwrap_or_default());
    let output = match tool.call(ctx, args).await {
        Ok(output) => output,
        Err(ToolError::Invalid(reason)) => ToolOutput::error(format!(
            "MCP error -32602: Input validation error: Invalid arguments for tool {name}: {reason}"
        )),
        Err(ToolError::Refused(reason)) => ToolOutput::error(reason),
        Err(ToolError::Failed(cause)) => {
            let error = chain(cause.as_ref());
            tracing::error!(tool = %name, room = %ctx.room, %error, "tool failed");
            ToolOutput::error(cause.to_string())
        }
    };
    result(output)
}

fn result(output: ToolOutput) -> CallToolResult {
    CallToolResult {
        content: output.content,
        is_error: output.is_error.then_some(true),
    }
}

/// A result as JSON. Every result type here is plain data, which always writes; were one ever
/// not to, the failure is logged and the result is `null`.
fn to_value(result: &impl serde::Serialize) -> Value {
    serde_json::to_value(result).unwrap_or_else(|error| {
        tracing::error!(%error, "an MCP result could not be written as JSON");
        Value::Null
    })
}
