//! The bar with Stop: an always-on-top panel at the top of the main screen, on every Space and
//! over full-screen apps, that never takes the focus. It says what the assistant is doing (the
//! whole screen, a recording) and its Stop ends that at once. It is made and changed on the
//! main thread only.

use std::cell::RefCell;
use std::sync::{Arc, OnceLock};

use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSButton, NSColor, NSFont, NSFontWeightMedium, NSLineBreakMode, NSPanel,
    NSScreen, NSStatusWindowLevel, NSTextField, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::desktop::Bar;

/// The bar's width and height, in points.
const SIZE: (f64, f64) = (520.0, 36.0);
/// Its gap below the menu bar.
const GAP: f64 = 8.0;

/// What Stop does: set once, at start-up.
static ON_STOP: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

thread_local! {
    /// The bar shown now, and Stop's target (main thread only).
    static SHOWN: RefCell<Option<(Retained<NSPanel>, Retained<StopTarget>)>> =
        const { RefCell::new(None) };
}

define_class!(
    /// Stop's target: calls [`ON_STOP`] off the main thread, so the bar never waits.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "EmberhutHandsStopTarget"]
    struct StopTarget;

    impl StopTarget {
        #[unsafe(method(stop:))]
        fn stop(&self, _sender: Option<&AnyObject>) {
            std::thread::spawn(|| {
                if let Some(stop) = ON_STOP.get() {
                    stop();
                }
            });
        }
    }
);

impl StopTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: NSObject's init.
        #[expect(unsafe_code, reason = "msg_send init on a new object")]
        unsafe {
            msg_send![Self::alloc(mtm), init]
        }
    }
}

/// Sets what Stop does (once).
pub fn on_stop(stop: impl Fn() + Send + Sync + 'static) {
    if ON_STOP.set(Box::new(stop)).is_err() {
        tracing::warn!("the bar's Stop was set twice");
    }
}

/// Shows the bar as `wanted` wants it when the main thread runs this (the latest ask, so an
/// older one can never win), and tells `wanted` whether it is up.
pub(super) fn show(wanted: Arc<Bar>) {
    DispatchQueue::main().exec_async(move || {
        let (asked, text) = wanted.wanted();
        let Some(mtm) = MainThreadMarker::new() else {
            wanted.shown(asked, false);
            return;
        };
        let up = SHOWN.with_borrow_mut(|shown| {
            if let Some((panel, _)) = shown.take() {
                panel.orderOut(None);
            }
            if let Some(text) = text {
                *shown = make(&text, mtm);
            }
            shown.is_some()
        });
        wanted.shown(asked, up);
    });
}

/// Makes and shows the bar.
#[expect(
    unsafe_code,
    reason = "a button's target and action, and keeping the panel alive"
)]
fn make(text: &str, mtm: MainThreadMarker) -> Option<(Retained<NSPanel>, Retained<StopTarget>)> {
    let screen = NSScreen::mainScreen(mtm)?;
    let (width, height) = SIZE;
    let frame = screen.frame();
    let visible = screen.visibleFrame();
    let rect = NSRect::new(
        NSPoint::new(
            frame.origin.x + (frame.size.width - width) / 2.0,
            visible.origin.y + visible.size.height - height - GAP,
        ),
        NSSize::new(width, height),
    );
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        rect,
        NSWindowStyleMask::NonactivatingPanel | NSWindowStyleMask::Borderless,
        NSBackingStoreType::Buffered,
        false,
    );
    // SAFETY: the panel is kept in SHOWN until it is ordered out.
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setLevel(NSStatusWindowLevel);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary,
    );
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(
        &NSColor::systemPurpleColor().colorWithAlphaComponent(0.92),
    ));
    panel.setHasShadow(true);
    panel.setHidesOnDeactivate(false);
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setTextColor(Some(&NSColor::whiteColor()));
    // SAFETY: the framework's own weight constant.
    let medium = unsafe { NSFontWeightMedium };
    label.setFont(Some(&NSFont::systemFontOfSize_weight(13.0, medium)));
    label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    label.setFrame(NSRect::new(
        NSPoint::new(14.0, 9.0),
        NSSize::new(width - 110.0, 18.0),
    ));
    let target = StopTarget::new(mtm);
    // SAFETY: the target answers `stop:` and is kept with the panel.
    let stop = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str("Stop"),
            Some(&target),
            Some(sel!(stop:)),
            mtm,
        )
    };
    stop.setFrame(NSRect::new(
        NSPoint::new(width - 86.0, 4.0),
        NSSize::new(76.0, 28.0),
    ));
    let content = panel.contentView()?;
    content.addSubview(&label);
    content.addSubview(&stop);
    panel.orderFrontRegardless();
    Some((panel, target))
}
