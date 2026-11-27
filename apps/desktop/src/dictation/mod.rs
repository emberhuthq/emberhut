//! Dictation in the composer: the mic records, a second click (or the check) stops it, the
//! recording goes to the server's speech-to-text with the words the app uses, and what was
//! said comes after what is already typed. The X drops a recording; so does sending or
//! clearing the field, and words still on their way then are dropped too.
//!
//! [`Dictation`] is the state with no GPUI in it; the composer drives it and draws it. The
//! microphone is behind [`mic::Microphone`], so the tests use their own.

//! Dictation in the composer: the mic records, a second click (or the check) stops it, the
//! recording goes to the server's speech-to-text with the words the app uses, and what was
//! said comes after what is already typed. The X drops a recording; so does sending or
//! clearing the field, and words still on their way then are dropped too. A recording stops by
//! itself at the longest the server takes ([`wav::max_seconds`]), and says so.
//!
//! [`Dictation`] is the state with no GPUI in it; the composer drives it and draws it. The
//! microphone is behind [`mic::Microphone`], so the tests use their own. Opening the device,
//! finishing a recording and making its WAV all happen off the UI thread: [`Dictation::begin`]
//! hands out the microphone to open, [`Dictation::stop`] the recording to finish, and
//! [`prepare`] makes the WAV there.

pub(crate) mod mic;
pub(crate) mod wav;

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use self::mic::{MicError, Microphone, Recording};
use self::wav::Captured;
use crate::backend;
use crate::store::composer::reason;

/// The names and terms the speech-to-text is told to expect, so they come out right.
pub(crate) const WORDS: &[&str] = &[
    "Emberhut",
    "Claude",
    "Claude Code",
    "MCP",
    "PR",
    "PRs",
    "Xcode",
    "TestFlight",
];

/// The file name the server is sent: its extension says the format.
pub(crate) const FILE_NAME: &str = "dictation.wav";

/// A recording shorter than this is a tap, not speech: it is dropped.
const SHORTEST: f64 = 0.3;

/// How many loudness readings the waveform keeps (about two and a half seconds of them).
const LEVELS: usize = 48;

/// How often the waveform takes a reading while recording.
pub(crate) const TICK: Duration = Duration::from_millis(50);

/// How long before the limit the recording shows the time left.
const WARN_BEFORE: Duration = Duration::from_secs(60);

/// Why a recording gave no words.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Unheard {
    /// It was a tap, too short to be speech.
    #[error("too short to be speech")]
    Tap,
    /// Every sample was zero: the input is muted, or the system holds the microphone back.
    #[error("the microphone gave only silence")]
    Silent,
    /// The speech-to-text failed.
    #[error("the speech-to-text failed")]
    Failed(#[source] backend::Error),
}

/// The WAV the server is sent for `captured`, unless it is a tap or silence. It reads every
/// sample: call it off the UI thread.
///
/// # Errors
///
/// [`Unheard::Tap`] or [`Unheard::Silent`].
pub(crate) fn prepare(captured: &Captured) -> Result<Vec<u8>, Unheard> {
    if captured.seconds() < SHORTEST {
        return Err(Unheard::Tap);
    }
    if captured.is_silent() {
        return Err(Unheard::Silent);
    }
    Ok(captured.to_wav())
}

/// What to say when the microphone gave only silence.
fn silent_note() -> &'static str {
    if cfg!(target_os = "macos") {
        "The microphone gave only silence. Allow it in System Settings → Privacy & Security."
    } else {
        "The microphone gave only silence: check that it is on and not muted."
    }
}

/// What to say when the microphone couldn't be opened.
fn open_failure(error: &MicError) -> String {
    format!("Could not start dictation: {error}.")
}

/// Where dictation stands.
enum Phase {
    /// Nothing going on.
    Idle,
    /// The microphone is being opened, off the UI thread.
    Opening,
    /// Recording since `started`.
    Listening {
        recording: Box<dyn Recording>,
        started: Instant,
    },
    /// The recording is being turned into text.
    Preparing,
}

/// A microphone to open off the UI thread, for [`Dictation::opened`].
pub(crate) struct Opening {
    /// Which recording it is.
    pub(crate) session: u64,
    /// The microphone.
    pub(crate) microphone: Arc<dyn Microphone>,
}

/// A recording to finish and turn into text off the UI thread, for [`Dictation::finish`].
pub(crate) struct Stopped {
    /// Which recording it is, to hand back with what was said.
    pub(crate) session: u64,
    /// The recording, still to finish.
    pub(crate) recording: Box<dyn Recording>,
}

/// The composer's dictation.
pub(crate) struct Dictation {
    microphone: Option<Arc<dyn Microphone>>,
    phase: Phase,
    /// Counts recordings: words for an earlier one, dropped since, are dropped.
    session: u64,
    /// The loudness readings while recording, newest last.
    levels: VecDeque<f32>,
    /// Why the last try failed or stopped, worded for the user.
    error: Option<String>,
}

impl Dictation {
    /// Dictation through `microphone`; with none, there is no mic to click.
    #[must_use]
    pub(crate) fn new(microphone: Option<Box<dyn Microphone>>) -> Self {
        Self {
            microphone: microphone.map(Arc::from),
            phase: Phase::Idle,
            session: 0,
            levels: VecDeque::with_capacity(LEVELS),
            error: None,
        }
    }

    /// Dictation through the system's microphone, where the app has one (see [`mic`]).
    #[must_use]
    pub(crate) fn system() -> Self {
        #[cfg(not(target_os = "macos"))]
        let microphone: Option<Box<dyn Microphone>> = Some(Box::new(mic::SystemMicrophone));
        #[cfg(target_os = "macos")]
        let microphone: Option<Box<dyn Microphone>> = None;
        Self::new(microphone)
    }

    /// Whether there is a microphone to dictate with.
    #[must_use]
    pub(crate) fn is_available(&self) -> bool {
        self.microphone.is_some()
    }

    /// Whether it is recording.
    #[must_use]
    pub(crate) fn is_listening(&self) -> bool {
        matches!(self.phase, Phase::Listening { .. })
    }

    /// Whether recording `session` is still going (or still opening).
    #[must_use]
    pub(crate) fn is_recording(&self, session: u64) -> bool {
        session == self.session && matches!(self.phase, Phase::Opening | Phase::Listening { .. })
    }

    /// Whether a recording is being turned into text.
    #[must_use]
    pub(crate) fn is_preparing(&self) -> bool {
        matches!(self.phase, Phase::Preparing)
    }

    /// How long it has been recording.
    #[must_use]
    pub(crate) fn elapsed(&self) -> Duration {
        match &self.phase {
            Phase::Listening { started, .. } => started.elapsed(),
            Phase::Idle | Phase::Opening | Phase::Preparing => Duration::ZERO,
        }
    }

    /// The time left before the recording stops by itself, once that is under a minute.
    #[must_use]
    pub(crate) fn time_left(&self) -> Option<Duration> {
        let longest = Duration::from_secs(wav::max_seconds());
        let left = longest.saturating_sub(self.elapsed());
        (self.is_listening() && left < WARN_BEFORE).then_some(left)
    }

    /// Whether the recording has reached the longest the server takes, so it has to stop.
    #[must_use]
    pub(crate) fn is_full(&self) -> bool {
        match &self.phase {
            Phase::Listening { recording, started } => {
                recording.is_full() || started.elapsed().as_secs() >= wav::max_seconds()
            }
            Phase::Idle | Phase::Opening | Phase::Preparing => false,
        }
    }

    /// The loudness readings, newest last.
    #[must_use]
    pub(crate) fn levels(&self) -> &VecDeque<f32> {
        &self.levels
    }

    /// Why the last try failed or stopped.
    #[must_use]
    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Hides the failure.
    pub(crate) fn dismiss_error(&mut self) {
        self.error = None;
    }

    /// Starts a recording (nothing while one goes on or is turned into text): the microphone
    /// to open off the UI thread, then hand to [`Dictation::opened`].
    pub(crate) fn begin(&mut self) -> Option<Opening> {
        if !matches!(self.phase, Phase::Idle) {
            return None;
        }
        let microphone = Arc::clone(self.microphone.as_ref()?);
        self.error = None;
        self.session += 1;
        self.phase = Phase::Opening;
        Some(Opening {
            session: self.session,
            microphone,
        })
    }

    /// The microphone for `session` opened, or why not. A recording dropped meanwhile comes
    /// back, to be let go of off the UI thread.
    pub(crate) fn opened(
        &mut self,
        session: u64,
        opened: Result<Box<dyn Recording>, MicError>,
    ) -> Option<Box<dyn Recording>> {
        if session != self.session || !matches!(self.phase, Phase::Opening) {
            return opened.ok();
        }
        match opened {
            Ok(recording) => {
                self.levels.clear();
                self.phase = Phase::Listening {
                    recording,
                    started: Instant::now(),
                };
            }
            Err(error) => {
                log::warn!("dictation didn't start: {error}");
                self.error = Some(open_failure(&error));
                self.phase = Phase::Idle;
            }
        }
        None
    }

    /// Takes a loudness reading for the waveform.
    pub(crate) fn tick(&mut self) {
        if let Phase::Listening { recording, .. } = &self.phase {
            if self.levels.len() == LEVELS {
                self.levels.pop_front();
            }
            self.levels.push_back(recording.level());
        }
    }

    /// The X, or the text was sent or cleared: the recording ends unused, and words still on
    /// their way are dropped. The recording comes back, to be let go of off the UI thread.
    pub(crate) fn discard(&mut self) -> Option<Box<dyn Recording>> {
        self.session += 1;
        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Listening { recording, .. } => Some(recording),
            Phase::Idle | Phase::Opening | Phase::Preparing => None,
        }
    }

    /// Stops recording: the recording to finish and turn into text off the UI thread. At the
    /// limit, it says it stopped there.
    pub(crate) fn stop(&mut self) -> Option<Stopped> {
        if self.is_full() {
            self.error = Some(format!(
                "Dictation stopped at {}, the longest recording the server takes.",
                clock(Duration::from_secs(wav::max_seconds()))
            ));
        }
        let Phase::Listening { recording, .. } = std::mem::replace(&mut self.phase, Phase::Idle)
        else {
            return None;
        };
        self.session += 1;
        self.phase = Phase::Preparing;
        Some(Stopped {
            session: self.session,
            recording,
        })
    }

    /// The speech-to-text answered for `session`: what was said, to add to the field (none
    /// when the recording was dropped since, was a tap, or nothing was said, or it failed:
    /// then it says why).
    pub(crate) fn finish(&mut self, session: u64, said: Result<String, Unheard>) -> Option<String> {
        if session != self.session || !self.is_preparing() {
            return None;
        }
        self.phase = Phase::Idle;
        match said {
            Ok(said) => Some(said.trim().to_owned()).filter(|said| !said.is_empty()),
            Err(Unheard::Tap) => None,
            Err(Unheard::Silent) => {
                self.error = Some(silent_note().to_owned());
                None
            }
            Err(Unheard::Failed(error)) => {
                self.error = Some(format!("Dictation failed: {}", reason(&error)));
                None
            }
        }
    }
}

/// `typed` with `said` after it, a space between unless the field is empty or ends in one.
#[must_use]
pub(crate) fn append(typed: &str, said: &str) -> String {
    if typed.is_empty() || typed.ends_with(char::is_whitespace) {
        format!("{typed}{said}")
    } else {
        format!("{typed} {said}")
    }
}

/// `elapsed` as the recording shows it: `0:07`, `1:30`.
#[must_use]
pub(crate) fn clock(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests;
