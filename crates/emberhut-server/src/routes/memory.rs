//! `GET /api/facts` (the "About you" page: the facts and the assistant's rules), what "Forget
//! everywhere" would take out with a fact (`GET /api/facts/forget`), everything kept as one zip
//! (`GET /api/memory/export`), and the quick notes, `GET /api/notes` and `POST /api/notes`.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use emberhut_api::error::ErrorBody;
use emberhut_api::facts::{
    FactsPage, ForgetPreview, Lesson as ApiLesson, RecordPiece, Trial as TrialRule,
};
use emberhut_api::ids::{LessonId, TrialId};
use emberhut_api::notes::{Note, NoteAuthor as NoteWho, NoteSave, NoteSaved, NotesPage};
use emberhut_api::time::Day;
use emberhut_core::memory::learning::{Lesson, Trial};
use emberhut_core::memory::notes::{Note as CoreNote, NoteAuthor};
use serde_json::Value;

use crate::app::AppState;
use crate::convert::timestamp_to_api;
use crate::error::ApiError;
use crate::extract::{JsonBody, SignedIn, Viewer};
use crate::services::facts::{FactError, fact_to_api};
use crate::services::notes::NoteError;

/// Every fact, most recently changed first, with the standing rules and the rules on trial.
#[utoipa::path(
    get,
    path = "/api/facts",
    tag = "memory",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The facts and rules", body = FactsPage),
        (
            status = 401,
            description = "Sign-in is enforced and there is no session",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn facts(
    State(app): State<AppState>,
    _viewer: Viewer,
) -> Result<Json<FactsPage>, ApiError> {
    let facts = app.facts().facts(None).await.map_err(fact_fault)?;
    let learning = app.learning();
    let lessons = learning.lessons().await.map_err(db_fault)?;
    let trials = learning.trials().await.map_err(db_fault)?;
    Ok(Json(FactsPage {
        facts: facts.into_iter().map(fact_to_api).collect(),
        lessons: lessons.into_iter().map(lesson_to_api).collect(),
        trials: trials.into_iter().map(trial_to_api).collect(),
    }))
}

/// `GET /api/facts/forget`'s query.
#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(super) struct ForgetQuery {
    /// The fact's id (`health/blood-test`).
    id: String,
}

/// What "Forget everywhere" would take out with a fact: the fact, and the pieces of the record
/// (chat, sessions, voice notes, notes, tasks) that say it. Nothing is changed; `factForget` (or
/// `factDrop`, for every piece listed) forgets.
#[utoipa::path(
    get,
    path = "/api/facts/forget",
    tag = "memory",
    params(ForgetQuery),
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The fact and the pieces that say it", body = ForgetPreview),
        (status = 400, description = "There is no such fact", body = ErrorBody),
        (
            status = 401,
            description = "Sign-in is enforced and there is no session",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn forget_preview(
    State(app): State<AppState>,
    _viewer: Viewer,
    Query(query): Query<ForgetQuery>,
) -> Result<Response, ApiError> {
    let facts = app.facts().facts(None).await.map_err(fact_fault)?;
    let Some(fact) = facts.into_iter().find(|fact| fact.id == query.id) else {
        return Ok(refused(format!("no fact {}", query.id)));
    };
    let pieces = match app.recall().saying(&fact.id).await {
        Ok(pieces) => pieces,
        Err(crate::services::recall::RecallError::Facts(FactError::Refused(why))) => {
            return Ok(refused(why));
        }
        Err(error) => {
            return Err(ApiError::State(
                crate::services::state::StateError::section(error),
            ));
        }
    };
    Ok(Json(ForgetPreview {
        fact: fact_to_api(fact),
        pieces: pieces
            .into_iter()
            .map(|doc| RecordPiece {
                id: doc.id,
                source: doc.source,
                title: doc.title,
                text: emberhut_core::memory::cut(&doc.text, 600).to_owned(),
                place: doc.place,
                at: doc.at.map(timestamp_to_api),
            })
            .collect(),
    })
    .into_response())
}

/// A 400 `{ ok: false, error }`.
fn refused(error: String) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorBody {
            ok: Some(false),
            ..ErrorBody::new(error)
        }),
    )
        .into_response()
}

/// Everything the assistant keeps about the person, as one zip: `context/` (one markdown file per
/// fact, the profile, the tombstones of what was forgotten) and `memory.json` (the quick notes and
/// diary, the corrections, the standing rules and the rules on trial). Only a signed-in device
/// gets it, in either auth mode.
#[utoipa::path(
    get,
    path = "/api/memory/export",
    tag = "memory",
    security(("bearer" = [])),
    responses(
        (
            status = 200,
            description = "The zip",
            content_type = "application/zip",
            body = Vec<u8>,
            headers(("content-disposition" = String, description = "attachment, with the \
                file's name: emberhut-memory-<UTC day>.zip"))
        ),
        (
            status = 401,
            description = "No signed-in device (in either auth mode)",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn export(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
) -> Result<Response, ApiError> {
    device.map_err(|error| match error {
        ApiError::DeviceRequired => ApiError::SignInRequired,
        other => other,
    })?;
    let learning = app.learning();
    let memory = serde_json::json!({
        "notes": app.notes().list(None, None).await.map_err(db_fault)?
            .into_iter().map(note_to_api).collect::<Vec<_>>(),
        "corrections": learning.corrections(36_500).await.map_err(db_fault)?
            .into_iter()
            .map(|correction| serde_json::json!({
                "id": correction.id,
                "at": emberhut_core::memory::iso(correction.at),
                "said": correction.said,
                "wrong": correction.wrong,
                "lesson": correction.lesson,
                "lessonToOwner": correction.lesson_to_owner,
                "source": correction.source,
            }))
            .collect::<Vec<_>>(),
        "lessons": learning.lessons().await.map_err(db_fault)?
            .into_iter().map(lesson_to_api).collect::<Vec<_>>(),
        "trials": learning.trials().await.map_err(db_fault)?
            .into_iter().map(trial_to_api).collect::<Vec<_>>(),
    });
    let folder = app.facts().folder().to_path_buf();
    let zip = tokio::task::spawn_blocking(move || crate::services::export::zip(&folder, &memory))
        .await
        .map_err(|_| ApiError::Internal)?
        .map_err(|error| {
            tracing::error!(%error, "the memory export failed");
            ApiError::Internal
        })?;
    let day =
        emberhut_core::memory::cut(&emberhut_core::memory::iso(app.facts().now()), 10).to_owned();
    let disposition = format!("attachment; filename=\"emberhut-memory-{day}.zip\"");
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/zip")
        .header(header::CACHE_CONTROL, "no-store")
        .header(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&disposition).map_err(|_| ApiError::Internal)?,
        )
        .body(axum::body::Body::from(zip))
        .map_err(|_| ApiError::Internal)
}

/// Every quick note and diary entry, newest first.
#[utoipa::path(
    get,
    path = "/api/notes",
    tag = "memory",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The notes", body = NotesPage),
        (
            status = 401,
            description = "Sign-in is enforced and there is no session",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn notes(
    State(app): State<AppState>,
    _viewer: Viewer,
) -> Result<Json<NotesPage>, ApiError> {
    let notes = app.notes().list(None, None).await.map_err(db_fault)?;
    Ok(Json(NotesPage {
        notes: notes.into_iter().map(note_to_api).collect(),
    }))
}

/// Saves a quick note (new, or an edit by its `uid`); an empty `text` deletes it.
#[utoipa::path(
    post,
    path = "/api/notes",
    tag = "memory",
    security(("bearer" = [])),
    request_body = NoteSave,
    responses(
        (status = 200, description = "Saved (`note`), or deleted (`note: null`)", body = NoteSaved),
        (
            status = 400,
            description = "A note the diary doesn't take, or a body that isn't JSON \
                (`{ ok: false, error }`)",
            body = ErrorBody
        ),
        (status = 413, description = "The body is over the server's limit", body = ErrorBody),
        (status = 401, description = "No signed-in device (in either auth mode)", body = ErrorBody),
    )
)]
pub(super) async fn save_note(
    State(app): State<AppState>,
    device: Result<SignedIn, ApiError>,
    JsonBody(body): JsonBody,
) -> Result<Json<NoteSaved>, NoteFailure> {
    // Saving or deleting a note changes the diary: a signed-in device only.
    SignedIn::acting(device).map_err(NoteFailure::Api)?;
    // Read leniently: a field of the wrong kind is as if left out.
    let text = |key: &str| body.get(key).and_then(Value::as_str);
    let note = app
        .notes()
        .save(
            text("uid").unwrap_or_default(),
            text("text").unwrap_or_default(),
            text("at"),
        )
        .await?;
    Ok(Json(NoteSaved {
        ok: true,
        note: note.map(note_to_api),
    }))
}

/// A note not saved: a refusal (400, `{ ok: false, error }`) or the server's failure.
#[derive(Debug)]
pub(super) enum NoteFailure {
    Refused(String),
    Api(ApiError),
}

impl From<NoteError> for NoteFailure {
    fn from(error: NoteError) -> Self {
        match error {
            NoteError::Refused(refusal) => Self::Refused(refusal.to_string()),
            NoteError::Database(error) => Self::Api(db_fault(error)),
        }
    }
}

impl IntoResponse for NoteFailure {
    fn into_response(self) -> Response {
        match self {
            Self::Refused(error) => (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody {
                    ok: Some(false),
                    ..ErrorBody::new(error)
                }),
            )
                .into_response(),
            Self::Api(error) => error.into_response(),
        }
    }
}

/// A database failure, answered as the server's fault (500).
fn db_fault(error: emberhut_db::DbError) -> ApiError {
    ApiError::State(error.into())
}

/// A failure reading the facts, answered as the server's fault (500).
fn fact_fault(error: FactError) -> ApiError {
    ApiError::State(crate::services::state::StateError::section(error))
}

fn lesson_to_api(lesson: Lesson) -> ApiLesson {
    ApiLesson {
        id: LessonId::new(lesson.id),
        text: lesson.text,
        at: timestamp_to_api(lesson.at),
    }
}

fn trial_to_api(trial: Trial) -> TrialRule {
    TrialRule {
        id: TrialId::new(trial.id),
        text: trial.text,
        at: timestamp_to_api(trial.at),
        ends: timestamp_to_api(trial.ends),
        said: trial.said,
        lesson: trial.lesson_to_owner,
    }
}

fn note_to_api(note: CoreNote) -> Note {
    Note {
        uid: emberhut_api::ids::NoteUid::new(note.uid),
        who: match note.who {
            NoteAuthor::Owner => NoteWho::Person,
            NoteAuthor::Assistant => NoteWho::Assistant,
        },
        day: Day::new(note.day),
        at: timestamp_to_api(note.at),
        text: note.text,
        created: timestamp_to_api(note.created),
        updated: timestamp_to_api(note.updated),
    }
}
