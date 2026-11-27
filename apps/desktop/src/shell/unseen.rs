//! Unseen runs: launches no one watches (the automation's `--drive` and `--bench`, the
//! `--self-check` smoke run, `--unseen`). The person at the computer may be working while one
//! runs, so none of its windows may show or take the keyboard: each opens without the focus
//! and, unless `--on-screen` asks for a display (a private one captured whole), out of sight.
//! The app never comes to the front either.
//!
//! Out of sight is beside every display on X11 and Windows (a frame left of the leftmost
//! display, [`beside_displays`]). On macOS a window no display shows gets no frames (GPUI's
//! display link needs the window's screen), so there each window stays on a display, below the
//! desktop picture and out of every way to reach it (the `mac` module).
//!
//! Every window the app opens goes through [`open_window`], which applies this; the app and
//! its windows come forward only through [`activate_app`] and [`bring_forward`], and a window
//! goes full screen only through [`toggle_fullscreen`], which do nothing in an unseen run.
//! `clippy.toml` refuses GPUI's own calls for these elsewhere, so a new kind of window can't
//! miss it.

use gpui_kit::{
    AnyWindowHandle, App, Bounds, Entity, Global, Pixels, Render, Size, Window, WindowOptions,
    point, px,
};

/// How far beside the displays an unseen window sits, past the shadow the system draws round
/// a window.
const GAP: f32 = 400.;

/// This launch is an unseen run, and where its windows go (absent in a normal run).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unseen {
    /// Out of sight (the default): beside every display, or below the desktop on macOS.
    OutOfSight,
    /// On a display as in a normal run, still without the focus (`--on-screen`).
    OnScreen,
}

impl Global for Unseen {}

/// Makes this launch an unseen run placed as `unseen`, before any window opens, with the
/// platform set up for it (on macOS, see the `mac` module).
///
/// # Errors
///
/// When macOS can't be set up to keep the windows out of sight and the app in the background:
/// the run is refused rather than show a window.
#[cfg_attr(
    not(target_os = "macos"),
    expect(clippy::unnecessary_wraps, reason = "only macOS's setup can fail")
)]
pub(crate) fn install(unseen: Unseen, cx: &mut App) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    mac::start()?;
    mark(unseen, cx);
    Ok(())
}

/// Makes this launch an unseen run placed as `unseen`, leaving the platform as it is: for
/// [`install`], and for tests, whose windows are GPUI's test platform's.
pub(crate) fn mark(unseen: Unseen, cx: &mut App) {
    cx.set_global(unseen);
}

/// Whether this launch is an unseen run.
#[must_use]
pub(crate) fn is_unseen(cx: &App) -> bool {
    cx.has_global::<Unseen>()
}

/// A frame of `size` that none of `displays` shows: left of the leftmost display's left edge
/// (and of the origin), level with the topmost top, so it misses every display however they
/// are arranged: side by side, stacked, or left of and above the primary display at negative
/// coordinates. Only the x axis keeps it clear, so the same frame works whichever way the y
/// axis points.
#[must_use]
#[cfg_attr(
    all(target_os = "macos", not(test)),
    expect(
        dead_code,
        reason = "macOS keeps an unseen window on a display, below the desktop"
    )
)]
pub(crate) fn beside_displays(displays: &[Bounds<Pixels>], size: Size<Pixels>) -> Bounds<Pixels> {
    let left = displays
        .iter()
        .map(|display| f32::from(display.origin.x))
        .fold(0., f32::min);
    let top = displays
        .iter()
        .map(|display| f32::from(display.origin.y))
        .fold(0., f32::min);
    Bounds::new(point(px(left - f32::from(size.width) - GAP), px(top)), size)
}

/// `options` as a window opens with in this run: unchanged in a normal run; in an unseen run
/// without the focus, and out of sight unless the run is on screen.
#[must_use]
pub(crate) fn options(mut options: WindowOptions, cx: &App) -> WindowOptions {
    let Some(unseen) = cx.try_global::<Unseen>().copied() else {
        return options;
    };
    options.focus = false;
    if unseen == Unseen::OutOfSight {
        out_of_sight(&mut options, cx);
    }
    options
}

/// On macOS: opened hidden, where GPUI puts it on a display; [`open_window`] orders it in
/// below the desktop once nothing can show it ([`mac::hide`]).
#[cfg(target_os = "macos")]
fn out_of_sight(options: &mut WindowOptions, _: &App) {
    options.show = false;
}

/// Elsewhere: windowed beside every display at the size asked for, on no display in
/// particular (GPUI then takes the bounds in its global coordinates, the primary display's top
/// left at the origin, as [`beside_displays`] works them out).
#[cfg(not(target_os = "macos"))]
fn out_of_sight(options: &mut WindowOptions, cx: &App) {
    /// The size when the options name none.
    const FALLBACK_SIZE: (f32, f32) = (800., 600.);
    let asked = options.window_bounds.map_or_else(
        || gpui_kit::size(px(FALLBACK_SIZE.0), px(FALLBACK_SIZE.1)),
        |bounds| bounds.get_bounds().size,
    );
    let displays: Vec<Bounds<Pixels>> = cx
        .displays()
        .iter()
        .map(|display| display.bounds())
        .collect();
    options.window_bounds = Some(gpui_kit::WindowBounds::Windowed(beside_displays(
        &displays, asked,
    )));
    options.display_id = None;
}

/// Opens a window over the view `build` makes: the one place the app opens a window, so an
/// unseen run's ([`options`]) never shows nor takes the focus.
///
/// # Errors
///
/// When the platform can't open the window, or (macOS) can't keep an unseen run's out of
/// sight; it is closed then, never having shown.
#[expect(
    clippy::disallowed_methods,
    reason = "the one place windows open, applying an unseen run's placement"
)]
pub(crate) fn open_window<V: Render>(
    options: WindowOptions,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> anyhow::Result<(AnyWindowHandle, Entity<V>)> {
    let options = self::options(options, cx);
    let opened = gpui_kit::open_window(options, cx, build)?;
    #[cfg(target_os = "macos")]
    if cx.try_global::<Unseen>() == Some(&Unseen::OutOfSight) {
        let handle = opened.0;
        let hidden = handle
            .update(cx, |_, window, _| mac::hide(window))
            .and_then(|hidden| hidden);
        if let Err(error) = hidden {
            if let Err(closed) = handle.update(cx, |_, window, _| window.remove_window()) {
                log::debug!("the unseen window closed already: {closed}");
            }
            return Err(error.context("couldn't keep an unseen run's window out of sight"));
        }
    }
    Ok(opened)
}

/// Brings the app to the front, ahead of the app in use; never in an unseen run.
#[expect(
    clippy::disallowed_methods,
    reason = "the one place the app comes forward, never in an unseen run"
)]
pub(crate) fn activate_app(cx: &mut App) {
    if !is_unseen(cx) {
        cx.activate(true);
    }
}

/// Brings `window` forward with the keyboard; never in an unseen run.
#[expect(
    clippy::disallowed_methods,
    reason = "the one place a window comes forward, never in an unseen run"
)]
pub(crate) fn bring_forward(window: &Window, cx: &App) {
    if !is_unseen(cx) {
        window.activate_window();
    }
}

/// Takes `window` into or out of full screen; never in an unseen run, where full screen would
/// put it on a display, in a Space of its own.
#[expect(
    clippy::disallowed_methods,
    reason = "the one place a window goes full screen, never in an unseen run"
)]
pub(crate) fn toggle_fullscreen(window: &Window, cx: &App) {
    if !is_unseen(cx) {
        window.toggle_fullscreen();
    }
}

/// What an unseen run needs on macOS besides the window's options. It is set up for the whole
/// process at launch (`start`), and the run is refused if any step fails, rather than show a
/// window:
///
/// - A window no display shows gets no frames: on macOS 13 GPUI's `CVDisplayLink` follows the
///   window's screen, which such a window hasn't got, and `AppKit` would constrain it back onto
///   a screen anyway. So each window stays on a display, out of sight and out of the way
///   (`hiding`): below the desktop picture (`kCGDesktopWindowLevel - 1`), ignoring the
///   mouse, hidden by Mission Control (transient), out of ⌘\` and the Window menu, without a
///   shadow and never full screen. It opens hidden and is ordered in (`hide`) only once it is
///   all of that, so it never shows at a normal level.
/// - Below the desktop the window is occluded, and GPUI stops drawing an occluded window and
///   tells the app it is out of sight. GPUI's window classes say they are visible
///   (`occlusionState`), so the window keeps drawing for the run's screenshots
///   (`screencapture -l` captures a window by its id) and the store keeps going.
/// - Nothing may bring the app to the front: GPUI's `activate`, and the report's web view,
///   which activates the app as it is made. GPUI's application class gets `activate` and
///   `activateIgnoringOtherApps:` that do nothing, so the app stays behind the one in use and
///   none of its windows becomes key.
/// - With every window out of sight App Nap would slow the run's timers; an activity held for
///   the process's life keeps it off.
#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{CStr, CString};
    use std::sync::OnceLock;

    use anyhow::{Context as _, bail};
    use objc2::encode::{Encode, Encoding};
    use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Method, Sel};
    use objc2::{MainThreadMarker, sel};
    use objc2_app_kit::{
        NSApplication, NSWindow, NSWindowCollectionBehavior, NSWindowLevel, NSWindowOcclusionState,
    };
    use objc2_core_graphics::{CGWindowLevelForKey, CGWindowLevelKey};
    use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};

    use gpui_kit::Window;

    /// GPUI's application class (gpui-macos).
    const APP_CLASS: &CStr = c"GPUIApplication";

    /// GPUI's window classes (gpui-macos).
    const WINDOW_CLASSES: [&CStr; 2] = [c"GPUIWindow", c"GPUIPanel"];

    /// The `occlusionState` the window classes were given, once [`start`] has run: each window
    /// is checked to answer with it as it opens.
    static OCCLUSION: OnceLock<Imp> = OnceLock::new();

    /// How an unseen window is kept out of sight and out of the way.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) struct Hiding {
        /// Its level: below the desktop picture.
        pub(super) level: NSWindowLevel,
        /// Whether clicks go through it, to whatever is under it.
        pub(super) ignores_mouse: bool,
        /// How Spaces, Mission Control, ⌘\` and full screen treat it.
        pub(super) behavior: NSWindowCollectionBehavior,
        /// Whether the Window menu lists it.
        pub(super) in_windows_menu: bool,
        /// Whether it casts a shadow (which would show over the desktop picture).
        pub(super) shadow: bool,
    }

    /// How every unseen window is kept: one level below the desktop picture, so the picture
    /// covers it on every display; clicks pass through; transient, so Mission Control hides it
    /// (stationary would show it there, and the two exclude each other); out of ⌘\` and the
    /// Window menu; never full screen; no shadow.
    #[must_use]
    pub(super) fn hiding() -> Hiding {
        Hiding {
            level: desktop_level() - 1,
            ignores_mouse: true,
            behavior: NSWindowCollectionBehavior::Transient
                | NSWindowCollectionBehavior::IgnoresCycle
                | NSWindowCollectionBehavior::FullScreenNone,
            in_windows_menu: false,
            shadow: false,
        }
    }

    /// The desktop picture's window level, as `AppKit` takes levels.
    #[must_use]
    pub(super) fn desktop_level() -> NSWindowLevel {
        // A Core Graphics level is an i32, which an NSWindowLevel (an isize) holds on a Mac.
        CGWindowLevelForKey(CGWindowLevelKey::DesktopWindowLevelKey) as NSWindowLevel
    }

    /// Sets the process up for an unseen run, before any window opens.
    ///
    /// # Errors
    ///
    /// Off the main thread, without GPUI's classes (a GPUI that renamed them), or when a class
    /// keeps a method of its own over the one the run needs.
    pub(super) fn start() -> anyhow::Result<()> {
        let main_thread =
            MainThreadMarker::new().context("an unseen run is set up on the main thread")?;
        let app_class = class(APP_CLASS)?;
        let activate: extern "C-unwind" fn(&AnyObject, Sel) = activate;
        let ignoring: extern "C-unwind" fn(&AnyObject, Sel, Bool) = activate_ignoring_other_apps;
        #[expect(
            unsafe_code,
            reason = "implementations made from functions whose signatures match the selectors"
        )]
        // SAFETY: each function takes the receiver, the selector and the selector's arguments
        // and returns its result, as the type encoding beside it says; Imp is the erased type
        // of such a function.
        let (activate, ignoring) = unsafe {
            (
                std::mem::transmute::<extern "C-unwind" fn(&AnyObject, Sel), Imp>(activate),
                std::mem::transmute::<extern "C-unwind" fn(&AnyObject, Sel, Bool), Imp>(ignoring),
            )
        };
        give(
            app_class,
            sel!(activate),
            activate,
            &types(&Encoding::Void, &[]),
        )?;
        let ignoring_types = types(&Encoding::Void, &[Bool::ENCODING]);
        give(
            app_class,
            sel!(activateIgnoringOtherApps:),
            ignoring,
            &ignoring_types,
        )?;
        // The application itself answers with them (it is GPUI's class, or one under it).
        let app = NSApplication::sharedApplication(main_thread);
        answers_with(app.class(), sel!(activate), activate)?;
        answers_with(app.class(), sel!(activateIgnoringOtherApps:), ignoring)?;

        let occlusion = *OCCLUSION.get_or_init(occlusion_imp);
        let occlusion_types = types(&NSWindowOcclusionState::ENCODING, &[]);
        for name in WINDOW_CLASSES {
            give(
                class(name)?,
                sel!(occlusionState),
                occlusion,
                &occlusion_types,
            )?;
        }

        let activity = NSProcessInfo::processInfo().beginActivityWithOptions_reason(
            NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
            &NSString::from_str("an unseen run drives the app"),
        );
        // Held until the process ends: ending it would let App Nap in.
        std::mem::forget(activity);
        Ok(())
    }

    /// Orders `window`, just opened hidden, in below the desktop once it is kept as
    /// [`hiding`] says, so it never shows at a normal level. Nothing to do before [`start`]
    /// (tests, on GPUI's test platform).
    ///
    /// # Errors
    ///
    /// When the window has no `NSWindow`, or its class doesn't answer with the run's
    /// `occlusionState`: it stays hidden.
    pub(super) fn hide(window: &Window) -> anyhow::Result<()> {
        let Some(&occlusion) = OCCLUSION.get() else {
            return Ok(());
        };
        let view = crate::ui::share::content_view(window).context("the window has no view")?;
        let ns_window = view.window().context("the window's view has no NSWindow")?;
        answers_with(ns_window.class(), sel!(occlusionState), occlusion)?;
        keep(&ns_window, &hiding());
        ns_window.orderBack(None);
        // Its first frame, drawn once it is in; GPUI's display link runs from there.
        view.setNeedsDisplay(true);
        Ok(())
    }

    /// Keeps `window` as `hiding` says.
    fn keep(window: &NSWindow, hiding: &Hiding) {
        window.setLevel(hiding.level);
        window.setIgnoresMouseEvents(hiding.ignores_mouse);
        window.setCollectionBehavior(hiding.behavior);
        window.setExcludedFromWindowsMenu(!hiding.in_windows_menu);
        window.setHasShadow(hiding.shadow);
    }

    /// The class called `name`.
    fn class(name: &CStr) -> anyhow::Result<&'static AnyClass> {
        AnyClass::get(name).with_context(|| {
            format!(
                "GPUI has no {} class to keep unseen",
                name.to_string_lossy()
            )
        })
    }

    /// `activate` that does nothing.
    extern "C-unwind" fn activate(_: &AnyObject, _: Sel) {}

    /// `activateIgnoringOtherApps:` that does nothing.
    extern "C-unwind" fn activate_ignoring_other_apps(_: &AnyObject, _: Sel, _: Bool) {}

    /// `occlusionState` that says the window is seen.
    extern "C-unwind" fn occlusion_state(_: &AnyObject, _: Sel) -> NSWindowOcclusionState {
        NSWindowOcclusionState::Visible
    }

    /// [`occlusion_state`] as an implementation.
    #[expect(
        unsafe_code,
        reason = "an implementation made from a function whose signature matches the selector"
    )]
    fn occlusion_imp() -> Imp {
        type Occlusion = extern "C-unwind" fn(&AnyObject, Sel) -> NSWindowOcclusionState;
        let occlusion: Occlusion = occlusion_state;
        // SAFETY: as in `start`.
        unsafe { std::mem::transmute::<Occlusion, Imp>(occlusion) }
    }

    /// A method's type encoding: what it returns, the receiver and the selector, then its
    /// arguments.
    fn types(returns: &Encoding, arguments: &[Encoding]) -> CString {
        let mut types = format!("{returns}{}{}", <*mut AnyObject>::ENCODING, Sel::ENCODING);
        for argument in arguments {
            types.push_str(&argument.to_string());
        }
        let types = CString::new(types);
        debug_assert!(types.is_ok(), "a type encoding has no NUL in it");
        types.unwrap_or_default()
    }

    /// Gives `class` `imp` as its own `selector`, over the one it inherits.
    ///
    /// # Errors
    ///
    /// When the class has a `selector` of its own that isn't `imp` (one an earlier call gave it
    /// is): the run can't rely on it.
    #[expect(unsafe_code, reason = "the Objective-C runtime's class_addMethod")]
    fn give(class: &AnyClass, selector: Sel, imp: Imp, types: &CStr) -> anyhow::Result<()> {
        // SAFETY: the class is a live class object, and every `imp` given here is a function
        // of the signature its `types` describe (see `start`).
        let added = unsafe {
            objc2::ffi::class_addMethod(
                std::ptr::from_ref(class).cast_mut(),
                selector,
                imp,
                types.as_ptr(),
            )
        };
        if added.as_bool() {
            Ok(())
        } else {
            answers_with(class, selector, imp)
        }
    }

    /// Whether instances of `class` answer `selector` with `imp`.
    ///
    /// # Errors
    ///
    /// When they answer with another implementation, or none.
    fn answers_with(class: &AnyClass, selector: Sel, imp: Imp) -> anyhow::Result<()> {
        let found = class.instance_method(selector).map(Method::implementation);
        if found.is_some_and(|found| std::ptr::fn_addr_eq(found, imp)) {
            return Ok(());
        }
        bail!(
            "{} answers {} with its own method, so an unseen run's windows could show",
            class.name().to_string_lossy(),
            selector.name().to_string_lossy()
        )
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::size;

    use super::*;

    fn display(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
    }

    /// The frame for `displays`, checked to miss each of them by the gap and keep the size
    /// asked for.
    fn placed(displays: &[Bounds<Pixels>]) -> Bounds<Pixels> {
        let asked = size(px(1100.), px(760.));
        let frame = beside_displays(displays, asked);
        assert_eq!(frame.size, asked);
        for display in displays {
            assert!(!frame.intersects(display), "{frame:?} is on {display:?}");
            assert!(
                frame.right() + px(GAP) <= display.left(),
                "clear of {display:?} by the gap"
            );
        }
        frame
    }

    #[test]
    fn a_frame_beside_one_display_is_left_of_it_at_its_top() {
        let frame = placed(&[display(0., 0., 1440., 900.)]);
        assert_eq!(frame.origin, point(px(-1100. - GAP), px(0.)));
    }

    #[test]
    fn a_frame_misses_displays_side_by_side_stacked_and_at_negative_coordinates() {
        // A second display to the right, another above the primary.
        placed(&[
            display(0., 0., 1512., 982.),
            display(1512., -200., 2560., 1440.),
            display(0., -1080., 1920., 1080.),
        ]);
        // The external display left of the primary and higher, at negative coordinates.
        let frame = placed(&[
            display(0., 0., 1512., 982.),
            display(-3008., -700., 3008., 1692.),
        ]);
        assert_eq!(frame.origin.y, px(-700.), "level with the topmost display");
        // Stacked with the y axis up, a display below the primary.
        placed(&[
            display(0., 0., 1440., 900.),
            display(-200., -1440., 2560., 1440.),
        ]);
    }

    #[test]
    fn with_no_display_known_the_frame_is_left_of_the_origin() {
        let frame = placed(&[]);
        assert!(frame.right() < px(0.));
    }

    /// On a Mac an unseen window stays on a display, so what keeps it unseen is how it is
    /// kept: below the desktop picture, deaf to the mouse, hidden by Mission Control, out of
    /// ⌘\` and the Window menu, never full screen, without a shadow.
    #[cfg(target_os = "macos")]
    #[test]
    fn on_the_mac_an_unseen_window_is_below_the_desktop_and_out_of_the_way() {
        use objc2_app_kit::{NSNormalWindowLevel, NSWindowCollectionBehavior as Behavior};

        let hiding = mac::hiding();
        assert!(
            hiding.level < mac::desktop_level(),
            "below the desktop picture"
        );
        assert!(hiding.level < NSNormalWindowLevel);
        assert!(hiding.ignores_mouse, "clicks go through it");
        assert!(
            hiding
                .behavior
                .contains(Behavior::Transient | Behavior::IgnoresCycle | Behavior::FullScreenNone),
            "{:?}",
            hiding.behavior
        );
        assert!(
            !hiding.behavior.intersects(
                Behavior::Managed
                    | Behavior::Stationary
                    | Behavior::CanJoinAllSpaces
                    | Behavior::MoveToActiveSpace
                    | Behavior::FullScreenPrimary
                    | Behavior::FullScreenAuxiliary
            ),
            "{:?}",
            hiding.behavior
        );
        assert!(!hiding.in_windows_menu && !hiding.shadow);
    }
}
