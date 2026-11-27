//! The app's links: `emberhut://…`, from the system (a link clicked anywhere, a script's
//! `open`), the command line, or another window. Each names a place in the app ([`Target`]):
//!
//! | Link | Opens |
//! | --- | --- |
//! | `emberhut://chat` | the main window, brought forward |
//! | `emberhut://chat?room=N` | room N's chat |
//! | `emberhut://chat?text=…` | the main window, with a message started in its chat (percent-encoded) |
//! | `emberhut://task/N` | task N |
//! | `emberhut://needs`, `tasks`, `machines`, `settings`, `scheduled`, `approvals` | that page, beside the chat |
//! | `emberhut://approval/N` | approval request N, beside the chat |
//! | `emberhut://browser`, `calendar`, `sessions`, `dark`, `diary`, `party`, `note` | that window |
//! | `emberhut://party/N` | party N, in the Party window |
//!
//! Parsing is all this module does; [`crate::shell`] takes the app there.

use emberhut_api::ids::{ApprovalId, RoomId, TaskId};

use crate::prose::links::task_id;
use crate::shell::WindowKind;
use crate::store::Page;

/// The app's own link scheme.
pub(crate) const SCHEME: &str = "emberhut";

/// The most characters a link's message (`emberhut://chat?text=…`) puts in the composer; the rest
/// is cut, and the cut logged.
pub(crate) const TALK_LIMIT: usize = 10_000;

/// A place in the app a link names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// The main window; in that room's chat when one is named.
    Chat(Option<RoomId>),
    /// The main window, with this message started in its chat's composer.
    Talk(String),
    /// A page, in the column beside the chat.
    Page(Page),
    /// A task.
    Task(TaskId),
    /// A window of its own.
    Window(WindowKind),
    /// One party, in the Party window.
    Party(i64),
}

/// Why a link was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum LinkError {
    /// Not one of the app's schemes.
    #[error("`{0}` isn't an emberhut:// link")]
    Scheme(String),
    /// The app's scheme, but no place it knows.
    #[error("`{0}` names no place in the app")]
    Place(String),
}

/// Reads `link` as a place in the app.
///
/// # Errors
///
/// [`LinkError::Scheme`] when it isn't one of the app's links, [`LinkError::Place`] when it
/// names nothing the app has.
pub(crate) fn parse(link: &str) -> Result<Target, LinkError> {
    let place = || LinkError::Place(link.to_owned());
    let rest = strip_scheme(link.trim()).ok_or_else(|| LinkError::Scheme(link.to_owned()))?;
    // No fragment is used; drop it.
    let rest = rest.split_once('#').map_or(rest, |(rest, _)| rest);
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut segments = path.split('/').filter(|segment| !segment.is_empty());
    let host = segments.next().ok_or_else(place)?.to_ascii_lowercase();
    let item = segments.next();
    if segments.next().is_some() {
        return Err(place());
    }
    let number = |text: &str| text.parse::<i64>().ok().filter(|id| *id > 0);
    match (host.as_str(), item) {
        ("chat", None) => {
            if let Some(text) = query_value(query, "text") {
                let text = percent_decode(text).ok_or_else(place)?;
                return Ok(Target::Talk(capped(text)));
            }
            let room = query_value(query, "room").map(|room| number(room).ok_or_else(place));
            Ok(Target::Chat(room.transpose()?.map(RoomId::new)))
        }
        ("task", Some(id)) => Ok(Target::Task(task_id(id).ok_or_else(place)?)),
        ("party", Some(id)) => Ok(Target::Party(number(id).ok_or_else(place)?)),
        ("approval", Some(id)) => Ok(Target::Page(Page::Approval(ApprovalId::new(
            number(id).ok_or_else(place)?,
        )))),
        ("approvals", None) => Ok(Target::Page(Page::Approvals(None))),
        ("needs", None) => Ok(Target::Page(Page::Needs)),
        ("tasks", None) => Ok(Target::Page(Page::Tasks)),
        ("machines", None) => Ok(Target::Page(Page::Machines)),
        ("settings", None) => Ok(Target::Page(Page::Settings)),
        ("scheduled", None) => Ok(Target::Page(Page::Scheduled)),
        (name, None) => WindowKind::from_link_name(name)
            .map(Target::Window)
            .ok_or_else(place),
        _ => Err(place()),
    }
}

/// `link` after `emberhut://` (in any case, as schemes are).
fn strip_scheme(link: &str) -> Option<&str> {
    let (scheme, rest) = link.split_once("://")?;
    scheme.eq_ignore_ascii_case(SCHEME).then_some(rest)
}

/// `text` cut to [`TALK_LIMIT`] characters, the cut logged.
fn capped(text: String) -> String {
    match text.char_indices().nth(TALK_LIMIT) {
        Some((end, _)) => {
            log::warn!(
                "a link's message was cut to {TALK_LIMIT} characters (it had {})",
                text.chars().count()
            );
            text[..end].to_owned()
        }
        None => text,
    }
}

/// The first value of `key` in `query` (`a=1&b=2`).
fn query_value<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find_map(|(name, value)| (name == key).then_some(value))
}

/// `text` with its `%XX` escapes (and `+` for a space, as forms write it) decoded; `None` when
/// an escape is broken or the bytes aren't UTF-8.
fn percent_decode(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.bytes();
    while let Some(byte) = rest.next() {
        match byte {
            b'%' => {
                let high = char::from(rest.next()?).to_digit(16)?;
                let low = char::from(rest.next()?).to_digit(16)?;
                bytes.push(u8::try_from(high * 16 + low).ok()?);
            }
            b'+' => bytes.push(b' '),
            byte => bytes.push(byte),
        }
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_links_bring_the_app_forward_or_open_a_room() {
        assert_eq!(parse("emberhut://chat"), Ok(Target::Chat(None)));
        assert_eq!(parse("emberhut://chat/"), Ok(Target::Chat(None)));
        assert_eq!(
            parse("emberhut://chat?room=7"),
            Ok(Target::Chat(Some(RoomId::new(7))))
        );
        assert_eq!(
            parse("emberhut://chat?from=push&room=2#x"),
            Ok(Target::Chat(Some(RoomId::new(2))))
        );
        assert!(matches!(
            parse("emberhut://chat?room=seven"),
            Err(LinkError::Place(_))
        ));
        assert!(matches!(
            parse("emberhut://chat?room=0"),
            Err(LinkError::Place(_))
        ));
    }

    #[test]
    fn a_chat_link_can_start_a_message() {
        assert_eq!(
            parse("emberhut://chat?text=What%27s+next%3F%20%E2%9C%93"),
            Ok(Target::Talk("What's next? \u{2713}".to_owned()))
        );
        assert!(matches!(
            parse("emberhut://chat?text=%E2%9C"),
            Err(LinkError::Place(_))
        ));
        assert!(matches!(
            parse("emberhut://chat?text=%zz"),
            Err(LinkError::Place(_))
        ));
    }

    #[test]
    fn a_links_message_is_cut_to_the_limit() {
        let long = "a".repeat(TALK_LIMIT + 5);
        let Ok(Target::Talk(text)) = parse(&format!("emberhut://chat?text={long}")) else {
            panic!("a message link");
        };
        assert_eq!(text.chars().count(), TALK_LIMIT);
        let Ok(Target::Talk(text)) = parse("emberhut://chat?text=short") else {
            panic!("a message link");
        };
        assert_eq!(text, "short");
    }

    #[test]
    fn the_scheme_reads_in_any_case() {
        assert_eq!(parse("EMBERHUT://browser"), parse("emberhut://browser"));
        assert!(parse("Emberhut://task/119").is_ok());
        assert!(matches!(
            parse("other://task/119"),
            Err(LinkError::Scheme(_))
        ));
    }

    #[test]
    fn tasks_pages_windows_and_parties() {
        assert_eq!(
            parse("emberhut://task/119"),
            Ok(Target::Task(TaskId::new(119)))
        );
        assert_eq!(parse("emberhut://needs"), Ok(Target::Page(Page::Needs)));
        assert_eq!(
            parse("emberhut://Settings"),
            Ok(Target::Page(Page::Settings))
        );
        assert_eq!(
            parse("emberhut://scheduled"),
            Ok(Target::Page(Page::Scheduled))
        );
        assert_eq!(
            parse("emberhut://approvals"),
            Ok(Target::Page(Page::Approvals(None)))
        );
        assert_eq!(
            parse("emberhut://approval/7"),
            Ok(Target::Page(Page::Approval(ApprovalId::new(7))))
        );
        assert!(parse("emberhut://approval/x").is_err());
        assert_eq!(
            parse("emberhut://browser"),
            Ok(Target::Window(WindowKind::Browser))
        );
        assert_eq!(
            parse("emberhut://diary"),
            Ok(Target::Window(WindowKind::Diary))
        );
        assert_eq!(
            parse("emberhut://party"),
            Ok(Target::Window(WindowKind::Party))
        );
        assert_eq!(parse("emberhut://party/12"), Ok(Target::Party(12)));
        assert_eq!(
            parse("emberhut://note"),
            Ok(Target::Window(WindowKind::QuickNote))
        );
    }

    #[test]
    fn other_links_are_refused() {
        assert!(matches!(
            parse("https://emberhut.com/chat"),
            Err(LinkError::Scheme(_))
        ));
        assert!(matches!(parse("chat"), Err(LinkError::Scheme(_))));
        for bad in [
            "emberhut://",
            "emberhut://task",
            "emberhut://task/x",
            "emberhut://task/1/2",
            "emberhut://nowhere",
            "emberhut://needs/3",
        ] {
            assert!(matches!(parse(bad), Err(LinkError::Place(_))), "{bad}");
        }
    }
}
