//! The windows of their own across launches, as macOS window restoration keeps them for the
//! reference client: each opens where the user last left it (its frame kept in `window.json`),
//! and the ones open when the app quit open again at launch, except a quick note, which starts
//! afresh each time. (A file's window keeps only its size, and isn't reopened: it shows a copy in
//! a temporary folder.) They wait for the gate: none opens over the sign-in screen, a device
//! the server doesn't let in, or an app still connecting.

use gpui_kit::{App, Bounds, Global, Pixels, Subscription, WindowBounds, px, size};

use super::WindowKind;
use super::{unseen, windows};
use crate::prefs::Kept;
use crate::store::gate::Gate;

/// The smallest kept frame a window of its own opens at (a smaller one is taken as damaged).
const SMALLEST: (f32, f32) = (200., 150.);

/// Whether a window of `kind` open at quit opens again at launch.
#[must_use]
fn reopens(kind: WindowKind) -> bool {
    !matches!(kind, WindowKind::QuickNote)
}

/// The windows to open again, from the names kept at the last quit: each known one that
/// reopens, once, in the kept order.
#[must_use]
fn to_reopen(names: &[String]) -> Vec<WindowKind> {
    let mut kinds = Vec::new();
    for kind in names
        .iter()
        .filter_map(|name| WindowKind::from_link_name(name))
    {
        if reopens(kind) && !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    kinds
}

/// Where a window of `kind` opens: where it was last left, if that is still on a display, else
/// centred at its first size.
#[must_use]
pub(super) fn bounds(kind: WindowKind, cx: &App) -> WindowBounds {
    let displays: Vec<Bounds<Pixels>> = cx
        .displays()
        .iter()
        .map(|display| display.bounds())
        .collect();
    Kept::get(cx)
        .own_windows
        .frames
        .get(kind.link_name())
        .and_then(|frame| frame.restore_at_least(&displays, SMALLEST))
        .unwrap_or_else(|| {
            let (width, height) = kind.default_size();
            WindowBounds::Windowed(Bounds::centered(None, size(px(width), px(height)), cx))
        })
}

/// Notes the frame of the window of `kind` as the user moved or sized it.
pub(super) fn note_frame(kind: WindowKind, bounds: WindowBounds, cx: &mut App) {
    Kept::note_own_window(cx, kind.link_name(), bounds);
}

/// Notes which windows of their own are open (the app quitting: they open again next time).
pub(crate) fn note_open(cx: &mut App) {
    let names = windows::open_singles(cx)
        .into_iter()
        .filter(|kind| reopens(*kind))
        .map(|kind| kind.link_name().to_owned())
        .collect();
    Kept::note_open_windows(cx, names);
}

/// The watch on the gate while the windows to reopen wait for it to open (held, never read).
struct Reopening {
    _watch: Option<Subscription>,
}

impl Global for Reopening {}

/// Opens again the windows of their own that were open when the app last quit, behind the main
/// window and without the keyboard, once the gate is open (at once if it is: the main window
/// opens after, in front).
pub(crate) fn reopen(cx: &mut App) {
    let kinds = to_reopen(&Kept::get(cx).own_windows.open);
    let Some(store) = windows::store(cx) else {
        return;
    };
    if kinds.is_empty() {
        return;
    }
    if store.read(cx).gate() == Gate::Open {
        open_behind(&kinds, cx);
        return;
    }
    let mut waiting = Some(kinds);
    let watch = cx.observe(&store, move |store, cx| {
        if store.read(cx).gate() != Gate::Open {
            return;
        }
        let Some(kinds) = waiting.take() else {
            return;
        };
        open_behind(&kinds, cx);
        // The main windows opened first: the one in front stays there.
        if let Some(main) = windows::front_main(cx)
            && let Err(error) = main.update(cx, |_, window, cx| unseen::bring_forward(window, cx))
        {
            log::debug!("the main window closed as the others reopened: {error}");
        }
        // Done watching (dropped after this call, not inside it).
        cx.defer(|cx| cx.set_global(Reopening { _watch: None }));
    });
    cx.set_global(Reopening {
        _watch: Some(watch),
    });
}

/// Opens each of `kinds` without the keyboard.
fn open_behind(kinds: &[WindowKind], cx: &mut App) {
    for &kind in kinds {
        if !windows::open_single_as(kind, false, cx) {
            log::debug!("the {} window isn't in this build", kind.title());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn the_windows_open_at_quit_open_again_but_a_quick_note_and_unknown_ones_dont() {
        assert_eq!(
            to_reopen(&names(&[
                "calendar", "note", "sessions", "calendar", "gone"
            ])),
            [WindowKind::Calendar, WindowKind::Sessions]
        );
        assert_eq!(to_reopen(&[]), []);
        for kind in WindowKind::ALL {
            assert_eq!(reopens(kind), kind != WindowKind::QuickNote, "{kind:?}");
        }
    }
}
