//! `GET /api/live?room=`: what the assistant is doing in a room, as server-sent events.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::http::HeaderName;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use emberhut_api::approvals::{APPROVALS_EVENT, ApprovalsEvent};
use emberhut_api::error::ErrorBody;
use emberhut_api::live::{Live, LiveEvent, PING_INTERVAL_SECS, RoomQuery};
use futures_util::Stream;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::app::AppState;
use crate::error::ApiError;
use crate::extract::{RoomParam, Viewer};

/// Asks proxies (nginx) not to buffer the stream.
const NO_BUFFERING: (HeaderName, &str) = (HeaderName::from_static("x-accel-buffering"), "no");

/// The room's live stream: the room as it is now, then each change, each event's data a
/// `LiveEvent` (a `Live` object while the assistant works, `null` when its turn ends); an
/// `approvals` event (`ApprovalsEvent`) when an approval request changes; a `: ping` comment
/// every 15 s on a quiet stream. Open until the client leaves or the server stops.
#[utoipa::path(
    get,
    path = "/api/live",
    tag = "live",
    security(("bearer" = [])),
    params(RoomQuery),
    responses(
        (
            status = 200,
            description = "A text/event-stream whose every unnamed event's data is a LiveEvent: \
                a Live object while the assistant works, null when its turn has ended; an event \
                named approvals carries an ApprovalsEvent when an approval request changes",
            content_type = "text/event-stream",
            body = LiveEvent
        ),
        (
            status = 401,
            description = "Sign-in is enforced and there is no session",
            body = ErrorBody
        ),
    )
)]
pub(super) async fn live(
    State(app): State<AppState>,
    Viewer(viewer): Viewer,
    RoomParam(asked): RoomParam,
) -> Result<impl IntoResponse, ApiError> {
    let room = app.state().resolve(asked).await?;
    let events = futures_util::stream::select(
        events(app.live().subscribe(room), app.stopping().clone()),
        // Approvals are a signed-in device's to hear of, never report mode's pass.
        approval_events(
            viewer.map(|_| app.approvals().events().subscribe()),
            app.stopping().clone(),
        ),
    );
    let keep_alive = KeepAlive::new()
        .interval(Duration::from_secs(PING_INTERVAL_SECS))
        .text("ping");
    Ok(([NO_BUFFERING], Sse::new(events).keep_alive(keep_alive)))
}

/// The receiver's current value, then each change, until the channel closes or `stopping` is
/// cancelled.
pub(super) fn events(
    receiver: watch::Receiver<Option<Live>>,
    stopping: CancellationToken,
) -> impl Stream<Item = Result<Event, Infallible>> {
    futures_util::stream::unfold(
        (receiver, stopping, true),
        |(mut receiver, stopping, first)| async move {
            if !first {
                tokio::select! {
                    changed = receiver.changed() => changed.ok()?,
                    () = stopping.cancelled() => return None,
                }
            }
            let event = LiveEvent::from(receiver.borrow_and_update().clone());
            Some((Ok(data(&event)), (receiver, stopping, false)))
        },
    )
}

/// Each approvals event sent from now on, as an `approvals` event, until the channel closes or
/// `stopping` is cancelled; none without a receiver.
fn approval_events(
    receiver: Option<watch::Receiver<Option<ApprovalsEvent>>>,
    stopping: CancellationToken,
) -> impl Stream<Item = Result<Event, Infallible>> {
    futures_util::stream::unfold((receiver, stopping), |(receiver, stopping)| async move {
        let mut receiver = receiver?;
        loop {
            tokio::select! {
                changed = receiver.changed() => changed.ok()?,
                () = stopping.cancelled() => return None,
            }
            let Some(event) = receiver.borrow_and_update().clone() else {
                continue;
            };
            // A struct of numbers and words: it always serialises, on one line.
            let json = serde_json::to_string(&event).unwrap_or_else(|_| "null".to_owned());
            let event = Event::default().event(APPROVALS_EVENT).data(json);
            return Some((Ok(event), (Some(receiver), stopping)));
        }
    })
}

/// The `data:` line of `event`: its JSON on one line.
fn data(event: &LiveEvent) -> Event {
    // A `LiveEvent` is a struct of strings or null: it always serialises, and compact JSON
    // escapes every newline, which an SSE data line may not hold.
    let json = serde_json::to_string(event).unwrap_or_else(|_| "null".to_owned());
    Event::default().data(json)
}
