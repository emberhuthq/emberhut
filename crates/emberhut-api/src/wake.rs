//! The assistant's own appointments and its activity, as the state carries them: its
//! check-ins (`wake_me_at`), its watches, the activity log (`did`), how fresh each source it
//! reads is (`awareness`); and `POST /api/events`, where a program tells it something.
//!
//! Field names, the fields left out when empty and the ones sent as `null` are fixed here, so
//! every app reads them alike.

use serde::{Deserialize, Serialize};

use crate::ids::RoomId;
use crate::time::Timestamp;

/// A check-in the assistant scheduled: at `at` it is woken with `why`, in the room that asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Checkin {
    /// When.
    pub at: Timestamp,
    /// Why: what to look at then.
    pub why: String,
    /// The room that asked; left out for Main.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<RoomId>,
}

/// Something the assistant keeps an eye on for a while: every `every` minutes it is reminded
/// to look, and it keeps `note` current with what it saw.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Watch {
    /// Its number.
    pub id: i64,
    /// A few plain words for the app.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The coding thread it watches, if one: then it reminds only after that thread changed.
    #[serde(default)]
    pub thread: Option<String>,
    /// What to look at, and what counts as news.
    pub what: String,
    /// Minutes between reminders.
    pub every: f64,
    /// When it ends by itself.
    pub until: Timestamp,
    /// The next reminder.
    pub next_at: Timestamp,
    /// What it last saw.
    #[serde(default)]
    pub note: String,
    /// When that was noted.
    #[serde(default)]
    pub note_at: Option<Timestamp>,
    /// Looks that failed in a row.
    #[serde(default)]
    pub fails: u32,
    /// The room that started it; left out for Main.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<RoomId>,
    /// When it last reminded the assistant to look; left out before the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_at: Option<Timestamp>,
}

/// One line of the assistant's recent activity, newest first in the state's `did`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DidLine {
    /// How long ago, as the chat says it (`5m`, `2h`).
    pub time: String,
    /// When.
    pub at: Timestamp,
    /// What was done.
    pub text: String,
}

/// How fresh one source the assistant reads is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AwarenessSource {
    /// Its id.
    pub id: String,
    /// Its name, for the app.
    pub name: String,
    /// What it brings, in a few words.
    pub detail: String,
    /// When it last came in; `null` when never.
    pub at: Option<Timestamp>,
    /// Why it can't be reached now, if it can't.
    pub error: Option<String>,
    /// `ok`, `stale` (nothing new for too long) or `down` (an error).
    pub state: SourceState,
}

/// How a source is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum SourceState {
    /// Fresh.
    Ok,
    /// Nothing new for longer than it should be.
    Stale,
    /// It can't be reached.
    Down,
}

/// `POST /api/events`: a program (a script when a long job ends, a webhook) tells the
/// assistant something. It lands in the project's room, Main when none or one the server
/// doesn't know.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct EventRequest {
    /// What to tell it; required: 1 to 100 000 characters. A number is read as its digits; an
    /// empty string, zero or `null` is no text.
    #[serde(default, deserialize_with = "crate::serde_util::loose_text")]
    #[cfg_attr(feature = "validate", garde(length(chars, min = 1, max = 100_000)))]
    pub text: Option<String>,
    /// What kind of news it is: at most 32 of `a-z`, `0-9` and `_`; `note` when left out or
    /// empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(custom(crate::rules::event_kind)))]
    pub kind: Option<String>,
    /// The project (its room's name, or its id) the news is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub project: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_checkin_leaves_main_out_and_a_watch_sends_its_nulls() {
        let checkin = Checkin {
            at: Timestamp::new("2026-11-26T10:00:00.000Z"),
            why: "the build".to_owned(),
            room: None,
        };
        assert_eq!(
            serde_json::to_value(&checkin).unwrap(),
            json!({ "at": "2026-11-26T10:00:00.000Z", "why": "the build" })
        );
        let watch = Watch {
            id: 1,
            title: None,
            thread: None,
            what: "the deploy".to_owned(),
            every: 30.0,
            until: Timestamp::new("2026-11-27T10:00:00.000Z"),
            next_at: Timestamp::new("2026-11-26T10:30:00.000Z"),
            note: String::new(),
            note_at: None,
            fails: 0,
            room: Some(RoomId::new(2)),
            check_at: None,
        };
        assert_eq!(
            serde_json::to_value(&watch).unwrap(),
            json!({ "id": 1, "thread": null, "what": "the deploy", "every": 30.0,
                "until": "2026-11-27T10:00:00.000Z", "nextAt": "2026-11-26T10:30:00.000Z",
                "note": "", "noteAt": null, "fails": 0, "room": 2 })
        );
        let back: Watch = serde_json::from_value(serde_json::to_value(&watch).unwrap()).unwrap();
        assert_eq!(back, watch);
    }

    #[test]
    fn an_event_reads_a_project_by_name_or_number() {
        let named: EventRequest =
            serde_json::from_value(json!({ "text": "done", "project": "Atlas" })).unwrap();
        assert_eq!(named.project, Some(json!("Atlas")));
        let empty: EventRequest = serde_json::from_value(json!({})).unwrap();
        assert_eq!(empty, EventRequest::default());
        for (text, read) in [
            (json!(42), Some("42")),
            (json!(""), None),
            (json!(0), None),
            (json!(null), None),
            (json!(true), None),
        ] {
            let event: EventRequest = serde_json::from_value(json!({ "text": text })).unwrap();
            assert_eq!(event.text.as_deref(), read, "{text}");
        }
        assert_eq!(
            serde_json::to_value(SourceState::Stale).unwrap(),
            json!("stale")
        );
    }
}
