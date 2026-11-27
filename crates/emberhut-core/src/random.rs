//! Where secrets get their randomness, behind a trait so a test can supply its own bytes.

use std::error::Error;
use std::fmt;

/// A source of random bytes fit for secrets.
pub trait RandomSource: fmt::Debug + Send + Sync {
    /// Fills `bytes` with random bytes.
    ///
    /// # Errors
    ///
    /// [`RandomError`] when no randomness is to be had.
    fn fill(&self, bytes: &mut [u8]) -> Result<(), RandomError>;
}

/// The operating system's generator: what the server uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OsRandom;

impl RandomSource for OsRandom {
    fn fill(&self, bytes: &mut [u8]) -> Result<(), RandomError> {
        getrandom::getrandom(bytes).map_err(RandomError::new)
    }
}

/// No random bytes could be had.
#[derive(Debug, thiserror::Error)]
#[error("no random bytes could be had")]
pub struct RandomError(#[source] Box<dyn Error + Send + Sync>);

impl RandomError {
    /// The error for a source that failed with `cause`.
    pub fn new(cause: impl Error + Send + Sync + 'static) -> Self {
        Self(Box::new(cause))
    }
}

/// `N` random bytes from `random`.
pub(crate) fn bytes<const N: usize>(random: &dyn RandomSource) -> Result<[u8; N], RandomError> {
    let mut bytes = [0; N];
    random.fill(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_random_bytes_differ() {
        let a = bytes::<32>(&OsRandom).unwrap();
        let b = bytes::<32>(&OsRandom).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn error_keeps_its_cause() {
        let error = RandomError::new(std::io::Error::other("no entropy"));
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("no entropy")
        );
    }
}
