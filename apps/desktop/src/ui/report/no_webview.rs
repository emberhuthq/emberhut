//! The stand-in for the web view where there is none (every platform but macOS, see
//! docs/adr/0006-report-webview.md): reports show as rich text.

use gpui_kit::{AnyElement, App, Window};

use crate::ui::palette::WebColours;

/// Why there is no web view.
#[derive(Debug, thiserror::Error)]
#[error("this platform has no web view")]
pub(super) struct Error;

/// A web view, which can't exist here.
#[derive(Debug)]
pub(super) enum ReportWeb {}

impl ReportWeb {
    /// Never a web view: says why, once per report shown.
    ///
    /// # Errors
    ///
    /// Always.
    pub(super) fn open(
        _html: &str,
        _colours: &[WebColours; 2],
        _window: &mut Window,
        _cx: &mut App,
    ) -> Result<Self, Error> {
        log::info!("the report shows as rich text: {Error}");
        Err(Error)
    }

    /// Gives the page Emberhut's appearance.
    pub(super) fn follow_appearance(&self, _dark: bool, _cx: &App) {
        match *self {}
    }

    /// Shows the page, or hides it under something of the app's own.
    pub(super) fn set_shown(&self, _shown: bool, _cx: &App) {
        match *self {}
    }

    /// The web view, to place in the panel.
    pub(super) fn element(&self) -> AnyElement {
        match *self {}
    }
}
