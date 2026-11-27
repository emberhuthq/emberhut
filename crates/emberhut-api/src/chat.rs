//! A room's chat: its messages, their files and references, and pages of history.
//!
//! The server keeps every room's chat and only adds to it; message ids grow with time. A client
//! holds what it has seen and joins each page it fetches into that ([`join`]).

use serde::{Deserialize, Serialize};

use crate::ids::{MessageId, RoomId, TaskId};
use crate::open_enum::open_enum;
use crate::serde_util::is_false;
use crate::time::Timestamp;

/// The most messages one page of history (and the state's `chat`) holds: the newest of the range
/// asked for.
pub const PAGE_SIZE: usize = 60;

open_enum! {
    /// What kind of file a chat file is, from its name.
    pub enum FileKind {
        /// A picture: shown in the bubble.
        Image = "image",
        /// A video: plays in place.
        Video = "video",
        /// An HTML report (files the assistant sends).
        Html = "html",
        /// Anything else.
        File = "file",
    }
}

/// A file in the chat: one the person attached (stored by `POST /api/uploads`) or one sent to
/// them. Send it back whole with a `TalkRequest`: the server only takes files whose
/// `path` is in its uploads folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct ChatFile {
    /// Its name, as shown.
    #[cfg_attr(feature = "validate", garde(custom(crate::rules::not_blank)))]
    pub name: String,
    /// Where to fetch it, relative to the server (`/uploads/...`); behind the same sign-in.
    #[cfg_attr(feature = "validate", garde(custom(crate::rules::not_blank)))]
    pub url: String,
    /// Picture, video or other.
    #[cfg_attr(feature = "validate", garde(skip))]
    pub kind: FileKind,
    /// Where it is on the server. Required when sent back, and never with `..` in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "validate",
        garde(required, custom(crate::rules::no_parent_segments))
    )]
    pub path: Option<String>,
}

impl ChatFile {
    /// Whether it is an HTML report: by its kind, or by its name for files stored as plain files.
    #[must_use]
    pub fn is_html(&self) -> bool {
        let name = self.name.to_ascii_lowercase();
        self.kind == FileKind::Html
            || std::path::Path::new(&name)
                .extension()
                .is_some_and(|ext| ext == "html" || ext == "htm")
    }
}

open_enum! {
    /// What a chip under an answer points at.
    pub enum RefKind {
        /// A task.
        Task = "task",
        /// A pull request.
        Pr = "pr",
        /// A report (an HTML file or page).
        Report = "report",
        /// A card on a board.
        Card = "card",
        /// A ticket in a tracker.
        Ticket = "ticket",
        /// A file.
        File = "file",
    }
}

/// What an answer is about, as a chip under it: a task, a pull request, a report, a card or a
/// ticket. Only the fields of its kind are sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MessageRef {
    /// What it points at.
    pub kind: RefKind,
    /// The chip's text: a task's key, `PR #3` or `Report`.
    pub label: String,
    /// The task it is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskId>,
    /// A longer name (the task's title, the repository).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Where it opens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// A file's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// The message a reply quotes, as its bubble shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Quote {
    /// The quoted message.
    pub id: MessageId,
    /// Whether the person wrote it.
    pub mine: bool,
    /// When it was said.
    pub at: Option<Timestamp>,
    /// Its first 160 characters or so.
    pub text: String,
    /// Its first picture or video.
    pub file: Option<ChatFile>,
}

/// One chat message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChatMessage {
    /// Its id: it keeps it whatever happens around it.
    pub id: MessageId,
    /// The person wrote it (otherwise the assistant did, or it is an event).
    pub mine: bool,
    /// The server's own line (an event, a check-in), not a conversation turn. Sent only when
    /// true.
    #[serde(default, skip_serializing_if = "is_false")]
    pub event: bool,
    /// The text (Markdown).
    pub text: String,
    /// How long ago, short (`4m`, `3h`), as of when the server answered.
    pub time: String,
    /// When it was said.
    pub at: Timestamp,
    /// What the person attached.
    #[serde(default)]
    pub files: Vec<ChatFile>,
    /// The message this one replies to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<Quote>,
    /// What an answer is about. Sent only when there are some.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<MessageRef>,
}

/// `GET /api/history`'s answer: up to 60 messages (`PAGE_SIZE`), oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChatPage {
    /// The messages, oldest first.
    pub chat: Vec<ChatMessage>,
}

impl ChatPage {
    /// Whether the page is full: there may be more messages beyond it.
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.chat.len() >= PAGE_SIZE
    }
}

/// `GET /api/history`'s query: a page of a room's chat.
///
/// With neither bound it is the newest page. `before` gives the page before that message
/// (older ones, for "Show earlier"); `after` the newest page of what came after it (to catch
/// up). A full page after `after` can leave a gap behind it: `chat::join` handles that.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema, utoipa::IntoParams), into_params(parameter_in = Query))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct HistoryQuery {
    /// The room; none: Main.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub room: Option<RoomId>,
    /// Only messages older than this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub before: Option<MessageId>,
    /// Only messages newer than this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub after: Option<MessageId>,
}

impl HistoryQuery {
    /// The newest page of a room.
    #[must_use]
    pub fn latest(room: RoomId) -> Self {
        Self {
            room: Some(room),
            before: None,
            after: None,
        }
    }

    /// The page before a message.
    #[must_use]
    pub fn before(room: RoomId, message: MessageId) -> Self {
        Self {
            room: Some(room),
            before: Some(message),
            after: None,
        }
    }

    /// The newest page of what came after a message.
    #[must_use]
    pub fn after(room: RoomId, message: MessageId) -> Self {
        Self {
            room: Some(room),
            before: None,
            after: Some(message),
        }
    }

    /// The query string, without the `?`: `room=2&before=40`.
    #[must_use]
    pub fn to_query(&self) -> String {
        [
            ("room", self.room.map(RoomId::get)),
            ("before", self.before.map(MessageId::get)),
            ("after", self.after.map(MessageId::get)),
        ]
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| format!("{key}={value}")))
        .collect::<Vec<_>>()
        .join("&")
    }
}

/// A page of a room's chat joined into what a client holds of it, oldest first.
///
/// The page's copies win. What is held before the page stays only when nothing can be missing
/// between the two: they overlap, or the page isn't full (so it reaches back to what is held).
/// After a full page with a gap, the page stands alone. What is held after the page stays: the
/// server's chat only grows, so those came since.
///
/// ```
/// use emberhut_api::chat::{join, ChatMessage};
/// # use emberhut_api::ids::MessageId;
/// # fn message(id: i64) -> ChatMessage {
/// #     serde_json::from_value(serde_json::json!({
/// #         "id": id, "mine": false, "text": "", "time": "now", "at": "2026-11-24T10:00:00.000Z", "files": []
/// #     })).unwrap()
/// # }
/// let held = vec![message(1), message(2), message(3)];
/// let page = vec![message(3), message(4)];
/// let ids: Vec<i64> = join(&held, &page).iter().map(|m| m.id.get()).collect();
/// assert_eq!(ids, [1, 2, 3, 4]);
/// ```
#[must_use]
pub fn join(held: &[ChatMessage], page: &[ChatMessage]) -> Vec<ChatMessage> {
    let (Some(first), Some(last)) = (page.first().map(|m| m.id), page.last().map(|m| m.id)) else {
        return held.to_vec();
    };
    let touches = page.len() < PAGE_SIZE || held.iter().any(|m| m.id >= first);
    let before = held.iter().filter(|m| touches && m.id < first);
    let after = held.iter().filter(|m| m.id > last);
    before.chain(page).chain(after).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: i64) -> ChatMessage {
        ChatMessage {
            id: MessageId::new(id),
            mine: false,
            event: false,
            text: format!("message {id}"),
            time: "now".to_owned(),
            at: Timestamp::new("2026-11-24T10:00:00.000Z"),
            files: Vec::new(),
            reply: None,
            refs: Vec::new(),
        }
    }

    fn ids(messages: &[ChatMessage]) -> Vec<i64> {
        messages.iter().map(|m| m.id.get()).collect()
    }

    fn messages(ids: impl IntoIterator<Item = i64>) -> Vec<ChatMessage> {
        ids.into_iter().map(message).collect()
    }

    #[test]
    fn an_empty_page_leaves_what_is_held() {
        assert_eq!(ids(&join(&messages([1, 2]), &[])), [1, 2]);
    }

    #[test]
    fn the_page_wins_where_they_overlap() {
        let held = messages([1, 2, 3]);
        let mut page = messages([2, 3, 4]);
        page[0].text = "edited".to_owned();
        let joined = join(&held, &page);
        assert_eq!(ids(&joined), [1, 2, 3, 4]);
        assert_eq!(joined[1].text, "edited");
    }

    #[test]
    fn a_short_page_reaches_back_to_what_is_held() {
        assert_eq!(
            ids(&join(&messages([1, 2]), &messages([5, 6]))),
            [1, 2, 5, 6]
        );
    }

    #[test]
    fn a_full_page_after_a_gap_stands_alone() {
        let page = messages(100..100 + 60);
        assert_eq!(ids(&join(&messages([1, 2]), &page)), ids(&page));
    }

    #[test]
    fn what_came_after_the_page_stays() {
        assert_eq!(
            ids(&join(&messages([5, 6, 9]), &messages([1, 2, 5]))),
            [1, 2, 5, 6, 9]
        );
    }

    #[test]
    fn html_files_by_kind_or_name() {
        let file = |name: &str, kind: &str| ChatFile {
            name: name.to_owned(),
            url: "/uploads/x".to_owned(),
            kind: FileKind::from(kind),
            path: None,
        };
        assert!(file("report.HTML", "file").is_html());
        assert!(file("notes.htm", "file").is_html());
        assert!(file("anything", "html").is_html());
        assert!(!file("photo.jpg", "image").is_html());
    }

    #[test]
    fn history_queries() {
        let room = RoomId::new(2);
        assert_eq!(HistoryQuery::latest(room).to_query(), "room=2");
        assert_eq!(
            HistoryQuery::before(room, MessageId::new(40)).to_query(),
            "room=2&before=40"
        );
        assert_eq!(
            HistoryQuery::after(room, MessageId::new(7)).to_query(),
            "room=2&after=7"
        );
        assert_eq!(HistoryQuery::default().to_query(), "");
    }

    #[test]
    fn page_fullness() {
        assert!(
            !ChatPage {
                chat: messages([1])
            }
            .is_full()
        );
        assert!(
            ChatPage {
                chat: messages(1..=60)
            }
            .is_full()
        );
    }
}
