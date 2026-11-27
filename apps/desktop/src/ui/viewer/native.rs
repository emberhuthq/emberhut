//! What the platform shows in the viewer that GPUI can't draw: on macOS a video in `AVKit`'s
//! player (the system's controls: play, scrub, volume, full screen), and a deck, document or
//! sheet in Quick Look's preview (slides scroll, a sheet has its tabs). Each is a native view
//! laid over the viewer's body in the window's view, placed where the body is drawn each frame
//! and taken out when the viewer lets go of it. Elsewhere there is none, and such a file opens
//! in its app.
//!
//! A native view sits above everything GPUI draws in its bounds, so it is only made where
//! nothing of the app's own lies over it: the viewer's body, between its bars.

#[cfg(target_os = "macos")]
pub(super) use self::mac::Native;
#[cfg(not(target_os = "macos"))]
pub(super) use self::other::Native;
#[cfg(all(test, not(target_os = "macos")))]
pub(super) use self::other::stand_in;

#[cfg(target_os = "macos")]
mod mac {
    use std::path::Path;

    use gpui_kit::prelude::*;
    use gpui_kit::{AnyElement, Bounds, Pixels, Window, canvas};
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::NSView;
    use objc2_av_foundation::AVPlayer;
    use objc2_av_kit::AVPlayerView;
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSURL};
    use objc2_quick_look_ui::{QLPreviewView, QLPreviewViewStyle};

    use crate::store::files::Kind;
    use crate::ui::share::content_view;

    /// A native view over the viewer's body.
    pub(in crate::ui::viewer) struct Native {
        /// The view, a subview of the window's.
        view: Retained<NSView>,
        /// The window's view it is in.
        host: Retained<NSView>,
        /// A video's player, paused when the view goes.
        player: Option<Retained<AVPlayer>>,
        /// Quick Look's view, closed when the view goes.
        preview: Option<Retained<QLPreviewView>>,
    }

    impl Native {
        /// The platform's view of the `kind` file kept at `path` in `window`, playing (a video)
        /// or previewing (a document); `None` for any other kind, or when it can't be made.
        #[expect(
            unsafe_code,
            reason = "AVKit's and Quick Look's setters are unsafe in objc2, as they take objects"
        )]
        pub(in crate::ui::viewer) fn make(
            kind: Kind,
            path: &Path,
            window: &Window,
        ) -> Option<Self> {
            let mtm = MainThreadMarker::new()?;
            let host = content_view(window)?;
            let url = NSURL::from_file_path(path)?;
            let frame = NSRect::new(NSPoint::new(0., 0.), NSSize::new(1., 1.));
            let made = match kind {
                Kind::Video => {
                    // SAFETY: a player for a file URL and a player view of a frame, the view
                    // given the live player to show (it keeps it), then playing.
                    let (player, view) = unsafe {
                        let player = AVPlayer::playerWithURL(&url, mtm);
                        let view = AVPlayerView::initWithFrame(mtm.alloc(), frame);
                        view.setPlayer(Some(&player));
                        player.play();
                        (player, view)
                    };
                    Self {
                        view: Retained::into_super(view),
                        host,
                        player: Some(player),
                        preview: None,
                    }
                }
                Kind::Document => {
                    // SAFETY: a Quick Look view of a file URL, which is a preview item.
                    let preview = unsafe {
                        QLPreviewView::initWithFrame_style(
                            mtm.alloc(),
                            frame,
                            QLPreviewViewStyle::Normal,
                        )
                    }?;
                    // SAFETY: an NSURL is a QLPreviewItem.
                    unsafe {
                        preview.setAutostarts(true);
                        preview.setPreviewItem(Some(ProtocolObject::from_ref(&*url)));
                    }
                    Self {
                        view: Retained::into_super(preview.clone()),
                        host,
                        player: None,
                        preview: Some(preview),
                    }
                }
                Kind::Picture | Kind::Pdf | Kind::Report | Kind::Note => return None,
            };
            made.view.setHidden(true);
            made.host.addSubview(&made.view);
            Some(made)
        }

        /// The element in the body that places the view where it is drawn.
        pub(in crate::ui::viewer) fn element(&self) -> AnyElement {
            let (view, host) = (self.view.clone(), self.host.clone());
            canvas(
                |_, _, _| {},
                move |bounds, (), _, _| place(&view, &host, bounds),
            )
            .size_full()
            .into_any_element()
        }
    }

    /// Puts `view` over `bounds` (GPUI's points, from the window's top left) in `host`.
    fn place(view: &NSView, host: &NSView, bounds: Bounds<Pixels>) {
        let (x, y) = (f64::from(bounds.origin.x), f64::from(bounds.origin.y));
        let (width, height) = (f64::from(bounds.size.width), f64::from(bounds.size.height));
        // An unflipped view's points run up from its bottom.
        let y = if host.isFlipped() {
            y
        } else {
            host.bounds().size.height - y - height
        };
        let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(width, height));
        if view.frame() != frame {
            view.setFrame(frame);
        }
        if view.isHidden() {
            view.setHidden(false);
        }
    }

    impl Drop for Native {
        #[expect(
            unsafe_code,
            reason = "AVPlayer's pause and Quick Look's close are unsafe in objc2"
        )]
        fn drop(&mut self) {
            // SAFETY: both are live objects of this view.
            unsafe {
                if let Some(player) = &self.player {
                    player.pause();
                }
                if let Some(preview) = &self.preview {
                    preview.close();
                }
            }
            self.view.removeFromSuperview();
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod other {
    use std::path::Path;

    use gpui_kit::{AnyElement, Window};

    use crate::store::files::Kind;

    /// No native view here: never made.
    #[cfg(not(test))]
    pub(in crate::ui::viewer) enum Native {}

    /// In the tests, a stand-in that counts while it is up ([`stand_in`]), made only when a
    /// test asks for it.
    #[cfg(test)]
    #[derive(Debug)]
    pub(in crate::ui::viewer) struct Native(());

    impl Native {
        /// Nothing here shows videos or documents: they open in their app.
        #[cfg(not(test))]
        pub(in crate::ui::viewer) fn make(
            _kind: Kind,
            _path: &Path,
            _window: &Window,
        ) -> Option<Self> {
            None
        }

        /// A stand-in for a video or a document, when the test turned them on.
        #[cfg(test)]
        pub(in crate::ui::viewer) fn make(
            kind: Kind,
            _path: &Path,
            _window: &Window,
        ) -> Option<Self> {
            let made = matches!(kind, Kind::Video | Kind::Document) && stand_in::on();
            made.then(|| {
                stand_in::count(1);
                Self(())
            })
        }

        /// Never called: there is no native view.
        #[cfg(not(test))]
        pub(in crate::ui::viewer) fn element(&self) -> AnyElement {
            match *self {}
        }

        /// The stand-in's place: an empty element.
        #[cfg(test)]
        #[expect(
            clippy::unused_self,
            reason = "called as the Mac's view is, which places itself"
        )]
        pub(in crate::ui::viewer) fn element(&self) -> AnyElement {
            use gpui_kit::prelude::*;
            gpui_kit::div().size_full().into_any_element()
        }
    }

    #[cfg(test)]
    impl Drop for Native {
        fn drop(&mut self) {
            stand_in::count(-1);
        }
    }

    /// The tests' stand-ins: turned on per thread, and how many are up.
    #[cfg(test)]
    pub(in crate::ui::viewer) mod stand_in {
        use std::cell::Cell;

        thread_local! {
            static ON: Cell<bool> = const { Cell::new(false) };
            static UP: Cell<isize> = const { Cell::new(0) };
        }

        /// Videos and documents get a stand-in from now on, on this thread.
        pub(in crate::ui::viewer) fn turn_on() {
            ON.with(|on| on.set(true));
        }

        /// Whether they do.
        pub(super) fn on() -> bool {
            ON.with(Cell::get)
        }

        /// How many stand-ins are up.
        pub(in crate::ui::viewer) fn up() -> isize {
            UP.with(Cell::get)
        }

        pub(super) fn count(by: isize) {
            UP.with(|up| up.set(up.get() + by));
        }
    }
}
