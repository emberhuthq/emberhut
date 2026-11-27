//! Device sessions: opening, renewing, checking and ending them (the rules are
//! [`emberhut_core::session`]'s).

use std::sync::Arc;

use emberhut_core::session::{DeviceSummary, IssuedTokens, RefreshMatch, RefreshVerdict, Session};
use emberhut_core::{Clock, DeviceDetails, DeviceId, RandomError, RandomSource, SecretHash};
use emberhut_db::{DbError, SessionStore};

/// Why a session could not be opened or renewed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AuthError {
    /// The refresh token is not current: never issued, already used (which, past a short
    /// grace, also signs its device out), or its device was signed out (also for being idle
    /// too long). The device has to prove itself again.
    #[error("refresh token not accepted")]
    RefreshRejected,
    /// Tokens could not be made.
    #[error("could not make tokens")]
    Random(#[from] RandomError),
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
}

/// Opens, renews, checks and ends device sessions.
#[derive(Clone, Debug)]
pub struct AuthService {
    sessions: SessionStore,
    clock: Arc<dyn Clock>,
    random: Arc<dyn RandomSource>,
}

impl AuthService {
    /// A service over `sessions` that tells the time with `clock` and makes tokens from
    /// `random`.
    #[must_use]
    pub fn new(
        sessions: SessionStore,
        clock: Arc<dyn Clock>,
        random: Arc<dyn RandomSource>,
    ) -> Self {
        Self {
            sessions,
            clock,
            random,
        }
    }

    /// Opens a session for a device that has just proven itself.
    ///
    /// # Errors
    ///
    /// [`AuthError::Random`] or [`AuthError::Database`].
    pub async fn open_session(&self, device: &DeviceDetails) -> Result<Session, AuthError> {
        let now = self.clock.now();
        let tokens = IssuedTokens::issue(self.random.as_ref(), now)?;
        let device = self.sessions.open(device, &tokens, now).await?;
        tracing::info!(%device, "opened a session");
        Ok(Session { device, tokens })
    }

    /// Trades a refresh token for new tokens. The old refresh token stops working; the old
    /// access token works until its own expiry, so requests already sent with it still pass.
    ///
    /// `install` is the app's install id: a device that signed in before its app sent one
    /// takes it here, so its next sign-in finds its row.
    ///
    /// # Errors
    ///
    /// [`AuthError::RefreshRejected`] when the token is neither current nor the one it
    /// replaced. The replaced token coming back within
    /// [`REFRESH_REUSE_GRACE`](emberhut_core::session::REFRESH_REUSE_GRACE) is a retry after a
    /// lost answer and gets a fresh pair (the pair issued moments before stops working); later
    /// it is a reuse, which signs the device out. A device idle too long is signed out too.
    /// [`AuthError::Random`] or [`AuthError::Database`] when the server fails.
    pub async fn refresh(
        &self,
        refresh: &str,
        install: Option<&str>,
    ) -> Result<Session, AuthError> {
        let now = self.clock.now();
        let current = SecretHash::of(refresh);
        let record = self
            .sessions
            .find_by_refresh(&current)
            .await?
            .ok_or(AuthError::RefreshRejected)?;
        let device = record.device;
        match record.verdict(now) {
            RefreshVerdict::Renew => {}
            RefreshVerdict::Reissue => {
                let RefreshMatch::Previous {
                    current: newest, ..
                } = record.matched
                else {
                    // Only the previous token is ever reissued from.
                    return Err(AuthError::RefreshRejected);
                };
                return self.reissue(device, &current, &newest, now).await;
            }
            RefreshVerdict::Reused => {
                self.sessions.sign_out(device, now).await?;
                tracing::warn!(
                    %device,
                    "a replaced refresh token was reused after the grace period: signed the \
                     device out"
                );
                return Err(AuthError::RefreshRejected);
            }
            RefreshVerdict::Idle => {
                self.sessions.sign_out(device, now).await?;
                tracing::info!(%device, "signed out a device that was idle too long");
                return Err(AuthError::RefreshRejected);
            }
        }

        let tokens = IssuedTokens::issue(self.random.as_ref(), now)?;
        let claim = record.install_to_claim(install);
        let rotated = self
            .sessions
            .rotate(device, &current, &tokens, claim.as_deref(), now)
            .await?;
        if !rotated {
            // Another request with the same token got there first.
            return Err(AuthError::RefreshRejected);
        }
        Ok(Session { device, tokens })
    }

    /// The device whose current access token this is, if any.
    ///
    /// Notes the device as seen (at most once a minute), which keeps an active device from
    /// being signed out as idle.
    ///
    /// # Errors
    ///
    /// [`AuthError::Database`].
    pub async fn authenticate(&self, access: &str) -> Result<Option<DeviceId>, AuthError> {
        let now = self.clock.now();
        let Some(record) = self
            .sessions
            .find_by_access(&SecretHash::of(access))
            .await?
        else {
            return Ok(None);
        };
        if !record.works_at(now) {
            return Ok(None);
        }
        if record.seen_is_stale(now) {
            self.sessions.mark_seen(record.device, now).await?;
        }
        Ok(Some(record.device))
    }

    /// A fresh pair for the holder of `device`'s replaced refresh token `previous`, in place
    /// of the pair issued moments before (whose refresh token has the hash `newest`).
    async fn reissue(
        &self,
        device: DeviceId,
        previous: &SecretHash,
        newest: &SecretHash,
        now: jiff::Timestamp,
    ) -> Result<Session, AuthError> {
        let tokens = IssuedTokens::issue(self.random.as_ref(), now)?;
        if !self
            .sessions
            .reissue(device, previous, newest, &tokens, now)
            .await?
        {
            // The session changed under us (signed out, or signed in again).
            return Err(AuthError::RefreshRejected);
        }
        tracing::info!(
            %device,
            "the replaced refresh token came back within the grace period, as after a lost \
             answer: issued a fresh pair in place of the newest"
        );
        Ok(Session { device, tokens })
    }

    /// The devices signed in now, the most recently seen first (signed-out and idle ones left
    /// out).
    ///
    /// # Errors
    ///
    /// [`AuthError::Database`].
    pub async fn devices(&self) -> Result<Vec<DeviceSummary>, AuthError> {
        Ok(self.sessions.signed_in(self.clock.now()).await?)
    }

    /// Signs a device out: its tokens stop working at once, until it proves itself again.
    ///
    /// # Errors
    ///
    /// [`AuthError::Database`].
    pub async fn sign_out(&self, device: DeviceId) -> Result<(), AuthError> {
        self.sessions.sign_out(device, self.clock.now()).await?;
        tracing::info!(%device, "signed a device out");
        Ok(())
    }
}
