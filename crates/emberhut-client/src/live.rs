//! Following a live stream: a room's, or a runner session's.

use std::io::BufRead;

use emberhut_api::approvals::{APPROVALS_EVENT, ApprovalsEvent};
use emberhut_api::live::LiveEvent;
use serde::de::DeserializeOwned;

use crate::Error;
use crate::sse::{self, SseEvent, SseItem, SseReader};
use crate::transport::TransportError;

/// How following the live stream ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LiveEnd {
    /// The stop condition said so.
    Stopped,
    /// The server closed the stream. Follow it again after a pause.
    Closed,
}

/// One event of a room's live stream (`GET /api/live`), told apart by its `event:` name.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RoomEvent {
    /// The assistant's line in the room: an event without a name (or named `message`).
    Live(LiveEvent),
    /// An approval request changed ([`APPROVALS_EVENT`]): what shows approvals fetches them
    /// again. Sent to a signed-in device's stream only.
    Approvals(ApprovalsEvent),
}

/// Reads a room's live stream: each line to `on_event`, until `stop` says so or the stream
/// ends; a named event (an approval's change, or one this client doesn't know) is left out.
/// `stop` is asked before each item is handed on, so a stopped follow ends at the next event or
/// ping (every 15 s at most).
pub(crate) fn follow(
    stream: impl BufRead,
    stop: impl FnMut() -> bool,
    mut on_event: impl FnMut(LiveEvent),
) -> Result<LiveEnd, Error> {
    follow_room(stream, stop, |event| {
        if let RoomEvent::Live(live) = event {
            on_event(live);
        }
    })
}

/// Reads a room's live stream as [`follow`] does, handing on every event this client knows:
/// the lines and the approvals' changes. An event of a name it doesn't know is skipped, as a
/// newer server may send new kinds.
pub(crate) fn follow_room(
    stream: impl BufRead,
    stop: impl FnMut() -> bool,
    on_event: impl FnMut(RoomEvent),
) -> Result<LiveEnd, Error> {
    follow_decoded(stream, stop, room_event, on_event)
}

/// Reads a stream of server-sent events whose data is JSON of type `T`, as [`follow`] does: an
/// event this client can't read is skipped (logged), as a newer server may send new kinds.
pub(crate) fn follow_events<T: DeserializeOwned>(
    stream: impl BufRead,
    stop: impl FnMut() -> bool,
    on_event: impl FnMut(T),
) -> Result<LiveEnd, Error> {
    follow_decoded(stream, stop, |event| decoded(&event.data), on_event)
}

/// Reads a stream of server-sent events, each made a `T` by `decode` (none: skipped) and
/// handed to `on_event`, until `stop` says so or the stream ends.
fn follow_decoded<T>(
    stream: impl BufRead,
    mut stop: impl FnMut() -> bool,
    mut decode: impl FnMut(&SseEvent) -> Option<T>,
    mut on_event: impl FnMut(T),
) -> Result<LiveEnd, Error> {
    for item in SseReader::new(stream) {
        let item = item.map_err(|error| {
            if sse::is_malformed(&error) {
                Error::Protocol(error)
            } else {
                TransportError::from(error).into()
            }
        })?;
        if stop() {
            return Ok(LiveEnd::Stopped);
        }
        let SseItem::Event(event) = item else {
            continue;
        };
        if let Some(decoded) = decode(&event) {
            on_event(decoded);
        }
    }
    Ok(LiveEnd::Closed)
}

/// `data` read as a `T`; none (logged) when it doesn't read.
fn decoded<T: DeserializeOwned>(data: &str) -> Option<T> {
    serde_json::from_str(data)
        .inspect_err(|error| tracing::warn!(%error, "skipped a live event this client can't read"))
        .ok()
}

/// A room stream's event, by its name: none for a name this client doesn't know, or data that
/// doesn't read.
fn room_event(event: &SseEvent) -> Option<RoomEvent> {
    match event.event.as_deref() {
        None | Some("message") => decoded(&event.data).map(RoomEvent::Live),
        Some(APPROVALS_EVENT) => decoded(&event.data).map(RoomEvent::Approvals),
        Some(other) => {
            tracing::debug!(
                event = other,
                "skipped a live event of a name this client doesn't know"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, ErrorKind, Read};

    use emberhut_api::live::Live;

    use super::*;

    const STREAM: &str =
        "data: {\"text\":\"\",\"doing\":\"Reading\"}\n\n: ping\n\ndata: not json\n\ndata: null\n\n";

    #[test]
    fn hands_on_every_event_until_the_stream_ends() {
        let mut events = Vec::new();
        let end = follow(STREAM.as_bytes(), || false, |e| events.push(e)).unwrap();
        assert_eq!(end, LiveEnd::Closed);
        let working = LiveEvent::Working(Live {
            text: String::new(),
            doing: Some("Reading".to_owned()),
        });
        assert_eq!(
            events,
            [working, LiveEvent::Idle],
            "the unreadable one is skipped"
        );
    }

    #[test]
    fn stops_when_asked() {
        let mut items = 0;
        let mut events = Vec::new();
        let stop = || {
            items += 1;
            items > 2
        };
        let end = follow(STREAM.as_bytes(), stop, |e| events.push(e)).unwrap();
        assert_eq!(end, LiveEnd::Stopped);
        assert_eq!(events.len(), 1, "the ping counted as the second item");
    }

    #[test]
    fn a_rooms_named_events_are_told_apart() {
        let stream = "event: approvals\ndata: {\"id\":7,\"state\":\"pending\",\"pending\":2}\n\n\
                      event: newer\ndata: {\"text\":\"\",\"doing\":null}\n\n\
                      event: message\ndata: null\n\n";
        let mut events = Vec::new();
        follow_room(stream.as_bytes(), || false, |e| events.push(e)).unwrap();
        let changed = ApprovalsEvent {
            id: emberhut_api::ids::ApprovalId::new(7),
            state: emberhut_api::approvals::ApprovalState::Pending,
            pending: 2,
        };
        assert_eq!(
            events,
            [
                RoomEvent::Approvals(changed),
                RoomEvent::Live(LiveEvent::Idle)
            ],
            "a name this client doesn't know is skipped"
        );
        let mut lines = Vec::new();
        follow(stream.as_bytes(), || false, |e| lines.push(e)).unwrap();
        assert_eq!(
            lines,
            [LiveEvent::Idle],
            "the lines alone, never an approval"
        );
    }

    #[test]
    fn a_stream_that_isnt_an_event_stream_is_a_protocol_error() {
        let error = follow(&b"data: \xff\n\n"[..], || false, |_| {}).unwrap_err();
        assert!(matches!(error, Error::Protocol(_)), "{error}");
        assert!(!error.is_transient());
    }

    #[test]
    fn invalid_data_from_the_stream_underneath_is_a_network_error() {
        struct Garbled;
        impl Read for Garbled {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::new(ErrorKind::InvalidData, "a bad record"))
            }
        }
        let error = follow(io::BufReader::new(Garbled), || false, |_| {}).unwrap_err();
        assert!(matches!(error, Error::Network(_)), "{error}");
    }

    #[test]
    fn a_broken_stream_is_a_network_error() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::new(ErrorKind::ConnectionReset, "reset"))
            }
        }
        let error = follow(io::BufReader::new(Broken), || false, |_| {}).unwrap_err();
        assert!(matches!(error, Error::Network(_)), "{error}");
    }
}
