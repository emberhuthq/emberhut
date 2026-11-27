//! Where the app's main windows are for the user, together: the store polls and marks rooms
//! read by the one [`WindowState`] it is told, so each window says what the platform says of
//! it here and the store hears the sum. The app is in front while any of its main windows is
//! the active one and shown while any is on screen; with none open (on macOS the app outlives
//! its last window) it is out of sight, and nothing is polled.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use gpui_kit::{App, AsyncApp, Context, Entity, Global, Window, WindowId};

use crate::store::Store;
use crate::store::presence::WindowState;

/// How long after a window opens its visibility is read again, from the platform itself.
const LOOK_AGAIN: Duration = Duration::from_secs(1);

/// What each window last said.
#[derive(Debug, Default)]
pub(crate) struct Windows {
    /// Each window's state.
    states: HashMap<WindowId, WindowState>,
    /// The windows whose visibility the platform has told, by a change or when read again
    /// after [`LOOK_AGAIN`]; until then a window counts as shown without it being known.
    told: HashSet<WindowId>,
}

impl Global for Windows {}

impl Windows {
    /// The sum of every window's state: active if one is, shown if one is; out of sight with
    /// none.
    #[must_use]
    pub(crate) fn sum(&self) -> WindowState {
        WindowState {
            active: self.states.values().any(|window| window.active),
            visible: self.states.values().any(|window| window.visible),
        }
    }

    /// Whether the app is known to be in sight: a window is active, or one the platform has
    /// told of is shown. Only then may the lock ask to unlock.
    #[must_use]
    fn surely_seen(&self) -> bool {
        self.states
            .iter()
            .any(|(window, state)| state.active || (state.visible && self.told.contains(window)))
    }

    /// Notes `window`'s state.
    fn set(&mut self, window: WindowId, state: WindowState) {
        self.states.insert(window, state);
    }

    /// Forgets a closed window.
    fn remove(&mut self, window: WindowId) {
        self.states.remove(&window);
        self.told.remove(&window);
    }
}

/// `window` is (or isn't) the active one now.
pub(crate) fn set_active(window: WindowId, active: bool, store: &Entity<Store>, cx: &mut App) {
    change(window, store, cx, |state| state.active = active);
}

/// `window` is (or isn't) shown now, as the platform tells.
pub(crate) fn set_visible(window: WindowId, visible: bool, store: &Entity<Store>, cx: &mut App) {
    cx.default_global::<Windows>().told.insert(window);
    change(window, store, cx, |state| state.visible = visible);
}

/// `window` just opened: it counts as shown, and as active if it is, until the platform tells
/// of a change or, [`LOOK_AGAIN`] later, its visibility is read again from the platform.
///
/// What a new window says of its visibility at once can't be trusted: `AppKit` works out a
/// window's occlusion only after it is ordered in, so one opening behind the app in use
/// (`--background`) often says it is hidden for its first moments, and GPUI tells only of
/// changes from that first answer. Taken at its word, the app would think itself out of sight
/// at launch and cover its windows ([`crate::lock`]) though they are on screen. A window that
/// really opens out of sight (launched hidden, on another Space, on a sleeping display) may
/// never change, so it is read again; and until the platform has told of a shown window, or
/// one is active, the lock doesn't ask to unlock.
pub(crate) fn opened(window: &Window, store: &Entity<Store>, cx: &mut App) {
    let handle = window.window_handle();
    let id = handle.window_id();
    change(id, store, cx, |state| state.visible = true);
    set_active(id, window.is_window_active(), store, cx);
    let store = store.clone();
    cx.spawn(async move |cx: &mut AsyncApp| {
        cx.background_executor().timer(LOOK_AGAIN).await;
        // A window closed by then has nothing to say.
        if let Ok(visible) = handle.update(cx, |_, window, _| on_screen(window)) {
            cx.update(|cx| set_visible(id, visible, &store, cx));
        }
    })
    .detach();
}

/// Whether `window` is on screen now, as the platform has it. On macOS that is the window's
/// occlusion read from `AppKit`, as GPUI's answer may still be the one from before the window
/// was ordered in.
fn on_screen(window: &Window) -> bool {
    #[cfg(target_os = "macos")]
    if let Some(ns_window) = crate::ui::share::content_view(window).and_then(|view| view.window()) {
        return ns_window
            .occlusionState()
            .contains(objc2_app_kit::NSWindowOcclusionState::Visible);
    }
    window.is_visible()
}

/// Makes `window`, one of the app's other windows (a window of its own, a file's), count for
/// where the app is as a main window does, for as long as it is open: with only it in front
/// the store still polls, and out of sight it doesn't (nor does the app stay unlocked).
pub(crate) fn follow<V: 'static>(
    window: &mut Window,
    store: &Entity<Store>,
    cx: &mut Context<'_, V>,
) {
    let id = window.window_handle().window_id();
    opened(window, store, cx);
    let active_store = store.clone();
    cx.observe_window_activation(window, move |_, window, cx| {
        set_active(id, window.is_window_active(), &active_store, cx);
    })
    .detach();
    let shown_store = store.clone();
    cx.observe_window_visibility(window, move |_, visibility, _, cx| {
        set_visible(id, visibility.is_visible(), &shown_store, cx);
    })
    .detach();
}

/// `window` closed.
pub(crate) fn closed(window: WindowId, store: &Entity<Store>, cx: &mut App) {
    cx.default_global::<Windows>().remove(window);
    tell(store, cx);
}

fn change(
    window: WindowId,
    store: &Entity<Store>,
    cx: &mut App,
    change: impl FnOnce(&mut WindowState),
) {
    let windows = cx.default_global::<Windows>();
    let mut state = windows.states.get(&window).copied().unwrap_or_default();
    change(&mut state);
    windows.set(window, state);
    tell(store, cx);
}

/// Tells the store the sum. In an unseen run no window is ever the active one (the app never
/// comes forward), yet the run drives the app as someone in front of it would: with a window
/// open the app counts as in front, so rooms are read, polls run and held answers wait as they
/// do in front.
fn tell(store: &Entity<Store>, cx: &mut App) {
    let windows = cx.default_global::<Windows>();
    let open = !windows.states.is_empty();
    let mut sum = windows.sum();
    let surely_seen = windows.surely_seen();
    if open && super::unseen::is_unseen(cx) {
        sum.active = true;
    }
    store.update(cx, |store, cx| {
        store.set_window_visible(sum.visible);
        store.set_window_active(sum.active, cx);
    });
    crate::lock::seen(sum.visible, surely_seen, cx);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::prelude::*;
    use gpui_kit::{TestAppContext, WindowVisibility, div};

    use super::*;
    use crate::backend::fake::FakeBackend;
    use crate::lock::{self, Lock};
    use crate::store::presence::Presence;

    /// A window's stand-in view.
    struct Blank;

    impl Render for Blank {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            div()
        }
    }

    /// Opens a window that says it is hidden before any change (behind the app in use, or
    /// really out of sight), followed for where the app is, over `lock`.
    fn open_said_hidden(
        lock: Lock,
        cx: &mut TestAppContext,
    ) -> (Entity<Store>, gpui_kit::WindowHandle<Blank>) {
        let store = cx.new(|_| Store::for_tests(Arc::new(FakeBackend::new([]))));
        cx.update(|cx| cx.set_global(lock));
        let window = cx.add_window(|_, _| Blank);
        cx.simulate_window_visibility_change(window.into(), WindowVisibility::Hidden);
        window
            .update(cx, |_, window, cx| follow(window, &store, cx))
            .unwrap();
        cx.run_until_parked();
        (store, window)
    }

    #[gpui_kit::test]
    fn a_window_that_starts_hidden_and_stays_hidden_is_covered_and_asks_nothing(
        cx: &mut TestAppContext,
    ) {
        // A Mac's lock, locked at launch.
        let (store, window) = open_said_hidden(Lock::at_launch(true, false), cx);
        let state = |cx: &mut TestAppContext| cx.update(|cx| lock::state(cx));
        // Asking would open it off the Mac, where there is nothing to ask with.
        assert!(
            state(cx).offers_unlock(),
            "not asked while it isn't known to show"
        );
        cx.executor().advance_clock(LOOK_AGAIN * 2);
        cx.run_until_parked();
        assert!(state(cx).covered() && state(cx).offers_unlock());
        assert_eq!(
            store.read_with(cx, |store, _| store.presence()),
            Presence::Hidden,
            "read again, it is out of sight: nothing polls"
        );
        let says_hidden = window.update(cx, |_, window, _| !window.is_visible());
        assert!(says_hidden.unwrap(), "the window still says it is hidden");
        // Only once it really shows does the lock ask (and, with nothing to ask with, open).
        #[cfg(not(target_os = "macos"))]
        {
            cx.simulate_window_visibility_change(window.into(), WindowVisibility::Visible);
            cx.run_until_parked();
            assert!(!state(cx).covered(), "asked once it showed");
        }
    }

    #[gpui_kit::test]
    fn a_window_opened_with_the_lock_open_and_out_of_sight_is_covered_once_read_again(
        cx: &mut TestAppContext,
    ) {
        // The lock opened at launch (`--no-prompt`).
        let (_, _window) = open_said_hidden(Lock::at_launch(true, true), cx);
        let covered = |cx: &mut TestAppContext| cx.update(|cx| lock::state(cx).covered());
        assert!(!covered(cx), "counted as shown at first");
        cx.executor().advance_clock(LOOK_AGAIN * 2);
        cx.run_until_parked();
        assert!(
            covered(cx),
            "really out of sight: covered for the Dock's snapshots"
        );
    }

    #[gpui_kit::test]
    fn launched_in_the_background_with_no_visibility_change_yet_it_isnt_covered(
        cx: &mut TestAppContext,
    ) {
        // A Mac's lock opened at launch (`--no-prompt`); behind the app in use, the new window
        // says it is hidden before any change.
        let (store, window) = open_said_hidden(Lock::at_launch(true, true), cx);
        let covered = |cx: &mut TestAppContext| cx.update(|cx| lock::state(cx).covered());
        assert!(!covered(cx), "not covered until it says it is out of sight");
        assert_ne!(
            store.read_with(cx, |store, _| store.presence()),
            Presence::Hidden,
            "and the store polls"
        );
        // Then the window really goes out of sight, and comes back.
        cx.simulate_window_visibility_change(window.into(), WindowVisibility::Visible);
        cx.simulate_window_visibility_change(window.into(), WindowVisibility::Hidden);
        cx.run_until_parked();
        assert!(covered(cx), "out of sight, it is covered");
        cx.simulate_window_visibility_change(window.into(), WindowVisibility::Visible);
        cx.run_until_parked();
        assert!(!covered(cx), "back soon, it opens");
    }

    fn id(n: u64) -> WindowId {
        WindowId::from(n)
    }

    #[test]
    fn the_app_is_in_front_while_any_main_window_is_and_hidden_with_none() {
        let mut windows = Windows::default();
        assert_eq!(windows.sum().presence(), Presence::Hidden, "no window");
        windows.set(id(1), WindowState::default());
        assert_eq!(windows.sum().presence(), Presence::Behind);
        windows.set(
            id(2),
            WindowState {
                active: true,
                visible: true,
            },
        );
        assert_eq!(windows.sum().presence(), Presence::Front, "the second is");
        windows.set(
            id(1),
            WindowState {
                active: false,
                visible: false,
            },
        );
        assert_eq!(
            windows.sum().presence(),
            Presence::Front,
            "one minimised, the other in front"
        );
        windows.remove(id(2));
        assert_eq!(
            windows.sum().presence(),
            Presence::Hidden,
            "the other one is minimised"
        );
        windows.remove(id(1));
        assert_eq!(
            windows.sum(),
            WindowState {
                active: false,
                visible: false
            }
        );
    }
}
