//! The live transcript's chunks: about ten seconds of a track at a time, as 16-bit mono WAV at
//! the track's own rate. A quiet chunk is not sent.

/// A chunk's length, in seconds.
pub const CHUNK_SECONDS: f64 = 10.0;
/// A chunk at or below this loudness (root mean square, full scale 1) is silence.
const QUIET: f64 = 0.002;

/// A chunk ready to send.
#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    /// Seconds from the call's start it begins at.
    pub start: f64,
    /// Its audio as a WAV file.
    pub wav: Vec<u8>,
}

/// One track's samples since its last chunk.
#[derive(Clone, Debug, Default)]
pub struct Chunker {
    start: f64,
    rate: f64,
    samples: Vec<i16>,
    energy: f64,
}

impl Chunker {
    /// Adds `samples` (floats, full scale 1) at `rate` Hz that begin `at` seconds into the
    /// call; a chunk once ten seconds are in and they weren't silence.
    pub fn feed(&mut self, at: f64, rate: f64, samples: &[f32]) -> Option<Chunk> {
        if self.samples.is_empty() {
            self.start = at;
            self.rate = rate;
            self.energy = 0.0;
        }
        for &sample in samples {
            let v = f64::from(sample.clamp(-1.0, 1.0));
            self.energy += v * v;
            // In range by the clamp above.
            #[expect(clippy::cast_possible_truncation, reason = "clamped to ±32767")]
            self.samples.push((v * 32767.0).round() as i16);
        }
        let count = self.samples.len();
        // A sample count far below 2^52 converts exactly.
        #[expect(clippy::cast_precision_loss, reason = "a few hundred thousand samples")]
        let seconds = count as f64 / self.rate.max(1.0);
        if seconds < CHUNK_SECONDS {
            return None;
        }
        #[expect(clippy::cast_precision_loss, reason = "a few hundred thousand samples")]
        let loudness = (self.energy / count as f64).sqrt();
        let samples = std::mem::take(&mut self.samples);
        if loudness <= QUIET {
            return None;
        }
        // Rates are whole hertz (48 000, 44 100).
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "an audio rate, a whole number of hertz well inside u32"
        )]
        let rate = self.rate.round().clamp(1.0, f64::from(u32::MAX)) as u32;
        Some(Chunk {
            start: self.start,
            wav: wav(&samples, rate),
        })
    }
}

/// `samples` as a 16-bit mono PCM WAV file at `rate` Hz.
#[must_use]
pub fn wav(samples: &[i16], rate: u32) -> Vec<u8> {
    let data = u32::try_from(samples.len() * 2).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&data.saturating_add(36).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1_u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&rate.saturating_mul(2).to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes()); // bytes a frame
    out.extend_from_slice(&16_u16.to_le_bytes()); // bits a sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

/// The first channel of `frames` frames of 32-bit float audio in `bytes` (native byte order):
/// every `channels`-th sample when interleaved, else the first `frames` samples (the first
/// channel's buffer comes first).
#[must_use]
pub fn first_channel(bytes: &[u8], frames: usize, channels: usize, interleaved: bool) -> Vec<f32> {
    let samples = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_ne_bytes(*b));
    if interleaved && channels > 1 {
        samples.step_by(channels).take(frames).collect()
    } else {
        samples.take(frames).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wav_has_its_header_and_samples() {
        let file = wav(&[0, 1, -1], 48_000);
        assert_eq!(file.len(), 44 + 6);
        assert_eq!(&file[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes([file[4], file[5], file[6], file[7]]), 42);
        assert_eq!(&file[8..16], b"WAVEfmt ");
        assert_eq!(
            u32::from_le_bytes([file[24], file[25], file[26], file[27]]),
            48_000
        );
        assert_eq!(&file[36..40], b"data");
        assert_eq!(
            u32::from_le_bytes([file[40], file[41], file[42], file[43]]),
            6
        );
        assert_eq!(&file[44..], &[0, 0, 1, 0, 0xff, 0xff]);
    }

    #[test]
    fn ten_seconds_make_a_chunk_from_their_start() {
        let mut chunker = Chunker::default();
        let second = vec![0.5_f32; 100];
        for i in 0..9 {
            assert_eq!(chunker.feed(2.0 + f64::from(i), 100.0, &second), None);
        }
        let chunk = chunker.feed(11.0, 100.0, &second).unwrap();
        assert!((chunk.start - 2.0).abs() < f64::EPSILON);
        assert_eq!(chunk.wav.len(), 44 + 1000 * 2);
        // The next chunk starts afresh.
        assert_eq!(chunker.feed(12.0, 100.0, &second), None);
        assert!((chunker.start - 12.0).abs() < f64::EPSILON);
    }

    #[test]
    fn the_first_channel_is_taken_interleaved_or_not() {
        let bytes: Vec<u8> = [1.0_f32, 2.0, 3.0, 4.0]
            .iter()
            .flat_map(|s| s.to_ne_bytes())
            .collect();
        assert_eq!(first_channel(&bytes, 2, 2, true), [1.0, 3.0]);
        assert_eq!(first_channel(&bytes, 2, 2, false), [1.0, 2.0]);
        assert_eq!(first_channel(&bytes, 9, 1, false), [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn silence_is_not_sent() {
        let mut chunker = Chunker::default();
        assert_eq!(chunker.feed(0.0, 10.0, &[0.0; 100]), None);
        assert_eq!(chunker.samples.len(), 0);
    }
}
