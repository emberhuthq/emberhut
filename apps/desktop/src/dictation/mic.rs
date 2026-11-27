//! The microphone: what a source of recordings offers ([`Microphone`], [`Recording`]), and the
//! system's default input device through `cpal` (ALSA on Linux). A recording collects its
//! samples, mixed down to one channel, on the audio thread, up to the longest the server
//! takes; how loud the last stretch was is kept for the waveform. Both are `Send`: the device
//! is opened and the recording finished off the UI thread.
//!
//! macOS has no system microphone here yet. The app bundle may not declare microphone use
//! (`NSMicrophoneUsageDescription`) while reports render in a web view without a permission
//! handler (docs/adr/0006-report-webview.md), and without that declaration macOS ends an app
//! that opens the microphone. Once it may, `cpal`'s Core Audio backend is this same code; the
//! system then hands out silence while the user refuses the microphone, which
//! [`Captured::is_silent`](super::wav::Captured::is_silent) tells apart.

#[cfg(not(target_os = "macos"))]
pub(crate) use self::system::SystemMicrophone;
use super::wav::Captured;

/// Why recording couldn't start.
#[derive(Debug, thiserror::Error)]
pub(crate) enum MicError {
    /// The system has no input device.
    #[cfg(any(test, not(target_os = "macos")))]
    #[error("there is no microphone")]
    NoDevice,
    /// The device couldn't be opened or started (busy, missing, or refused).
    #[cfg(not(target_os = "macos"))]
    #[error("the microphone couldn't be opened")]
    Device(#[source] cpal::Error),
    /// The device records in a sample format this app doesn't read.
    #[cfg(not(target_os = "macos"))]
    #[error("the microphone records {0} samples, which this app doesn't read")]
    UnsupportedFormat(cpal::SampleFormat),
}

/// A source of recordings.
pub(crate) trait Microphone: Send + Sync {
    /// Starts recording. It may take a while (the audio server opens the device): call it off
    /// the UI thread.
    ///
    /// # Errors
    ///
    /// No device, or it couldn't be opened.
    fn start(&self) -> Result<Box<dyn Recording>, MicError>;
}

/// A recording going on: it stops when finished or dropped.
pub(crate) trait Recording: Send {
    /// How loud the last stretch was, `0.0..=1.0`.
    fn level(&self) -> f32;

    /// Whether it holds the longest recording the server takes, and records no more.
    fn is_full(&self) -> bool;

    /// Stops and hands over what was recorded. It waits for the audio thread: call it off the
    /// UI thread.
    fn finish(self: Box<Self>) -> Captured;
}

/// A microphone that records the same thing each time, for the tests.
#[cfg(test)]
pub(crate) mod fake {
    use super::{Captured, MicError, Microphone, Recording};
    use crate::dictation::wav::max_seconds;

    /// It records `captured`, or, with none, finds no device.
    #[derive(Clone, Debug)]
    pub(crate) struct FakeMicrophone(Option<Captured>);

    impl FakeMicrophone {
        /// One that records `captured`.
        pub(crate) fn new(captured: Captured) -> Self {
            Self(Some(captured))
        }

        /// One that finds no device.
        pub(crate) fn none() -> Self {
            Self(None)
        }

        /// One that records `seconds` of a steady voice.
        pub(crate) fn speaking(seconds: u32) -> Self {
            let samples = vec![0.2; usize::try_from(seconds * 1000).unwrap_or_default()];
            Self::new(Captured {
                samples,
                rate: 1000,
            })
        }
    }

    struct FakeRecording(Captured);

    impl Microphone for FakeMicrophone {
        fn start(&self) -> Result<Box<dyn Recording>, MicError> {
            let captured = self.0.clone().ok_or(MicError::NoDevice)?;
            Ok(Box::new(FakeRecording(captured)))
        }
    }

    impl Recording for FakeRecording {
        fn level(&self) -> f32 {
            0.5
        }

        fn is_full(&self) -> bool {
            let samples = u64::try_from(self.0.samples.len()).unwrap_or(u64::MAX);
            samples >= max_seconds() * u64::from(self.0.rate)
        }

        fn finish(self: Box<Self>) -> Captured {
            self.0
        }
    }
}

/// The system's default input device, through `cpal`.
#[cfg(not(target_os = "macos"))]
mod system {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::{Arc, Mutex, PoisonError, TryLockError};

    use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
    use cpal::{FromSample, Sample, SampleFormat, SizedSample};

    use super::{MicError, Microphone, Recording};
    use crate::dictation::wav::{Captured, Loudness, max_seconds};

    /// How many seconds of samples a recording has room for from the start, so the audio
    /// thread seldom grows its buffer.
    const ROOM_AHEAD: u32 = 60;

    /// The system's default input device.
    #[derive(Clone, Copy, Debug, Default)]
    pub(crate) struct SystemMicrophone;

    impl Microphone for SystemMicrophone {
        fn start(&self) -> Result<Box<dyn Recording>, MicError> {
            let device = cpal::default_host()
                .default_input_device()
                .ok_or(MicError::NoDevice)?;
            let config = device.default_input_config().map_err(MicError::Device)?;
            let format = config.sample_format();
            let config: cpal::StreamConfig = config.into();
            let rate = config.sample_rate;
            let limit = usize::try_from(max_seconds() * u64::from(rate)).unwrap_or(usize::MAX);
            let ahead = usize::try_from(ROOM_AHEAD * rate).unwrap_or_default();
            let shared = Arc::new(Shared {
                samples: Mutex::new(Vec::with_capacity(ahead.min(limit))),
                level: AtomicU32::new(0),
                full: AtomicBool::new(false),
                limit,
            });
            let stream = match format {
                SampleFormat::F32 => open::<f32>(&device, &config, &shared)?,
                SampleFormat::I16 => open::<i16>(&device, &config, &shared)?,
                SampleFormat::U16 => open::<u16>(&device, &config, &shared)?,
                SampleFormat::I32 => open::<i32>(&device, &config, &shared)?,
                other => return Err(MicError::UnsupportedFormat(other)),
            };
            stream.play().map_err(MicError::Device)?;
            Ok(Box::new(SystemRecording {
                stream,
                shared,
                rate,
            }))
        }
    }

    /// What the audio thread hands the app.
    #[derive(Debug)]
    struct Shared {
        /// The samples so far, one channel.
        samples: Mutex<Vec<f32>>,
        /// The last stretch's loudness, as `f32` bits.
        level: AtomicU32,
        /// Whether `samples` reached `limit`.
        full: AtomicBool,
        /// The most samples kept: the longest recording the server takes.
        limit: usize,
    }

    /// Opens an input stream of `T` samples, each frame mixed down to one channel into `shared`.
    /// The callback takes the samples' lock only when it is free (finishing holds it), keeping
    /// a stretch in a buffer of its own meanwhile, and allocates only to grow a buffer.
    fn open<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        shared: &Arc<Shared>,
    ) -> Result<cpal::Stream, MicError>
    where
        T: SizedSample,
        f32: FromSample<T>,
    {
        let channels = usize::from(config.channels.max(1));
        let sink = Arc::clone(shared);
        let mut spare: Vec<f32> = Vec::new();
        device
            .build_input_stream(
                *config,
                move |data: &[T], _: &cpal::InputCallbackInfo| {
                    let mut guard = match sink.samples.try_lock() {
                        Ok(guard) => Some(guard),
                        Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
                        Err(TryLockError::WouldBlock) => None,
                    };
                    let buffer = match guard.as_deref_mut() {
                        Some(samples) => {
                            samples.append(&mut spare);
                            samples
                        }
                        None => &mut spare,
                    };
                    let mut loudness = Loudness::default();
                    for frame in data.chunks(channels) {
                        let sample = mix(frame);
                        loudness.add(sample);
                        if buffer.len() < sink.limit {
                            buffer.push(sample);
                        } else {
                            sink.full.store(true, Ordering::Relaxed);
                        }
                    }
                    sink.level
                        .store(loudness.level().to_bits(), Ordering::Relaxed);
                },
                |error| log::warn!("the microphone's stream failed: {error}"),
                None,
            )
            .map_err(MicError::Device)
    }

    /// A frame of samples, one a channel, as one sample: their mean.
    fn mix<T>(frame: &[T]) -> f32
    where
        T: Sample,
        f32: FromSample<T>,
    {
        let sum: f32 = frame.iter().map(|&sample| sample.to_sample::<f32>()).sum();
        #[expect(
            clippy::cast_precision_loss,
            reason = "a frame has a handful of channels"
        )]
        let count = frame.len().max(1) as f32;
        sum / count
    }

    /// A recording from the system's microphone.
    struct SystemRecording {
        stream: cpal::Stream,
        shared: Arc<Shared>,
        rate: u32,
    }

    impl Recording for SystemRecording {
        fn level(&self) -> f32 {
            f32::from_bits(self.shared.level.load(Ordering::Relaxed))
        }

        fn is_full(&self) -> bool {
            self.shared.full.load(Ordering::Relaxed)
        }

        fn finish(self: Box<Self>) -> Captured {
            if let Err(error) = self.stream.pause() {
                log::debug!("the microphone didn't pause: {error}");
            }
            drop(self.stream);
            let samples = std::mem::take(
                &mut *self
                    .shared
                    .samples
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner),
            );
            Captured {
                samples,
                rate: self.rate,
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_frame_mixes_down_to_its_mean() {
            assert!(mix(&[0.5_f32, -0.5]).abs() < f32::EPSILON);
            assert!((mix(&[1.0_f32, 0.0]) - 0.5).abs() < f32::EPSILON);
            let from_i16 = mix(&[i16::MAX, i16::MAX]);
            assert!((from_i16 - 1.0).abs() < 1e-3, "{from_i16}");
        }
    }
}
