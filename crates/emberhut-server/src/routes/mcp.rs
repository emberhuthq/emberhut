//! `POST` [`emberhut_api::paths::MCP_ROOM`]: the MCP endpoint a room's agent session calls, with the
//! room's token. Any other method is 405.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::extract::rejection::BytesRejection;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use emberhut_api::error::ErrorBody;
use emberhut_api::mcp::{RpcRequest, RpcResponse, codes};

use crate::app::AppState;
use crate::mcp::tools::{ToolContext, ToolSet, agent_tools, family_tools, room_tools};
use crate::mcp::{self, Answer};
use crate::services::family::{FAMILY_ROOMS, NOT_AVAILABLE};

/// One MCP message (JSON-RPC 2.0) from a room's agent session: `initialize`, `ping`,
/// `tools/list` or `tools/call`. The bearer is the room's agent token, not a device's.
#[utoipa::path(
    post,
    // `emberhut_api::wire::mcp::ROOM_PATH`; the macro takes only a literal path, and a test
    // checks the two agree.
    path = "/mcp/emberhut",
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
            description = "No agent token, or not one the server issued",
            body = ErrorBody
        ),
        (
            status = 501,
            description = "The session is in a family member's room, or whose room it is can't \
                be told: family rooms are not available yet",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn mcp_room(
    State(app): State<AppState>,
    request: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let Some(room) = bearer(&request).and_then(|token| app.agent_tokens().room_of(token)) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody::new("unknown caller")),
        )
            .into_response();
    };
    let body = match body {
        Ok(body) => body,
        Err(rejection) => {
            let status = rejection.status();
            let reply = RpcResponse::error(
                serde_json::Value::Null,
                codes::INVALID_REQUEST,
                format!("Invalid Request: {}", rejection.body_text()),
            );
            return (status, Json(reply)).into_response();
        }
    };
    // A family member's room gets their short list; when it can't be told whose room it is,
    // the short list too.
    let family = match app.family().store().person_of(room).await {
        Ok(person) => person.is_some(),
        Err(error) => {
            tracing::error!(%room, %error, "could not read whose room an agent session is in");
            true
        }
    };
    let tools = match (family, FAMILY_ROOMS) {
        (false, _) => tools_of(&app, room).await,
        (true, true) => family_tools(),
        // Their tools reach the whole installation: none until they are scoped to the room.
        (true, false) => {
            return (
                StatusCode::NOT_IMPLEMENTED,
                Json(ErrorBody::new(NOT_AVAILABLE)),
            )
                .into_response();
        }
    };
    let ctx = ToolContext {
        app,
        room,
        worker: None,
    };
    match mcp::answer(&body, &tools, &ctx).await {
        Answer::Reply(reply) => (StatusCode::OK, Json(reply)).into_response(),
        Answer::Unreadable(reply) => (StatusCode::BAD_REQUEST, Json(reply)).into_response(),
        Answer::Accepted => StatusCode::ACCEPTED.into_response(),
    }
}

/// The tools `room`'s session gets: the default agent's room tools, or a configured agent's
/// own set. When whose room it is can't be read, none.
async fn tools_of(app: &AppState, room: emberhut_core::RoomId) -> ToolSet {
    let store = app.agents().store();
    let agent = match store.by_room(room).await {
        Ok(agent) => agent,
        Err(error) => {
            tracing::error!(%room, %error, "could not read whose agent a room is");
            return ToolSet::new(Vec::new());
        }
    };
    if agent.id.is_main() {
        return room_tools();
    }
    match store.tools(&agent.id).await {
        Ok(listed) => agent_tools(&agent, &listed),
        Err(error) => {
            tracing::error!(%room, %error, "could not read an agent's tools");
            agent_tools(&agent, &[])
        }
    }
}

/// Any other method on the endpoint: 405 `{ error: "POST only" }`, before the caller is looked
/// at.
pub(super) async fn post_only() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [(header::ALLOW, "POST")],
        Json(ErrorBody::new("POST only")),
    )
        .into_response()
}

/// The token in `Authorization: Bearer <token>` (the prefix in any case).
pub(super) fn bearer(request: &HeaderMap) -> Option<&str> {
    let value = request.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.trim())
        .filter(|token| !token.is_empty())
}
