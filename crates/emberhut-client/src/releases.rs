//! The release feed and its payloads (`/api/releases`): the feed of a platform in the newest
//! format the caller reads, and a payload streamed to disk under a hard cap, hashed on the way.
//!
//! Neither is trusted here: the app checks a payload's size, hash and signature itself. This
//! module only makes sure the bytes come from this server, with this device's session, and
//! that no more than the caller allows ever reach the disk.

use std::io::{Read as _, Write};

use emberhut_api::paths;
use emberhut_api::releases::{Platform, ReleaseFeed};
use sha2::{Digest as _, Sha256};

use crate::client::server_path;
use crate::transport::TransportError;
use crate::{Client, Error, exchange};

/// A payload streamed to disk: how many bytes came, and their SHA-256.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Downloaded {
    /// The bytes written.
    pub size: u64,
    /// The SHA-256 of those bytes.
    pub sha256: [u8; 32],
}

impl Client {
    /// The release feed of `platform` (`GET /api/releases/v{format}/{platform}`), asking for
    /// `format` first and, while the server answers `404`, for each lower format down to 1. A
    /// server that has published nothing in any of them gives `None`.
    ///
    /// The feed's own `format` is not checked: one higher than asked means a misconfigured
    /// server, and what to do about it is the caller's to say.
    ///
    /// # Errors
    ///
    /// [`Error::SignInRequired`], [`Error::Status`] (other than 404), the transport's and
    /// decoding errors.
    pub fn release_feed(
        &self,
        platform: &Platform,
        format: u32,
    ) -> Result<Option<ReleaseFeed>, Error> {
        for format in (1..=format).rev() {
            match exchange::json(self.get(&paths::release_feed(format, platform), "")?) {
                Ok(feed) => return Ok(Some(feed)),
                Err(error) if error.status() == Some(404) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(None)
    }

    /// Streams the payload at the server path `path` into `to`, never more than `max_len`
    /// bytes, hashing it as it goes. The request asks for the bytes as they are
    /// (`Accept-Encoding: identity`), with no overall timeout, only one per read.
    ///
    /// A payload whose `Content-Length` is over `max_len` is refused before a byte is read; one
    /// sent without a size is cut off as it passes `max_len`. Whether the size and hash are the
    /// ones expected is the caller's to check. What `to` holds after an error is a part of the
    /// payload: the caller deletes it.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidPath`] for anything but a server path; [`Error::TooLarge`] past
    /// `max_len`; [`Error::Write`] when `to` fails; [`Error::SignInRequired`],
    /// [`Error::Status`] (404 for a payload that isn't there), the transport's errors.
    pub fn download_release(
        &self,
        path: &str,
        to: &mut (impl Write + ?Sized),
        max_len: u64,
    ) -> Result<Downloaded, Error> {
        let (path, query) = server_path(path)?;
        let response = self.get_file(path, query)?;
        if !response.status().is_success() {
            return Err(exchange::refusal(response));
        }
        let announced = response
            .headers()
            .get("content-length")
            .and_then(|length| length.to_str().ok()?.parse::<u64>().ok());
        if let Some(size) = announced.filter(|&size| size > max_len) {
            return Err(Error::TooLarge {
                size,
                limit: max_len,
            });
        }
        let mut body = response.into_body();
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; 64 << 10];
        let mut size = 0_u64;
        loop {
            let read = match body.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(TransportError::from(error).into()),
            };
            size = size.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
            if size > max_len {
                return Err(Error::TooLarge {
                    size,
                    limit: max_len,
                });
            }
            let chunk = &buffer[..read];
            hasher.update(chunk);
            to.write_all(chunk).map_err(Error::Write)?;
        }
        to.flush().map_err(Error::Write)?;
        Ok(Downloaded {
            size,
            sha256: hasher.finalize().into(),
        })
    }
}
