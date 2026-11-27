//! Agents, approvers and trusted requesters, from a signed-in device only (in either auth
//! mode): `GET`/`POST /api/agents`, `GET`/`PATCH`/`DELETE /api/agents/{id}`,
//! `PUT /api/agents/{id}/channels|tools|accounts|policies`, `GET`/`POST /api/approvers`,
//! `PATCH`/`DELETE /api/approvers/{id}`, `GET`/`PUT /api/trusted-requesters`.
//!
//! No agent's token reaches these: an agent never writes agents, policies, approvers or
//! trusted requesters.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use emberhut_api::agents::{
    AccountsRequest, AgentChange, AgentPage, AgentsPage, Approver, ApproverChange, ApproversPage,
    ChannelsRequest, NewAgent, PoliciesRequest, ToolsRequest, TrustedRequestersPage,
};
use emberhut_api::error::ErrorBody;

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::{Invalid, SignedIn, Valid};
use crate::services::agents::AgentServiceError;
use crate::services::approvals::ApprovalError;

/// Why an agents' or approvals' request failed: its status and `{ error }`, or the API's own
/// answer (no session, a body the route doesn't take). A server fault is logged and answered
/// with a fixed sentence.
#[derive(Debug)]
pub(super) enum Failure {
    /// The API's own refusal.
    Api(ApiError),
    /// The agents' service.
    Agents(AgentServiceError),
    /// The approvals' service.
    Approvals(ApprovalError),
}

impl From<ApiError> for Failure {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

impl From<Invalid> for Failure {
    fn from(invalid: Invalid) -> Self {
        Self::Api(ApiError::Invalid(invalid))
    }
}

impl From<AgentServiceError> for Failure {
    fn from(error: AgentServiceError) -> Self {
        Self::Agents(error)
    }
}

impl From<ApprovalError> for Failure {
    fn from(error: ApprovalError) -> Self {
        Self::Approvals(error)
    }
}

impl From<emberhut_db::DbError> for Failure {
    fn from(error: emberhut_db::DbError) -> Self {
        Self::Agents(AgentServiceError::Db(error))
    }
}

/// The status an approvals' error answers with.
pub(super) fn approval_status(error: &ApprovalError) -> StatusCode {
    match error {
        ApprovalError::NoAgent(_) | ApprovalError::NotFound => StatusCode::NOT_FOUND,
        ApprovalError::File(_)
        | ApprovalError::Invalid(_)
        | ApprovalError::OverCap(_)
        | ApprovalError::Malformed(_) => StatusCode::BAD_REQUEST,
        ApprovalError::Unproven(_) => StatusCode::UNAUTHORIZED,
        ApprovalError::NotYours(_) | ApprovalError::Disabled(_) => StatusCode::FORBIDDEN,
        ApprovalError::Mismatch | ApprovalError::Closed => StatusCode::CONFLICT,
        ApprovalError::Gone => StatusCode::GONE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let (status, error) = match self {
            Self::Api(error) => return error.into_response(),
            Self::Agents(error) => (
                match &error {
                    AgentServiceError::Refused(_) => StatusCode::BAD_REQUEST,
                    AgentServiceError::NotFound(_) => StatusCode::NOT_FOUND,
                    AgentServiceError::Conflict(_) => StatusCode::CONFLICT,
                    _ => StatusCode::INTERNAL_SERVER_ERROR,
                },
                error.to_string(),
            ),
            Self::Approvals(error) => (approval_status(&error), error.to_string()),
        };
        if status.is_server_error() {
            tracing::error!(%error, "an agents or approvals request failed");
            return (
                status,
                Json(ErrorBody::new("Something went wrong on the server.")),
            )
                .into_response();
        }
        (status, Json(ErrorBody::new(error))).into_response()
    }
}

/// The signed-in device acting, then the body: a request without a session is refused before
/// its body is read.
fn signed_in_with<T>(
    device: Result<SignedIn, ApiError>,
    body: Result<Valid<T>, Invalid>,
) -> Result<T, Failure> {
    SignedIn::acting(device)?;
    let Valid(body) = body?;
    Ok(body)
}

/// Every agent, the default agent first, with how many of its requests wait on an answer.
#[utoipa::path(
    get,
    path = "/api/agents",
    tag = "agents",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Every agent", body = AgentsPage),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
    )
)]
pub(super) async fn list(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
) -> Result<Json<AgentsPage>, Failure> {
    SignedIn::acting(device)?;
    Ok(Json(app.agents().list().await?))
}

/// Makes an agent, in the room it names or in a new room of its own.
#[utoipa::path(
    post,
    path = "/api/agents",
    tag = "agents",
    security(("bearer" = [])),
    request_body = NewAgent,
    responses(
        (status = 200, description = "The agent, with its (empty) lists", body = AgentPage),
        (status = 400, description = "Not an agent: a field breaks a rule, Main as its room, a room or project that isn't there", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 409, description = "The id is taken, or the room is another agent's", body = ErrorBody),
    )
)]
pub(super) async fn create(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    body: Result<Valid<NewAgent>, Invalid>,
) -> Result<Json<AgentPage>, Failure> {
    let body = signed_in_with(device, body)?;
    Ok(Json(app.agents().create(&body).await?))
}

/// An agent with its channels, tools, accounts and policies.
#[utoipa::path(
    get,
    path = "/api/agents/{id}",
    tag = "agents",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The agent's id (`main`: the default agent)")),
    responses(
        (status = 200, description = "The agent and its lists", body = AgentPage),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "No such agent", body = ErrorBody),
    )
)]
pub(super) async fn page(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Path(id): Path<String>,
) -> Result<Json<AgentPage>, Failure> {
    SignedIn::acting(device)?;
    Ok(Json(app.agents().page(&id).await?))
}

/// Changes an agent: its name, persona, principal, project, cap, or whether it is enabled.
/// The default agent keeps its name, persona and principal.
#[utoipa::path(
    patch,
    path = "/api/agents/{id}",
    tag = "agents",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The agent's id")),
    request_body = AgentChange,
    responses(
        (status = 200, description = "The agent as it is now", body = AgentPage),
        (status = 400, description = "A change the rules refuse", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "No such agent", body = ErrorBody),
    )
)]
pub(super) async fn change(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Path(id): Path<String>,
    body: Result<Valid<AgentChange>, Invalid>,
) -> Result<Json<AgentPage>, Failure> {
    let body = signed_in_with(device, body)?;
    Ok(Json(app.agents().change(&id, &body).await?))
}

/// Deletes an agent, what is configured around it and its empty room. The default agent is
/// never deleted, and an agent with approval history, tasks or a conversation in its room is
/// disabled instead (nothing of it becomes the owner's).
#[utoipa::path(
    delete,
    path = "/api/agents/{id}",
    tag = "agents",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The agent's id")),
    responses(
        (status = 200, description = "The agents left", body = AgentsPage),
        (status = 400, description = "The default agent", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "No such agent", body = ErrorBody),
        (status = 409, description = "The agent has approval history, tasks or a conversation: disable it instead", body = ErrorBody),
    )
)]
pub(super) async fn delete(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Path(id): Path<String>,
) -> Result<Json<AgentsPage>, Failure> {
    SignedIn::acting(device)?;
    Ok(Json(app.agents().delete(&id).await?))
}

/// Replaces an agent's channels, in one transaction.
#[utoipa::path(
    put,
    path = "/api/agents/{id}/channels",
    tag = "agents",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The agent's id")),
    request_body = ChannelsRequest,
    responses(
        (status = 200, description = "The agent as it is now", body = AgentPage),
        (status = 400, description = "A list the rules refuse", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "No such agent", body = ErrorBody),
    )
)]
pub(super) async fn channels(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Path(id): Path<String>,
    body: Result<Valid<ChannelsRequest>, Invalid>,
) -> Result<Json<AgentPage>, Failure> {
    let body = signed_in_with(device, body)?;
    Ok(Json(app.agents().replace_channels(&id, &body).await?))
}

/// Replaces an agent's tools, in one transaction. A tool that acts as the owner is refused for
/// any agent but the default, whose tools are never narrowed.
#[utoipa::path(
    put,
    path = "/api/agents/{id}/tools",
    tag = "agents",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The agent's id")),
    request_body = ToolsRequest,
    responses(
        (status = 200, description = "The agent as it is now", body = AgentPage),
        (status = 400, description = "A list the rules refuse", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "No such agent", body = ErrorBody),
    )
)]
pub(super) async fn tools(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Path(id): Path<String>,
    body: Result<Valid<ToolsRequest>, Invalid>,
) -> Result<Json<AgentPage>, Failure> {
    let body = signed_in_with(device, body)?;
    Ok(Json(app.agents().replace_tools(&id, &body).await?))
}

/// Replaces an agent's accounts, in one transaction.
#[utoipa::path(
    put,
    path = "/api/agents/{id}/accounts",
    tag = "agents",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The agent's id")),
    request_body = AccountsRequest,
    responses(
        (status = 200, description = "The agent as it is now", body = AgentPage),
        (status = 400, description = "A list the rules refuse", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "No such agent", body = ErrorBody),
    )
)]
pub(super) async fn accounts(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Path(id): Path<String>,
    body: Result<Valid<AccountsRequest>, Invalid>,
) -> Result<Json<AgentPage>, Failure> {
    let body = signed_in_with(device, body)?;
    Ok(Json(app.agents().replace_accounts(&id, &body).await?))
}

/// Replaces an agent's policies, in one transaction. A fixed-core kind (payment, credential,
/// secret, account security, as the owner) given to anyone but the owner is refused.
#[utoipa::path(
    put,
    path = "/api/agents/{id}/policies",
    tag = "agents",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The agent's id")),
    request_body = PoliciesRequest,
    responses(
        (status = 200, description = "The agent as it is now", body = AgentPage),
        (status = 400, description = "A list the fixed core or the rules refuse", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "No such agent", body = ErrorBody),
    )
)]
pub(super) async fn policies(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Path(id): Path<String>,
    body: Result<Valid<PoliciesRequest>, Invalid>,
) -> Result<Json<AgentPage>, Failure> {
    let body = signed_in_with(device, body)?;
    Ok(Json(app.agents().replace_policies(&id, &body).await?))
}

/// Every external approver, with how each is reached.
#[utoipa::path(
    get,
    path = "/api/approvers",
    tag = "agents",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Every approver", body = ApproversPage),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
    )
)]
pub(super) async fn approvers(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
) -> Result<Json<ApproversPage>, Failure> {
    SignedIn::acting(device)?;
    Ok(Json(app.agents().approvers().await?))
}

/// Adds an external approver.
#[utoipa::path(
    post,
    path = "/api/approvers",
    tag = "agents",
    security(("bearer" = [])),
    request_body = Approver,
    responses(
        (status = 200, description = "The approver", body = Approver),
        (status = 400, description = "Not an approver: no way to reach them, a bad Slack id or URL", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 409, description = "The id is taken", body = ErrorBody),
    )
)]
pub(super) async fn create_approver(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    body: Result<Valid<Approver>, Invalid>,
) -> Result<Json<Approver>, Failure> {
    let body = signed_in_with(device, body)?;
    Ok(Json(app.agents().create_approver(&body).await?))
}

/// Changes an external approver.
#[utoipa::path(
    patch,
    path = "/api/approvers/{id}",
    tag = "agents",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The approver's id")),
    request_body = ApproverChange,
    responses(
        (status = 200, description = "The approver as it is now", body = Approver),
        (status = 400, description = "A change that leaves no approver", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "No such approver", body = ErrorBody),
    )
)]
pub(super) async fn change_approver(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Path(id): Path<String>,
    body: Result<Valid<ApproverChange>, Invalid>,
) -> Result<Json<Approver>, Failure> {
    let body = signed_in_with(device, body)?;
    Ok(Json(app.agents().change_approver(&id, &body).await?))
}

/// Deletes an external approver no policy names.
#[utoipa::path(
    delete,
    path = "/api/approvers/{id}",
    tag = "agents",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "The approver's id")),
    responses(
        (status = 200, description = "The approvers left", body = ApproversPage),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "No such approver", body = ErrorBody),
        (status = 409, description = "A policy names the approver", body = ErrorBody),
    )
)]
pub(super) async fn delete_approver(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Path(id): Path<String>,
) -> Result<Json<ApproversPage>, Failure> {
    SignedIn::acting(device)?;
    Ok(Json(app.agents().delete_approver(&id).await?))
}

/// The trusted requesters.
#[utoipa::path(
    get,
    path = "/api/trusted-requesters",
    tag = "agents",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The trusted requesters", body = TrustedRequestersPage),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
    )
)]
pub(super) async fn trusted_requesters(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
) -> Result<Json<TrustedRequestersPage>, Failure> {
    SignedIn::acting(device)?;
    Ok(Json(app.agents().trusted_requesters().await?))
}

/// Replaces the trusted requesters, in one transaction.
#[utoipa::path(
    put,
    path = "/api/trusted-requesters",
    tag = "agents",
    security(("bearer" = [])),
    request_body = TrustedRequestersPage,
    responses(
        (status = 200, description = "The trusted requesters as they are now", body = TrustedRequestersPage),
        (status = 400, description = "A list the rules refuse, or naming an agent that isn't there", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
    )
)]
pub(super) async fn replace_trusted_requesters(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    body: Result<Valid<TrustedRequestersPage>, Invalid>,
) -> Result<Json<TrustedRequestersPage>, Failure> {
    let body = signed_in_with(device, body)?;
    Ok(Json(app.agents().replace_trusted_requesters(&body).await?))
}
