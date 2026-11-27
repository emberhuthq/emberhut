//! The session in use, and renewing its access token one renewal at a time.
//!
//! The refresh token turns over on every use: a second renewal sending the token the first one
//! just used is refused. If that refusal signed the device out, every overlap (the poll, the live
//! stream and a post all renewing at once) would sign it out. So, as the reference clients do:
//!
//! - one renewal at a time: the others wait for it;
//! - after the wait, the session is read again, and if its token is fresh now (another renewal
//!   got there first) it is used as it is;
//! - a renewal's answer is applied only to the session it renewed: if the session changed while
//!   the renewal was out (signed out, paired again), the answer is dropped;
//! - a refused renewal signs the device out only if the session still holds the refresh token
//!   that was refused;
//! - a grant is in use as soon as it arrives, even if the store can't save it (the server has
//!   already turned the old token over): the save is tried again at the next change or renewal,
//!   and from requests at most once a minute, which don't wait for it (the failure is logged as
//!   an error once a minute too);
//! - a store that holds a newer client's session, at open or later, is never written over: the
//!   change fails with `StoreError::NewerFormat` (`NewerFormatInStore` for a store that isn't a
//!   file).
//!
//! This holds within one process. Two processes over one store don't share the lock: one
//! process per store. A process that hands the store to another (an update's new copy)
//! [stops](Sessions::stop) first: it waits for a renewal in flight, saves the session, and
//! renews nothing from then on.
//!
//! Locks are taken in the order `renewing`, `changing`, `current`. `current` is held only to
//! read or swap the value, never across I/O, so the quick getters never wait on a disk.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};
use std::time::{Duration, Instant, SystemTime};

use emberhut_api::auth::{AccessToken, AuthRefusal, RefreshRequest, RefreshToken, SessionGrant};
use emberhut_api::ids::{DeviceId, InstallId};

use crate::Error;
use crate::session::{Session, SessionStore, SessionTokens, StoreError};

/// How often a request tries a failed save again, and how often the failure is logged as an
/// error (in between, at debug level): a full disk shouldn't log on every poll.
const SAVE_RETRY_INTERVAL: Duration = Duration::from_mins(1);

/// What the server said to a renewal.
#[derive(Debug)]
pub(crate) enum Renewal {
    /// A new session, and the answer's `Date`.
    Granted(SessionGrant, Option<SystemTime>),
    /// The refresh token is no longer good.
    Refused(AuthRefusal),
}

/// The session in use, kept in step with its store.
#[derive(Debug)]
pub(crate) struct Sessions {
    store: Box<dyn SessionStore>,
    /// The session in use.
    current: Mutex<Session>,
    /// `current` is newer than the store's: a save failed, and is tried again.
    unsaved: AtomicBool,
    /// When a failed save was last tried again from a request, and last logged as an error.
    last_retry: Mutex<Option<Instant>>,
    last_logged: Mutex<Option<Instant>>,
    /// How often a request tries a failed save again and the failure is logged as an error:
    /// [`SAVE_RETRY_INTERVAL`], as given to [`Sessions::open_with`].
    save_retry_interval: Duration,
    /// Held across a read, compare and write of the session, so changes don't interleave.
    changing: Mutex<()>,
    /// Held for the whole of a renewal.
    renewing: Mutex<()>,
    /// Set by [`Sessions::stop`]: no token is handed out or renewed any more.
    stopped: AtomicBool,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A thread that panicked holding it left a whole value: a session is replaced, never edited
    // in place across a panic point.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Sessions {
    /// The store's session, or a new install saved to it when there is none (or what it holds
    /// isn't a session any more).
    pub(crate) fn open(store: Box<dyn SessionStore>) -> Result<Self, Error> {
        Self::open_with(store, SAVE_RETRY_INTERVAL)
    }

    /// [`Sessions::open`], retrying a failed save from requests every `save_retry_interval`
    /// (tests use zero).
    fn open_with(
        store: Box<dyn SessionStore>,
        save_retry_interval: Duration,
    ) -> Result<Self, Error> {
        let session = match store.load() {
            Ok(Some(session)) => session,
            Ok(None) => Self::new_install(&*store)?,
            Err(StoreError::Corrupt { path, source }) => {
                tracing::warn!(path = %path.display(), error = %source, "the saved session doesn't read: starting a new one");
                Self::new_install(&*store)?
            }
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            store,
            current: Mutex::new(session),
            unsaved: AtomicBool::new(false),
            last_retry: Mutex::new(None),
            last_logged: Mutex::new(None),
            save_retry_interval,
            changing: Mutex::new(()),
            renewing: Mutex::new(()),
            stopped: AtomicBool::new(false),
        })
    }

    fn new_install(store: &dyn SessionStore) -> Result<Session, Error> {
        let session = Session::new_install().map_err(Error::Random)?;
        store.save(&session)?;
        Ok(session)
    }

    pub(crate) fn install(&self) -> InstallId {
        locked(&self.current).install.clone()
    }

    pub(crate) fn is_signed_in(&self) -> bool {
        locked(&self.current).is_signed_in()
    }

    pub(crate) fn device(&self) -> Option<DeviceId> {
        locked(&self.current).tokens.as_ref().and_then(|t| t.device)
    }

    /// Stops for good, so another process can take the store over: no token is handed out or
    /// renewed from now on, a renewal in flight is waited for, and the session is saved if a
    /// save of it failed before.
    ///
    /// # Errors
    ///
    /// The session couldn't be saved (the store then holds the one before).
    pub(crate) fn stop(&self) -> Result<(), Error> {
        self.stopped.store(true, Ordering::Release);
        let _no_renewal = locked(&self.renewing);
        let _changing = locked(&self.changing);
        if self.unsaved.load(Ordering::Acquire) {
            let current = locked(&self.current).clone();
            self.store.save(&current)?;
            self.unsaved.store(false, Ordering::Release);
        }
        Ok(())
    }

    /// Whether [`Sessions::stop`] was called.
    pub(crate) fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    /// Keeps a pairing's grant: in use at once, then saved.
    ///
    /// # Errors
    ///
    /// The store couldn't save it. The session is in use all the same; the save is tried again
    /// on the next call.
    pub(crate) fn keep(
        &self,
        grant: SessionGrant,
        server_date: Option<SystemTime>,
    ) -> Result<(), Error> {
        let _changing = locked(&self.changing);
        let mut session = self.stored()?;
        session.tokens = Some(SessionTokens::from_grant(
            grant,
            SystemTime::now(),
            server_date,
        ));
        Ok(self.put(&session)?)
    }

    /// Drops the tokens (the install stays): at once, then saved.
    ///
    /// # Errors
    ///
    /// The store couldn't save it. The device is signed out here all the same; the save is
    /// tried again on the next call.
    pub(crate) fn sign_out(&self) -> Result<(), Error> {
        let _changing = locked(&self.changing);
        let mut session = self.stored()?;
        session.tokens = None;
        Ok(self.put(&session)?)
    }

    /// The session as it stands: `current` while it is unsaved (after trying the save again),
    /// else the store's, put in use. Call with `changing` held.
    ///
    /// # Errors
    ///
    /// The store now holds a newer client's session: this one stops changing it (the change
    /// isn't made), as it doesn't overwrite a newer session at open either. Any other read
    /// failure is logged and the session in use goes on.
    fn stored(&self) -> Result<Session, StoreError> {
        if self.unsaved.load(Ordering::Acquire) {
            let current = locked(&self.current).clone();
            self.try_save(&current);
            return Ok(current);
        }
        match self.store.load() {
            Ok(loaded) => {
                let mut current = locked(&self.current);
                match loaded {
                    Some(session) => *current = session,
                    None => current.tokens = None,
                }
                Ok(current.clone())
            }
            Err(
                error @ (StoreError::NewerFormat { .. } | StoreError::NewerFormatInStore { .. }),
            ) => Err(error),
            Err(error) => {
                tracing::error!(%error, "couldn't read the session store; going on with the session in use");
                Ok(locked(&self.current).clone())
            }
        }
    }

    /// Saves an unsaved session again; a failure is logged at most once a minute.
    fn try_save(&self, session: &Session) {
        match self.store.save(session) {
            Ok(()) => self.unsaved.store(false, Ordering::Release),
            Err(error) => self.note_save_failure(&error),
        }
    }

    /// Logs a failed save as an error at most once a [`SAVE_RETRY_INTERVAL`], else at debug.
    fn note_save_failure(&self, error: &StoreError) {
        let mut last = locked(&self.last_logged);
        if last.is_none_or(|at| at.elapsed() >= self.save_retry_interval) {
            *last = Some(Instant::now());
            tracing::error!(%error, "the session can't be saved; it is in use and the save is tried again");
        } else {
            tracing::debug!(%error, "the session still can't be saved");
        }
    }

    /// From a request, while the session is unsaved: tries the save again, at most once a
    /// [`SAVE_RETRY_INTERVAL`], and never waits for a change in progress.
    fn retry_save_now_and_then(&self) {
        let due =
            locked(&self.last_retry).is_none_or(|at| at.elapsed() >= self.save_retry_interval);
        if !due {
            return;
        }
        // Poisoning is ignored here as in `locked`; a change in progress is left to finish (it
        // saves the session itself), without using up this minute's try.
        let _changing = match self.changing.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return,
        };
        *locked(&self.last_retry) = Some(Instant::now());
        if self.unsaved.load(Ordering::Acquire) {
            let current = locked(&self.current).clone();
            self.try_save(&current);
        }
    }

    /// Puts `next` in use, then saves it. Call with `changing` held.
    fn put(&self, next: &Session) -> Result<(), StoreError> {
        locked(&self.current).clone_from(next);
        match self.store.save(next) {
            Ok(()) => {
                self.unsaved.store(false, Ordering::Release);
                Ok(())
            }
            Err(error) => {
                self.unsaved.store(true, Ordering::Release);
                Err(error)
            }
        }
    }

    /// Like [`Self::put`] on the renewal path, where nobody waits for the store: a failed save
    /// is logged, and tried again on the next call.
    fn put_logged(&self, next: &Session) {
        if let Err(error) = self.put(next) {
            self.note_save_failure(&error);
        }
    }

    /// The access token to send now, renewed first with `renew` when it is (nearly) out, or
    /// when it is `rejected` (the server refused it although this clock says it is good); none
    /// when not signed in.
    ///
    /// A renewal that can't reach the server, while the token hasn't run out and wasn't
    /// rejected, uses the token as it is; otherwise the error is returned.
    pub(crate) fn bearer(
        &self,
        rejected: Option<&AccessToken>,
        renew: impl FnOnce(&RefreshRequest) -> Result<Renewal, Error>,
    ) -> Result<Option<AccessToken>, Error> {
        let usable = |tokens: &SessionTokens, now| {
            !tokens.needs_renewal(now) && Some(&tokens.access) != rejected
        };
        if self.is_stopped() {
            return Err(Error::Stopped);
        }
        if self.unsaved.load(Ordering::Acquire) {
            self.retry_save_now_and_then();
        }
        match &locked(&self.current).tokens {
            None => return Ok(None),
            Some(tokens) if usable(tokens, SystemTime::now()) => {
                return Ok(Some(tokens.access.clone()));
            }
            Some(_) => {}
        }
        let _one_at_a_time = locked(&self.renewing);
        if self.is_stopped() {
            return Err(Error::Stopped);
        }
        let session = {
            let _changing = locked(&self.changing);
            self.stored()?
        };
        let Some(tokens) = session.tokens else {
            return Ok(None);
        };
        let now = SystemTime::now();
        if usable(&tokens, now) {
            return Ok(Some(tokens.access));
        }
        let request = RefreshRequest {
            refresh: tokens.refresh.clone(),
            install: Some(session.install),
        };
        match renew(&request) {
            Ok(Renewal::Granted(grant, date)) => Ok(self.renewed(&request.refresh, grant, date)?),
            Ok(Renewal::Refused(refusal)) => Ok(self.refused(&request.refresh, &refusal)?),
            Err(error) if !tokens.is_expired(now) && Some(&tokens.access) != rejected => {
                tracing::warn!(%error, "couldn't renew the access token; using it while it lasts");
                Ok(Some(tokens.access))
            }
            Err(error) => Err(error),
        }
    }

    /// A renewal of `sent` was granted: keep it, unless the session changed meanwhile.
    fn renewed(
        &self,
        sent: &RefreshToken,
        grant: SessionGrant,
        date: Option<SystemTime>,
    ) -> Result<Option<AccessToken>, StoreError> {
        let _changing = locked(&self.changing);
        let mut session = self.stored()?;
        if session.tokens.as_ref().map(|t| &t.refresh) != Some(sent) {
            tracing::info!(
                "the session changed while it was being renewed: the renewal is dropped"
            );
            return Ok(session.tokens.map(|t| t.access));
        }
        let tokens = SessionTokens::from_grant(grant, SystemTime::now(), date);
        let access = tokens.access.clone();
        session.tokens = Some(tokens);
        self.put_logged(&session);
        Ok(Some(access))
    }

    /// The server refused `sent`: sign out, unless the session holds another token by now.
    fn refused(
        &self,
        sent: &RefreshToken,
        refusal: &AuthRefusal,
    ) -> Result<Option<AccessToken>, StoreError> {
        let _changing = locked(&self.changing);
        let mut session = self.stored()?;
        Ok(match session.tokens {
            Some(tokens) if &tokens.refresh != sent => {
                tracing::info!(
                    "a renewal was refused, but the session was renewed meanwhile: keeping it"
                );
                Some(tokens.access)
            }
            Some(_) => {
                tracing::warn!(status = refusal.status, reason = %refusal.error, "the server ended this device's session");
                session.tokens = None;
                self.put_logged(&session);
                None
            }
            None => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    use super::*;
    use crate::session::MemorySessionStore;
    use crate::transport::TransportError;

    fn tokens(refresh: &str, expires: SystemTime) -> SessionTokens {
        SessionTokens {
            access: AccessToken::new(format!("access-for-{refresh}")),
            refresh: RefreshToken::new(refresh),
            device: Some(DeviceId::new(3)),
            access_expires: expires,
        }
    }

    fn grant(refresh: &str) -> Renewal {
        Renewal::Granted(
            SessionGrant {
                access: AccessToken::new(format!("access-for-{refresh}")),
                refresh: RefreshToken::new(refresh),
                device: Some(DeviceId::new(3)),
                expires: None,
            },
            None,
        )
    }

    fn session(tokens: Option<SessionTokens>) -> Session {
        Session {
            install: InstallId::new("zr-test"),
            tokens,
        }
    }

    fn store_with(tokens: Option<SessionTokens>) -> Arc<MemorySessionStore> {
        Arc::new(MemorySessionStore::with_session(session(tokens)))
    }

    fn never(_: &RefreshRequest) -> Result<Renewal, Error> {
        panic!("no renewal expected")
    }

    fn later() -> SystemTime {
        SystemTime::now() + Duration::from_hours(1)
    }

    fn token(refresh: &str) -> AccessToken {
        AccessToken::new(format!("access-for-{refresh}"))
    }

    fn stored_refresh(store: &MemorySessionStore) -> Option<RefreshToken> {
        store.load().unwrap().unwrap().tokens.map(|t| t.refresh)
    }

    /// A store whose saves fail while `failing` is set.
    #[derive(Debug, Default)]
    struct Flaky {
        inner: MemorySessionStore,
        failing: AtomicBool,
        saves: AtomicUsize,
    }

    impl SessionStore for Flaky {
        fn load(&self) -> Result<Option<Session>, StoreError> {
            self.inner.load()
        }
        fn save(&self, session: &Session) -> Result<(), StoreError> {
            self.saves.fetch_add(1, Ordering::SeqCst);
            if self.failing.load(Ordering::SeqCst) {
                return Err(StoreError::Backend("the disk is full".into()));
            }
            self.inner.save(session)
        }
        fn clear(&self) -> Result<(), StoreError> {
            self.inner.clear()
        }
    }

    #[test]
    fn a_new_store_gets_an_install() {
        let store = Arc::new(MemorySessionStore::new());
        let sessions = Sessions::open(Box::new(Arc::clone(&store))).unwrap();
        assert!(!sessions.is_signed_in());
        assert_eq!(store.load().unwrap().unwrap().install, sessions.install());
        assert_eq!(sessions.bearer(None, never).unwrap(), None);
    }

    #[test]
    fn a_fresh_token_is_used_as_it_is() {
        let sessions = Sessions::open(Box::new(store_with(Some(tokens("r1", later()))))).unwrap();
        assert_eq!(sessions.bearer(None, never).unwrap(), Some(token("r1")));
        assert_eq!(sessions.device(), Some(DeviceId::new(3)));
    }

    #[test]
    fn an_old_token_is_renewed_and_saved() {
        let store = store_with(Some(tokens("r1", SystemTime::UNIX_EPOCH)));
        let sessions = Sessions::open(Box::new(Arc::clone(&store))).unwrap();
        let bearer = sessions.bearer(None, |request| {
            assert_eq!(request.refresh, RefreshToken::new("r1"));
            assert_eq!(request.install, Some(InstallId::new("zr-test")));
            Ok(grant("r2"))
        });
        assert_eq!(bearer.unwrap(), Some(token("r2")));
        assert_eq!(stored_refresh(&store), Some(RefreshToken::new("r2")));
    }

    #[test]
    fn a_rejected_token_is_renewed_although_it_looks_fresh() {
        let store = store_with(Some(tokens("r1", later())));
        let sessions = Sessions::open(Box::new(Arc::clone(&store))).unwrap();
        let rejected = AccessToken::new("access-for-r1");
        assert_eq!(
            sessions
                .bearer(Some(&rejected), |_| Ok(grant("r2")))
                .unwrap(),
            Some(token("r2"))
        );
        // Another call that saw the same refusal finds the renewed token and doesn't renew again.
        assert_eq!(
            sessions.bearer(Some(&rejected), never).unwrap(),
            Some(token("r2"))
        );
    }

    #[test]
    fn a_refused_renewal_signs_out_when_the_store_still_holds_that_token() {
        let store = store_with(Some(tokens("r1", SystemTime::UNIX_EPOCH)));
        let sessions = Sessions::open(Box::new(Arc::clone(&store))).unwrap();
        let refusal = AuthRefusal {
            status: 401,
            error: "Sign in again.".to_owned(),
        };
        assert_eq!(
            sessions
                .bearer(None, |_| Ok(Renewal::Refused(refusal)))
                .unwrap(),
            None
        );
        assert!(!sessions.is_signed_in());
        let stored = store.load().unwrap().unwrap();
        assert!(stored.tokens.is_none());
        assert_eq!(
            stored.install,
            InstallId::new("zr-test"),
            "the install stays"
        );
    }

    #[test]
    fn a_refused_renewal_keeps_a_session_renewed_meanwhile() {
        let store = store_with(Some(tokens("r1", SystemTime::UNIX_EPOCH)));
        let sessions = Sessions::open(Box::new(Arc::clone(&store))).unwrap();
        let bearer = sessions.bearer(None, |_| {
            // Another client on the same store renews with this token first.
            store.save(&session(Some(tokens("r2", later())))).unwrap();
            Ok(Renewal::Refused(AuthRefusal {
                status: 401,
                error: "Sign in again.".to_owned(),
            }))
        });
        assert_eq!(bearer.unwrap(), Some(token("r2")));
        assert!(sessions.is_signed_in());
    }

    #[test]
    fn a_sign_out_during_a_renewal_stays() {
        let store = store_with(Some(tokens("r1", SystemTime::UNIX_EPOCH)));
        let sessions = Sessions::open(Box::new(Arc::clone(&store))).unwrap();
        let bearer = sessions.bearer(None, |_| {
            sessions.sign_out().unwrap();
            Ok(grant("r2"))
        });
        assert_eq!(bearer.unwrap(), None);
        assert!(!sessions.is_signed_in());
        assert_eq!(
            stored_refresh(&store),
            None,
            "the late grant isn't saved over the sign-out"
        );
    }

    #[test]
    fn a_pairing_during_a_renewal_wins() {
        let store = store_with(Some(tokens("r1", SystemTime::UNIX_EPOCH)));
        let sessions = Sessions::open(Box::new(Arc::clone(&store))).unwrap();
        let bearer = sessions.bearer(None, |_| {
            let paired = SessionGrant {
                access: token("paired"),
                refresh: RefreshToken::new("paired"),
                device: Some(DeviceId::new(3)),
                expires: None,
            };
            sessions.keep(paired, None).unwrap();
            Ok(grant("r2"))
        });
        assert_eq!(bearer.unwrap(), Some(token("paired")));
        assert_eq!(stored_refresh(&store), Some(RefreshToken::new("paired")));
    }

    #[test]
    fn a_grant_that_cant_be_saved_stays_in_use_and_is_saved_later() {
        let store = Arc::new(Flaky::default());
        store
            .inner
            .save(&session(Some(tokens("r1", SystemTime::UNIX_EPOCH))))
            .unwrap();
        let sessions = Sessions::open_with(Box::new(Arc::clone(&store)), Duration::ZERO).unwrap();
        store.failing.store(true, Ordering::SeqCst);

        assert_eq!(
            sessions.bearer(None, |_| Ok(grant("r2"))).unwrap(),
            Some(token("r2"))
        );
        assert_eq!(
            stored_refresh(&store.inner),
            Some(RefreshToken::new("r1")),
            "not saved"
        );

        // The next renewal sends the new token, not the one the server turned over.
        let rejected = AccessToken::new("access-for-r2");
        let bearer = sessions.bearer(Some(&rejected), |request| {
            assert_eq!(request.refresh, RefreshToken::new("r2"));
            Ok(grant("r3"))
        });
        assert_eq!(bearer.unwrap(), Some(token("r3")));

        store.failing.store(false, Ordering::SeqCst);
        assert_eq!(sessions.bearer(None, never).unwrap(), Some(token("r3")));
        assert_eq!(
            stored_refresh(&store.inner),
            Some(RefreshToken::new("r3")),
            "saved once the store works"
        );
    }

    #[test]
    fn an_unsaved_session_doesnt_slow_requests_down() {
        let store = Arc::new(Flaky::default());
        store
            .inner
            .save(&session(Some(tokens("r1", later()))))
            .unwrap();
        let sessions = Arc::new(Sessions::open(Box::new(Arc::clone(&store))).unwrap());
        store.failing.store(true, Ordering::SeqCst);
        assert!(sessions.sign_out().is_err());
        store.saves.store(0, Ordering::SeqCst);
        // A renewal in flight holds `renewing`: requests don't wait behind it while unsaved.
        let renewal_in_flight = locked(&sessions.renewing);
        let (done, finished) = std::sync::mpsc::channel();
        let polling = Arc::clone(&sessions);
        std::thread::spawn(move || {
            for _ in 0..10 {
                assert_eq!(polling.bearer(None, never).unwrap(), None);
            }
            done.send(()).unwrap();
        });
        finished
            .recv_timeout(Duration::from_secs(5))
            .expect("the polls didn't wait for the renewal");
        drop(renewal_in_flight);
        assert_eq!(
            store.saves.load(Ordering::SeqCst),
            1,
            "tried again once a minute, not on every poll"
        );
    }

    #[test]
    fn a_poisoned_lock_doesnt_stop_requests_retrying_a_save() {
        let store = Arc::new(Flaky::default());
        store
            .inner
            .save(&session(Some(tokens("r1", later()))))
            .unwrap();
        let sessions =
            Arc::new(Sessions::open_with(Box::new(Arc::clone(&store)), Duration::ZERO).unwrap());
        store.failing.store(true, Ordering::SeqCst);
        assert!(sessions.sign_out().is_err());
        let poisoner = Arc::clone(&sessions);
        let panicked = std::thread::spawn(move || {
            let _changing = poisoner.changing.lock();
            panic!("a panic while changing the session");
        })
        .join();
        assert!(panicked.is_err() && sessions.changing.is_poisoned());
        store.failing.store(false, Ordering::SeqCst);
        assert_eq!(sessions.bearer(None, never).unwrap(), None);
        assert_eq!(
            stored_refresh(&store.inner),
            None,
            "the request saved the sign-out"
        );
    }

    #[test]
    fn a_change_in_progress_doesnt_use_up_the_retry() {
        let store = Arc::new(Flaky::default());
        store
            .inner
            .save(&session(Some(tokens("r1", later()))))
            .unwrap();
        let sessions =
            Sessions::open_with(Box::new(Arc::clone(&store)), Duration::from_hours(1)).unwrap();
        store.failing.store(true, Ordering::SeqCst);
        assert!(sessions.sign_out().is_err());
        store.failing.store(false, Ordering::SeqCst);
        {
            let _change_in_progress = locked(&sessions.changing);
            sessions.retry_save_now_and_then();
        }
        assert_eq!(
            stored_refresh(&store.inner),
            Some(RefreshToken::new("r1")),
            "not while busy"
        );
        sessions.retry_save_now_and_then();
        assert_eq!(
            stored_refresh(&store.inner),
            None,
            "and its try wasn't used up"
        );
    }

    #[test]
    fn a_newer_clients_session_appearing_mid_run_isnt_overwritten() {
        #[derive(Debug)]
        struct TakenOver {
            inner: MemorySessionStore,
            newer: AtomicBool,
        }
        impl SessionStore for TakenOver {
            fn load(&self) -> Result<Option<Session>, StoreError> {
                if self.newer.load(Ordering::SeqCst) {
                    return Err(StoreError::NewerFormat {
                        path: "session.json".into(),
                        version: 2,
                    });
                }
                self.inner.load()
            }
            fn save(&self, session: &Session) -> Result<(), StoreError> {
                assert!(
                    !self.newer.load(Ordering::SeqCst),
                    "a newer session is never overwritten"
                );
                self.inner.save(session)
            }
            fn clear(&self) -> Result<(), StoreError> {
                self.inner.clear()
            }
        }
        let store = Arc::new(TakenOver {
            inner: MemorySessionStore::with_session(session(Some(tokens(
                "r1",
                SystemTime::UNIX_EPOCH,
            )))),
            newer: AtomicBool::new(false),
        });
        let sessions = Sessions::open(Box::new(Arc::clone(&store))).unwrap();
        store.newer.store(true, Ordering::SeqCst);
        let newer = |error: Option<Error>| {
            matches!(error, Some(Error::Store(StoreError::NewerFormat { .. })))
        };
        assert!(newer(sessions.sign_out().err()));
        assert!(newer(sessions.bearer(None, never).err()));
    }

    #[test]
    fn a_failed_save_of_a_sign_out_is_reported_and_tried_again() {
        let store = Arc::new(Flaky::default());
        store
            .inner
            .save(&session(Some(tokens("r1", later()))))
            .unwrap();
        let sessions = Sessions::open_with(Box::new(Arc::clone(&store)), Duration::ZERO).unwrap();
        store.failing.store(true, Ordering::SeqCst);
        assert!(matches!(sessions.sign_out(), Err(Error::Store(_))));
        assert!(!sessions.is_signed_in(), "signed out here all the same");
        assert_eq!(sessions.bearer(None, never).unwrap(), None);
        store.failing.store(false, Ordering::SeqCst);
        assert_eq!(sessions.bearer(None, never).unwrap(), None);
        assert_eq!(stored_refresh(&store.inner), None);
    }

    #[test]
    fn a_waiting_renewal_uses_the_one_that_got_there_first() {
        let store = store_with(Some(tokens("r1", SystemTime::UNIX_EPOCH)));
        let sessions = Sessions::open(Box::new(Arc::clone(&store))).unwrap();
        // Stands in for a renewal that finished while this one waited for its turn.
        store.save(&session(Some(tokens("r2", later())))).unwrap();
        assert_eq!(sessions.bearer(None, never).unwrap(), Some(token("r2")));
    }

    #[test]
    fn an_unreachable_server_leaves_a_token_that_still_lasts() {
        let soon = SystemTime::now() + Duration::from_secs(30);
        let sessions = Sessions::open(Box::new(store_with(Some(tokens("r1", soon))))).unwrap();
        let bearer = sessions.bearer(None, |_| {
            Err(TransportError::new("connection failed").into())
        });
        assert_eq!(bearer.unwrap(), Some(token("r1")));
        assert!(sessions.is_signed_in());
        let rejected = AccessToken::new("access-for-r1");
        let error = sessions.bearer(Some(&rejected), |_| {
            Err(TransportError::new("connection failed").into())
        });
        assert!(
            matches!(error, Err(Error::Network(_))),
            "a rejected token isn't sent again"
        );
    }

    #[test]
    fn an_unreachable_server_with_a_token_that_ran_out_is_an_error() {
        let sessions = Sessions::open(Box::new(store_with(Some(tokens(
            "r1",
            SystemTime::UNIX_EPOCH,
        )))))
        .unwrap();
        let error = sessions
            .bearer(None, |_| {
                Err(TransportError::new("connection failed").into())
            })
            .unwrap_err();
        assert!(matches!(error, Error::Network(_)), "{error}");
        assert!(
            sessions.is_signed_in(),
            "the session stays for the next try"
        );
    }

    #[test]
    fn renewals_run_one_at_a_time() {
        let store = store_with(Some(tokens("r1", SystemTime::UNIX_EPOCH)));
        let sessions = Arc::new(Sessions::open(Box::new(store)).unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let start = Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (sessions, calls, start) = (
                    Arc::clone(&sessions),
                    Arc::clone(&calls),
                    Arc::clone(&start),
                );
                std::thread::spawn(move || {
                    start.wait();
                    sessions.bearer(None, |_| {
                        calls.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(50));
                        Ok(grant("r2"))
                    })
                })
            })
            .collect();
        for thread in threads {
            assert_eq!(thread.join().unwrap().unwrap(), Some(token("r2")));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_stopped_session_waits_for_its_renewal_saves_and_renews_nothing_more() {
        /// Its first save fails, the others work.
        #[derive(Debug, Default)]
        struct FailsOnce {
            inner: MemorySessionStore,
            failed: AtomicBool,
        }
        impl SessionStore for FailsOnce {
            fn load(&self) -> Result<Option<Session>, StoreError> {
                self.inner.load()
            }
            fn save(&self, session: &Session) -> Result<(), StoreError> {
                if !self.failed.swap(true, Ordering::SeqCst) {
                    return Err(StoreError::Backend("the disk is full".into()));
                }
                self.inner.save(session)
            }
            fn clear(&self) -> Result<(), StoreError> {
                self.inner.clear()
            }
        }
        let store = Arc::new(FailsOnce::default());
        store
            .inner
            .save(&session(Some(tokens("r1", SystemTime::UNIX_EPOCH))))
            .unwrap();
        let sessions = Arc::new(Sessions::open(Box::new(Arc::clone(&store))).unwrap());
        // A renewal is out when the stop comes; its grant's save fails.
        let out = Arc::new(std::sync::Barrier::new(2));
        let renewing = {
            let (sessions, out) = (Arc::clone(&sessions), Arc::clone(&out));
            std::thread::spawn(move || {
                sessions.bearer(None, |_| {
                    out.wait();
                    std::thread::sleep(Duration::from_millis(50));
                    Ok(grant("r2"))
                })
            })
        };
        out.wait();
        sessions.stop().unwrap();
        assert_eq!(renewing.join().unwrap().unwrap(), Some(token("r2")));
        assert_eq!(
            stored_refresh(&store.inner),
            Some(RefreshToken::new("r2")),
            "the renewal in flight was waited for, and its session saved by the stop"
        );
        assert!(sessions.is_stopped());
        assert!(matches!(sessions.bearer(None, never), Err(Error::Stopped)));
        let rejected = token("r2");
        assert!(matches!(
            sessions.bearer(Some(&rejected), never),
            Err(Error::Stopped)
        ));
    }

    #[test]
    fn a_corrupt_store_starts_a_new_install() {
        #[derive(Debug, Default)]
        struct Corrupt(MemorySessionStore);
        impl SessionStore for Corrupt {
            fn load(&self) -> Result<Option<Session>, StoreError> {
                match self.0.load()? {
                    Some(session) => Ok(Some(session)),
                    None => Err(StoreError::Corrupt {
                        path: "session.json".into(),
                        source: serde_json::from_str::<Session>("{").unwrap_err(),
                    }),
                }
            }
            fn save(&self, session: &Session) -> Result<(), StoreError> {
                self.0.save(session)
            }
            fn clear(&self) -> Result<(), StoreError> {
                self.0.clear()
            }
        }
        let sessions = Sessions::open(Box::new(Corrupt::default())).unwrap();
        assert!(sessions.install().as_str().starts_with("zr-"));
    }

    #[test]
    fn a_newer_clients_session_in_a_store_that_isnt_a_file_isnt_overwritten() {
        #[derive(Debug)]
        struct TakenOver {
            inner: MemorySessionStore,
            newer: AtomicBool,
        }
        impl SessionStore for TakenOver {
            fn load(&self) -> Result<Option<Session>, StoreError> {
                if self.newer.load(Ordering::SeqCst) {
                    return Err(StoreError::NewerFormatInStore { version: 2 });
                }
                self.inner.load()
            }
            fn save(&self, session: &Session) -> Result<(), StoreError> {
                assert!(
                    !self.newer.load(Ordering::SeqCst),
                    "a newer session is never overwritten"
                );
                self.inner.save(session)
            }
            fn clear(&self) -> Result<(), StoreError> {
                self.inner.clear()
            }
        }
        let store = Arc::new(TakenOver {
            inner: MemorySessionStore::with_session(session(Some(tokens(
                "r1",
                SystemTime::UNIX_EPOCH,
            )))),
            newer: AtomicBool::new(false),
        });
        let sessions = Sessions::open(Box::new(Arc::clone(&store))).unwrap();
        store.newer.store(true, Ordering::SeqCst);
        assert!(matches!(
            sessions.sign_out(),
            Err(Error::Store(StoreError::NewerFormatInStore { version: 2 }))
        ));
    }

    #[test]
    fn a_newer_clients_session_stops_the_client() {
        #[derive(Debug)]
        struct Newer;
        impl SessionStore for Newer {
            fn load(&self) -> Result<Option<Session>, StoreError> {
                Err(StoreError::NewerFormat {
                    path: "session.json".into(),
                    version: 2,
                })
            }
            fn save(&self, _: &Session) -> Result<(), StoreError> {
                panic!("a newer session is never overwritten")
            }
            fn clear(&self) -> Result<(), StoreError> {
                Ok(())
            }
        }
        assert!(matches!(
            Sessions::open(Box::new(Newer)),
            Err(Error::Store(StoreError::NewerFormat { .. }))
        ));
    }
}
