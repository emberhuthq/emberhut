//! Dictation's state, with a microphone that records the same thing each time.

use super::mic::fake::FakeMicrophone;
use super::*;

fn dictation(seconds: u32, sample: f32) -> Dictation {
    let samples = vec![sample; usize::try_from(seconds * 1000).unwrap()];
    let microphone = FakeMicrophone::new(Captured {
        samples,
        rate: 1000,
    });
    Dictation::new(Some(Box::new(microphone)))
}

/// Opens the microphone as the composer does off the UI thread.
fn open(dictation: &mut Dictation) {
    let opening = dictation.begin().unwrap();
    let opened = opening.microphone.start();
    assert!(dictation.opened(opening.session, opened).is_none());
}

/// Finishes the recording and prepares its WAV, as the store does off the UI thread.
fn finished(stopped: Stopped) -> Result<Vec<u8>, Unheard> {
    prepare(&stopped.recording.finish())
}

#[test]
fn speech_goes_to_be_turned_into_text_and_comes_back() {
    let mut dictation = dictation(2, 0.2);
    open(&mut dictation);
    assert!(dictation.is_listening());
    dictation.tick();
    assert_eq!(dictation.levels().len(), 1);
    let stopped = dictation.stop().unwrap();
    assert!(dictation.is_preparing());
    let session = stopped.session;
    let wav = finished(stopped).unwrap();
    assert_eq!(&wav[0..4], b"RIFF");
    assert_eq!(
        dictation.finish(session, Ok(" hello there ".into())),
        Some("hello there".into())
    );
    assert!(!dictation.is_preparing());
}

#[test]
fn a_tap_is_not_speech() {
    let microphone = FakeMicrophone::new(Captured {
        samples: vec![0.2; 100],
        rate: 1000,
    });
    let mut dictation = Dictation::new(Some(Box::new(microphone)));
    open(&mut dictation);
    let stopped = dictation.stop().unwrap();
    let session = stopped.session;
    let unheard = finished(stopped).unwrap_err();
    assert!(matches!(unheard, Unheard::Tap));
    assert_eq!(dictation.finish(session, Err(unheard)), None);
    assert_eq!(dictation.error(), None);
    assert!(!dictation.is_preparing());
}

#[test]
fn silence_says_to_check_the_microphone() {
    let mut dictation = dictation(1, 0.0);
    open(&mut dictation);
    let stopped = dictation.stop().unwrap();
    let session = stopped.session;
    let unheard = finished(stopped).unwrap_err();
    assert!(matches!(unheard, Unheard::Silent));
    dictation.finish(session, Err(unheard));
    assert_eq!(dictation.error(), Some(silent_note()));
    #[cfg(not(target_os = "macos"))]
    assert!(
        !silent_note().contains("System Settings"),
        "no macOS settings elsewhere"
    );
}

#[test]
fn no_microphone_says_so() {
    let mut dictation = Dictation::new(Some(Box::new(FakeMicrophone::none())));
    let opening = dictation.begin().unwrap();
    let opened = opening.microphone.start();
    assert!(dictation.opened(opening.session, opened).is_none());
    assert!(!dictation.is_listening());
    assert_eq!(
        dictation.error(),
        Some("Could not start dictation: there is no microphone.")
    );
    assert!(!Dictation::new(None).is_available());
    assert!(Dictation::new(None).begin().is_none());
}

#[test]
fn a_microphone_opened_after_the_recording_was_dropped_is_let_go_of() {
    let mut dictation = dictation(1, 0.2);
    let opening = dictation.begin().unwrap();
    assert!(dictation.begin().is_none(), "one at a time");
    assert!(dictation.discard().is_none());
    let opened = opening.microphone.start();
    assert!(dictation.opened(opening.session, opened).is_some());
    assert!(!dictation.is_listening());
    assert!(!dictation.is_recording(opening.session));
}

#[test]
fn words_for_a_dropped_recording_are_dropped() {
    let mut dictation = dictation(1, 0.2);
    open(&mut dictation);
    let stopped = dictation.stop().unwrap();
    assert!(dictation.discard().is_none());
    assert_eq!(dictation.finish(stopped.session, Ok("late".into())), None);
}

#[test]
fn a_failure_says_why() {
    let mut dictation = dictation(1, 0.2);
    open(&mut dictation);
    let stopped = dictation.stop().unwrap();
    let down = backend::Error::Refused {
        status: Some(502),
        message: Some("the service is down".into()),
    };
    assert_eq!(
        dictation.finish(stopped.session, Err(Unheard::Failed(down))),
        None
    );
    assert_eq!(
        dictation.error(),
        Some("Dictation failed: the service is down")
    );
    dictation.dismiss_error();
    assert_eq!(dictation.error(), None);
}

#[test]
fn a_recording_stops_at_the_longest_the_server_takes() {
    let longest = u32::try_from(wav::max_seconds()).unwrap();
    let mut dictation = dictation(longest + 5, 0.2);
    open(&mut dictation);
    assert!(dictation.is_full(), "past the limit");
    let stopped = dictation.stop().unwrap();
    assert!(
        dictation
            .error()
            .unwrap()
            .starts_with("Dictation stopped at 13:39")
    );
    let wav = finished(stopped).unwrap();
    let size = u64::try_from(wav.len()).unwrap();
    assert_eq!(size, 44 + 2 * wav::max_seconds() * 1000, "cut at the limit");
    assert!(size <= emberhut_api::transcribe::MAX_SIZE, "{size}");
}

#[test]
fn the_time_left_shows_only_near_the_limit() {
    let mut dictation = dictation(2, 0.2);
    assert_eq!(dictation.time_left(), None);
    open(&mut dictation);
    assert_eq!(dictation.time_left(), None, "far from it");
}

#[test]
fn what_was_said_comes_after_what_is_typed() {
    assert_eq!(append("", "hello"), "hello");
    assert_eq!(append("Say", "hello"), "Say hello");
    assert_eq!(append("Say ", "hello"), "Say hello");
    assert_eq!(append("Line\n", "hello"), "Line\nhello");
}

#[test]
fn the_clock_reads_minutes_and_seconds() {
    assert_eq!(clock(Duration::from_secs(7)), "0:07");
    assert_eq!(clock(Duration::from_secs(90)), "1:30");
}
