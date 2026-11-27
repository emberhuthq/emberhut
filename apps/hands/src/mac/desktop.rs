//! [`MacDesktop`]: the [`Desktop`] trait over the Mac's own frameworks.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use objc2_core_foundation::{CFBoolean, CFString};
use objc2_core_graphics::CGPreflightScreenCaptureAccess;
use objc2_screen_capture_kit::SCShareableContent;

use super::ax::{self, Element};
use super::input::{self, To};
use super::{apps, bar, pictures, session, windows};
use crate::desktop::{
    AxError, Bar, Desktop, Flag, Focus, InstalledApp, Link, List, Posting, RunningApp,
    ScreenPicture, Text, Window,
};
use crate::keys::Input;

/// How long the screen-recording check waits for `ScreenCaptureKit`.
const CAPTURE_CHECK: Duration = Duration::from_secs(3);

/// The Mac.
#[derive(Debug)]
pub struct MacDesktop {
    home: PathBuf,
}

impl MacDesktop {
    /// The Mac, with the owner's home folder `home` (for `~/Applications`).
    #[must_use]
    pub fn new(home: PathBuf) -> Self {
        Self { home }
    }
}

/// An attribute's name.
const fn text_name(attribute: Text) -> &'static str {
    match attribute {
        Text::Role => "AXRole",
        Text::Subrole => "AXSubrole",
        Text::Title => "AXTitle",
        Text::Description => "AXDescription",
        Text::Placeholder => "AXPlaceholderValue",
        Text::Help => "AXHelp",
        Text::Value => "AXValue",
        Text::MenuKey => "AXMenuItemCmdChar",
    }
}

/// An attribute's name.
const fn flag_name(attribute: Flag) -> &'static str {
    match attribute {
        Flag::Enabled => "AXEnabled",
        Flag::Focused => "AXFocused",
        Flag::Minimized => "AXMinimized",
    }
}

/// Whether the helper may capture the screen. The preflight alone can stay false for a launch
/// agent after the owner turned the grant on, so `ScreenCaptureKit` is asked for the windows
/// themselves too.
#[expect(unsafe_code, reason = "SCShareableContent's calls are unsafe in objc2")]
#[must_use]
pub(super) fn screen_recording() -> bool {
    if CGPreflightScreenCaptureAccess() {
        return true;
    }
    let (done, got) = mpsc::channel::<bool>();
    let handler = block2::RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut objc2_foundation::NSError| {
            let _sent = done.send(!content.is_null() && error.is_null());
        },
    );
    // SAFETY: a valid completion handler, called once.
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            false, true, &handler,
        );
    }
    got.recv_timeout(CAPTURE_CHECK).unwrap_or(false)
}

impl Desktop for MacDesktop {
    type Element = Element;

    fn accessibility(&self) -> bool {
        ax::trusted()
    }

    fn screen_recording(&self) -> bool {
        screen_recording()
    }

    fn running(&self) -> Vec<RunningApp> {
        apps::running()
    }

    fn running_with_bundle(&self, bundle: &str) -> Vec<RunningApp> {
        apps::with_bundle(bundle)
    }

    fn front(&self) -> Option<i32> {
        apps::front()
    }

    fn app(&self, pid: i32) -> Option<RunningApp> {
        apps::app(pid)
    }

    fn app_at(&self, x: f64, y: f64) -> Option<i32> {
        Element::pid_at(x, y)
    }

    fn activate(&self, pid: i32) {
        apps::activate(pid);
    }

    fn installed(&self, want: &str) -> Option<InstalledApp> {
        apps::installed(want, &self.home)
    }

    fn open(&self, app: &InstalledApp, hidden: bool) -> Result<(), String> {
        apps::open(app, hidden)
    }

    fn windows(&self, pid: i32) -> Vec<Window> {
        windows::of(pid)
    }

    fn application(&self, pid: i32) -> Element {
        Element::application(pid)
    }

    fn focused(&self) -> Focus<Element> {
        Element::focused()
    }

    fn focus_in(&self, app: &Element) -> Focus<Element> {
        app.focus()
    }

    fn text(&self, element: &Element, attribute: Text) -> Option<String> {
        element.text(text_name(attribute))
    }

    fn flag(&self, element: &Element, attribute: Flag) -> Option<bool> {
        element.flag(flag_name(attribute))
    }

    fn link(&self, element: &Element, attribute: Link) -> Option<Element> {
        element.link(match attribute {
            Link::TitleElement => "AXTitleUIElement",
            Link::MenuBar => "AXMenuBar",
        })
    }

    fn list(&self, element: &Element, attribute: List) -> Vec<Element> {
        element.list(match attribute {
            List::Children => "AXChildren",
            List::Windows => "AXWindows",
        })
    }

    fn actions(&self, element: &Element) -> Vec<String> {
        element.actions()
    }

    fn perform(&self, element: &Element, action: &str) -> Result<(), AxError> {
        element.perform(action)
    }

    fn settable(&self, element: &Element) -> bool {
        element.settable("AXValue")
    }

    fn set_value(&self, element: &Element, text: &str) -> Result<(), AxError> {
        element.set("AXValue", &CFString::from_str(text))
    }

    fn set_flag(&self, element: &Element, attribute: Flag, on: bool) -> Result<(), AxError> {
        element.set(flag_name(attribute), CFBoolean::new(on))
    }

    fn post_to_app(
        &self,
        pid: i32,
        inputs: &[Input],
        go_on: &mut dyn FnMut(Posting<'_>) -> bool,
    ) -> bool {
        input::post(To::App(pid), inputs, go_on)
    }

    fn post_to_screen(&self, inputs: &[Input], go_on: &mut dyn FnMut(Posting<'_>) -> bool) -> bool {
        input::post(To::Screen, inputs, go_on)
    }

    fn idle_seconds(&self) -> f64 {
        input::idle_seconds()
    }

    fn input_count(&self) -> u64 {
        input::count()
    }

    fn window_picture(&self, id: u32, max: u32) -> Option<Vec<u8>> {
        pictures::window(id, max)
    }

    fn screen_picture(&self) -> Option<ScreenPicture> {
        pictures::screen()
    }

    fn locked(&self) -> bool {
        session::locked()
    }

    fn lock(&self) -> Result<(), String> {
        session::lock()
    }

    fn show_bar(&self, wanted: &Arc<Bar>) {
        bar::show(Arc::clone(wanted));
    }

    fn pause(&self, time: Duration) {
        std::thread::sleep(time);
    }
}
