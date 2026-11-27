//! Share: the system's share menu (mail, messages, nearby devices, the apps that take a file)
//! where the platform has one, for a link or a file. On macOS that is
//! `NSSharingServicePicker`, shown at the pointer over the window's view, with the link or the
//! file itself (its real name, so the app it goes to gets that). Other platforms have none
//! here: Share isn't offered.

use std::path::PathBuf;

use gpui_kit::{App, Window};

/// What is shared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Shared {
    /// A link.
    Link(String),
    /// A file on this device.
    File(PathBuf),
}

/// Whether the platform has a share menu (else Share isn't offered).
pub(crate) const HAS_MENU: bool = platform::HAS_MENU;

/// Opens the share menu for `shared` at the pointer in `window`.
pub(crate) fn share(shared: Shared, window: &Window, cx: &mut App) {
    platform::share(shared, window, cx);
}

#[cfg(target_os = "macos")]
mod platform {
    use std::cell::RefCell;

    use gpui_kit::{App, Window};
    use objc2::AnyThread as _;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSSharingServicePicker, NSView};
    use objc2_foundation::{NSArray, NSPoint, NSRect, NSRectEdge, NSSize, NSString, NSURL};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use super::Shared;

    /// The Mac has a share menu.
    pub(super) const HAS_MENU: bool = true;

    thread_local! {
        /// The menu shown last, kept until the next one: `AppKit` doesn't say it keeps a picker
        /// alive while it shows.
        static SHOWN: RefCell<Option<Retained<NSSharingServicePicker>>> =
            const { RefCell::new(None) };
    }

    /// Opens the share menu for `shared` at the pointer.
    pub(super) fn share(shared: Shared, window: &Window, cx: &mut App) {
        let url = match shared {
            Shared::Link(link) => NSURL::URLWithString(&NSString::from_str(&link)),
            Shared::File(path) => NSURL::from_file_path(path),
        };
        let Some(url) = url else {
            log::debug!("what is shared isn't a URL");
            return;
        };
        let Some(view) = content_view(window) else {
            return;
        };
        let at = window.mouse_position();
        let (x, y) = (f64::from(at.x), f64::from(at.y));
        // Shown from the main thread's queue, once the click's dispatch is over: the menu may
        // track the mouse in an event loop of its own.
        cx.spawn(async move |_| show(&url, &view, x, y)).detach();
    }

    /// The window's view (its content), retained.
    #[expect(
        unsafe_code,
        reason = "the window handle gives the view as a raw pointer"
    )]
    pub(crate) fn content_view(window: &Window) -> Option<Retained<NSView>> {
        // The raw-window-handle trait's handle, not GPUI's own `Window::window_handle`.
        let handle = match HasWindowHandle::window_handle(window) {
            Ok(handle) => handle,
            Err(error) => {
                log::debug!("no window view: {error}");
                return None;
            }
        };
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            log::debug!("the window isn't an AppKit one");
            return None;
        };
        // SAFETY: an AppKit handle's `ns_view` points to the window's NSView, alive while the
        // handle borrows the window; retaining it keeps it alive after that.
        unsafe { Retained::retain(appkit.ns_view.as_ptr().cast::<NSView>()) }
    }

    /// Shows the menu for `url` at (`x`, `y`), in points from the view's top left.
    #[expect(unsafe_code, reason = "the picker's items are an untyped NSArray")]
    fn show(url: &NSURL, view: &NSView, x: f64, y: f64) {
        let item: &AnyObject = url;
        let items = NSArray::<AnyObject>::from_slice(&[item]);
        // SAFETY: the one item is an NSURL, a type the picker takes (NSPasteboardWriting).
        let picker = unsafe {
            NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), &items)
        };
        // GPUI's points run down from the top; an unflipped view's run up from the bottom.
        let (y, below) = if view.isFlipped() {
            (y, NSRectEdge::MaxY)
        } else {
            (view.bounds().size.height - y, NSRectEdge::MinY)
        };
        let at = NSRect::new(NSPoint::new(x, y), NSSize::new(1.0, 1.0));
        picker.showRelativeToRect_ofView_preferredEdge(at, view, below);
        SHOWN.with_borrow_mut(|shown| *shown = Some(picker));
    }
}

#[cfg(target_os = "macos")]
pub(crate) use platform::content_view;

#[cfg(not(target_os = "macos"))]
mod platform {
    use gpui_kit::{App, ClipboardItem, Window};

    use super::Shared;

    /// No share menu here.
    pub(super) const HAS_MENU: bool = false;

    /// Share isn't offered here; were it asked, a link is copied instead.
    pub(super) fn share(shared: Shared, _window: &Window, cx: &mut App) {
        match shared {
            Shared::Link(link) => cx.write_to_clipboard(ClipboardItem::new_string(link)),
            Shared::File(path) => log::debug!("no share menu here for {}", path.display()),
        }
    }
}
