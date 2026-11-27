//! `GET /api/facts` (the "About you" page: the facts and the assistant's rules) and the quick
//! notes, `GET /api/notes` and `POST /api/notes`.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use emberhut_api::error::ErrorBody;
use emberhut_api::facts::{FactsPage, Lesson as ApiLesson, Trial as TrialRule};
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
