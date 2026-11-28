//! `POST /mcp/worker`: the MCP endpoint a worker's session calls, with its task's token (its
//! tools act on its own task); `POST /ultra/gate`: the hook a worker runs before each shell
//! command (and an ultra run's lead before each new sub-agent), with the same token. Any other
//! method on the endpoint is 405.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::extract::rejection::BytesRejection;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use emberhut_api::error::ErrorBody;
use emberhut_api::mcp::{RpcRequest, RpcResponse, codes};
use emberhut_api::ultra::GateAnswer;
use emberhut_core::RoomId;
use emberhut_core::task::TaskId;
use serde_json::Value;

use crate::app::AppState;
use crate::mcp::tools::{PLAN_TOOLS, ToolContext, ToolSet, agent_worker_tools, worker_tools};
use crate::mcp::{self, Answer};
use crate::services::agents::scope::TaskOwner;

/// The token in `Authorization: Bearer <token>`.
fn bearer(request: &HeaderMap) -> Option<&str> {
    let value = request.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.trim())
        .filter(|token| !token.is_empty())
}

fn unknown() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorBody::new("unknown caller")),
    )
        .into_response()
}

/// One MCP message (JSON-RPC 2.0) from a worker's session: `initialize`, `ping`,
/// `tools/list` or `tools/call`. The bearer is its task's worker token.
#[utoipa::path(
    post,
    path = "/mcp/worker",
    tag = "mcp",
    security(("bearer" = [])),
    request_body = RpcRequest,
    responses(
        (
            status = 200,
            description = "The request's answer: its result, or a JSON-RPC error (an unknown \
                method). A tool call always has a result, `isError` when it failed",
            body = RpcResponse
        ),
        (status = 202, description = "A notification, taken; no body"),
        (
            status = 400,
            description = "Not JSON, or not one JSON-RPC message (a JSON-RPC error)",
            body = RpcResponse
        ),
        (
            status = 413,
            description = "The body is over the server's limit (a JSON-RPC error)",
            body = RpcResponse
        ),
        (
            status = 401,
            description = "No worker token, or not one the server issued",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn mcp_worker(
    State(app): State<AppState>,
    request: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let Some(task) = bearer(&request).and_then(|token| app.worker_tokens().task_of(token)) else {
        return unknown();
    };
    let body = match body {
        Ok(body) => body,
        Err(rejection) => {
            let status = rejection.status();
            let reply = RpcResponse::error(
                Value::Null,
                codes::INVALID_REQUEST,
                format!("Invalid Request: {}", rejection.body_text()),
            );
            return (status, Json(reply)).into_response();
        }
    };
    let ultra = app.tasks().status_of(task).await.is_ok()
        && app
            .tasks()
            .detail(task)
            .await
            .ok()
            .flatten()
            .is_some_and(|d| d.task.ultra);
    let (room, tools) = worker_tools_of(&app, task, ultra).await;
    let tools = if app.task_plans() {
        tools
    } else {
        tools.without(&PLAN_TOOLS)
    };
    let ctx = ToolContext {
        app,
        room,
        worker: Some(task),
        session: None,
        runner: false,
    };
    match mcp::answer(&body, &tools, &ctx).await {
        Answer::Reply(reply) => (StatusCode::OK, Json(reply)).into_response(),
        Answer::Unreadable(reply) => (StatusCode::BAD_REQUEST, Json(reply)).into_response(),
        Answer::Accepted => StatusCode::ACCEPTED.into_response(),
    }
}

/// The room a worker on `task` acts in and its tools: a configured agent's task's worker acts
/// for that agent, in its room, with an agent worker's tools; the owner's, the owner's worker's,
/// from Main. A task no agent owns now (or one whose owner can't be read) gets no tools: it
/// never becomes the owner's.
async fn worker_tools_of(app: &AppState, task: TaskId, ultra: bool) -> (RoomId, ToolSet) {
    match app.guard().task_owner(task).await {
        Ok(TaskOwner::Owner) => (RoomId::MAIN, worker_tools(ultra)),
        Ok(TaskOwner::Agent(agent)) => (agent.room, agent_worker_tools(ultra)),
        Ok(TaskOwner::Orphaned) => (RoomId::MAIN, ToolSet::new(Vec::new())),
        Err(error) => {
            tracing::error!(%error, "could not read whose a task is");
            (RoomId::MAIN, ToolSet::new(Vec::new()))
        }
    }
}

/// The gate: a worker's hook input (`tool_name`, `tool_input`, `tool_use_id`, `cwd`) before a
/// shell command, or an ultra run's lead's before a new sub-agent (`Agent`, `Task`,
/// `Workflow`). An empty object lets it run; a `PreToolUse` refusal holds it, with why (an
/// outward step the owner's words don't cover, a plan not cleared, a cap). The bearer is its
/// task's worker token.
#[utoipa::path(
    post,
    path = "/ultra/gate",
    tag = "mcp",
    security(("bearer" = [])),
    request_body(content = Object, description = "The CLI's PreToolUse hook input"),
    responses(
        (status = 200, description = "Let through (`{}`) or held, with why", body = GateAnswer),
        (status = 400, description = "Not a JSON object", body = ErrorBody),
        (status = 401, description = "No worker token, or not one the server issued", body = ErrorBody),
    )
)]
pub(super) async fn ultra_gate(
    State(app): State<AppState>,
    request: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let Some(task) = bearer(&request).and_then(|token| app.worker_tokens().task_of(token)) else {
        return unknown();
    };
    let hook: Value = match body.ok().and_then(|b| serde_json::from_slice(&b).ok()) {
        Some(hook @ Value::Object(_)) => hook,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody::new("a hook's input, a JSON object")),
            )
                .into_response();
        }
    };
    let answer = match app.workers().gate(task, &hook).await {
        Ok(None) => GateAnswer::allow(),
        Ok(Some(why)) => GateAnswer::deny(why),
        Err(error) => {
            tracing::warn!(%error, "the gate failed");
            GateAnswer::deny("The server couldn't check this step; wait a minute and try again.")
        }
    };
    (StatusCode::OK, Json(answer)).into_response()
}
