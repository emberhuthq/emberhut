//! What a recording becomes before it goes to the server: one channel at 16 kHz in 16-bit PCM,
//! as a WAV file (speech-to-text needs no more, and it keeps an upload of a few minutes small),
//! no longer than the server takes; and how loud a stretch of it is, for the waveform.

use emberhut_api::transcribe::MAX_SIZE;

/// The rate the server is sent: speech needs no more.
pub(crate) const RATE: u32 = 16_000;

/// A WAV file's header, in bytes.
const HEADER: u64 = 44;

/// Bytes a sample, in 16-bit PCM.
const SAMPLE_BYTES: u64 = 2;

/// The longest recording the server takes, in whole seconds: as many as fit in a WAV under its
/// limit ([`MAX_SIZE`], 25 MiB) at [`RATE`], 819 (about 13.6 minutes).
#[must_use]
pub(crate) fn max_seconds() -> u64 {
    (MAX_SIZE - HEADER) / SAMPLE_BYTES / u64::from(RATE)
}

/// What the microphone captured: its samples, `-1.0..=1.0`, one channel, at `rate` per second.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Captured {
    /// The samples.
    pub(crate) samples: Vec<f32>,
    /// Samples a second.
    pub(crate) rate: u32,
}

impl Captured {
    /// How long it lasts, in seconds.
    #[must_use]
    pub(crate) fn seconds(&self) -> f64 {
        if self.rate == 0 {
            return 0.0;
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "exact below 2^53 samples, about 5,900 years at 48 kHz"
        )]
        let samples = self.samples.len() as f64;
        samples / f64::from(self.rate)
    }

    /// Whether nothing at all came in: every sample exactly zero. A real microphone always
    /// picks up some noise; the Mac hands out pure silence when the app may not use it.
    #[must_use]
    pub(crate) fn is_silent(&self) -> bool {
        self.samples.iter().all(|&sample| sample == 0.0)
    }

    /// The recording at [`RATE`]: each new sample the mean of the stretch it stands for (a
    /// box filter, enough against aliasing for speech), or the samples as they are when the
    /// rate is that already or lower.
    #[must_use]
    pub(crate) fn resampled(&self) -> Vec<f32> {
        if self.rate <= RATE {
            return self.samples.clone();
        }
        let (from, to) = (u64::from(self.rate), u64::from(RATE));
        let len = u64::try_from(self.samples.len()).unwrap_or(u64::MAX);
        let out_len = len * to / from;
        (0..out_len)
            .map(|i| {
                let start = usize::try_from(i * from / to).unwrap_or(usize::MAX);
                let end = usize::try_from((i + 1) * from / to)
                    .unwrap_or(usize::MAX)
                    .min(self.samples.len());
                mean(self.samples.get(start..end).unwrap_or_default())
            })
            .collect()
    }

    /// The recording as a WAV file: one channel, 16-bit, at [`RATE`] (or its own rate when that
    /// is lower), cut at [`max_seconds`] so that the server takes it.
    #[must_use]
    pub(crate) fn to_wav(&self) -> Vec<u8> {
        let rate = self.rate.min(RATE);
        let mut samples = self.resampled();
        let longest = usize::try_from(max_seconds() * u64::from(rate)).unwrap_or(usize::MAX);
        samples.truncate(longest);
        // Under 25 MiB once cut, so the header's sizes fit in a u32 with room to spare.
        let data_len = u32::try_from(samples.len() * 2).unwrap_or(u32::MAX);
        let riff_len = data_len.saturating_add(36);
        let mut wav = Vec::with_capacity(44 + samples.len() * 2);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&riff_len.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        // PCM, one channel.
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        // Bytes a second, bytes a frame, bits a sample.
        wav.extend_from_slice(&(rate * 2).to_le_bytes());
        wav.extend_from_slice(&2_u16.to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        for sample in samples {
            wav.extend_from_slice(&to_i16(sample).to_le_bytes());
        }
        wav
    }
}

/// The mean of `samples` (0 for none).
fn mean(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "a stretch of samples, far fewer than f32 counts exactly (2^24)"
    )]
    let count = samples.len() as f32;
    samples.iter().sum::<f32>() / count
}

/// A sample as 16-bit PCM, clipped.
fn to_i16(sample: f32) -> i16 {
    let scaled = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round();
    // In range after the clamp: the cast only drops the fraction `round` already removed.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "clamped to i16's range and rounded first"
    )]
    let value = scaled as i16;
    value
}

/// How loud a stretch of samples is, taken in one pass as they come (the audio thread
/// allocates nothing for it). Only the microphone outside macOS measures it.
#[cfg(any(test, not(target_os = "macos")))]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Loudness {
    /// The sum of the samples' squares.
    squares: f32,
    /// How many samples.
    count: u32,
}

#[cfg(any(test, not(target_os = "macos")))]
impl Loudness {
    /// One more sample.
    pub(crate) fn add(&mut self, sample: f32) {
        self.squares += sample * sample;
        self.count = self.count.saturating_add(1);
    }

    /// How loud the samples were, `0.0..=1.0`, as the waveform draws it: their RMS in
    /// decibels, -50 dB (a quiet room) as 0 and -5 dB (shouting close by) as 1.
    #[must_use]
    pub(crate) fn level(self) -> f32 {
        if self.count == 0 {
            return 0.0;
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "an audio buffer's samples, far fewer than f32 counts exactly (2^24)"
        )]
        let count = self.count as f32;
        let rms = (self.squares / count).sqrt();
        if rms <= 0.0 {
            return 0.0;
        }
        let decibels = 20.0 * rms.log10();
        ((decibels + 50.0) / 45.0).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn captured(samples: Vec<f32>, rate: u32) -> Captured {
        Captured { samples, rate }
    }

    fn level(samples: &[f32]) -> f32 {
        let mut loudness = Loudness::default();
        for &sample in samples {
            loudness.add(sample);
        }
        loudness.level()
    }

    #[test]
    fn the_longest_recording_fits_under_the_servers_limit() {
        assert_eq!(max_seconds(), 819);
        let size = HEADER + SAMPLE_BYTES * max_seconds() * u64::from(RATE);
        assert!(size <= MAX_SIZE, "{size}");
        assert!(
            size + SAMPLE_BYTES * u64::from(RATE) > MAX_SIZE,
            "a second more won't fit"
        );
    }

    #[test]
    fn a_wav_is_cut_at_the_longest_the_server_takes() {
        let rate = 100;
        let longest = usize::try_from(max_seconds() * u64::from(rate)).unwrap();
        let wav = captured(vec![0.1; longest + 50], rate).to_wav();
        assert_eq!(wav.len(), 44 + 2 * longest);
        assert_eq!(
            u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]),
            u32::try_from(2 * longest).unwrap()
        );
    }

    #[test]
    fn a_recording_knows_how_long_it_is() {
        assert!((captured(vec![0.1; 24_000], 48_000).seconds() - 0.5).abs() < 1e-9);
        assert!(captured(Vec::new(), 0).seconds().abs() < f64::EPSILON);
    }

    #[test]
    fn pure_silence_is_a_microphone_held_back() {
        assert!(captured(vec![0.0; 100], 16_000).is_silent());
        assert!(!captured(vec![0.0, 0.0001], 16_000).is_silent());
    }

    #[test]
    fn a_recording_comes_down_to_sixteen_kilohertz() {
        let samples: Vec<f32> = (0..48)
            .map(|i| if i % 3 == 0 { 0.3 } else { 0.0 })
            .collect();
        let resampled = captured(samples, 48_000).resampled();
        assert_eq!(resampled.len(), 16);
        assert!(
            resampled.iter().all(|&s| (s - 0.1).abs() < 1e-6),
            "{resampled:?}"
        );
        let low = captured(vec![0.5; 8], 8_000);
        assert_eq!(low.resampled(), low.samples, "not raised");
    }

    #[test]
    fn the_wav_has_a_header_and_sixteen_bit_samples() {
        let wav = captured(vec![1.0, -1.0, 0.0, 2.0], 16_000).to_wav();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes([wav[4], wav[5], wav[6], wav[7]]), 36 + 8);
        assert_eq!(
            u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]),
            16_000
        );
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]), 8);
        let samples: Vec<i16> = wav[44..]
            .chunks(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(samples, [i16::MAX, -i16::MAX, 0, i16::MAX], "clipped");
    }

    #[test]
    fn loudness_runs_from_a_quiet_room_to_a_shout() {
        assert!(level(&[]).abs() < f32::EPSILON);
        assert!(level(&[0.0; 10]).abs() < f32::EPSILON);
        assert!(level(&[0.001; 10]) < 0.3, "-60 dB is quiet");
        assert!(
            (level(&[1.0; 10]) - 1.0).abs() < f32::EPSILON,
            "0 dB is loud"
        );
        let middle = level(&[0.03; 10]);
        assert!(middle > 0.3 && middle < 0.5, "{middle}");
    }
}
