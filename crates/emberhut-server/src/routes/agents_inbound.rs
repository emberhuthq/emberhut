//! `POST /api/agents/{id}/inbound`: a bridge passes on a message from one of the agent's
//! channels; `POST /api/agents/{id}/notes`: the bridge's own note about itself;
//! `POST /api/agents/{id}/slack-actions`: a click on one of the agent's approval buttons, as
//! the bridge's Slack Socket Mode connection received it. The bridge proves itself with the
//! agent's inbound bearer token (no session); the bearer is checked before the body is read.
//! No message or note is ever an approval: only a button click, decided by the approvals'
//! own checks.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{FromRequest as _, Path, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use emberhut_api::agents::{InboundMessage, InboundNote, InboundReply};
use emberhut_api::error::ErrorBody;

use super::agents::Failure;
use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::Valid;
use crate::services::agents::InboundError;
use crate::services::approvals::ApprovalError;

impl IntoResponse for InboundError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::Unproven => StatusCode::UNAUTHORIZED,
            Self::NotItsChannel(_) => StatusCode::FORBIDDEN,
            Self::NoAgent(_) => StatusCode::NOT_FOUND,
            Self::Talk(_) | Self::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        if status.is_server_error() {
            tracing::error!(error = %self, "an agent's inbound message or bridge note was not taken");
            return (status, Json(ErrorBody::new("It was not taken."))).into_response();
        }
        (status, Json(ErrorBody::new(self.to_string()))).into_response()
    }
}

/// A message from one of the agent's channels, as its bridge passes it on. A direct message,
/// a mention, or a reply in a thread the agent is in becomes a turn in its room; anything
/// else is taken and left (`turn: false`, with why).
#[utoipa::path(
    post,
    path = "/api/agents/{id}/inbound",
    tag = "agents",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The agent's id")),
    request_body = InboundMessage,
    responses(
        (status = 200, description = "Taken: a turn in the agent's room, or left with why", body = InboundReply),
        (status = 400, description = "Not a message", body = ErrorBody),
        (
            status = 401,
            description = "Not the agent's inbound token (`agent-<id>-inbound`), or no such \
                secret saved",
            body = ErrorBody
        ),
        (status = 403, description = "Not one of the agent's channels", body = ErrorBody),
        (status = 404, description = "No such configured agent (the default agent takes no bridge)", body = ErrorBody),
    )
)]
pub(super) async fn inbound(
    State(app): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let conversations = app.agent_conversations();
    // The bearer first: the body of a caller that isn't the bridge is never read.
    let agent = match conversations
        .bridge_of(&id, super::mcp::bearer(request.headers()))
        .await
    {
        Ok(agent) => agent,
        Err(error) => return error.into_response(),
    };
    let Valid(message) = match Valid::<InboundMessage>::from_request(request, &()).await {
        Ok(body) => body,
        Err(invalid) => return ApiError::Invalid(invalid).into_response(),
    };
    match conversations.inbound(&agent, &message).await {
        Ok(reply) => Json(reply).into_response(),
        Err(error) => error.into_response(),
    }
}

/// A note from the agent's bridge about the bridge itself (its connection was down and is
/// back, a button on one of the agent's messages was tapped): a turn in the agent's room,
/// framed as the bridge's, never as anyone's words in a channel and never an approval.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/notes",
    tag = "agents",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The agent's id")),
    request_body = InboundNote,
    responses(
        (status = 200, description = "Taken: a turn in the agent's room, or left with why", body = InboundReply),
        (status = 400, description = "Not a note", body = ErrorBody),
        (
            status = 401,
            description = "Not the agent's inbound token (`agent-<id>-inbound`), or no such \
                secret saved",
            body = ErrorBody
        ),
        (status = 404, description = "No such configured agent (the default agent takes no bridge)", body = ErrorBody),
    )
)]
pub(super) async fn notes(
    State(app): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let conversations = app.agent_conversations();
    // The bearer first, as for a message.
    let agent = match conversations
        .bridge_of(&id, super::mcp::bearer(request.headers()))
        .await
    {
        Ok(agent) => agent,
        Err(error) => return error.into_response(),
    };
    let Valid(note) = match Valid::<InboundNote>::from_request(request, &()).await {
        Ok(body) => body,
        Err(invalid) => return ApiError::Invalid(invalid).into_response(),
    };
    match conversations.note(&agent, &note).await {
        Ok(reply) => Json(reply).into_response(),
        Err(error) => error.into_response(),
    }
}

/// A click on one of the agent's approval buttons in Slack, passed on by its bridge: for a
/// Slack app on Socket Mode, whose clicks reach only its connection and carry no signature to
/// check. The body is Slack's `block_actions` payload as the connection received it. The click
/// decides only one of this agent's requests, and only as a signed one would: the approver's
/// own team and user, on the request's own message, on its hash.
#[utoipa::path(
    post,
    path = "/api/agents/{id}/slack-actions",
    tag = "agents",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The agent's id")),
    request_body(
        content = Object,
        description = "Slack's `block_actions` payload, as a Socket Mode envelope's `payload` holds it"
    ),
    responses(
        (status = 200, description = "Decided"),
        (status = 400, description = "Not an approval's button", body = ErrorBody),
        (
            status = 401,
            description = "Not the agent's inbound token (`agent-<id>-inbound`), or no such \
                secret saved",
            body = ErrorBody
        ),
        (status = 403, description = "The click is not the approver's, or not on the request's own message", body = ErrorBody),
        (status = 404, description = "No such request of this agent's, or no such configured agent", body = ErrorBody),
        (status = 409, description = "The hash is not the request's, or it was decided already", body = ErrorBody),
    )
)]
pub(super) async fn slack_action(
    State(app): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    // The bearer first, as for a message.
    let agent = match app
        .agent_conversations()
        .bridge_of(&id, super::mcp::bearer(request.headers()))
        .await
    {
        Ok(agent) => agent,
        Err(error) => return error.into_response(),
    };
    let Ok(body) = Bytes::from_request(request, &()).await else {
        return Failure::Approvals(ApprovalError::Malformed("not an approval's button"))
            .into_response();
    };
    let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return Failure::Approvals(ApprovalError::Malformed("not an approval's button"))
            .into_response();
    };
    match app.approvals().bridged_slack_action(&agent, &payload).await {
        Ok(_) => StatusCode::OK.into_response(),
        Err(error) => Failure::Approvals(error).into_response(),
    }
}
