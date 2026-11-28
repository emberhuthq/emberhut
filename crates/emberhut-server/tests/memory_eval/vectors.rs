//! A real embedding model's vectors for every text the eval embeds, recorded once
//! (`data/vectors.bin`), so the meaning search runs in CI without the model.
//!
//! Recording: run the eval with `EMBERHUT_EVAL_EMBED_URL` set to an OpenAI-style embeddings
//! endpoint (`http://127.0.0.1:8795`, the one recall uses). Every text is then embedded by it and
//! the file is written anew with exactly the texts this run asked for. Without it, the vectors
//! come from the file, and a text it doesn't hold is counted as missed (the eval then fails and
//! says to record again: what is embedded changed).
//!
//! The file: `EHV1`, the dimension (u32), then per text its FNV-1a hash (u64), a scale (f32) and
//! the vector as `i8`s (each part = `i8` × scale). Little-endian throughout.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use emberhut_server::services::recall::{EmbedError, Embedder, HttpEmbedder};
use futures_util::future::BoxFuture;

use crate::corpus::data;

const MAGIC: &[u8; 4] = b"EHV1";

fn file() -> PathBuf {
    data().join("vectors.bin")
}

/// FNV-1a, 64 bits: stable across builds and platforms.
fn hash(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        h ^= u64::from(*byte);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn quantise(vector: &[f32]) -> (f32, Vec<i8>) {
    let top = vector.iter().fold(0.0_f32, |top, x| top.max(x.abs()));
    let scale = if top > 0.0 { top / 127.0 } else { 1.0 };
    #[expect(
        clippy::cast_possible_truncation,
        reason = "rounded and within -127..=127"
    )]
    let parts = vector
        .iter()
        .map(|x| (x / scale).round().clamp(-127.0, 127.0) as i8)
        .collect();
    (scale, parts)
}

#[derive(Debug)]
pub(crate) struct Recorded {
    dimension: usize,
    known: HashMap<u64, Vec<f32>>,
    live: Option<HttpEmbedder>,
    learned: Mutex<HashMap<u64, Vec<f32>>>,
    missed: Mutex<Vec<String>>,
}

impl Recorded {
    /// The recorded vectors, or the live model when `EMBERHUT_EVAL_EMBED_URL` is set.
    pub(crate) fn open() -> Self {
        let live = std::env::var("EMBERHUT_EVAL_EMBED_URL")
            .ok()
            .filter(|url| !url.is_empty())
            .map(|url| {
                // Whichever provider is installed first serves; the eval needs no other.
                let _installed = rustls::crypto::ring::default_provider().install_default();
                HttpEmbedder::new(reqwest::Client::new(), &url, None)
            });
        let mut known = HashMap::new();
        let mut dimension = 0;
        if let Ok(bytes) = std::fs::read(file())
            && bytes.get(..4) == Some(MAGIC.as_slice())
        {
            dimension = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
            for entry in bytes[8..].chunks_exact(12 + dimension) {
                let key = u64::from_le_bytes(entry[..8].try_into().unwrap());
                let scale = f32::from_le_bytes(entry[8..12].try_into().unwrap());
                let vector = entry[12..]
                    .iter()
                    .map(|part| f32::from(part.cast_signed()) * scale)
                    .collect();
                known.insert(key, vector);
            }
        }
        Self {
            dimension,
            known,
            live,
            learned: Mutex::new(HashMap::new()),
            missed: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn recording(&self) -> bool {
        self.live.is_some()
    }

    pub(crate) fn available(&self) -> bool {
        self.live.is_some() || !self.known.is_empty()
    }

    /// The texts asked for that the file doesn't hold.
    pub(crate) fn missed(&self) -> Vec<String> {
        self.missed.lock().unwrap().clone()
    }

    /// Writes every vector this run learned from the live model (when recording).
    pub(crate) fn save(&self) {
        if self.live.is_none() {
            return;
        }
        let learned = self.learned.lock().unwrap();
        let dimension = learned.values().next().map_or(0, Vec::len);
        let mut keys: Vec<&u64> = learned.keys().collect();
        keys.sort_unstable();
        let mut out = MAGIC.to_vec();
        out.extend(u32::try_from(dimension).unwrap().to_le_bytes());
        for key in keys {
            let (scale, parts) = quantise(&learned[key]);
            out.extend(key.to_le_bytes());
            out.extend(scale.to_le_bytes());
            out.extend(parts.iter().map(|part| part.cast_unsigned()));
        }
        std::fs::write(file(), out).unwrap();
    }
}

impl Embedder for Recorded {
    fn embed<'a>(
        &'a self,
        texts: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<Vec<f32>>, EmbedError>> {
        Box::pin(async move {
            if let Some(live) = &self.live {
                let vectors = live.embed(texts).await?;
                let mut learned = self.learned.lock().unwrap();
                for (text, vector) in texts.iter().zip(&vectors) {
                    learned.insert(hash(text), vector.clone());
                }
                return Ok(vectors);
            }
            Ok(texts
                .iter()
                .map(|text| {
                    self.known.get(&hash(text)).cloned().unwrap_or_else(|| {
                        self.missed.lock().unwrap().push(text.clone());
                        vec![0.0; self.dimension.max(1)]
                    })
                })
                .collect())
        })
    }
}
