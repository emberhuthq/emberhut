//! What the push services' tokens are made of: key files, PEM keys, base64url and compact JWTs
//! signed with ring.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use serde_json::Value;

/// Why a signed token could not be made.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum KeyError {
    /// The key's file could not be read.
    #[error("the key file could not be read")]
    Read(#[source] std::io::Error),
    /// The text is not a PEM `PRIVATE KEY` block (PKCS #8).
    #[error("not a PEM private key (PKCS #8)")]
    NotPem,
    /// The key is not one of the kind the service signs with.
    #[error("the key was not accepted: {0}")]
    Rejected(String),
    /// Signing failed.
    #[error("signing failed")]
    Sign,
}

/// The DER bytes of the PEM `PRIVATE KEY` block in `text` (a PKCS #8 key, as Apple's `.p8`
/// files and Google's service account keys hold).
///
/// # Errors
///
/// [`KeyError::NotPem`] when `text` holds no such block, or its body is not base64.
pub fn pkcs8_der(text: &str) -> Result<Vec<u8>, KeyError> {
    const BEGIN: &str = "-----BEGIN PRIVATE KEY-----";
    const END: &str = "-----END PRIVATE KEY-----";
    let start = text.find(BEGIN).ok_or(KeyError::NotPem)? + BEGIN.len();
    let length = text[start..].find(END).ok_or(KeyError::NotPem)?;
    let body: String = text[start..start + length]
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    STANDARD.decode(body).map_err(|_| KeyError::NotPem)
}

/// `bytes` in base64url without padding, as JWTs write them.
#[must_use]
pub fn base64url(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The signed part of a compact JWT: its header and claims, each base64url-encoded JSON,
/// joined by a dot.
#[must_use]
pub fn signing_input(header: &Value, claims: &Value) -> String {
    format!(
        "{}.{}",
        base64url(header.to_string().as_bytes()),
        base64url(claims.to_string().as_bytes())
    )
}

/// How long a key file's text, once read, is used before the file is read again.
pub const KEY_RECHECK: Duration = Duration::from_secs(60);

/// A key file in the secrets directory. It is read with tokio's file system calls, which run
/// off the async workers, and its text is kept for [`KEY_RECHECK`], so most pushes and state
/// polls read nothing. A file that is not there is looked for again at its next use, so a key
/// put there later is used without a restart.
#[derive(Debug)]
pub(crate) struct KeyFile {
    path: PathBuf,
    kept: Mutex<Option<(Arc<str>, Instant)>>,
}

impl KeyFile {
    /// The key file at `path`, not read yet.
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            kept: Mutex::new(None),
        }
    }

    /// Its text: the text kept from a read within [`KEY_RECHECK`], or the file's, read now.
    ///
    /// # Errors
    ///
    /// [`KeyError::Read`] when the file can't be read (it isn't there yet, most often).
    pub(crate) async fn text(&self) -> Result<Arc<str>, KeyError> {
        if let Some((text, read_at)) = self.kept().as_ref()
            && read_at.elapsed() < KEY_RECHECK
        {
            return Ok(Arc::clone(text));
        }
        match tokio::fs::read_to_string(&self.path).await {
            Ok(text) => {
                let text: Arc<str> = text.into();
                *self.kept() = Some((Arc::clone(&text), Instant::now()));
                Ok(text)
            }
            Err(error) => {
                self.forget();
                Err(KeyError::Read(error))
            }
        }
    }

    /// Drops the kept text, so the next use reads the file again (the service refused the
    /// key, or a token made with it).
    pub(crate) fn forget(&self) {
        *self.kept() = None;
    }

    fn kept(&self) -> std::sync::MutexGuard<'_, Option<(Arc<str>, Instant)>> {
        self.kept.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
