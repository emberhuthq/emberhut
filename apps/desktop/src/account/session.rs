//! The account on this device, with no GPUI in it: its tokens (kept by the [`Vault`] and
//! renewed one renewal at a time), the provider's discovery, the device grant's start and
//! polls, the code's exchange, and the directory's calls with the rules for their refusals:
//! 401 `sign-in` signs the account out (after one renewal), 429 and 503 ask to try later and
//! never sign out. Every method blocks: the model calls them on background threads.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use emberhut_api::connect::LinkStatementRequest;
use emberhut_api::directory::{DirectoryErrorCode, LinkedMachine};
use emberhut_identity::{AccountId, MachineId};
use serde::Deserialize;

use super::device_grant::{self, DeviceAuthorization, GrantEnd, Poll};
use super::directory::{self, Refusal};
use super::http::{Http, Request, Response, Unreachable};
use super::oidc::{Discovery, DiscoveryError, TokenAnswer, TokenReply, discovery_url};
use super::pkce::{CallbackError, CodeFlow};
use super::switch::{AccountSettings, is_loopback_host};
use super::tokens::{AccountTokens, subject_of};
use super::vault::{Vault, VaultError};
use crate::backend::{self, Backend};

/// Why an account action didn't happen, in words for the person.
#[derive(Debug, thiserror::Error)]
pub(crate) enum AccountError {
    /// The account's sign-in is gone (refused by the directory or the provider): sign in again.
    #[error("You're signed out of your account. Sign in again.")]
    SignedOut,
    /// The directory is busy or out for a moment (429, 503).
    #[error("{}", later_words(*.0))]
    Later(Option<Duration>),
    /// The directory refused, by its code.
    #[error("{words}")]
    Refused {
        /// The refusal's code.
        code: DirectoryErrorCode,
        /// The words for it.
        words: String,
    },
    /// No answer came.
    #[error("Can't reach {}. Check the connection and try again.", .0.host)]
    Unreachable(#[from] Unreachable),
    /// The provider's discovery document isn't usable.
    #[error("{0}")]
    Discovery(#[from] DiscoveryError),
    /// The provider has no device grant to sign in with.
    #[error("The account's provider doesn't offer sign-in with a code.")]
    NoDeviceGrant,
    /// The provider or the directory answered something this app can't read.
    #[error("The account's service sent an answer this app can't read ({0}).")]
    Unreadable(String),
    /// The device grant ended without tokens.
    #[error("{0}")]
    Grant(GrantEnd),
    /// The web authentication session came back without a code.
    #[error("{0}")]
    Callback(#[from] CallbackError),
    /// The server this app talks to isn't on this computer.
    #[error(
        "Adding a computer works with the server on this computer only; this app talks to {0}."
    )]
    NotThisComputer(String),
    /// The server on this computer didn't sign the link.
    #[error("The server on this computer didn't sign the link: {0}")]
    Server(String),
    /// The system's random source failed.
    #[error("This computer couldn't make a random sign-in request.")]
    Random,
    /// A sign-out or another sign-in came while this one was finishing: its tokens aren't kept.
    #[error("Another sign-in or a sign-out came first.")]
    Superseded,
    /// The provider named a sign-in page that isn't `https://`.
    #[error("The account's provider gave a sign-in page that isn't https.")]
    InsecurePage,
}

/// "Try again later", with when if the directory said.
fn later_words(retry_after: Option<Duration>) -> String {
    match retry_after.map(|wait| wait.as_secs()) {
        Some(seconds @ 1..=90) => {
            format!("The account is busy right now. Try again in {seconds} seconds.")
        }
        Some(seconds) if seconds > 90 => format!(
            "The account is busy right now. Try again in {} minutes.",
            seconds.div_ceil(60)
        ),
        _ => "The account is busy right now. Try again in a minute.".to_owned(),
    }
}

impl AccountError {
    /// Whether this error means the account is signed out here now.
    #[must_use]
    pub(crate) fn is_signed_out(&self) -> bool {
        matches!(self, Self::SignedOut)
    }
}

/// The account on this device.
pub(crate) struct AccountSession {
    settings: AccountSettings,
    http: Arc<dyn Http>,
    vault: Box<dyn Vault>,
    /// The tokens, and the sign-in they belong to. Held only for a moment, never across the
    /// keychain, so the main thread (which signs out and cancels) never waits on it.
    held: Mutex<Held>,
    /// Held across every keychain read and write, so they happen in the order their sign-ins
    /// and sign-outs did. Taken on background threads only.
    keychain: Mutex<()>,
    discovery: Mutex<Option<Discovery>>,
    /// Held through a renewal, so two never send the same refresh token.
    renewing: Mutex<()>,
    /// Why the tokens are kept for this launch only, when the vault couldn't keep them.
    unkept: Mutex<Option<String>>,
}

impl fmt::Debug for AccountSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccountSession")
            .field("issuer", &self.settings.issuer)
            .field("signed_in", &self.is_signed_in())
            .finish_non_exhaustive()
    }
}

/// The tokens held, and which sign-in they belong to.
#[derive(Default)]
struct Held {
    /// Raised by every sign-in and sign-out, so the end of a renewal or a sign-in that started
    /// before one of them is dropped.
    generation: u64,
    /// The tokens, while signed in.
    tokens: Option<AccountTokens>,
}

/// Whether keeping tokens starts a sign-in or renews the current one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Keep {
    /// A new sign-in: a sign-in or renewal still under way for the one before is dropped.
    NewSignIn,
    /// The current sign-in's renewed tokens.
    Renewal,
}

/// Locks `mutex`, whatever a panicking holder left (the values here stay consistent).
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl AccountSession {
    /// The account of `settings`, talking through `http`, its tokens kept by `vault`.
    pub(crate) fn new(
        settings: AccountSettings,
        http: Arc<dyn Http>,
        vault: Box<dyn Vault>,
    ) -> Self {
        Self {
            settings,
            http,
            vault,
            held: Mutex::new(Held::default()),
            keychain: Mutex::new(()),
            discovery: Mutex::new(None),
            renewing: Mutex::new(()),
            unkept: Mutex::new(None),
        }
    }

    /// The build's settings.
    #[must_use]
    pub(crate) fn settings(&self) -> &AccountSettings {
        &self.settings
    }

    /// Reads the tokens kept by an earlier launch; whether the account is signed in.
    ///
    /// # Errors
    ///
    /// [`VaultError`] when the keychain can't be read; the account is then signed out here.
    pub(crate) fn restore(&self) -> Result<bool, VaultError> {
        let _keychain = lock(&self.keychain);
        let since = self.generation();
        let kept = self.vault.load()?;
        let mut held = lock(&self.held);
        // A sign-in or sign-out made while the keychain was read stands.
        if held.generation == since && held.tokens.is_none() {
            held.tokens = kept;
        }
        Ok(held.tokens.is_some())
    }

    /// Whether the account is signed in here.
    #[must_use]
    pub(crate) fn is_signed_in(&self) -> bool {
        lock(&self.held).tokens.is_some()
    }

    /// The current sign-in's number: a sign-in started now passes it to
    /// [`AccountSession::signed_in`], which keeps its tokens only if nothing came between.
    #[must_use]
    pub(crate) fn generation(&self) -> u64 {
        lock(&self.held).generation
    }

    /// Why the sign-in lasts only until the app quits, when it does.
    #[must_use]
    pub(crate) fn unkept(&self) -> Option<String> {
        lock(&self.unkept).clone()
    }

    /// The provider's endpoints, read once.
    fn discovery(&self) -> Result<Discovery, AccountError> {
        if let Some(found) = lock(&self.discovery).clone() {
            return Ok(found);
        }
        let response = self
            .http
            .send(&Request::get(discovery_url(&self.settings.issuer)))?;
        if response.status != 200 {
            return Err(AccountError::Unreadable(format!(
                "discovery, status {}",
                response.status
            )));
        }
        let found = Discovery::read(&response.body, &self.settings.issuer)?;
        *lock(&self.discovery) = Some(found.clone());
        Ok(found)
    }

    /// Starts the device grant: the code to show and the page to open.
    ///
    /// # Errors
    ///
    /// [`AccountError`] when the provider can't be reached, has no device grant, or refuses.
    pub(crate) fn start_device_grant(&self) -> Result<DeviceAuthorization, AccountError> {
        let found = self.discovery()?;
        let endpoint = found
            .device_authorization_endpoint
            .ok_or(AccountError::NoDeviceGrant)?;
        let response = self.http.send(&device_grant::start_request(
            &endpoint,
            &self.settings.client_id,
        ))?;
        if response.status != 200 {
            return Err(AccountError::Grant(GrantEnd::Failed(format!(
                "status {}",
                response.status
            ))));
        }
        let mut started: DeviceAuthorization = serde_json::from_slice(&response.body)
            .map_err(|_| AccountError::Unreadable("the device grant".to_owned()))?;
        if !started.verification_uri.starts_with("https://") {
            return Err(AccountError::InsecurePage);
        }
        started.verification_uri_complete = started
            .verification_uri_complete
            .filter(|page| page.starts_with("https://"));
        Ok(started)
    }

    /// Asks once whether `authorization` was approved.
    #[must_use]
    pub(crate) fn poll_device_grant(&self, authorization: &DeviceAuthorization) -> Poll {
        let Ok(found) = self.discovery() else {
            return Poll::Unanswered;
        };
        let request =
            found.device_poll_request(&self.settings.client_id, &authorization.device_code);
        match self.http.send(&request) {
            Ok(response) => Poll::from_reply(TokenReply::read(&response)),
            Err(error) => {
                log::info!("the device grant's poll got no answer: {error}");
                Poll::Unanswered
            }
        }
    }

    /// The page the web authentication session opens for `flow`.
    ///
    /// # Errors
    ///
    /// [`AccountError`] when the provider's discovery can't be had.
    pub(crate) fn authorize_url(&self, flow: &CodeFlow) -> Result<String, AccountError> {
        let found = self.discovery()?;
        Ok(flow.authorize_url(&found.authorization_endpoint, &self.settings.client_id))
    }

    /// Finishes the web authentication session: the code in `callback` (its `state` checked)
    /// traded with `flow`'s verifier for tokens, kept if sign-in `since` is still current.
    ///
    /// # Errors
    ///
    /// [`AccountError`] when the callback isn't this sign-in's or the exchange fails.
    pub(crate) fn finish_code_flow(
        &self,
        flow: &CodeFlow,
        callback: &str,
        since: u64,
    ) -> Result<(), AccountError> {
        let code = flow.code_from(callback)?;
        let found = self.discovery()?;
        let request = found.code_request(&self.settings.client_id, &code, flow.verifier());
        match TokenReply::read(&self.http.send(&request)?) {
            TokenReply::Granted(answer) => self.signed_in(answer, since),
            TokenReply::Refused(error) => Err(AccountError::Grant(GrantEnd::Failed(error))),
            TokenReply::Unavailable { retry_after, .. } => Err(AccountError::Later(retry_after)),
            TokenReply::Failed(status) => Err(AccountError::Unreadable(format!(
                "the code's exchange, status {status}"
            ))),
        }
    }

    /// Keeps the tokens a sign-in gave (in the vault, or for this launch when it can't), when
    /// no sign-in or sign-out came since the sign-in that got them began, at generation
    /// `since` ([`AccountSession::generation`]).
    ///
    /// # Errors
    ///
    /// [`AccountError::Superseded`] when one did; [`AccountError`] when the account's id can't
    /// be had.
    pub(crate) fn signed_in(&self, answer: TokenAnswer, since: u64) -> Result<(), AccountError> {
        let named = answer.id_token.as_deref().and_then(|token| {
            subject_of(
                token,
                &self.settings.issuer,
                &self.settings.client_id,
                SystemTime::now(),
            )
        });
        let account = match named {
            Some(account) => account,
            None => self.user_info(&answer.access_token)?,
        };
        let tokens = AccountTokens::from_answer(answer, account, SystemTime::now());
        self.keep(since, tokens, Keep::NewSignIn)
    }

    /// The account's id, from the provider's user info endpoint.
    fn user_info(&self, access: &str) -> Result<AccountId, AccountError> {
        #[derive(Deserialize)]
        struct UserInfo {
            sub: String,
        }
        let unreadable = || AccountError::Unreadable("the account's id".to_owned());
        let endpoint = self.discovery()?.userinfo_endpoint.ok_or_else(unreadable)?;
        let response = self.http.send(&Request::get(endpoint).bearer(access))?;
        let info: UserInfo = (response.status == 200)
            .then(|| serde_json::from_slice(&response.body).ok())
            .flatten()
            .ok_or_else(unreadable)?;
        AccountId::new(info.sub).map_err(|_| unreadable())
    }

    /// Makes `tokens` the account's, in the vault and here, when nothing came since
    /// generation `since`. A sign-out or cancel that comes while the vault is written puts the
    /// vault back to what is held here, before the keychain's lock is let go.
    fn keep(&self, since: u64, tokens: AccountTokens, keep: Keep) -> Result<(), AccountError> {
        let _keychain = lock(&self.keychain);
        if self.generation() != since {
            return Err(AccountError::Superseded);
        }
        let unkept = self.vault.save(&tokens).err().map(|error| {
            log::warn!("the account's sign-in is kept for this launch only: {error}");
            error.to_string()
        });
        let mut held = lock(&self.held);
        if held.generation != since {
            // A sign-out or a cancel came during the write: the keychain goes back to what is
            // held here, before anything else may write it.
            let now_held = held.tokens.clone();
            drop(held);
            self.put_back(now_held.as_ref());
            return Err(AccountError::Superseded);
        }
        if keep == Keep::NewSignIn {
            held.generation += 1;
        }
        held.tokens = Some(tokens);
        *lock(&self.unkept) = unkept;
        Ok(())
    }

    /// Makes the vault hold `tokens` again, or nothing. Call it with the keychain's lock held.
    fn put_back(&self, tokens: Option<&AccountTokens>) {
        let put = match tokens {
            Some(tokens) => self.vault.save(tokens),
            None => self.vault.clear(),
        };
        if let Err(error) = put {
            log::warn!("the account's Keychain item couldn't be put back: {error}");
        }
    }

    /// Signs the account out here at once: its tokens forgotten in memory, and every renewal
    /// or sign-in under way dropped when it ends. Returns the sign-out's generation, for
    /// [`AccountSession::forget_kept`]. No device session is touched, and no keychain: this
    /// never waits on one.
    pub(crate) fn sign_out(&self) -> u64 {
        let mut held = lock(&self.held);
        held.generation += 1;
        held.tokens = None;
        *lock(&self.unkept) = None;
        held.generation
    }

    /// Drops the sign-in under way (cancelled, or failed): whatever it was still waiting for
    /// (a code's exchange, the account's id) is then neither kept nor acted on. The account
    /// stays as it is. Never waits on the keychain.
    pub(crate) fn abandon(&self) {
        lock(&self.held).generation += 1;
    }

    /// Removes the vault's tokens, when nothing came since the sign-out at `generation` (a
    /// sign-in made since keeps its own). Blocks on the keychain.
    pub(crate) fn forget_kept(&self, generation: u64) {
        let _keychain = lock(&self.keychain);
        if self.generation() != generation {
            return;
        }
        if let Err(error) = self.vault.clear() {
            log::warn!("signed out of the account, but its Keychain item stays: {error}");
        }
    }

    /// Forgets the tokens of sign-in `generation`, which the provider or the directory
    /// refused. Says whether it did: not when a sign-in or sign-out already came since.
    fn forget(&self, generation: u64) -> bool {
        let _keychain = lock(&self.keychain);
        {
            let mut held = lock(&self.held);
            if held.generation != generation {
                return false;
            }
            held.generation += 1;
            held.tokens = None;
            *lock(&self.unkept) = None;
        }
        if let Err(error) = self.vault.clear() {
            log::warn!("signed out of the account, but its Keychain item stays: {error}");
        }
        true
    }

    /// The error for a refusal that signed sign-in `generation` out, or found a newer one.
    fn signed_out(&self, generation: u64) -> AccountError {
        if self.forget(generation) {
            AccountError::SignedOut
        } else {
            AccountError::Superseded
        }
    }

    /// The tokens held and their generation.
    fn current(&self) -> Result<(u64, AccountTokens), AccountError> {
        let held = lock(&self.held);
        let tokens = held.tokens.clone().ok_or(AccountError::SignedOut)?;
        Ok((held.generation, tokens))
    }

    /// A fresh access token and its generation: the one held, or a renewed one.
    fn access(&self) -> Result<(u64, String), AccountError> {
        let (generation, tokens) = self.current()?;
        if tokens.is_fresh(SystemTime::now()) {
            Ok((generation, tokens.access))
        } else {
            self.renew(&tokens.access)
        }
    }

    /// Renews the access token that was `stale`, one renewal at a time: when another renewal
    /// already replaced it, its token is used. The provider's `invalid_grant` signs the
    /// account out; no answer, or a busy one, keeps the tokens for a later try. A sign-out or
    /// sign-in that comes while it runs wins: its answer is then neither kept nor acted on.
    fn renew(&self, stale: &str) -> Result<(u64, String), AccountError> {
        let _renewing = lock(&self.renewing);
        let (generation, held) = self.current()?;
        if held.access != stale {
            return Ok((generation, held.access));
        }
        let Some(refresh) = held.refresh.clone() else {
            return Err(self.signed_out(generation));
        };
        let found = self.discovery()?;
        let response = self
            .http
            .send(&found.refresh_request(&self.settings.client_id, &refresh))?;
        match TokenReply::read(&response) {
            TokenReply::Granted(answer) => {
                let renewed = held.renewed(answer, SystemTime::now());
                let access = renewed.access.clone();
                self.keep(generation, renewed, Keep::Renewal)?;
                Ok((generation, access))
            }
            TokenReply::Refused(error) if error == "invalid_grant" => {
                log::info!("the account's refresh token was refused: signed out");
                Err(self.signed_out(generation))
            }
            TokenReply::Refused(error) => Err(AccountError::Grant(GrantEnd::Failed(error))),
            TokenReply::Unavailable { retry_after, .. } => Err(AccountError::Later(retry_after)),
            TokenReply::Failed(status) => Err(AccountError::Unreadable(format!(
                "the token's renewal, status {status}"
            ))),
        }
    }

    /// Sends the directory request `make` builds with the access token, and reads its answer
    /// with `read`. A 401 renews the token once and tries again; a second 401 signs the
    /// account out (unless a sign-in or sign-out came meanwhile).
    fn authorized<T>(
        &self,
        make: impl Fn() -> Request,
        read: impl Fn(&Response) -> Result<T, Refusal>,
    ) -> Result<T, AccountError> {
        let (_, access) = self.access()?;
        match read(&self.http.send(&make().bearer(&access))?) {
            Err(Refusal::SignIn) => {}
            other => return other.map_err(refused),
        }
        let (generation, renewed) = self.renew(&access)?;
        match read(&self.http.send(&make().bearer(&renewed))?) {
            Err(Refusal::SignIn) => Err(self.signed_out(generation)),
            other => other.map_err(refused),
        }
    }

    /// The account's machines.
    ///
    /// # Errors
    ///
    /// [`AccountError`]; [`AccountError::SignedOut`] when the directory no longer takes the
    /// account's sign-in.
    pub(crate) fn machines(&self) -> Result<Vec<LinkedMachine>, AccountError> {
        let base = &self.settings.directory;
        self.authorized(|| directory::list_request(base), directory::read_list)
    }

    /// Unlinks machine `id` from the account.
    ///
    /// # Errors
    ///
    /// As [`AccountSession::machines`].
    pub(crate) fn unlink(&self, id: &MachineId) -> Result<(), AccountError> {
        let base = &self.settings.directory;
        self.authorized(
            || directory::unlink_request(base, id),
            directory::read_unlinked,
        )
    }

    /// Links the computer this app runs on: the server on it (`backend`, over loopback, with
    /// this device's session) signs a link statement for the account and this relay, and the
    /// statement goes to the directory, as it is, with the account's token. The server never
    /// sees the account's token.
    ///
    /// # Errors
    ///
    /// [`AccountError::NotThisComputer`] when the app's server isn't on loopback; the server's
    /// refusal; the directory's (`linked-elsewhere`, `account-full`).
    pub(crate) fn link_this_computer(
        &self,
        backend: &dyn Backend,
    ) -> Result<LinkedMachine, AccountError> {
        let server = backend.server().as_str();
        let authority = server.split_once("://").map_or(server, |(_, rest)| rest);
        if !is_loopback_host(authority) {
            return Err(AccountError::NotThisComputer(server.to_owned()));
        }
        let (_, tokens) = self.current()?;
        let account = tokens.account;
        let request = LinkStatementRequest {
            account,
            relay_domain: self.settings.relay_domain.clone(),
        };
        let link = backend
            .link_statement(&request)
            .map_err(|error| AccountError::Server(server_words(&error)))?;
        if link.account != request.account || link.relay_domain != request.relay_domain {
            return Err(AccountError::Server(
                "it signed a link for another account or relay".to_owned(),
            ));
        }
        let base = &self.settings.directory;
        let body = directory::link_request(base, &link)
            .map_err(|error| AccountError::Unreadable(error.to_string()))?;
        self.authorized(|| body.clone(), directory::read_linked)
    }
}

/// The local server's refusal, in words.
fn server_words(error: &backend::Error) -> String {
    match (error, error.server_message()) {
        (_, Some(words)) => words.to_owned(),
        (
            backend::Error::Refused {
                status: Some(404), ..
            },
            None,
        ) => "it predates linking. Update it and try again.".to_owned(),
        (backend::Error::SignInRequired, None) => {
            "this app isn't signed in to it. Pair it first.".to_owned()
        }
        (other, None) => other.to_string(),
    }
}

/// The error for a directory refusal other than sign-in.
fn refused(refusal: Refusal) -> AccountError {
    match refusal {
        Refusal::SignIn => AccountError::SignedOut,
        Refusal::Later(wait) => AccountError::Later(wait),
        Refusal::Refused { code, words } => AccountError::Refused { code, words },
        Refusal::Unreadable(status) => {
            AccountError::Unreadable(format!("the directory, status {status}"))
        }
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
