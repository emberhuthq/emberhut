//! The account as the views see it, one GPUI entity: where sign-in stands (reading the
//! Keychain, signed out, signing in with a code or in the sign-in window, signed in), the
//! account's machines, linking this computer and unlinking one, and the line saying how the
//! last action went. The blocking work runs on background threads through
//! [`AccountSession`]; this module only sequences it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use emberhut_api::directory::LinkedMachine;
use emberhut_identity::MachineId;
use gpui_kit::{AppContext as _, AsyncApp, Context, Entity, WeakEntity};

use super::device_grant::{DeviceGrant, Next};
use super::pkce::CodeFlow;
use super::session::{AccountError, AccountSession};
use super::web_session::{self, WebSession};
use crate::backend::Backend;
use crate::store::Store;
use crate::store::presence::Presence;
use crate::store::settings::SettingsPane;

/// How old the machines' list may get while Settings shows it in front.
const FRESH_FOR: Duration = Duration::from_secs(30);

/// Where sign-in stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// The Keychain is being read.
    Reading,
    /// Not signed in.
    SignedOut,
    /// Signing in.
    SigningIn(SignIn),
    /// Signed in.
    SignedIn,
}

/// A sign-in under way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SignIn {
    /// Asking the provider for a code, or for its sign-in page.
    Starting,
    /// The device grant's code, waiting to be approved on `page`.
    Code {
        /// The code the person types there.
        user_code: String,
        /// The page to open.
        page: String,
    },
    /// The system's sign-in window is open.
    Window,
}

/// The account.
pub(crate) struct Account {
    session: Arc<AccountSession>,
    backend: Arc<dyn Backend>,
    phase: Phase,
    machines: Option<Vec<LinkedMachine>>,
    fetched_at: Option<Instant>,
    fetching: bool,
    linking: bool,
    unlink_asked: Option<MachineId>,
    note: Option<String>,
    /// Counts sign-ins, so the answers of one cancelled are dropped.
    attempt: u64,
    /// The sign-in window, while it is open.
    window: Option<WebSession>,
}

impl Account {
    /// The account of `session`, linking through `backend` (the server this app talks to, and
    /// then whichever the store moves to), refreshing its machines while `store` shows Settings
    /// in front. Reads the Keychain first.
    pub(crate) fn new(
        session: Arc<AccountSession>,
        backend: Arc<dyn Backend>,
        store: &Entity<Store>,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        cx.observe(store, |account, store, cx| {
            let store = store.read(cx);
            if !Arc::ptr_eq(&account.backend, store.backend()) {
                account.backend = Arc::clone(store.backend());
            }
            let wanted = store.shows_settings(&[SettingsPane::Account])
                && store.presence() == Presence::Front;
            if wanted {
                account.refresh_if_stale(cx);
            }
        })
        .detach();
        let reading = Arc::clone(&session);
        cx.spawn(async move |this, cx| {
            let read = cx.background_spawn(async move { reading.restore() }).await;
            update(&this, cx, |account, cx| {
                match read {
                    Ok(true) => {
                        account.phase = Phase::SignedIn;
                        account.refresh(cx);
                    }
                    Ok(false) => account.phase = Phase::SignedOut,
                    Err(error) => {
                        log::warn!("couldn't read the account's sign-in: {error}");
                        account.phase = Phase::SignedOut;
                        account.note =
                            Some(format!("Couldn't read your account's sign-in: {error}."));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        Self {
            session,
            backend,
            phase: Phase::Reading,
            machines: None,
            fetched_at: None,
            fetching: false,
            linking: false,
            unlink_asked: None,
            note: None,
            attempt: 0,
            window: None,
        }
    }

    /// Where sign-in stands.
    #[must_use]
    pub(crate) fn phase(&self) -> &Phase {
        &self.phase
    }

    /// The account's machines, once fetched.
    #[must_use]
    pub(crate) fn machines(&self) -> Option<&[LinkedMachine]> {
        self.machines.as_deref()
    }

    /// How the last action went, or what went wrong.
    #[must_use]
    pub(crate) fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// Whether this computer is being added.
    #[must_use]
    pub(crate) fn is_linking(&self) -> bool {
        self.linking
    }

    /// The machine whose Unlink waits to be confirmed.
    #[must_use]
    pub(crate) fn unlink_asked(&self) -> Option<&MachineId> {
        self.unlink_asked.as_ref()
    }

    /// Why the sign-in lasts only until the app quits, when it does.
    #[must_use]
    pub(crate) fn unkept(&self) -> Option<String> {
        self.session.unkept()
    }

    /// Why "Add this computer" can't work here, when it can't: the app's server isn't on this
    /// computer, or this device has no session with it.
    #[must_use]
    pub(crate) fn cant_add_this_computer(&self) -> Option<String> {
        let server = self.backend.server().as_str();
        let authority = server.split_once("://").map_or(server, |(_, rest)| rest);
        if !super::switch::is_loopback_host(authority) {
            return Some(format!(
                "To add a computer, run this app on it, talking to its own server. This app \
                 talks to {server}."
            ));
        }
        (!self.backend.is_signed_in()).then(|| {
            "Pair this app with the server on this computer first (Sign in, above).".to_owned()
        })
    }

    /// Starts signing in: the system's sign-in window in a build that claims the callback's
    /// domain on a Mac that has it, the device grant otherwise.
    pub(crate) fn sign_in(&mut self, cx: &mut Context<'_, Self>) {
        if matches!(self.phase, Phase::SigningIn(_)) {
            return;
        }
        self.attempt += 1;
        self.phase = Phase::SigningIn(SignIn::Starting);
        self.note = None;
        if self.session.settings().web_session && web_session::available() {
            self.sign_in_with_window(cx);
        } else {
            self.sign_in_with_code(cx);
        }
        cx.notify();
    }

    /// The device grant: a code, then polls until it is approved, refused or runs out.
    fn sign_in_with_code(&mut self, cx: &mut Context<'_, Self>) {
        let (session, attempt) = (Arc::clone(&self.session), self.attempt);
        let since = session.generation();
        cx.spawn(async move |this, cx| {
            let starting = Arc::clone(&session);
            let started = cx
                .background_spawn(async move { starting.start_device_grant() })
                .await;
            let authorization = match started {
                Ok(authorization) => authorization,
                Err(error) => {
                    update(&this, cx, |account, cx| {
                        account.sign_in_failed(attempt, &error, cx);
                    });
                    return;
                }
            };
            let mut grant = DeviceGrant::new(&authorization, Instant::now());
            let shown = update(&this, cx, |account, cx| {
                if account.attempt != attempt {
                    return false;
                }
                account.phase = Phase::SigningIn(SignIn::Code {
                    user_code: authorization.user_code.clone(),
                    page: authorization.page().to_owned(),
                });
                cx.notify();
                true
            });
            if shown != Some(true) {
                return;
            }
            let authorization = Arc::new(authorization);
            loop {
                cx.background_executor().timer(grant.interval()).await;
                if !is_current(&this, cx, attempt) {
                    return;
                }
                let (polling, asked) = (Arc::clone(&session), Arc::clone(&authorization));
                let poll = cx
                    .background_spawn(async move { polling.poll_device_grant(&asked) })
                    .await;
                match grant.after(poll, Instant::now()) {
                    Next::Wait(_) => {}
                    Next::Done(tokens) => {
                        // Cancelled or signed out while this poll was out: nothing is kept.
                        if !is_current(&this, cx, attempt) {
                            return;
                        }
                        let keeping = Arc::clone(&session);
                        let kept = cx
                            .background_spawn(async move { keeping.signed_in(tokens, since) })
                            .await;
                        update(&this, cx, |account, cx| {
                            account.sign_in_ended(attempt, kept, cx);
                        });
                        return;
                    }
                    Next::Stop(end) => {
                        update(&this, cx, |account, cx| {
                            account.sign_in_failed(attempt, &AccountError::Grant(end), cx);
                        });
                        return;
                    }
                }
            }
        })
        .detach();
    }

    /// The system's sign-in window, with PKCE and the HTTPS callback.
    fn sign_in_with_window(&mut self, cx: &mut Context<'_, Self>) {
        let (session, attempt) = (Arc::clone(&self.session), self.attempt);
        let since = session.generation();
        let Ok(flow) = CodeFlow::new() else {
            self.sign_in_failed(attempt, &AccountError::Random, cx);
            return;
        };
        cx.spawn(async move |this, cx| {
            let (asking, asked) = (Arc::clone(&session), flow.clone());
            let page = cx
                .background_spawn(async move { asking.authorize_url(&asked) })
                .await;
            let answer = update(&this, cx, |account, cx| {
                if account.attempt != attempt {
                    return None;
                }
                let started = page.and_then(|page| {
                    web_session::start(&page).map_err(|ended| {
                        AccountError::Grant(super::device_grant::GrantEnd::Failed(
                            ended.to_string(),
                        ))
                    })
                });
                match started {
                    Ok((window, answer)) => {
                        account.window = Some(window);
                        account.phase = Phase::SigningIn(SignIn::Window);
                        cx.notify();
                        Some(answer)
                    }
                    Err(error) => {
                        account.sign_in_failed(attempt, &error, cx);
                        None
                    }
                }
            })
            .flatten();
            let Some(answer) = answer else {
                return;
            };
            let callback = answer.await;
            let current = update(&this, cx, |account, _| {
                let current = account.attempt == attempt;
                if current {
                    account.window = None;
                }
                current
            });
            if current != Some(true) {
                return;
            }
            let kept = match callback {
                Ok(callback) => {
                    cx.background_spawn(
                        async move { session.finish_code_flow(&flow, &callback, since) },
                    )
                    .await
                }
                Err(ended) => Err(AccountError::Grant(super::device_grant::GrantEnd::Failed(
                    ended.to_string(),
                ))),
            };
            update(&this, cx, |account, cx| {
                account.sign_in_ended(attempt, kept, cx);
            });
        })
        .detach();
    }

    /// Sign-in attempt `attempt` ended with `kept`.
    fn sign_in_ended(
        &mut self,
        attempt: u64,
        kept: Result<(), AccountError>,
        cx: &mut Context<'_, Self>,
    ) {
        match kept {
            Ok(()) if self.attempt == attempt => {
                self.forget_machines();
                self.phase = Phase::SignedIn;
                self.note = None;
                self.refresh(cx);
                cx.notify();
            }
            Ok(()) => {}
            Err(error) => self.sign_in_failed(attempt, &error, cx),
        }
    }

    /// Sign-in attempt `attempt` failed: signed out, saying why.
    fn sign_in_failed(&mut self, attempt: u64, error: &AccountError, cx: &mut Context<'_, Self>) {
        if self.attempt != attempt {
            return;
        }
        log::info!("the account's sign-in failed: {error}");
        self.abandon_sign_in();
        self.phase = Phase::SignedOut;
        self.note = Some(error.to_string());
        cx.notify();
    }

    /// Stops the sign-in under way (its late answers are dropped).
    pub(crate) fn cancel_sign_in(&mut self, cx: &mut Context<'_, Self>) {
        if !matches!(self.phase, Phase::SigningIn(_)) {
            return;
        }
        self.attempt += 1;
        if let Some(window) = self.window.take() {
            window.cancel();
        }
        self.abandon_sign_in();
        self.phase = Phase::SignedOut;
        cx.notify();
    }

    /// Drops the sign-in under way in the session too, so a code's exchange or a request for
    /// the account's id still out can't keep its tokens.
    fn abandon_sign_in(&self) {
        if matches!(self.phase, Phase::SigningIn(_)) {
            self.session.abandon();
        }
    }

    /// Signs out of the account here. The device's sessions with its machines stay.
    pub(crate) fn sign_out(&mut self, cx: &mut Context<'_, Self>) {
        self.attempt += 1;
        self.phase = Phase::SignedOut;
        self.forget_machines();
        self.note =
            Some("Signed out of your account. This device stays signed in to its huts.".to_owned());
        if let Some(window) = self.window.take() {
            window.cancel();
        }
        // At once in memory, so nothing under way keeps its tokens; the Keychain after.
        let generation = self.session.sign_out();
        let session = Arc::clone(&self.session);
        cx.background_spawn(async move { session.forget_kept(generation) })
            .detach();
        cx.notify();
    }

    /// Fetches the machines again when the list is older than [`FRESH_FOR`].
    fn refresh_if_stale(&mut self, cx: &mut Context<'_, Self>) {
        if self.fetched_at.is_none_or(|at| at.elapsed() >= FRESH_FOR) {
            self.refresh(cx);
        }
    }

    /// Fetches the account's machines (one fetch at a time).
    pub(crate) fn refresh(&mut self, cx: &mut Context<'_, Self>) {
        if self.phase != Phase::SignedIn || self.fetching {
            return;
        }
        self.fetching = true;
        self.fetched_at = Some(Instant::now());
        let (session, attempt) = (Arc::clone(&self.session), self.attempt);
        cx.spawn(async move |this, cx| {
            let fetched = cx.background_spawn(async move { session.machines() }).await;
            update(&this, cx, |account, cx| {
                account.fetched(attempt, fetched, cx);
            });
        })
        .detach();
    }

    /// The machines fetched for sign-in `attempt`: dropped when the account signed out or in
    /// again meanwhile.
    fn fetched(
        &mut self,
        attempt: u64,
        fetched: Result<Vec<LinkedMachine>, AccountError>,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.is_still(attempt) {
            return;
        }
        self.fetching = false;
        match fetched {
            Ok(machines) => self.machines = Some(machines),
            Err(error) => self.failed(&error),
        }
        cx.notify();
    }

    /// Whether the account is still signed in at sign-in `attempt`.
    fn is_still(&self, attempt: u64) -> bool {
        self.attempt == attempt && self.phase == Phase::SignedIn
    }

    /// Forgets the machines, as the account signs out or in again.
    fn forget_machines(&mut self) {
        self.machines = None;
        self.fetched_at = None;
        self.fetching = false;
        self.linking = false;
        self.unlink_asked = None;
    }

    /// Unlink pressed on machine `id`: it waits to be confirmed.
    pub(crate) fn ask_unlink(&mut self, id: MachineId, cx: &mut Context<'_, Self>) {
        self.unlink_asked = Some(id);
        cx.notify();
    }

    /// The unlink isn't confirmed after all.
    pub(crate) fn cancel_unlink(&mut self, cx: &mut Context<'_, Self>) {
        if self.unlink_asked.take().is_some() {
            cx.notify();
        }
    }

    /// Unlinks machine `id` (named `name`) from the account, confirmed.
    pub(crate) fn unlink(&mut self, id: MachineId, name: String, cx: &mut Context<'_, Self>) {
        self.unlink_asked = None;
        let (session, attempt) = (Arc::clone(&self.session), self.attempt);
        cx.spawn(async move |this, cx| {
            let done = cx
                .background_spawn(async move { session.unlink(&id) })
                .await;
            update(&this, cx, |account, cx| {
                account.unlinked(attempt, &id, &name, done, cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// Machine `id` (`name`) unlinked for sign-in `attempt`, or why not: dropped when the
    /// account signed out or in again meanwhile.
    fn unlinked(
        &mut self,
        attempt: u64,
        id: &MachineId,
        name: &str,
        done: Result<(), AccountError>,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.is_still(attempt) {
            return;
        }
        match done {
            Ok(()) => {
                self.note = Some(format!("Unlinked {name} from your account."));
                if let Some(machines) = &mut self.machines {
                    machines.retain(|machine| machine.machine.id() != id);
                }
            }
            Err(error) => self.failed(&error),
        }
        self.fetched_at = None;
        self.refresh(cx);
        cx.notify();
    }

    /// Adds the computer this app runs on to the account: the server on it signs the link,
    /// and the directory takes it.
    pub(crate) fn add_this_computer(&mut self, cx: &mut Context<'_, Self>) {
        if self.linking || self.phase != Phase::SignedIn {
            return;
        }
        self.linking = true;
        self.note = None;
        let (session, backend) = (Arc::clone(&self.session), Arc::clone(&self.backend));
        let attempt = self.attempt;
        cx.spawn(async move |this, cx| {
            let linked = cx
                .background_spawn(async move { session.link_this_computer(&*backend) })
                .await;
            update(&this, cx, |account, cx| account.linked(attempt, linked, cx));
        })
        .detach();
        cx.notify();
    }

    /// This computer linked for sign-in `attempt`, or why not: dropped when the account signed
    /// out or in again meanwhile.
    fn linked(
        &mut self,
        attempt: u64,
        linked: Result<LinkedMachine, AccountError>,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.is_still(attempt) {
            return;
        }
        self.linking = false;
        match linked {
            Ok(machine) => {
                self.note = Some(format!("Added {} to your account.", machine.name));
                self.fetched_at = None;
                self.refresh(cx);
            }
            Err(error) => self.failed(&error),
        }
        cx.notify();
    }

    /// An action failed: signed out when the account's sign-in is gone, a line saying why
    /// otherwise (a busy or unreachable directory never signs out).
    fn failed(&mut self, error: &AccountError) {
        if error.is_signed_out() {
            self.attempt += 1;
            self.phase = Phase::SignedOut;
            self.forget_machines();
        }
        self.note = Some(error.to_string());
    }
}

/// Whether sign-in attempt `attempt` is still the account's.
fn is_current(this: &WeakEntity<Account>, cx: &mut AsyncApp, attempt: u64) -> bool {
    update(this, cx, |account, _| account.attempt == attempt).unwrap_or(false)
}

/// Runs `change` on the account if it still exists (it may not as the app quits).
fn update<R>(
    this: &WeakEntity<Account>,
    cx: &mut AsyncApp,
    change: impl FnOnce(&mut Account, &mut Context<'_, Account>) -> R,
) -> Option<R> {
    this.update(cx, change)
        .inspect_err(|error| log::debug!("the account is gone: {error}"))
        .ok()
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
