//! Who an account token belongs to: the [`AccountVerifier`] trait, and Clerk's implementation
//! of it, [`ClerkVerifier`] (ADR 0007, section 5, "Checking Clerk tokens"). Another account
//! provider replaces Clerk by implementing the trait.

use std::future::Future;
use std::time::Duration;

use emberhut_identity::AccountId;

mod clerk;
mod jwks;

pub use clerk::{ClerkSettings, ClerkSetupError, ClerkVerifier, JwksUrl, JwksUrlError};
pub use jwks::KeyTiming;

/// Checks an account token and says whose account it is.
pub trait AccountVerifier: Send + Sync + 'static {
    /// The account `token` (the bearer token, as sent) was issued for.
    ///
    /// # Errors
    ///
    /// [`AccountRefusal::SignIn`] for a token that fails a check, and
    /// [`AccountRefusal::Unavailable`] when it can't be checked right now.
    fn verify(&self, token: &str)
    -> impl Future<Output = Result<AccountId, AccountRefusal>> + Send;
}

/// Why a token was not taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AccountRefusal {
    /// The token failed a check: the client signs in to the account again (401).
    #[error("the account token failed a check")]
    SignIn,
    /// The token can't be checked right now (the provider's keys can't be had): the client
    /// tries again after `retry_after` and stays signed in (503).
    #[error("the account token can't be checked right now")]
    Unavailable {
        /// When to try again.
        retry_after: Duration,
    },
}
