//! Where the app's main windows are for the user, together: the store polls and marks rooms
//! read by the one [`WindowState`] it is told, so each window says what the platform says of
//! it here and the store hears the sum. The app is in front while any of its main windows is
//! the active one and shown while any is on screen; with none open (on macOS the app outlives
//! its last window) it is out of sight, and nothing is polled.

use std::collections::HashMap;

use gpui_kit::{App, Context, Entity, Global, Window, WindowId};

use crate::store::Store;
use crate::store::presence::WindowState;

/// What each main window last said.
#[derive(Debug, Default)]
pub(crate) struct Windows(HashMap<WindowId, WindowState>);

impl Global for Windows {}

impl Windows {
    /// The sum of every window's state: active if one is, shown if one is; out of sight with
    /// none.
    #[must_use]
    pub(crate) fn sum(&self) -> WindowState {
        WindowState {
            active: self.0.values().any(|window| window.active),
            visible: self.0.values().any(|window| window.visible),
        }
    }

    /// Notes `window`'s state.
    fn set(&mut self, window: WindowId, state: WindowState) {
        self.0.insert(window, state);
    }

    /// Forgets a closed window.
    fn remove(&mut self, window: WindowId) {
        self.0.remove(&window);
    }
}

/// `window` is (or isn't) the active one now.
pub(crate) fn set_active(window: WindowId, active: bool, store: &Entity<Store>, cx: &mut App) {
    change(window, store, cx, |state| state.active = active);
}

/// `window` is (or isn't) shown now.
pub(crate) fn set_visible(window: WindowId, visible: bool, store: &Entity<Store>, cx: &mut App) {
    change(window, store, cx, |state| state.visible = visible);
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
    set_visible(id, window.is_visible(), store, cx);
    set_active(id, window.is_window_active(), store, cx);
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
    let mut state = windows.0.get(&window).copied().unwrap_or_default();
    change(&mut state);
    windows.set(window, state);
    tell(store, cx);
}

/// Tells the store the sum.
fn tell(store: &Entity<Store>, cx: &mut App) {
    let sum = cx.default_global::<Windows>().sum();
    store.update(cx, |store, cx| {
        store.set_window_visible(sum.visible);
        store.set_window_active(sum.active, cx);
    });
    crate::lock::seen(sum.visible, cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::presence::Presence;

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
