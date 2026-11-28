//! "Light it" with a plan the person already pays for: their Claude Pro or Max accounts,
//! signed in on Emberhut here through Anthropic's own sign-in ([`crate::hut`]'s accounts).
//! "Sign in with Claude" adds an account and opens Anthropic's page in the browser; the store
//! looks every few seconds until the CLI says it is signed in, then starts Emberhut here again
//! on the accounts, and the step closes once the server says the ember thinks. More accounts
//! can be added, one put first (the next work goes to it), and the ember let move to one with
//! room when one runs out (off until the person turns it on).
//!
//! Only Emberhut here (the app's own hut) signs accounts in this way; for another computer
//! the step says to sign in there.

use std::time::Duration;

use gpui_kit::AppContext as _;
use gpui_kit::Context;

use super::super::{Store, update_or_log};
use crate::hut::{HutAccount, LocalHut};

/// How often a sign-in under way is looked at.
const LOOK_EVERY: Duration = Duration::from_secs(2);

/// How long a sign-in is waited for before the step stops looking (the person can try again).
const WAIT_AT_MOST: Duration = Duration::from_secs(10 * 60);

/// The plans' accounts as the step shows them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Plans {
    /// The accounts, in the order the server tries them (the first takes the next work).
    pub(crate) accounts: Vec<HutAccount>,
    /// Whether the ember moves to an account with room when one runs out.
    pub(crate) moves: bool,
    /// The account whose sign-in is under way in the browser.
    pub(crate) waiting: Option<String>,
    /// Emberhut here is starting again on the accounts.
    pub(crate) restarting: bool,
    /// What went wrong last, in words.
    pub(crate) problem: Option<String>,
    /// Which look this is, so a newer one wins.
    look: u64,
}

impl Plans {
    /// The signed-in accounts.
    pub(crate) fn signed_in(&self) -> impl Iterator<Item = &HutAccount> {
        self.accounts.iter().filter(|account| account.signed_in)
    }

    /// The step's line once lit with the accounts: "Lit with your Claude Max plan
    /// (you@example.com)." or how many.
    #[must_use]
    pub(crate) fn lit_line(&self) -> Option<String> {
        let signed_in: Vec<&HutAccount> = self.signed_in().collect();
        match signed_in.as_slice() {
            [] => None,
            [one] => Some(match &one.email {
                Some(email) => format!("Lit with your {} plan ({email}).", one.plan_name()),
                None => format!("Lit with your {} plan.", one.plan_name()),
            }),
            many => Some(format!("Lit with {} Claude accounts.", many.len())),
        }
    }
}

impl Store {
    /// The hut, when Emberhut here is the app's own.
    fn plan_hut(&self) -> Option<LocalHut> {
        self.setup.home.hut()
    }

    /// Whether accounts can be signed in from here (Emberhut here is the app's own).
    #[must_use]
    pub(crate) fn signs_in_plans(&self) -> bool {
        self.plan_hut().is_some()
    }

    /// The plans' accounts.
    #[must_use]
    pub(crate) fn plans(&self) -> &Plans {
        &self.setup.plans
    }

    /// Reads the accounts as their CLI says now.
    pub(crate) fn load_plans(&mut self, cx: &mut Context<'_, Self>) {
        let Some(hut) = self.plan_hut() else {
            return;
        };
        let read = cx.background_spawn(async move { (hut.accounts_now(), hut.moves_now()) });
        cx.spawn(async move |this, cx| {
            let (accounts, moves) = read.await;
            update_or_log(&this, cx, |store, cx| {
                store.setup.plans.accounts = accounts;
                store.setup.plans.moves = moves;
                cx.notify();
            });
        })
        .detach();
    }

    /// "Sign in with Claude" (or "Add another Claude account"): an account is added and
    /// Anthropic's sign-in opens in the browser.
    pub(crate) fn sign_in_with_claude(&mut self, cx: &mut Context<'_, Self>) {
        let Some(hut) = self.plan_hut() else {
            return;
        };
        if self.setup.plans.waiting.is_some() {
            return;
        }
        self.setup.plans.problem = None;
        let added = cx.background_spawn(async move { hut.add_account_now() });
        cx.spawn(async move |this, cx| {
            let added = added.await;
            update_or_log(&this, cx, |store, cx| match added {
                Ok(id) => store.wait_for_sign_in(id, cx),
                Err(error) => {
                    store.setup.plans.problem = Some(error.to_string());
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// "Sign in again" on an account whose sign-in ran out or was left.
    pub(crate) fn sign_in_again(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let Some(hut) = self.plan_hut() else {
            return;
        };
        let id = id.to_owned();
        let again = {
            let id = id.clone();
            cx.background_spawn(async move { hut.sign_in_again_now(&id) })
        };
        cx.spawn(async move |this, cx| {
            let again = again.await;
            update_or_log(&this, cx, |store, cx| match again {
                Ok(()) => store.wait_for_sign_in(id, cx),
                Err(error) => {
                    store.setup.plans.problem = Some(error.to_string());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Looks every few seconds until account `id` is signed in, then starts Emberhut here
    /// again on the accounts.
    fn wait_for_sign_in(&mut self, id: String, cx: &mut Context<'_, Self>) {
        let Some(hut) = self.plan_hut() else {
            return;
        };
        self.setup.plans.waiting = Some(id.clone());
        self.setup.plans.look += 1;
        let look = self.setup.plans.look;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let tries = WAIT_AT_MOST.as_secs() / LOOK_EVERY.as_secs();
            for _ in 0..tries {
                cx.background_executor().timer(LOOK_EVERY).await;
                let hut = hut.clone();
                let accounts = cx.background_spawn(async move { hut.accounts_now() }).await;
                let done = accounts.iter().any(|account| account.id == id && account.signed_in);
                let current = this
                    .update(cx, |store, cx| {
                        if store.setup.plans.look != look {
                            return false;
                        }
                        store.setup.plans.accounts = accounts;
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !current {
                    return;
                }
                if done {
                    update_or_log(&this, cx, |store, cx| {
                        store.setup.plans.waiting = None;
                        store.use_plans(cx);
                    });
                    return;
                }
            }
            update_or_log(&this, cx, |store, cx| {
                if store.setup.plans.look == look {
                    store.setup.plans.waiting = None;
                    store.setup.plans.problem =
                        Some("The sign-in wasn't finished. Try again when you're ready.".to_owned());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Stops waiting for a sign-in (the person closed the browser page); the account stays,
    /// signed out, with "Sign in again".
    pub(crate) fn stop_waiting_for_sign_in(&mut self, cx: &mut Context<'_, Self>) {
        self.setup.plans.look += 1;
        self.setup.plans.waiting = None;
        cx.notify();
    }

    /// Starts Emberhut here again on the accounts as they are, then asks the server whether
    /// the ember thinks (which closes "Light it" when it does).
    fn use_plans(&mut self, cx: &mut Context<'_, Self>) {
        let Some(hut) = self.plan_hut() else {
            return;
        };
        self.setup.plans.restarting = true;
        cx.notify();
        let used = cx.background_spawn(async move {
            let used = hut.use_accounts_now();
            (used, hut.accounts_now(), hut.moves_now())
        });
        cx.spawn(async move |this, cx| {
            let (used, accounts, moves) = used.await;
            update_or_log(&this, cx, |store, cx| {
                store.setup.plans.restarting = false;
                store.setup.plans.accounts = accounts;
                store.setup.plans.moves = moves;
                match used {
                    Ok(_) => store.read_setup_page(cx),
                    Err(error) => store.setup.plans.problem = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Puts account `id` first: the next work goes to it.
    pub(crate) fn put_plan_first(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let id = id.to_owned();
        self.change_plans(cx, move |hut| hut.put_first_now(&id));
    }

    /// Takes account `id` off.
    pub(crate) fn remove_plan(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let id = id.to_owned();
        self.change_plans(cx, move |hut| hut.remove_account_now(&id));
    }

    /// Lets the ember move to an account with room when one runs out, or not.
    pub(crate) fn set_plans_move(&mut self, moves: bool, cx: &mut Context<'_, Self>) {
        self.setup.plans.moves = moves;
        self.change_plans(cx, move |hut| hut.set_moves_now(moves));
    }

    /// Makes `change` to the hut's accounts, then starts Emberhut here again on them.
    fn change_plans(
        &mut self,
        cx: &mut Context<'_, Self>,
        change: impl FnOnce(&LocalHut) -> Result<(), crate::hut::HutError> + Send + 'static,
    ) {
        let Some(hut) = self.plan_hut() else {
            return;
        };
        let changed = cx.background_spawn(async move { change(&hut) });
        cx.spawn(async move |this, cx| {
            let changed = changed.await;
            update_or_log(&this, cx, |store, cx| match changed {
                Ok(()) => store.use_plans(cx),
                Err(error) => {
                    store.setup.plans.problem = Some(error.to_string());
                    cx.notify();
                }
            });
        })
        .detach();
    }
}
