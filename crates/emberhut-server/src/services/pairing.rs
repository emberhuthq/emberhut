//! Pairing codes: making them, and trading a typed one for a session (the rules are
//! [`emberhut_core::pairing`]'s).

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use emberhut_core::pairing::{
    Attempt, CODE_LIFETIME, PairingCode, PairingRefusal, TypedCode, WrongCodeBrake,
};
use emberhut_core::session::{IssuedTokens, Session};
use emberhut_core::{Clock, DeviceDetails, DeviceId, RandomError, RandomSource};
use emberhut_db::{DbError, PairingStore, Redemption};
use jiff::Timestamp;

/// Who made a pairing code, for the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CodeMaker {
    /// A signed-in device.
    Device(DeviceId),
    /// The server's command line, on the machine it runs on.
    CommandLine,
}

impl fmt::Display for CodeMaker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Device(device) => write!(f, "device {device}"),
            Self::CommandLine => f.write_str("the command line"),
        }
    }
}

/// A code that was just made, and when it stops working.
#[derive(Clone, Debug)]
pub struct IssuedCode {
    /// The code, to show once.
    pub code: PairingCode,
    /// When it stops working.
    pub expires_at: Timestamp,
}

/// Why a pairing code was not taken, or could not be made.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PairingError {
    /// The code does not work (see [`PairingRefusal`]).
    #[error(transparent)]
    Refused(#[from] PairingRefusal),
    /// A code or tokens could not be made.
    #[error("could not make a code or tokens")]
    Random(#[from] RandomError),
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
}

/// Makes pairing codes and trades typed ones for sessions.
#[derive(Debug)]
pub struct PairingService {
    codes: PairingStore,
    clock: Arc<dyn Clock>,
    random: Arc<dyn RandomSource>,
    brake: Mutex<WrongCodeBrake>,
}

impl PairingService {
    /// A service over `codes` that tells the time with `clock` and makes codes and tokens
    /// from `random`.
    #[must_use]
    pub fn new(codes: PairingStore, clock: Arc<dyn Clock>, random: Arc<dyn RandomSource>) -> Self {
        Self {
            codes,
            clock,
            random,
            brake: Mutex::default(),
        }
    }

    /// Makes a one-time code for a new device, clearing out expired ones on the way.
    ///
    /// # Errors
    ///
    /// [`PairingError::Random`] or [`PairingError::Database`].
    pub async fn new_code(&self, made_by: CodeMaker) -> Result<IssuedCode, PairingError> {
        let now = self.clock.now();
        self.codes.delete_expired(now).await?;
        let code = PairingCode::generate(self.random.as_ref())?;
        let expires_at = now + CODE_LIFETIME;
        self.codes.insert(&code.hash(), now, expires_at).await?;
        tracing::info!(%made_by, "made a pairing code for a new device");
        Ok(IssuedCode { code, expires_at })
    }

    /// Trades a typed code for a session for the device that typed it. The code is used up in
    /// the same transaction that opens the session.
    ///
    /// # Errors
    ///
    /// [`PairingError::Refused`] for a code that does not work, and while too many wrong codes
    /// are recent even for the right one; [`PairingError::Random`] or
    /// [`PairingError::Database`] when the server fails.
    pub async fn redeem(
        &self,
        typed: &str,
        device: &DeviceDetails,
    ) -> Result<Session, PairingError> {
        let now = self.clock.now();
        // The try counts as wrong from here, before the lookup, so tries sent all at once are
        // braked like tries sent one after another.
        let attempt = self.begin(now)?;
        let Some(code) = TypedCode::parse(typed) else {
            self.brake().forgive(attempt);
            return Err(PairingRefusal::Missing.into());
        };
        // A code that does not exist (or a lookup that fails) leaves the try counted.
        let Some(record) = self.codes.find(&code.hash()).await? else {
            return Err(PairingRefusal::Unknown.into());
        };
        self.brake().forgive(attempt);
        record.check(now)?;

        let tokens = IssuedTokens::issue(self.random.as_ref(), now)?;
        let device = match self.codes.redeem(record.id, device, &tokens, now).await? {
            Redemption::Paired(device) => device,
            Redemption::AlreadyUsed => return Err(PairingRefusal::Used.into()),
            Redemption::Expired => return Err(PairingRefusal::Expired.into()),
        };
        tracing::info!(%device, "paired a device with a one-time code");
        Ok(Session { device, tokens })
    }

    /// The refusal for a request with no code (missing, not text, or blank): while the brake
    /// is on, that it is on, as for any code; otherwise "type the code", not counted.
    #[must_use]
    pub fn refuse_missing(&self) -> PairingError {
        match self.begin(self.clock.now()) {
            Ok(attempt) => {
                self.brake().forgive(attempt);
                PairingRefusal::Missing.into()
            }
            Err(braked) => braked,
        }
    }

    /// The refusal for a code no code can be (longer than any code typed): a wrong code,
    /// counted on the brake like any other.
    #[must_use]
    pub fn refuse_wrong(&self) -> PairingError {
        match self.begin(self.clock.now()) {
            // The try stays counted.
            Ok(_counted) => PairingRefusal::Unknown.into(),
            Err(braked) => braked,
        }
    }

    /// Starts a try on the brake, or the lockout while it is on.
    fn begin(&self, now: Timestamp) -> Result<Attempt, PairingError> {
        self.brake().begin(now).map_err(|lockout| {
            if lockout.first {
                tracing::warn!(
                    "too many wrong pairing codes: no code is taken for up to ten minutes \
                     (restarting the server lifts the brake)"
                );
            }
            PairingRefusal::TooManyWrongCodes {
                retry_after: lockout.retry_after,
            }
            .into()
        })
    }

    fn brake(&self) -> MutexGuard<'_, WrongCodeBrake> {
        // The brake is a list of instants: a panic elsewhere cannot leave it half-written.
        self.brake.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
