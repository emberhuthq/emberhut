//! The system seam: everything the helper asks of the Mac (its apps, their accessibility trees,
//! input, pictures, the lock, the bar with Stop) as one trait, so the rules above it are tested
//! with a fake desktop on any platform. The real one is `mac::MacDesktop`.

use std::sync::{Condvar, Mutex, PoisonError};
use std::time::Duration;

use crate::keys::Input;

/// An app that runs, as the Dock would list it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningApp {
    /// Its process id.
    pub pid: i32,
    /// Its name as the Mac shows it.
    pub name: String,
    /// Its bundle id (empty when it has none).
    pub bundle: String,
    /// Hidden (⌘H).
    pub hidden: bool,
}

/// An app installed on the Mac.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledApp {
    /// Its bundle's path.
    pub path: String,
    /// Its bundle id (empty when it has none).
    pub bundle: String,
    /// Its display name.
    pub name: String,
}

/// One of an app's windows as the window server knows it (for its picture).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    /// The window server's id.
    pub id: u32,
    /// Its title (empty when it has none).
    pub title: String,
    /// On screen now.
    pub on_screen: bool,
}

/// A picture of the main screen.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenPicture {
    /// A JPEG, one pixel a point.
    pub jpeg: Vec<u8>,
    /// The screen's width in points.
    pub width: f64,
    /// The screen's height in points.
    pub height: f64,
}

/// A text attribute of an element.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Text {
    /// `AXRole`.
    Role,
    /// `AXSubrole`.
    Subrole,
    /// `AXTitle`.
    Title,
    /// `AXDescription`.
    Description,
    /// `AXPlaceholderValue`.
    Placeholder,
    /// `AXHelp`.
    Help,
    /// `AXValue`, as text (a number or a boolean as JSON writes it). Never asked of a secret
    /// field.
    Value,
    /// `AXMenuItemCmdChar`: a menu item's shortcut key.
    MenuKey,
}

/// A yes-or-no attribute of an element.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Flag {
    /// `AXEnabled`.
    Enabled,
    /// `AXFocused`.
    Focused,
    /// `AXMinimized` (a window).
    Minimized,
}

/// What has the keyboard focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Focus<E> {
    /// This element.
    On(E),
    /// Nothing: the app (or the Mac) says no element has it.
    Nothing,
    /// It can't be told: the app didn't answer, or answered with an error.
    Unknown,
}

/// Where an event is in its posting, as the posting's callback is told.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Posting<'a> {
    /// About to be posted: the callback says whether it may be.
    Before(&'a Input),
    /// Posted (the callback's answer is not used).
    After(&'a Input),
}

/// The bar with Stop as the helper wants it, and as the main thread last showed it. The words
/// are worked out and asked for under one lock, so the last ask always carries the latest
/// state, and the main thread always shows the latest ask.
#[derive(Debug, Default)]
pub struct Bar {
    state: Mutex<BarState>,
    shown: Condvar,
}

/// See [`Bar`].
#[derive(Debug, Default)]
struct BarState {
    text: Option<String>,
    asked: u64,
    done: u64,
    up: bool,
}

impl Bar {
    /// Asks for the words `text` works out (`None`: no bar), under the bar's lock; the ask's
    /// number, to wait for.
    pub fn ask(&self, text: impl FnOnce() -> Option<String>) -> u64 {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.text = text();
        state.asked += 1;
        state.asked
    }

    /// The words wanted now, and the ask's number.
    #[must_use]
    pub fn wanted(&self) -> (u64, Option<String>) {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        (state.asked, state.text.clone())
    }

    /// The main thread showed ask `number`: the bar is `up` or not.
    pub fn shown(&self, number: u64, up: bool) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if number >= state.done {
            state.done = number;
            state.up = up;
        }
        self.shown.notify_all();
    }

    /// Waits up to `time` for ask `number` (or a later one) to be shown: whether the bar is up.
    #[must_use]
    pub fn wait(&self, number: u64, time: Duration) -> bool {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let (state, _) = self
            .shown
            .wait_timeout_while(state, time, |state| state.done < number)
            .unwrap_or_else(PoisonError::into_inner);
        state.done >= number && state.up
    }
}

/// An attribute that names another element.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Link {
    /// `AXTitleUIElement`: the element that labels this one.
    TitleElement,
    /// `AXMenuBar`: an app's menu bar.
    MenuBar,
}

/// An attribute that lists elements.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum List {
    /// `AXChildren`.
    Children,
    /// `AXWindows` (an app).
    Windows,
}

/// Why an accessibility call failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AxError {
    /// The element is gone (`kAXErrorInvalidUIElement`).
    #[error("the element is gone")]
    Gone,
    /// Any other error, by its code.
    #[error("accessibility error {0}")]
    Code(i32),
}

/// The Mac, as the helper uses it. Every call is synchronous; the helper runs one request at a
/// time on a worker thread.
pub trait Desktop: Send + Sync + 'static {
    /// An accessibility element (an app, a window, a button…).
    type Element: Clone + PartialEq + Send + Sync + 'static;

    /// Whether the helper holds the Accessibility grant.
    fn accessibility(&self) -> bool;
    /// Whether the helper holds the Screen Recording grant.
    fn screen_recording(&self) -> bool;

    /// The apps that run with a Dock icon.
    fn running(&self) -> Vec<RunningApp>;
    /// The running copies of the app with `bundle`, Dock icon or not (one just opened may not
    /// be listed yet).
    fn running_with_bundle(&self, bundle: &str) -> Vec<RunningApp>;
    /// The app in front.
    fn front(&self) -> Option<i32>;
    /// The running app `pid`, Dock icon or not; `None` once it quit.
    fn app(&self, pid: i32) -> Option<RunningApp>;
    /// The app whose window is under the point `x`, `y` (screen points).
    fn app_at(&self, x: f64, y: f64) -> Option<i32>;
    /// Brings the app `pid` to the front (putting the owner's app back).
    fn activate(&self, pid: i32);
    /// The installed app named `want` (a bundle id, or a name in the usual folders).
    fn installed(&self, want: &str) -> Option<InstalledApp>;
    /// Opens `app` without bringing it forward, hidden or not.
    ///
    /// # Errors
    ///
    /// The Mac's words when it doesn't open.
    fn open(&self, app: &InstalledApp, hidden: bool) -> Result<(), String>;
    /// The app `pid`'s ordinary windows the window server knows.
    fn windows(&self, pid: i32) -> Vec<Window>;

    /// The app `pid` as an accessibility element.
    fn application(&self, pid: i32) -> Self::Element;
    /// What has the keyboard focus, whichever app has it.
    fn focused(&self) -> Focus<Self::Element>;
    /// What has the keyboard focus in the app whose root is `app`.
    fn focus_in(&self, app: &Self::Element) -> Focus<Self::Element>;
    /// A text attribute.
    fn text(&self, element: &Self::Element, attribute: Text) -> Option<String>;
    /// A yes-or-no attribute.
    fn flag(&self, element: &Self::Element, attribute: Flag) -> Option<bool>;
    /// An attribute naming another element.
    fn link(&self, element: &Self::Element, attribute: Link) -> Option<Self::Element>;
    /// An attribute listing elements.
    fn list(&self, element: &Self::Element, attribute: List) -> Vec<Self::Element>;
    /// The actions it takes (`AXPress`, `AXShowMenu`…).
    fn actions(&self, element: &Self::Element) -> Vec<String>;
    /// Performs `action`.
    ///
    /// # Errors
    ///
    /// [`AxError`].
    fn perform(&self, element: &Self::Element, action: &str) -> Result<(), AxError>;
    /// Whether its value can be set.
    fn settable(&self, element: &Self::Element) -> bool;
    /// Sets its value to `text`.
    ///
    /// # Errors
    ///
    /// [`AxError`].
    fn set_value(&self, element: &Self::Element, text: &str) -> Result<(), AxError>;
    /// Sets a yes-or-no attribute (focus, minimized).
    ///
    /// # Errors
    ///
    /// [`AxError`].
    fn set_flag(&self, element: &Self::Element, attribute: Flag, on: bool) -> Result<(), AxError>;

    /// Posts `inputs` to the app `pid` only, asking `go_on` before each event and telling it
    /// after each one posted: whether all were posted.
    fn post_to_app(
        &self,
        pid: i32,
        inputs: &[Input],
        go_on: &mut dyn FnMut(Posting<'_>) -> bool,
    ) -> bool;
    /// Posts `inputs` as the real mouse and keyboard, from an event source of the helper's
    /// own, asking `go_on` before each event (it says no once the owner pressed Stop or
    /// touched the mouse or keyboard) and telling it after each one posted: whether all were
    /// posted.
    fn post_to_screen(&self, inputs: &[Input], go_on: &mut dyn FnMut(Posting<'_>) -> bool) -> bool;
    /// Seconds since the last mouse or key input, whoever made it.
    fn idle_seconds(&self) -> f64;
    /// How many pointer moves, button and key presses and scrolls the Mac has counted in this
    /// login session, whoever made them (see [`Input::counted`]).
    fn input_count(&self) -> u64;

    /// A JPEG of window `id`, covered or hidden, its longest side at most `max` pixels.
    fn window_picture(&self, id: u32, max: u32) -> Option<Vec<u8>>;
    /// A JPEG of the main screen, one pixel a point.
    fn screen_picture(&self) -> Option<ScreenPicture>;

    /// Whether the screen is locked.
    fn locked(&self) -> bool;
    /// Locks the screen now.
    ///
    /// # Errors
    ///
    /// The Mac's words when it can't.
    fn lock(&self) -> Result<(), String>;
    /// Shows the bar with Stop as `bar` wants it now (on the main thread, later), and tells
    /// `bar` when it did.
    fn show_bar(&self, bar: &std::sync::Arc<Bar>);

    /// Waits `time` for the Mac to catch up.
    fn pause(&self, time: Duration);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn the_bar_shows_the_latest_ask_and_says_when_it_is_up() {
        let bar = Arc::new(Bar::default());
        let first = bar.ask(|| Some("one".to_owned()));
        let second = bar.ask(|| Some("two".to_owned()));
        assert_eq!(bar.wanted(), (second, Some("two".to_owned())));
        assert!(!bar.wait(first, Duration::ZERO));
        let shower = Arc::clone(&bar);
        std::thread::spawn(move || shower.shown(second, true));
        assert!(bar.wait(first, Duration::from_secs(5)));
        // A late answer for an older ask doesn't take the bar down.
        bar.shown(first, false);
        assert!(bar.wait(second, Duration::ZERO));
    }
}
