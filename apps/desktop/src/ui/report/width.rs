//! The report panel's width: a share of the window, so it keeps its proportion when the window
//! is resized, and the pixels that share comes to in a given window, kept within bounds that
//! leave both the report and the chat beside it usable.

use gpui_kit::{Pixels, px};

/// The narrowest the panel gets, unless the content area itself is narrower.
pub(crate) const MIN_PANEL: Pixels = px(380.);

/// The narrowest the chat beside the panel gets, while the content area has room for both.
pub(crate) const MIN_CHAT: Pixels = px(360.);

/// The width the panel has to share: the window's, and its content area's (the window less the
/// sidebar), where the chat and the panel sit side by side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Space {
    window: Pixels,
    content: Pixels,
}

impl Space {
    /// The space in a window `window` wide whose content area is `content` wide; the content
    /// area is never negative or wider than the window.
    #[must_use]
    pub(super) fn new(window: Pixels, content: Pixels) -> Self {
        let window = window.max(Pixels::ZERO);
        Self {
            window,
            content: content.max(Pixels::ZERO).min(window),
        }
    }

    /// The content area's width.
    #[must_use]
    pub(super) fn content(self) -> Pixels {
        self.content
    }

    /// `width` brought within the panel's bounds: at least [`MIN_PANEL`], at most what leaves
    /// [`MIN_CHAT`] to the chat. A content area too narrow for both gives the panel its minimum
    /// and the chat what is left, and the panel never outgrows the content area.
    fn clamp(self, width: Pixels) -> Pixels {
        let widest = self.content - MIN_CHAT;
        if widest < MIN_PANEL {
            return MIN_PANEL.min(self.content);
        }
        width.max(MIN_PANEL).min(widest)
    }
}

/// The panel's width as a share of the window's: more than 0, at most 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PanelWidth(f32);

impl PanelWidth {
    /// The width the panel opens at until it is resized, and returns to on a reset.
    pub(super) const DEFAULT: Self = Self(0.46);

    /// The width that is `share` of the window; `None` unless it is a number more than 0 and
    /// at most 1 (a value read back from disk may be anything).
    #[must_use]
    pub(super) fn from_share(share: f32) -> Option<Self> {
        (share > 0. && share <= 1.).then_some(Self(share))
    }

    /// The share of the window.
    #[must_use]
    pub(super) fn share(self) -> f32 {
        self.0
    }

    /// The width a drag asks for, `width` in `space`, as it shows: in whole pixels, within the
    /// bounds, and taken as a share of the window; `None` in a window with no width.
    #[must_use]
    pub(super) fn from_pixels(width: Pixels, space: Space) -> Option<Self> {
        Self::from_share(space.clamp(width.round()) / space.window)
    }

    /// The width in `space`, in whole pixels, within the bounds.
    #[must_use]
    pub(super) fn pixels(self, space: Space) -> Pixels {
        space.clamp((space.window * self.0).round())
    }
}

impl Default for PanelWidth {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window `window` wide with the sidebar's 260 px on the left.
    fn with_sidebar(window: f32) -> Space {
        Space::new(px(window), px(window - 260.))
    }

    #[test]
    fn the_default_is_46_percent_of_the_window() {
        assert_eq!(PanelWidth::default(), PanelWidth::DEFAULT);
        assert_eq!(PanelWidth::DEFAULT.pixels(with_sidebar(1500.)), px(690.));
    }

    #[test]
    fn the_panel_keeps_its_minimum() {
        let narrow = PanelWidth::from_share(0.2).unwrap();
        assert_eq!(narrow.pixels(with_sidebar(1500.)), MIN_PANEL, "20% is 300");
        let no_sidebar = Space::new(px(1600.), px(1600.));
        assert_eq!(narrow.pixels(no_sidebar), MIN_PANEL, "20% is 320");
    }

    #[test]
    fn the_chat_keeps_its_minimum() {
        // 1140 px of content: the panel stops 360 px short of it.
        let wide = PanelWidth::from_share(0.9).unwrap();
        assert_eq!(wide.pixels(with_sidebar(1400.)), px(780.));
        assert_eq!(
            PanelWidth::DEFAULT.pixels(with_sidebar(1100.)),
            px(480.),
            "46% is 506"
        );
    }

    #[test]
    fn a_window_too_narrow_for_both_gives_the_panel_its_minimum() {
        // 640 px of content: 380 for the panel, 260 for the chat.
        let space = with_sidebar(900.);
        for share in [0.1, 0.46, 1.] {
            let width = PanelWidth::from_share(share).unwrap();
            assert_eq!(width.pixels(space), MIN_PANEL, "{share}");
        }
        // Narrower than the panel's minimum: the panel fills the content area, no more.
        let space = with_sidebar(500.);
        assert_eq!(PanelWidth::DEFAULT.pixels(space), px(240.));
        let space = with_sidebar(100.);
        assert_eq!(space.content(), Pixels::ZERO);
        assert_eq!(PanelWidth::DEFAULT.pixels(space), Pixels::ZERO);
    }

    #[test]
    fn a_dragged_width_is_kept_as_a_share_of_the_window() {
        let space = with_sidebar(1260.);
        let width = PanelWidth::from_pixels(px(504.), space).unwrap();
        assert!((width.share() - 0.4).abs() < 1e-6, "{width:?}");
        assert_eq!(width.pixels(space), px(504.));
        // In a window twice as wide it is twice as wide.
        assert_eq!(width.pixels(with_sidebar(2520.)), px(1008.));
    }

    #[test]
    fn a_dragged_width_is_brought_within_the_bounds_first() {
        let space = with_sidebar(1260.);
        let narrowest = PanelWidth::from_pixels(px(10.), space).unwrap();
        assert_eq!(narrowest.pixels(space), MIN_PANEL);
        let widest = PanelWidth::from_pixels(px(5000.), space).unwrap();
        assert_eq!(widest.pixels(space), px(640.));
        let past_the_edge = PanelWidth::from_pixels(px(-40.), space).unwrap();
        assert_eq!(past_the_edge.pixels(space), MIN_PANEL);
    }

    #[test]
    fn a_dragged_width_is_kept_as_it_shows_in_whole_pixels() {
        let space = with_sidebar(1260.);
        let width = PanelWidth::from_pixels(px(503.6), space).unwrap();
        assert_eq!(width, PanelWidth::from_pixels(px(504.), space).unwrap());
        assert_eq!(width.pixels(space), px(504.));
    }

    #[test]
    fn a_window_with_no_width_takes_no_drag() {
        let space = Space::new(Pixels::ZERO, Pixels::ZERO);
        assert_eq!(PanelWidth::from_pixels(px(400.), space), None);
        assert_eq!(PanelWidth::DEFAULT.pixels(space), Pixels::ZERO);
    }

    #[test]
    fn only_a_share_above_0_and_up_to_1_is_a_width() {
        for share in [0.01, 0.46, 1.] {
            assert_eq!(
                PanelWidth::from_share(share).map(PanelWidth::share),
                Some(share)
            );
        }
        for share in [
            0.,
            -0.2,
            1.01,
            7.,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ] {
            assert_eq!(PanelWidth::from_share(share), None, "{share}");
        }
    }

    #[test]
    fn the_content_area_is_never_negative_or_wider_than_the_window() {
        assert_eq!(Space::new(px(800.), px(-60.)).content(), Pixels::ZERO);
        assert_eq!(Space::new(px(800.), px(900.)).content(), px(800.));
        assert_eq!(Space::new(px(-5.), px(10.)).content(), Pixels::ZERO);
    }
}
