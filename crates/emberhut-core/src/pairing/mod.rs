//! Pairing: how a new device gets a session, with a one-time code made somewhere already
//! trusted.
//!
//! A signed-in device (or the server's command line, for the very first device) makes a
//! [`PairingCode`]; the new device types it and gets its own session. A code works once, for
//! [`CODE_LIFETIME`], and only its hash is kept. Wrong codes are braked for every caller at
//! once ([`WrongCodeBrake`]).

use jiff::{SignedDuration, Timestamp};

use crate::ids::PairingCodeId;

mod code;
mod wrong_codes;

pub use code::{PairingCode, TypedCode};
pub use wrong_codes::{Attempt, Lockout, WrongCodeBrake};

/// How long a pairing code works.
pub const CODE_LIFETIME: SignedDuration = SignedDuration::from_mins(10);

/// A stored pairing code, as found by its hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PairingRecord {
    /// The code's id.
    pub id: PairingCodeId,
    /// Whether it has paired a device already.
    pub used: bool,
    /// When it stops working.
    pub expires_at: Timestamp,
}

impl PairingRecord {
    /// Whether the code can pair a device at `now`.
    ///
    /// # Errors
    ///
    /// [`PairingRefusal::Used`] or [`PairingRefusal::Expired`].
    pub fn check(&self, now: Timestamp) -> Result<(), PairingRefusal> {
        if self.used {
            Err(PairingRefusal::Used)
        } else if now >= self.expires_at {
            Err(PairingRefusal::Expired)
        } else {
            Ok(())
        }
    }
}

/// Why a typed pairing code was not taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PairingRefusal {
    /// Too many wrong codes were tried lately; no code is taken until `retry_after` has
    /// passed.
    #[error("too many wrong pairing codes; try again in {retry_after:#}")]
    TooManyWrongCodes {
        /// How long until a code is taken again.
        retry_after: SignedDuration,
    },
    /// No code was given.
    #[error("no pairing code given")]
    Missing,
    /// No such code was made.
    #[error("unknown pairing code")]
    Unknown,
    /// The code has paired a device already.
    #[error("pairing code already used")]
    Used,
    /// The code is too old.
    #[error("pairing code expired")]
    Expired,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_second(1_800_000_000 + seconds).unwrap()
    }

    #[test]
    fn a_code_works_once_until_it_expires() {
        let mut record = PairingRecord {
            id: PairingCodeId::new(1),
            used: false,
            expires_at: at(600),
        };
        assert_eq!(record.check(at(599)), Ok(()));
        assert_eq!(record.check(at(600)), Err(PairingRefusal::Expired));
        record.used = true;
        assert_eq!(record.check(at(0)), Err(PairingRefusal::Used));
    }
}
