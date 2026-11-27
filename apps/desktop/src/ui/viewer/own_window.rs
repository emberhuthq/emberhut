//! A file in a window of its own (Open in a window, or ⌃⌘F from the sheet): resizable, full
//! screen with ⌃⌘F (F11 off the Mac) or the window's own button, Esc leaving full screen. Each
//! opens at the size the user last gave one (85% of the display the first time), kept across
//! launches.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::{App, Bounds, Entity, TitlebarOptions, WindowBounds, WindowOptions, px, size};

use super::FileViewer;
use crate::brand::APP_ID;
use crate::prefs::Kept;
use crate::shell::unseen;
use crate::store::Store;
use crate::store::files::RemoteFile;
use crate::ui::look;

/// How long the new window takes to be in front before it goes full screen.
const SETTLE: Duration = Duration::from_millis(400);

/// The smallest a file's window is.
const SMALLEST: (f32, f32) = (420., 320.);

/// The size a file's window opens at: the last one given, else 85% of a `display` big.
#[must_use]
fn opening_size(kept: Option<(f32, f32)>, display: (f32, f32)) -> (f32, f32) {
    let (width, height) = kept.unwrap_or((display.0 * 0.85, display.1 * 0.85));
    (width.max(SMALLEST.0), height.max(SMALLEST.1))
}

/// Notes the size of `bounds`, a file's window as the user left it, for the next one.
pub(super) fn note_size(bounds: WindowBounds, cx: &mut App) {
    if let WindowBounds::Windowed(bounds) = bounds {
        let (width, height) = (bounds.size.width, bounds.size.height);
        Kept::note_file_window(cx, f32::from(width), f32::from(height));
    }
}

/// Opens `file` in a window of its own, full screen once it is in front when `full_screen`.
pub(crate) fn open(store: &Entity<Store>, file: RemoteFile, full_screen: bool, cx: &mut App) {
    let display = cx.primary_display().map_or((1440., 900.), |display| {
        let bounds = display.bounds();
        (f32::from(bounds.size.width), f32::from(bounds.size.height))
    });
    let (width, height) = opening_size(Kept::get(cx).file_window, display);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(width), px(height)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(file.name.clone().into()),
            ..TitlebarOptions::default()
        }),
        app_id: Some(APP_ID.into()),
        window_min_size: Some(size(px(SMALLEST.0), px(SMALLEST.1))),
        window_background: look::background_now(cx),
        ..WindowOptions::default()
    };
    let store = store.clone();
    let opened = unseen::open_window(options, cx, |window, cx| {
        let page = store.read(cx).theme().page().clone();
        look::apply(&page, window, cx);
        cx.new(|cx| FileViewer::in_window(store.clone(), file, window, cx))
    });
    match opened {
        Ok((handle, _)) => {
            unseen::activate_app(cx);
            if full_screen {
                cx.spawn(async move |cx| {
                    cx.background_executor().timer(SETTLE).await;
                    if let Err(error) =
                        handle.update(cx, |_, window, cx| unseen::toggle_fullscreen(window, cx))
                    {
                        log::debug!("the file's window closed before full screen: {error}");
                    }
                })
                .detach();
            }
        }
        Err(error) => log::error!("couldn't open the file's window: {error:#}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_opens_at_the_last_size_or_most_of_the_display() {
        assert_eq!(
            opening_size(Some((900., 640.)), (1440., 900.)),
            (900., 640.)
        );
        assert_eq!(opening_size(None, (1000., 800.)), (850., 680.));
        assert_eq!(
            opening_size(Some((100., 100.)), (1440., 900.)),
            (420., 320.)
        );
    }
}
