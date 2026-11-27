//! Approval requests:
//!
//! - the owner's, from a signed-in device (in either auth mode): `GET /api/approvals`,
//!   `GET /api/approvals/{id}`, `POST /api/approvals/{id}/decision` and `/withdraw`;
//! - an agent's MCP server's, with the agent's room token: `POST /api/approvals/{id}/use`;
//! - an approver's, each proving itself: `POST /api/approvals/slack` (Slack's signature) and
//!   `POST /api/approvals/{id}/webhook` (the approver's own HMAC). The signed link's page is
//!   `routes::approve`.

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use emberhut_api::approvals::{
    ApprovalReply, ApprovalsPage, ApprovalsQuery, Decision as ApiDecision, DecisionRequest,
    UseReply, UseRequest, WithdrawRequest,
};
use emberhut_api::error::ErrorBody;
use emberhut_core::ApprovalId;
use emberhut_core::approvals::Decision;
use emberhut_core::approvals::{slack, webhook};
use serde::Deserialize;

use super::agents::Failure;
use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::{Invalid, SignedIn, Valid};
use crate::services::approvals::{ApprovalError, InboundCall, StepUse, UseOutcome};

/// The request id a path names; one that can't be an id is no request.
fn request_id(text: &str) -> Result<ApprovalId, Failure> {
    text.parse::<i64>()
        .ok()
        .filter(|id| *id > 0 && text.bytes().all(|b| b.is_ascii_digit()))
        .map(ApprovalId::new)
        .ok_or(Failure::Approvals(ApprovalError::NotFound))
}

/// A decision as the domain has it.
pub(super) fn decision(decision: ApiDecision) -> Decision {
    match decision {
        ApiDecision::Approve => Decision::Approve,
        ApiDecision::Reject => Decision::Reject,
    }
}

/// The list's query, read leniently: a word no request's state is matches none.
#[derive(Debug, Default, Deserialize)]
pub(super) struct ListQuery {
    state: Option<String>,
    agent: Option<String>,
}

/// Approval requests, newest first: what waits on the owner (`waitsOnOwner`), and what waits
/// on others or has ended, for information.
#[utoipa::path(
    get,
    path = "/api/approvals",
    tag = "approvals",
    security(("bearer" = [])),
    params(ApprovalsQuery),
    responses(
        (status = 200, description = "The requests asked for (a state or agent no request has: none)", body = ApprovalsPage),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
    )
)]
pub(super) async fn list(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    query: Result<Query<ListQuery>, QueryRejection>,
) -> Result<Json<ApprovalsPage>, Failure> {
    SignedIn::acting(device)?;
    let query = query.map_or_default(|Query(query)| query);
    Ok(Json(ApprovalsPage {
        approvals: app
            .approvals()
            .list(query.state.as_deref(), query.agent.as_deref())
            .await?,
    }))
}

/// One request, with its exact payload, its decision and its use.
#[utoipa::path(
    get,
    path = "/api/approvals/{id}",
    tag = "approvals",
    security(("bearer" = [])),
    params(("id" = i64, Path, description = "The request's number")),
    responses(
        (status = 200, description = "The request", body = ApprovalReply),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "No such request", body = ErrorBody),
    )
)]
pub(super) async fn get(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Path(id): Path<String>,
) -> Result<Json<ApprovalReply>, Failure> {
    SignedIn::acting(device)?;
    let id = request_id(&id)?;
    Ok(Json(ApprovalReply {
        approval: app.approvals().approval(id).await?,
    }))
}

/// The owner's decision on a request that waits on the owner, on the payload hash they were
/// shown, from a signed-in device. A request that waits on an external approver is theirs.
#[utoipa::path(
    post,
    path = "/api/approvals/{id}/decision",
    tag = "approvals",
    security(("bearer" = [])),
    params(("id" = i64, Path, description = "The request's number")),
    request_body = DecisionRequest,
    responses(
        (status = 200, description = "Decided: the request as it is now", body = ApprovalReply),
        (status = 400, description = "Not a decision", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode); an agent's token is not one", body = ErrorBody),
        (status = 403, description = "The request waits on an external approver", body = ErrorBody),
        (status = 404, description = "No such request", body = ErrorBody),
        (status = 409, description = "The hash is not the request's, or it was decided, withdrawn or has expired", body = ErrorBody),
    )
)]
pub(super) async fn decide(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Path(id): Path<String>,
    body: Result<Valid<DecisionRequest>, Invalid>,
) -> Result<Json<ApprovalReply>, Failure> {
    let device = SignedIn::acting(device)?;
    let Valid(body) = body?;
    let id = request_id(&id)?;
    let request = app
        .approvals()
        .decide_as_owner(
            id,
            decision(body.decision),
            &body.payload_hash,
            device,
            body.reason.as_deref(),
        )
        .await?;
    Ok(Json(ApprovalReply {
        approval: app.approvals().show(&request).await?,
    }))
}

/// Withdraws a request, pending or approved and unused, from a signed-in device.
#[utoipa::path(
    post,
    path = "/api/approvals/{id}/withdraw",
    tag = "approvals",
    security(("bearer" = [])),
    params(("id" = i64, Path, description = "The request's number")),
    request_body = WithdrawRequest,
    responses(
        (status = 200, description = "Withdrawn: the request as it is now", body = ApprovalReply),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
        (status = 404, description = "No such request", body = ErrorBody),
        (status = 409, description = "It was used, rejected, withdrawn or has expired", body = ErrorBody),
    )
)]
pub(super) async fn withdraw(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    Path(id): Path<String>,
    body: Result<Valid<WithdrawRequest>, Invalid>,
) -> Result<Json<ApprovalReply>, Failure> {
    SignedIn::acting(device)?;
    let reason = body.ok().and_then(|Valid(body)| body.reason);
    let id = request_id(&id)?;
    let request = app
        .approvals()
        .withdraw(id, None, reason.as_deref())
        .await?;
    Ok(Json(ApprovalReply {
        approval: app.approvals().show(&request).await?,
    }))
}

/// Uses an approval before a step an agent's own MCP server takes, with the agent's room
/// token: the step's exact words and files (read now) must be what was approved, the approval
/// unused and unexpired, and the agent's policy must still name its approver. Nothing goes out
/// unless `used` is true.
#[utoipa::path(
    post,
    path = "/api/approvals/{id}/use",
    tag = "approvals",
    security(("bearer" = [])),
    params(("id" = i64, Path, description = "The request's number")),
    request_body = UseRequest,
    responses(
        (status = 200, description = "Used once, or refused with the reason (`used: false`)", body = UseReply),
        (status = 400, description = "Not a step", body = ErrorBody),
        (status = 401, description = "Not an agent's room token", body = ErrorBody),
        (status = 404, description = "No such request of this agent's", body = ErrorBody),
    )
)]
pub(super) async fn use_approval(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Valid<UseRequest>, Invalid>,
) -> Result<Json<UseReply>, Failure> {
    let Some(room) =
        super::mcp::bearer(&headers).and_then(|token| app.agent_tokens().room_of(token))
    else {
        return Err(Failure::Approvals(ApprovalError::Unproven(
            "not an agent's room token",
        )));
    };
    let Valid(body) = body?;
    let id = request_id(&id)?;
    let agent = app.agents().store().by_room(room).await?;
    let step = StepUse {
        text: body.text,
        files: body.files.into_iter().map(Into::into).collect(),
        by: format!("{} (use route)", agent.id),
    };
    let reply = match app.approvals().use_for_step(&agent.id, id, &step).await? {
        UseOutcome::Used(used) => UseReply {
            used: true,
            message: app.approvals().receipt(&used),
        },
        UseOutcome::Refused(why) => UseReply {
            used: false,
            message: why,
        },
    };
    Ok(Json(reply))
}

/// A header's value as text.
fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

/// Slack's interactivity call for an approval DM's buttons: its form body's `payload`, signed
/// with the workspace's signing secret (`slack-<team>-signing`) at most five minutes before.
/// The click counts only from the approver's own team and user, on the request's hash; the DM
/// then says what was decided.
#[utoipa::path(
    post,
    path = "/api/approvals/slack",
    tag = "approvals",
    request_body(
        content = String,
        content_type = "application/x-www-form-urlencoded",
        description = "Slack's form: `payload`, the block action's JSON"
    ),
    responses(
        (status = 200, description = "Decided"),
        (status = 400, description = "Not an approval's button", body = ErrorBody),
        (status = 401, description = "Not proven by Slack's signature: none, wrong, stale, or no signing secret for the workspace", body = ErrorBody),
        (status = 403, description = "The click is not the approver's", body = ErrorBody),
        (status = 404, description = "No such request", body = ErrorBody),
        (status = 409, description = "The hash is not the request's, or it was decided already", body = ErrorBody),
    )
)]
pub(super) async fn slack_action(
    State(app): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, Failure> {
    let call = InboundCall {
        timestamp: header(&headers, slack::TIMESTAMP_HEADER),
        signature: header(&headers, slack::SIGNATURE_HEADER),
        body: &body,
    };
    app.approvals().slack_action(call).await?;
    Ok(StatusCode::OK)
}

/// An approver's own app answers a request, signed with the approver's secret
/// (`x-emberhut-timestamp`, `x-emberhut-signature: v1=<HMAC-SHA256 over "<ts>.POST.<path>.<body>">`),
/// at most five minutes old; `by` is recorded as the app reports it.
#[utoipa::path(
    post,
    path = "/api/approvals/{id}/webhook",
    tag = "approvals",
    params(("id" = i64, Path, description = "The request's number")),
    request_body = emberhut_api::approvals::WebhookDecision,
    responses(
        (status = 200, description = "Decided: the request as it is now", body = ApprovalReply),
        (status = 400, description = "Not a decision", body = ErrorBody),
        (status = 401, description = "Not proven by the approver's signature: none, wrong or stale, or no such request (all alike)", body = ErrorBody),
        (status = 409, description = "The hash is not the request's, or it was decided already", body = ErrorBody),
    )
)]
pub(super) async fn webhook_answer(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<ApprovalReply>, Failure> {
    let call = InboundCall {
        timestamp: header(&headers, webhook::TIMESTAMP_HEADER),
        signature: header(&headers, webhook::SIGNATURE_HEADER),
        body: &body,
    };
    let path = format!("{}/{id}/webhook", emberhut_api::paths::APPROVALS);
    let request = app.approvals().webhook_decision(&id, &path, call).await?;
    Ok(Json(ApprovalReply {
        approval: app.approvals().show(&request).await?,
    }))
}
