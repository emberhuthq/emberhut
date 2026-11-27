//! The app lock: at launch, and when the app comes back after more than 5 minutes out of sight,
//! every window is covered by "Emberhut is locked" until the user unlocks it with Touch ID or the
//! Mac's password. While no window is on screen (all hidden or minimised) the cover is up too,
//! so the Dock's and Mission Control's snapshots show nothing. With nothing to ask with (a Mac
//! without a password, or not a Mac) it opens; when the system can't ask for another reason it
//! stays locked, and the cover says so, with Unlock to try again.
//!
//! The lock is one for the whole app ([`Lock`], a global), as every window shows the same chat.

mod system;

use std::time::{Duration, Instant};

use gpui_kit::{App, Global};

use self::system::CantAsk;
use crate::brand::APP_NAME;

/// How long the app may be out of sight before it locks again.
pub(crate) const AWAY: Duration = Duration::from_mins(5);

/// Whether the lock is shut.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Shut {
    /// Open: the windows show the app.
    #[default]
    Open,
    /// Locked: only Unlock opens it.
    Locked,
    /// Locked, and the system is asking for Touch ID or the password now.
    Unlocking,
}

/// The lock's state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Lock {
    /// This system can ask who is there; without it the app never locks.
    enabled: bool,
    /// Shut or open.
    shut: Shut,
    /// When the app went out of sight, while it is.
    left_at: Option<Instant>,
    /// No window is on screen.
    hidden: bool,
    /// The system couldn't ask the last time Unlock was pressed.
    couldnt_ask: bool,
}

impl Global for Lock {}

impl Lock {
    /// The lock at launch: locked where the system can ask (`enabled`), unless `open` (a run
    /// for screenshots or a test harness, which still locks after time away).
    #[must_use]
    pub(crate) const fn at_launch(enabled: bool, open: bool) -> Self {
        Self {
            enabled,
            shut: if enabled && !open {
                Shut::Locked
            } else {
                Shut::Open
            },
            left_at: None,
            hidden: false,
            couldnt_ask: false,
        }
    }

    /// Whether the windows are covered.
    #[must_use]
    pub(crate) const fn covered(&self) -> bool {
        self.offers_unlock() || self.hidden
    }

    /// Whether the cover offers Unlock (it is locked, not only out of sight).
    #[must_use]
    pub(crate) const fn offers_unlock(&self) -> bool {
        !matches!(self.shut, Shut::Open)
    }

    /// Whether the system couldn't ask the last time Unlock was pressed (the cover says so).
    #[must_use]
    pub(crate) const fn couldnt_ask(&self) -> bool {
        self.couldnt_ask && matches!(self.shut, Shut::Locked)
    }

    /// The app went out of sight at `now` (the first time counts while it stays out).
    fn hide(&mut self, now: Instant) {
        self.hidden = true;
        self.left_at.get_or_insert(now);
    }

    /// The app is on screen again at `now`: it locks if it was away longer than [`AWAY`].
    /// Whether to ask to unlock now.
    fn show(&mut self, now: Instant) -> bool {
        self.hidden = false;
        if let Some(left) = self.left_at.take()
            && self.enabled
            && now.saturating_duration_since(left) > AWAY
            && self.shut == Shut::Open
        {
            self.shut = Shut::Locked;
        }
        self.offers_unlock()
    }

    /// Starts asking to unlock; false while already asking, or when it isn't locked.
    fn begin_unlock(&mut self) -> bool {
        if self.shut != Shut::Locked {
            return false;
        }
        self.shut = Shut::Unlocking;
        self.couldnt_ask = false;
        true
    }

    /// The system answered: `opened`, it unlocks.
    fn finish_unlock(&mut self, opened: bool) {
        self.shut = if opened { Shut::Open } else { Shut::Locked };
    }

    /// The system couldn't ask (`why`): with nothing to ask with it opens; for any other reason
    /// it stays locked, Unlock there to try again.
    fn cant_ask(&mut self, why: &CantAsk) {
        match why {
            CantAsk::NothingToAskWith => self.shut = Shut::Open,
            CantAsk::Failed(_) => {
                self.shut = Shut::Locked;
                self.couldnt_ask = true;
            }
        }
    }
}

/// Sets up the lock at launch: locked unless `open`, or when this system has no way to ask.
pub(crate) fn install(open: bool, cx: &mut App) {
    cx.set_global(Lock::at_launch(system::AVAILABLE, open));
}

/// The lock now (open when it isn't set up, as in tests).
#[must_use]
pub(crate) fn state(cx: &App) -> Lock {
    cx.try_global::<Lock>().copied().unwrap_or_default()
}

/// Whether any of the app's windows is on screen, as the windows say it (all of them summed).
pub(crate) fn seen(visible: bool, cx: &mut App) {
    let Some(lock) = cx.try_global::<Lock>().copied() else {
        return;
    };
    let mut next = lock;
    let ask = if visible {
        next.show(Instant::now())
    } else {
        next.hide(Instant::now());
        false
    };
    if next != lock {
        cx.set_global(next);
        cx.refresh_windows();
    }
    if ask {
        unlock(cx);
    }
}

/// Asks for Touch ID or the password, and opens the app if the user gives it. With nothing to
/// ask with (no password set on this Mac), it opens at once; when the system can't ask for any
/// other reason, it stays locked.
pub(crate) fn unlock(cx: &mut App) {
    let mut lock = state(cx);
    if !lock.begin_unlock() {
        return;
    }
    cx.set_global(lock);
    match system::ask(&format!("unlock {APP_NAME}")) {
        Ok(asking) => {
            cx.spawn(async move |cx| {
                let opened = asking.await;
                cx.update(|cx| finish(|lock| lock.finish_unlock(opened), cx));
            })
            .detach();
        }
        Err(why) => {
            match &why {
                CantAsk::NothingToAskWith => {
                    log::info!("the app lock opens: there is no password to ask for");
                }
                CantAsk::Failed(said) => {
                    log::warn!("the app lock stays locked: the system can't ask now ({said})");
                }
            }
            finish(|lock| lock.cant_ask(&why), cx);
        }
    }
}

/// The lock changed by `change`, and the windows drawn again.
fn finish(change: impl FnOnce(&mut Lock), cx: &mut App) {
    let mut lock = state(cx);
    change(&mut lock);
    cx.set_global(lock);
    cx.refresh_windows();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_locked_at_launch_unless_told_to_open() {
        assert!(Lock::at_launch(true, false).covered());
        assert!(Lock::at_launch(true, false).offers_unlock());
        assert!(!Lock::at_launch(true, true).covered());
        assert!(
            !Lock::at_launch(false, false).covered(),
            "nothing to ask with"
        );
    }

    #[test]
    fn without_a_way_to_ask_it_never_locks_again() {
        let start = Instant::now();
        let mut lock = Lock::at_launch(false, false);
        lock.hide(start);
        assert!(!lock.show(start + AWAY * 2));
        assert!(!lock.covered());
    }

    #[test]
    fn out_of_sight_it_is_covered_and_back_soon_it_opens() {
        let start = Instant::now();
        let mut lock = Lock::at_launch(true, true);
        lock.hide(start);
        assert!(lock.covered() && !lock.offers_unlock());
        // Hidden again while hidden: the first time counts.
        lock.hide(start + Duration::from_secs(200));
        assert!(!lock.show(start + Duration::from_secs(299)));
        assert!(!lock.covered());
    }

    #[test]
    fn away_more_than_five_minutes_it_locks_again() {
        let start = Instant::now();
        let mut lock = Lock::at_launch(true, true);
        lock.hide(start);
        lock.hide(start + Duration::from_secs(120));
        assert!(lock.show(start + AWAY + Duration::from_secs(1)));
        assert!(lock.covered() && lock.offers_unlock());
        // Shown again without leaving: nothing changes.
        assert!(lock.show(start + AWAY * 3));
    }

    #[test]
    fn one_unlock_at_a_time_and_a_refusal_keeps_it_locked() {
        let mut lock = Lock::at_launch(true, false);
        assert!(lock.begin_unlock());
        assert!(!lock.begin_unlock(), "already asking");
        lock.finish_unlock(false);
        assert!(lock.covered());
        assert!(lock.begin_unlock());
        lock.finish_unlock(true);
        assert!(!lock.covered());
        assert!(!lock.begin_unlock(), "nothing to unlock");
    }

    #[test]
    fn only_nothing_to_ask_with_opens_it_without_asking() {
        let mut lock = Lock::at_launch(true, false);
        assert!(lock.begin_unlock());
        lock.cant_ask(&CantAsk::Failed("Biometry is locked out.".into()));
        assert!(lock.covered() && lock.offers_unlock(), "it stays locked");
        assert!(lock.couldnt_ask(), "the cover says why");
        // Unlock is there to try again.
        assert!(lock.begin_unlock());
        assert!(!lock.couldnt_ask(), "asking again");
        lock.cant_ask(&CantAsk::NothingToAskWith);
        assert!(!lock.covered(), "no password set: nothing to ask with");
        assert!(!lock.couldnt_ask());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn off_the_mac_unlock_opens_as_there_is_nothing_to_ask_with() {
        assert_eq!(system::ask("unlock").err(), Some(CantAsk::NothingToAskWith));
    }
}
