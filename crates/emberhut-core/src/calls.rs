//! Recorded calls: the owner's calls, recorded on their Mac (their microphone and the Mac's
//! sound as two tracks), or a recording they share, turned into a transcript the assistant
//! reads; and, while a call is being recorded, its live transcript and the copilot's hints.
//!
//! These are the rules with no I/O: a call's id and title, its tracks and their speakers, which
//! files are audio, and the live transcript's words (the microphone's echo of the other side
//! left out, a hint cleaned up).

use std::fmt;

use jiff::Timestamp;

use crate::RoomId;
use crate::random::{RandomError, RandomSource, bytes};

/// The longest title a call keeps, in characters.
pub const TITLE_LENGTH: usize = 80;

/// A call's id: `2026-11-25-1a2b3c` when the server or the Mac's recorder makes it (the UTC day
/// and six random hex digits); one kept from elsewhere may be any 6 to 40 letters, digits, `_`
/// and `-`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CallId(String);

impl CallId {
    /// `text` as a call's id, when it is 6 to 40 ASCII letters, digits, `_` or `-`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let fits = (6..=40).contains(&text.len())
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        fits.then(|| Self(text.to_owned()))
    }

    /// An id as the database keeps it, taken as it is.
    #[must_use]
    pub fn stored(text: String) -> Self {
        Self(text)
    }

    /// A new id for a call started at `now`.
    ///
    /// # Errors
    ///
    /// [`RandomError`] when no randomness is to be had.
    pub fn new(now: Timestamp, random: &dyn RandomSource) -> Result<Self, RandomError> {
        let [a, b, c] = bytes::<3>(random)?;
        let day = now.strftime("%Y-%m-%d");
        Ok(Self(format!("{day}-{a:02x}{b:02x}{c:02x}")))
    }

    /// Whether the id is a new call's, made as [`CallId::new`] makes one (`<UTC day>-<6
    /// lowercase hex digits>`, the Mac's recorder makes its own the same way) for a call started
    /// on `now`'s UTC day or the day before (a call may run past midnight).
    #[must_use]
    pub fn is_new_on(&self, now: Timestamp) -> bool {
        let Some((day, hex)) = self.0.rsplit_once('-') else {
            return false;
        };
        let random = hex.len() == 6
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        let yesterday = now.checked_sub(jiff::SignedDuration::from_hours(24)).ok();
        random
            && [Some(now), yesterday]
                .into_iter()
                .flatten()
                .any(|at| at.strftime("%Y-%m-%d").to_string() == day)
    }

    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where a call is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CallStatus {
    /// Being recorded on the Mac; its audio isn't here yet.
    Recording,
    /// Its audio is here, waiting to be transcribed.
    Queued,
    /// Being transcribed now.
    Transcribing,
    /// Transcribed.
    Done,
    /// The transcription failed; its audio is kept.
    Failed,
}

impl CallStatus {
    /// The status's word, as stored and written.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Recording => "recording",
            Self::Queued => "queued",
            Self::Transcribing => "transcribing",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    /// The status called `word`.
    #[must_use]
    pub fn from_name(word: &str) -> Option<Self> {
        Some(match word {
            "recording" => Self::Recording,
            "queued" => Self::Queued,
            "transcribing" => Self::Transcribing,
            "done" => Self::Done,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
}

impl fmt::Display for CallStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One of a recording's two tracks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrackKind {
    /// The owner's microphone.
    Mic,
    /// The Mac's sound: the other side of the call.
    Sys,
}

impl TrackKind {
    /// The track's word in a request (`mic`, `sys`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mic => "mic",
            Self::Sys => "sys",
        }
    }

    /// The track called `word`.
    #[must_use]
    pub fn from_name(word: &str) -> Option<Self> {
        match word {
            "mic" => Some(Self::Mic),
            "sys" => Some(Self::Sys),
            _ => None,
        }
    }

    /// Who speaks on it, as the transcript labels their lines.
    #[must_use]
    pub const fn who(self) -> &'static str {
        match self {
            Self::Mic => "Me",
            Self::Sys => "Others",
        }
    }
}

/// An audio file of a call, and who speaks on it (none for a single shared recording, where
/// the transcript tells the speakers apart by what they say).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    /// The file, in the call's folder.
    pub file: String,
    /// Who speaks on it.
    pub who: Option<String>,
}

/// A call, as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    /// Its id.
    pub id: CallId,
    /// Who and what it is.
    pub title: String,
    /// The task its transcript is for, if any.
    pub task: Option<i64>,
    /// The room its transcript goes to.
    pub room: RoomId,
    /// Where it is.
    pub status: CallStatus,
    /// Its audio files.
    pub tracks: Vec<Track>,
    /// The shared recording it was copied from, when it is one.
    pub source: Option<String>,
    /// The transcript's file, once done.
    pub transcript: Option<String>,
    /// Why the transcription failed.
    pub error: Option<String>,
    /// How long it is, in seconds, when the recorder said.
    pub seconds: Option<f64>,
    /// When it started.
    pub at: Timestamp,
    /// When it was transcribed or failed.
    pub done_at: Option<Timestamp>,
    /// Its audio has been deleted (the transcript is kept).
    pub audio_gone: bool,
}

/// A call to store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewCall {
    /// Its id.
    pub id: CallId,
    /// Who and what it is.
    pub title: String,
    /// The task its transcript is for.
    pub task: Option<i64>,
    /// The room its transcript goes to.
    pub room: RoomId,
    /// Where it starts.
    pub status: CallStatus,
}

/// A line heard during a call, to store.
#[derive(Clone, Debug, PartialEq)]
pub struct HeardLine {
    /// The line.
    pub line: LiveLine,
    /// The language it was heard in.
    pub lang: Option<String>,
    /// The transcriber was sure of the language.
    pub sure: bool,
}

/// A copilot's hint during a call.
#[derive(Clone, Debug, PartialEq)]
pub struct CallHint {
    /// Seconds from the call's start it was given at.
    pub t: Option<f64>,
    /// The hint.
    pub text: String,
    /// When it was given.
    pub at: Timestamp,
}

/// The title of a call no one named: `Call on 2026-10-31 01:20 UTC`.
#[must_use]
pub fn untitled(now: Timestamp) -> String {
    format!("Call on {} UTC", now.strftime("%Y-%m-%d %H:%M"))
}

/// `given` cut to [`TITLE_LENGTH`] characters, or [`untitled`] when that leaves nothing.
#[must_use]
pub fn title_or_untitled(given: &str, now: Timestamp) -> String {
    let title = cut(given, TITLE_LENGTH);
    if title.is_empty() {
        untitled(now)
    } else {
        title
    }
}

/// The first `limit` characters of `text`.
#[must_use]
pub fn cut(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

/// The last `limit` characters of `text`.
#[must_use]
pub fn last_chars(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    text.chars().skip(count.saturating_sub(limit)).collect()
}

/// The extensions of the audio (and video) files a call can be transcribed from.
const AUDIO: &[&str] = &[
    "m4a", "mp3", "wav", "aac", "caf", "ogg", "opus", "mp4", "mov", "webm", "flac", "aif", "aiff",
];

/// The audio extension `name` ends with, lowercased and with its dot (`.m4a`), if it is one.
#[must_use]
pub fn audio_extension(name: &str) -> Option<String> {
    let (_, extension) = name.rsplit_once('.')?;
    let extension = extension.to_ascii_lowercase();
    AUDIO
        .contains(&extension.as_str())
        .then(|| format!(".{extension}"))
}

/// Seconds from a call's start as `m:ss` (`75.4` is `1:15`).
#[must_use]
pub fn clock(seconds: f64) -> String {
    let whole = if seconds.is_finite() && seconds > 0.0 {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a call's offset in whole seconds: positive, and far below u64's range"
        )]
        let whole = seconds.floor() as u64;
        whole
    } else {
        0
    };
    format!("{}:{:02}", whole / 60, whole % 60)
}

/// A line of a call's live transcript.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveLine {
    /// The track it was heard on.
    pub track: TrackKind,
    /// Seconds from the call's start.
    pub t: f64,
    /// What was said.
    pub text: String,
}

/// `text` lowercased, with only its letters and digits: what two lines are compared on.
fn letters(text: &str) -> Vec<char> {
    text.chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// How much of `a` is found in `b`, in order (their longest common subsequence over `a`'s
/// length), 0 to 1.
fn shared(a: &[char], b: &[char]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut previous = vec![0_usize; b.len() + 1];
    for ch in a {
        let mut current = vec![0_usize; b.len() + 1];
        for (j, other) in b.iter().enumerate() {
            current[j + 1] = if ch == other {
                previous[j] + 1
            } else {
                previous[j + 1].max(current[j])
            };
        }
        previous = current;
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "a ratio of two line lengths, far below f64's exact range"
    )]
    let ratio = previous[b.len()] as f64 / a.len() as f64;
    ratio
}

/// How far apart, in seconds, a microphone line and the Mac's lines it may echo are.
const ECHO_WINDOW: f64 = 12.0;

/// `lines` without the microphone's echo: on speakers the microphone hears the other side too,
/// so a microphone line that mostly says what the Mac played within `ECHO_WINDOW` seconds of
/// it is theirs again, and is left out. Words only: the final transcript has the loudness too.
#[must_use]
pub fn unecho(lines: &[LiveLine]) -> Vec<LiveLine> {
    lines
        .iter()
        .filter(|line| {
            if line.track != TrackKind::Mic {
                return true;
            }
            let played: Vec<&str> = lines
                .iter()
                .filter(|other| {
                    other.track == TrackKind::Sys && (other.t - line.t).abs() < ECHO_WINDOW
                })
                .map(|other| other.text.as_str())
                .collect();
            shared(&letters(&line.text), &letters(&played.join(" "))) < 0.5
        })
        .cloned()
        .collect()
}

/// The copilot's answer as a hint: its first line, without surrounding quotes; none when it is
/// empty or only hyphens (the copilot's "nothing to add").
#[must_use]
pub fn hint_of(answer: &str) -> Option<String> {
    let first = answer.trim().lines().next().unwrap_or_default();
    let first = first
        .strip_prefix(['"', '\u{201c}'])
        .unwrap_or(first)
        .trim_end_matches(['"', '\u{201d}'])
        .trim();
    (!first.is_empty() && !first.chars().all(|c| c == '-')).then(|| first.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_call_s_id_is_today_s_or_yesterday_s_day_and_six_hex_digits() {
        let now: Timestamp = "2026-11-26T00:30:00Z".parse().unwrap();
        for id in ["2026-11-26-1a2b3c", "2026-11-25-abcdef"] {
            assert!(CallId::parse(id).unwrap().is_new_on(now), "{id}");
        }
        for id in [
            "2026-11-24-1a2b3c",
            "2026-11-27-1a2b3c",
            "2026-11-26-1A2B3C",
            "2026-11-26-1a2b3",
            "2026-11-26-1a2b3cd",
            "mac-call-1",
            "abcdef",
        ] {
            assert!(!CallId::parse(id).unwrap().is_new_on(now), "{id}");
        }
    }

    #[derive(Debug)]
    struct Fixed;

    impl RandomSource for Fixed {
        fn fill(&self, bytes: &mut [u8]) -> Result<(), RandomError> {
            bytes.fill(0xab);
            Ok(())
        }
    }

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn ids_are_the_day_and_six_hex_digits_and_parse_back() {
        let id = CallId::new(at("2026-10-31T01:20:00Z"), &Fixed).unwrap();
        assert_eq!(id.as_str(), "2026-10-31-ababab");
        assert_eq!(CallId::parse(id.as_str()), Some(id));
        assert!(CallId::parse("abc12").is_none(), "too short");
        assert!(CallId::parse(&"a".repeat(41)).is_none(), "too long");
        assert!(CallId::parse("../../etc").is_none());
        assert!(CallId::parse("mac_call-01").is_some());
    }

    #[test]
    fn a_call_no_one_named_is_called_by_its_time() {
        let now = at("2026-10-31T01:20:59Z");
        assert_eq!(untitled(now), "Call on 2026-10-31 01:20 UTC");
        assert_eq!(title_or_untitled("", now), "Call on 2026-10-31 01:20 UTC");
        assert_eq!(title_or_untitled("Weekly", now), "Weekly");
        assert_eq!(title_or_untitled(&"x".repeat(90), now).len(), 80);
    }

    #[test]
    fn audio_files_are_known_by_their_extension() {
        assert_eq!(audio_extension("Memo.M4A").as_deref(), Some(".m4a"));
        assert_eq!(audio_extension("a/b.aiff").as_deref(), Some(".aiff"));
        assert_eq!(audio_extension("notes.txt"), None);
        assert_eq!(audio_extension("m4a"), None);
    }

    #[test]
    fn the_clock_is_minutes_and_seconds() {
        assert_eq!(clock(75.4), "1:15");
        assert_eq!(clock(5.0), "0:05");
        assert_eq!(clock(-3.0), "0:00");
        assert_eq!(clock(3_600.0), "60:00");
    }

    fn line(track: TrackKind, t: f64, text: &str) -> LiveLine {
        LiveLine {
            track,
            t,
            text: text.to_owned(),
        }
    }

    #[test]
    fn the_microphone_s_echo_of_the_other_side_is_left_out() {
        let lines = [
            line(TrackKind::Sys, 10.0, "We can ship it on Friday."),
            line(TrackKind::Mic, 12.0, "we can ship it on friday"),
            line(TrackKind::Mic, 14.0, "Great, I'll tell the team."),
            line(TrackKind::Mic, 40.0, "We can ship it on Friday."),
        ];
        let kept = unecho(&lines);
        assert_eq!(
            kept.iter().map(|l| l.t).collect::<Vec<_>>(),
            [10.0, 14.0, 40.0],
            "the echo within 12 s goes; the same words later are the owner's own"
        );
    }

    #[test]
    fn a_hint_is_the_first_line_without_quotes_and_a_hyphen_is_none() {
        assert_eq!(
            hint_of("\"Fact: the price was 40.\"\nmore").as_deref(),
            Some("Fact: the price was 40.")
        );
        assert_eq!(
            hint_of("\u{201c}Ask: when?\u{201d}").as_deref(),
            Some("Ask: when?")
        );
        assert_eq!(hint_of("-"), None);
        assert_eq!(hint_of(" --- "), None);
        assert_eq!(hint_of(""), None);
    }

    #[test]
    fn statuses_and_tracks_read_back_from_their_words() {
        for status in [
            CallStatus::Recording,
            CallStatus::Queued,
            CallStatus::Transcribing,
            CallStatus::Done,
            CallStatus::Failed,
        ] {
            assert_eq!(CallStatus::from_name(status.as_str()), Some(status));
        }
        assert_eq!(TrackKind::from_name("mic"), Some(TrackKind::Mic));
        assert_eq!(
            TrackKind::from_name("sys").map(TrackKind::who),
            Some("Others")
        );
        assert_eq!(TrackKind::from_name("both"), None);
    }
}
