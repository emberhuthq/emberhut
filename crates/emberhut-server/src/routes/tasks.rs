//! The tasks' routes: any task by key or words (`GET /api/tasks`), one with its page
//! (`GET /api/tasks/{id}`), the person's reply on it (`POST /api/tasks/{id}/message`), a job a
//! program on this machine hands the assistant (`POST /api/tasks`), what a task produced
//! (`GET /api/artifacts/{id}`, `/thumb`) and the Scheduled page (`GET /api/scheduled`).

use axum::Json;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use emberhut_api::error::ErrorBody;
use emberhut_api::scheduled::ScheduledPage;
use emberhut_api::task_message::{TaskMessageReply, TaskMessageRequest};
use emberhut_api::task_page::TaskPage;
use emberhut_api::task_search::FoundTasks;
use emberhut_api::tasks::{NewTaskReply, NewTaskRequest};
use emberhut_core::task::{ArtifactId, TaskId};
use serde_json::Value;
use subtle::ConstantTimeEq as _;

use super::uploads::{empty, send};
use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::{JsonBody, SignedIn, Viewer};
use crate::services::tasks::{MessageFile, TaskError, TasksQuery};
use crate::services::uploads::Served;

/// The longest reply on a task's page, in characters (a task's brief's limit).
const MESSAGE_MAX: usize = emberhut_core::task::BODY_MAX;

/// The most files a reply on a task's page takes.
const MESSAGE_FILES_MAX: usize = 20;

/// A refusal: `{ error }` with its status.
fn refusal(status: StatusCode, text: impl Into<String>) -> Response {
    (status, Json(ErrorBody::new(text))).into_response()
}

/// A task error as these routes answer it: a rule's refusal is 400 with its words, a fault is
/// logged and 500.
fn answer(error: TaskError) -> Response {
    match error {
        TaskError::Refused(text) => refusal(StatusCode::BAD_REQUEST, text),
        other => {
            tracing::error!(error = %other, "a task route failed");
            ApiError::Internal.into_response()
        }
    }
}

/// A task's number from the path: digits only.
fn task_id(path: Result<Path<String>, PathRejection>) -> Option<TaskId> {
    let Ok(Path(id)) = path else { return None };
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
        .then(|| id.parse::<i64>().ok())
        .flatten()
        .map(TaskId::new)
}

/// Any task, done and canceled ones too: by key (`q=` a key or its number) or words of its
/// title and description, then the newest; filtered by project (`none`: no project), area,
/// statuses and owner; `closed=1` only done and canceled ones, `archived=1` only canceled ones.
#[utoipa::path(
    get,
    path = "/api/tasks",
    tag = "tasks",
    security(("bearer" = [])),
    params(TasksQuery),
    responses(
        (status = 200, description = "The tasks found, at most `limit` (50; 200 at most)", body = FoundTasks),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
    )
)]
pub(super) async fn list(
    State(app): State<AppState>,
    _viewer: Viewer,
    query: Result<Query<TasksQuery>, QueryRejection>,
) -> Response {
    let query = query.map_or_default(|Query(query)| query);
    match app.tasks().search(&query).await {
        Ok(tasks) => Json(FoundTasks { tasks }).into_response(),
        Err(error) => answer(error),
    }
}

/// A program on this machine hands the assistant a job the person asked it for: a task of the
/// assistant's with its brief, the person's request. Only with the server's program key as the
/// bearer, never a device's session.
#[utoipa::path(
    post,
    path = "/api/tasks",
    tag = "tasks",
    security(("bearer" = [])),
    request_body = NewTaskRequest,
    responses(
        (status = 200, description = "The task made", body = NewTaskReply),
        (status = 400, description = "No title or no body", body = ErrorBody),
        (status = 401, description = "Not the program key", body = ErrorBody),
    )
)]
pub(super) async fn create(
    State(app): State<AppState>,
    headers: HeaderMap,
    body: Result<JsonBody, crate::extract::Invalid>,
) -> Response {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim);
    let allowed = match (app.program_key(), bearer) {
        (Some(key), Some(given)) => bool::from(key.as_bytes().ct_eq(given.as_bytes())),
        _ => false,
    };
    if !allowed {
        return refusal(StatusCode::UNAUTHORIZED, "programs use the key");
    }
    let request: NewTaskRequest = match body {
        Ok(JsonBody(value)) => {
            let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
            NewTaskRequest {
                title: text("title"),
                body: text("body"),
                from: text("from"),
            }
        }
        Err(invalid) => return ApiError::from(invalid).into_response(),
    };
    match app.tasks().from_program(&request).await {
        Ok(reply) => Json(reply).into_response(),
        Err(error) => answer(error),
    }
}

/// One task with everything its page shows: its history, subtasks, findings, links and runs.
#[utoipa::path(
    get,
    path = "/api/tasks/{id}",
    tag = "tasks",
    security(("bearer" = [])),
    params(("id" = i64, Path, description = "The task's number")),
    responses(
        (status = 200, description = "The task", body = TaskPage),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
        (status = 404, description = "No such task", body = ErrorBody),
    )
)]
pub(super) async fn detail(
    State(app): State<AppState>,
    _viewer: Viewer,
    path: Result<Path<String>, PathRejection>,
) -> Response {
    let Some(id) = task_id(path) else {
        return ApiError::NotFound.into_response();
    };
    match app.tasks().detail(id).await {
        Ok(Some(detail)) => Json(detail).into_response(),
        Ok(None) => refusal(StatusCode::NOT_FOUND, "no such task"),
        Err(error) => answer(error),
    }
}

/// The person's reply on a task's page: it goes to the task (its worker, or the assistant told
/// to answer on the task), not the chat, and clears the ask.
#[utoipa::path(
    post,
    path = "/api/tasks/{id}/message",
    tag = "tasks",
    security(("bearer" = [])),
    params(("id" = i64, Path, description = "The task's number")),
    request_body = TaskMessageRequest,
    responses(
        (status = 200, description = "Where it went", body = TaskMessageReply),
        (
            status = 400,
            description = "Nothing to send, no such task, or a reply over 20 000 characters or \
                20 files",
            body = ErrorBody
        ),
        (
            status = 401,
            description = "No signed-in device: a reply acts on the task, so it needs one in \
                either mode",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn message(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    path: Result<Path<String>, PathRejection>,
    body: Result<JsonBody, crate::extract::Invalid>,
) -> Response {
    // A reply acts on the owner's task: report mode's pass without a session reads, never acts.
    if let Err(error) = device {
        return match error {
            ApiError::DeviceRequired => ApiError::SignInRequired,
            other => other,
        }
        .into_response();
    }
    let Some(id) = task_id(path) else {
        return ApiError::NotFound.into_response();
    };
    let value = match body {
        Ok(JsonBody(value)) => value,
        Err(invalid) => return ApiError::from(invalid).into_response(),
    };
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if text.chars().count() > MESSAGE_MAX {
        return refusal(
            StatusCode::BAD_REQUEST,
            format!("A reply is at most {MESSAGE_MAX} characters."),
        );
    }
    if value
        .get("files")
        .and_then(Value::as_array)
        .is_some_and(|files| files.len() > MESSAGE_FILES_MAX)
    {
        return refusal(
            StatusCode::BAD_REQUEST,
            format!("A reply takes at most {MESSAGE_FILES_MAX} files."),
        );
    }
    let files: Vec<MessageFile> =
        value
            .get("files")
            .and_then(Value::as_array)
            .map_or_default(|files| {
                files
                    .iter()
                    .filter(|f| f.is_object())
                    .map(|f| MessageFile {
                        kind: f.get("kind").and_then(Value::as_str).map(str::to_owned),
                        path: f
                            .get("path")
                            .or_else(|| f.get("url"))
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                    })
                    .collect()
            });
    match app.tasks().message(id, text, &files).await {
        Ok(reply) => Json(reply).into_response(),
        Err(error) => answer(error),
    }
}

async fn serve(
    path: std::path::PathBuf,
    content_type: &'static str,
    headers: &HeaderMap,
) -> Response {
    let Ok(file) = tokio::fs::File::open(&path).await else {
        return empty(StatusCode::NOT_FOUND);
    };
    send_file(file, content_type, headers).await
}

/// `file`, whole or in the byte range `headers` ask for.
async fn send_file(
    file: tokio::fs::File,
    content_type: &'static str,
    headers: &HeaderMap,
) -> Response {
    let Ok(meta) = file.metadata().await else {
        return empty(StatusCode::NOT_FOUND);
    };
    let range = headers.get(header::RANGE).and_then(|r| r.to_str().ok());
    send(
        Served {
            file,
            size: meta.len(),
            content_type,
        },
        range,
    )
    .await
    .unwrap_or_else(IntoResponse::into_response)
}

fn artifact_id(path: Result<Path<String>, PathRejection>) -> Option<ArtifactId> {
    task_id(path).map(|id| ArtifactId::new(id.get()))
}

/// A file a task produced, by its artifact's id (never by its path), in byte ranges when asked
/// (videos stream and seek). A link, or a file that is gone, private or of a kind the app
/// doesn't open, is 404.
#[utoipa::path(
    get,
    path = "/api/artifacts/{id}",
    tag = "tasks",
    security(("bearer" = [])),
    params(
        ("id" = i64, Path, description = "The artifact's id"),
        ("range" = Option<String>, Header, description = "One byte range: bytes=a-b, a- or -n"),
    ),
    responses(
        (status = 200, description = "The whole file", content_type = "application/octet-stream"),
        (status = 206, description = "The bytes asked for", content_type = "application/octet-stream"),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
        (status = 404, description = "No such file"),
        (status = 416, description = "The range starts past the end of the file"),
    )
)]
pub(super) async fn artifact(
    State(app): State<AppState>,
    _viewer: Viewer,
    path: Result<Path<String>, PathRejection>,
    headers: HeaderMap,
) -> Response {
    let Some(id) = artifact_id(path) else {
        return empty(StatusCode::NOT_FOUND);
    };
    match app.tasks().artifact_file(id).await {
        Ok(Some((file, content_type))) => send_file(file, content_type, &headers).await,
        Ok(None) => empty(StatusCode::NOT_FOUND),
        Err(error) => answer(error),
    }
}

/// A small JPEG of a picture or a video a task produced, made once.
#[utoipa::path(
    get,
    path = "/api/artifacts/{id}/thumb",
    tag = "tasks",
    security(("bearer" = [])),
    params(("id" = i64, Path, description = "The artifact's id")),
    responses(
        (status = 200, description = "The thumbnail", content_type = "image/jpeg"),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
        (status = 404, description = "No such picture or video, or no thumbnail could be made"),
    )
)]
pub(super) async fn thumb(
    State(app): State<AppState>,
    _viewer: Viewer,
    path: Result<Path<String>, PathRejection>,
    headers: HeaderMap,
) -> Response {
    let Some(id) = artifact_id(path) else {
        return empty(StatusCode::NOT_FOUND);
    };
    match app.tasks().artifact_thumb(id).await {
        Ok(Some(file)) => serve(file, "image/jpeg", &headers).await,
        Ok(None) => empty(StatusCode::NOT_FOUND),
        Err(error) => answer(error),
    }
}

/// Everything with a next run, every room's, soonest first.
#[utoipa::path(
    get,
    path = "/api/scheduled",
    tag = "tasks",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The scheduled items", body = ScheduledPage),
        (status = 401, description = "Sign-in is enforced and there is no session", body = ErrorBody),
    )
)]
pub(super) async fn scheduled(State(app): State<AppState>, _viewer: Viewer) -> Response {
    match app.scheduled().list().await {
        Ok(items) => Json(ScheduledPage { items }).into_response(),
        Err(error) => {
            tracing::error!(%error, "the scheduled items could not be read");
            ApiError::Internal.into_response()
        }
    }
}
