//! Where a recording's tracks and live chunks go on the server: `POST /api/calls/in` and
//! `POST /api/calls/live`, with the call, the track and the times in the query.

use url::Url;

/// A call's track.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Track {
    /// The owner's microphone.
    Mic,
    /// The Mac's sound: the other side of the call.
    Sys,
}

impl Track {
    /// Both tracks, in the order they are sent.
    pub const ALL: [Self; 2] = [Self::Mic, Self::Sys];

    /// Its name on the wire and its file's (`mic.m4a`, `sys.m4a`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mic => "mic",
            Self::Sys => "sys",
        }
    }
}

/// `server` with `segments` added to its path.
fn at(server: &Url, segments: &[&str]) -> Url {
    let mut url = server.clone();
    if let Ok(mut path) = url.path_segments_mut() {
        path.pop_if_empty().extend(segments);
    }
    url.set_query(None);
    url
}

/// Where a whole track of call `call` goes: the last one (`last`) queues it to be transcribed.
#[must_use]
pub fn track_url(
    server: &Url,
    call: &str,
    track: Track,
    seconds: f64,
    title: &str,
    last: bool,
) -> Url {
    let mut url = at(server, &["api", "calls", "in"]);
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("call", call)
            .append_pair("track", track.name())
            .append_pair("seconds", &format!("{}", seconds.max(0.0).floor()))
            .append_pair("title", title);
        if last {
            query.append_pair("last", "1");
        }
    }
    url
}

/// Where a live chunk of call `call` that begins `start` seconds in goes.
#[must_use]
pub fn chunk_url(server: &Url, call: &str, track: Track, start: f64) -> Url {
    let mut url = at(server, &["api", "calls", "live"]);
    url.query_pairs_mut()
        .append_pair("call", call)
        .append_pair("track", track.name())
        .append_pair("t", &format!("{start:.1}"));
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_track_goes_to_calls_in_with_its_query() {
        let server = Url::parse("https://emberhut.example/").unwrap();
        let url = track_url(
            &server,
            "2026-11-26-abc123",
            Track::Mic,
            65.7,
            "A & B",
            false,
        );
        assert_eq!(
            url.as_str(),
            "https://emberhut.example/api/calls/in?call=2026-11-26-abc123&track=mic&seconds=65&title=A+%26+B"
        );
        let last = track_url(&server, "abcdef", Track::Sys, 1.0, "", true);
        assert!(
            last.as_str().ends_with("track=sys&seconds=1&title=&last=1"),
            "{last}"
        );
    }

    #[test]
    fn a_server_under_a_path_keeps_it() {
        let server = Url::parse("http://mac-server.internal:8791/emberhut").unwrap();
        let url = chunk_url(&server, "abcdef", Track::Sys, 12.34);
        assert_eq!(
            url.as_str(),
            "http://mac-server.internal:8791/emberhut/api/calls/live?call=abcdef&track=sys&t=12.3"
        );
    }
}
