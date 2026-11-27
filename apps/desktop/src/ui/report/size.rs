//! The panel's size as the window holds it: its layout, and where a width the user picks is kept.

use gpui_kit::{App, Pixels};

use super::kept::KeptWidth;
use super::layout::PanelLayout;
use super::width::{PanelWidth, Space};
use crate::config::AppFiles;

/// The panel's layout, and where its width is kept (nowhere on a system with no home).
#[derive(Debug, Default)]
pub(crate) struct ReportSize {
    layout: PanelLayout,
    kept: Option<KeptWidth>,
}

impl ReportSize {
    /// The size at the width kept in `files`, else at the default width. Reads a file: call it
    /// before the app's run loop starts.
    #[must_use]
    pub(crate) fn load(files: Option<&AppFiles>) -> Self {
        let kept = files.cloned().map(KeptWidth::load);
        let width = kept.as_ref().map_or_default(KeptWidth::width);
        Self {
            layout: PanelLayout::new(width),
            kept,
        }
    }

    /// Whether the panel fills the content area.
    #[must_use]
    pub(crate) fn is_expanded(&self) -> bool {
        self.layout.is_expanded()
    }

    /// Lets the panel fill the content area, in `space`; a drag still held ends first, and the
    /// width it made is kept.
    pub(crate) fn expand(&mut self, space: Space, cx: &App) {
        let changed = self.layout.expand(space);
        self.keep(changed, cx);
    }

    /// Brings an expanded panel back to its width.
    pub(crate) fn restore(&mut self) {
        self.layout.restore();
    }

    /// The panel's width in `space`.
    #[must_use]
    pub(super) fn pixels(&self, space: Space) -> Pixels {
        self.layout.pixels(space)
    }

    /// The pointer went down on the edge at `x`.
    pub(super) fn press(&mut self, x: Pixels, space: Space) {
        self.layout.press(x, space);
    }

    /// The held edge follows the pointer to `x`; whether the width changed.
    pub(super) fn drag_to(&mut self, x: Pixels, space: Space) -> bool {
        self.layout.drag_to(x, space)
    }

    /// The pointer let go: a width the drag changed on screen is kept.
    pub(super) fn release(&mut self, space: Space, cx: &App) {
        let changed = self.layout.release(space);
        self.keep(changed, cx);
    }

    /// Back to the default width, which is kept.
    pub(super) fn reset(&mut self, cx: &App) {
        let changed = self.layout.reset();
        self.keep(changed, cx);
    }

    fn keep(&mut self, changed: Option<PanelWidth>, cx: &App) {
        if let (Some(width), Some(kept)) = (changed, self.kept.as_mut()) {
            kept.keep(width, cx);
        }
    }
}
