//! The operating system's random source.

use std::io;

/// The operating system couldn't provide randomness. Its source is the operating system's
/// error, when it gave one.
#[derive(Debug, thiserror::Error)]
#[error("the operating system's random source failed")]
pub struct RandomError {
    #[source]
    source: io::Error,
}

impl RandomError {
    fn new(error: getrandom::Error) -> Self {
        let source = error.raw_os_error().map_or_else(
            || io::Error::other(error.to_string()),
            io::Error::from_raw_os_error,
        );
        Self { source }
    }
}

/// Fills `bytes` from the operating system's random source.
pub(crate) fn fill_random(bytes: &mut [u8]) -> Result<(), RandomError> {
    getrandom::getrandom(bytes).map_err(RandomError::new)
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::*;

    #[test]
    fn the_os_error_is_the_source() {
        let error = RandomError::new(getrandom::Error::UNSUPPORTED);
        assert!(error.source().is_some());
        assert_eq!(
            error.to_string(),
            "the operating system's random source failed"
        );
    }
}
