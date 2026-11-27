//! `POST /api/agents/{id}/inbound`: a bridge passes on a message from one of the agent's
//! channels. It proves itself with the agent's inbound bearer token (no session); the bearer
//! is checked before the body is read. Nothing that arrives here is ever an approval.

use axum::Json;
use axum::extract::{FromRequest as _, Path, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use emberhut_api::agents::{InboundMessage, InboundReply};
use emberhut_api::error::ErrorBody;

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::Valid;
use crate::services::agents::InboundError;

impl IntoResponse for InboundError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::Unproven => StatusCode::UNAUTHORIZED,
            Self::NotItsChannel(_) => StatusCode::FORBIDDEN,
            Self::NoAgent(_) => StatusCode::NOT_FOUND,
            Self::Talk(_) | Self::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        if status.is_server_error() {
            tracing::error!(error = %self, "an agent's inbound message was not taken");
            return (status, Json(ErrorBody::new("The message was not taken."))).into_response();
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
