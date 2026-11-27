//! The reply as it is being written: `GET /api/live?room=`, a stream of server-sent events.
//!
//! The stream is `text/event-stream`. Each event is one line, `data: <json>`, then a blank line,
//! and its JSON is a [`LiveEvent`]: a [`Live`] object while the assistant works
//! ([`LiveEvent::Working`]), and `null` when nothing is being written ([`LiveEvent::Idle`]: its
//! turn has ended and the answer is in the chat now, so fetch the state). The first event, sent
//! at once, is the room as it is. A comment line `: ping` comes every [`PING_INTERVAL_SECS`]
//! seconds so proxies keep a quiet stream open. The events carry no `event:` or `id:` field.

use serde::{Deserialize, Serialize};

use crate::ids::RoomId;

/// How often the server writes a `: ping` comment on a quiet stream, in seconds.
pub const PING_INTERVAL_SECS: u64 = 15;

/// What the assistant is doing while it works in a room.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Live {
    /// The answer so far. May stay empty until the final answer reaches the chat.
    pub text: String,
    /// Its current step, one plain line; none at the very start.
    pub doing: Option<String>,
}

/// One event of the live stream: the JSON of its `data:` line.
///
/// | `data:`                          | event                       |
/// |----------------------------------|-----------------------------|
/// | `{"text":"","doing":"Reading"}`  | `Working(Live { .. })`      |
/// | `{"text":"","doing":null}`       | `Working(Live { doing: None, .. })`: started, no step yet |
/// | `null`                           | `Idle`                      |
///
/// ```
/// use emberhut_api::live::LiveEvent;
///
/// let idle: LiveEvent = serde_json::from_str("null").unwrap();
/// assert_eq!(idle, LiveEvent::Idle);
/// let working: LiveEvent = serde_json::from_str(r#"{"text":"","doing":"Reading"}"#).unwrap();
/// assert_eq!(working.doing(), Some("Reading"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Option<Live>", into = "Option<Live>")]
pub enum LiveEvent {
    /// The assistant is working: what it is doing.
    Working(Live),
    /// Nothing is being written: the turn has ended (the answer is in the chat) or none runs.
    Idle,
}

impl LiveEvent {
    /// What the assistant is doing, while it works.
    #[must_use]
    pub fn live(&self) -> Option<&Live> {
        match self {
            Self::Working(live) => Some(live),
            Self::Idle => None,
        }
    }

    /// Its current step, while it works and has one.
    #[must_use]
    pub fn doing(&self) -> Option<&str> {
        self.live().and_then(|live| live.doing.as_deref())
    }
}

impl From<Option<Live>> for LiveEvent {
    fn from(live: Option<Live>) -> Self {
        live.map_or(Self::Idle, Self::Working)
    }
}

impl From<LiveEvent> for Option<Live> {
    fn from(event: LiveEvent) -> Self {
        match event {
            LiveEvent::Working(live) => Some(live),
            LiveEvent::Idle => None,
        }
    }
}

#[cfg(feature = "openapi")]
impl utoipa::PartialSchema for LiveEvent {
    /// A [`Live`] object, or `null`.
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        <Option<Live> as utoipa::PartialSchema>::schema()
    }
}

#[cfg(feature = "openapi")]
impl utoipa::ToSchema for LiveEvent {
    fn schemas(
        schemas: &mut Vec<(
            String,
            utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>,
        )>,
    ) {
        schemas.push((
            <Live as utoipa::ToSchema>::name().into_owned(),
            <Live as utoipa::PartialSchema>::schema(),
        ));
        <Live as utoipa::ToSchema>::schemas(schemas);
    }
}

/// The query of `GET /api/state` and `GET /api/live`: the room.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema, utoipa::IntoParams), into_params(parameter_in = Query))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct RoomQuery {
    /// The room; none, or one the server doesn't know: Main.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub room: Option<RoomId>,
}

impl RoomQuery {
    /// The query for a room.
    #[must_use]
    pub fn new(room: RoomId) -> Self {
        Self { room: Some(room) }
    }

    /// The query string, without the `?`: `room=2`, or empty for Main by default.
    #[must_use]
    pub fn to_query(&self) -> String {
        self.room
            .map_or_else(String::new, |room| format!("room={room}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_convert_to_and_from_options() {
        let live = Live {
            text: String::new(),
            doing: Some("Reading".to_owned()),
        };
        assert_eq!(
            LiveEvent::from(Some(live.clone())),
            LiveEvent::Working(live.clone())
        );
        assert_eq!(
            Option::<Live>::from(LiveEvent::Working(live.clone())),
            Some(live)
        );
        assert_eq!(Option::<Live>::from(LiveEvent::Idle), None);
        assert_eq!(LiveEvent::Idle.doing(), None);
        assert_eq!(LiveEvent::Idle.live(), None);
    }

    #[test]
    fn room_queries() {
        assert_eq!(RoomQuery::new(RoomId::new(3)).to_query(), "room=3");
        assert_eq!(RoomQuery::default().to_query(), "");
    }
}
