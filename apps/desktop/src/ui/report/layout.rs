//! How the report panel is laid out: its width, whether it fills the content area, and a drag of
//! its inner edge in progress. Expanding leaves the width alone, so restoring returns to it.
//!
//! The width moves only when what shows moves: a drag that leaves the panel as wide on screen
//! as it was (held against a bound, or in a window too narrow for both) changes nothing, and
//! nothing is kept.

use gpui_kit::Pixels;

use super::width::{PanelWidth, Space};

/// A drag of the panel's edge: where the pointer went down, and the panel then.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Drag {
    from: Pixels,
    /// The panel's width on screen.
    shown: Pixels,
    /// The width before the drag, to go back to when the drag shows nothing new.
    before: PanelWidth,
}

/// The panel's layout.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct PanelLayout {
    width: PanelWidth,
    expanded: bool,
    drag: Option<Drag>,
}

impl PanelLayout {
    /// The layout at `width`, not expanded.
    #[must_use]
    pub(super) fn new(width: PanelWidth) -> Self {
        Self {
            width,
            expanded: false,
            drag: None,
        }
    }

    /// The width the panel has when it isn't expanded.
    #[cfg(test)]
    #[must_use]
    pub(super) fn width(&self) -> PanelWidth {
        self.width
    }

    /// Whether the panel fills the content area.
    #[must_use]
    pub(super) fn is_expanded(&self) -> bool {
        self.expanded
    }

    /// The panel's width in `space`: all of the content area when expanded.
    #[must_use]
    pub(super) fn pixels(&self, space: Space) -> Pixels {
        if self.expanded {
            space.content()
        } else {
            self.width.pixels(space)
        }
    }

    /// Lets the panel fill the content area. A drag still held ends first, as if let go: the
    /// width to keep, if it changed.
    pub(super) fn expand(&mut self, space: Space) -> Option<PanelWidth> {
        let changed = self.release(space);
        self.expanded = true;
        changed
    }

    /// Brings an expanded panel back to its width.
    pub(super) fn restore(&mut self) {
        self.expanded = false;
    }

    /// The pointer went down on the edge at `x` (in window coordinates), in `space`. An expanded
    /// panel has no edge to drag.
    pub(super) fn press(&mut self, x: Pixels, space: Space) {
        if self.expanded {
            return;
        }
        self.drag = Some(Drag {
            from: x,
            shown: self.width.pixels(space),
            before: self.width,
        });
    }

    /// The pointer moved to `x` while the edge is held: the edge follows it, within the bounds.
    /// Whether the panel shows wider or narrower (so it is drawn again).
    pub(super) fn drag_to(&mut self, x: Pixels, space: Space) -> bool {
        let Some(drag) = self.drag else {
            return false;
        };
        // The panel's right side stays put: the pointer moving left widens it.
        let asked = drag.shown - (x - drag.from);
        match PanelWidth::from_pixels(asked, space) {
            Some(width) if width.pixels(space) != self.width.pixels(space) => {
                self.width = width;
                true
            }
            Some(_) | None => false,
        }
    }

    /// The pointer let go, in `space`: the width to keep, if the panel shows wider or narrower
    /// than before the drag. A drag that ends where it began keeps the width it began with.
    pub(super) fn release(&mut self, space: Space) -> Option<PanelWidth> {
        let drag = self.drag.take()?;
        if self.width.pixels(space) == drag.shown {
            self.width = drag.before;
            return None;
        }
        Some(self.width)
    }

    /// Back to [`PanelWidth::DEFAULT`] (a double click on the edge), ending any drag: the width
    /// to keep, if it changed.
    pub(super) fn reset(&mut self) -> Option<PanelWidth> {
        // A drag not yet let go hasn't been kept: what is kept is the width before it.
        let kept = self.drag.take().map_or(self.width, |drag| drag.before);
        self.width = PanelWidth::DEFAULT;
        (kept != PanelWidth::DEFAULT).then_some(PanelWidth::DEFAULT)
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::px;

    use super::*;

    /// 1260 px wide, 1000 px of it the content area: the panel between 380 and 640 px.
    fn space() -> Space {
        Space::new(px(1260.), px(1000.))
    }

    fn at(share: f32) -> PanelLayout {
        PanelLayout::new(PanelWidth::from_share(share).unwrap())
    }

    #[test]
    fn dragging_the_edge_left_widens_the_panel() {
        // 504 px, its edge at 756.
        let mut layout = at(0.4);
        layout.press(px(756.), space());
        assert!(layout.drag_to(px(700.), space()));
        assert_eq!(layout.pixels(space()), px(560.));
        assert!(layout.drag_to(px(800.), space()));
        assert_eq!(layout.pixels(space()), px(460.));
        let kept = layout.release(space()).unwrap();
        assert_eq!(kept, layout.width());
        assert_eq!(kept.pixels(space()), px(460.));
    }

    #[test]
    fn where_on_the_edge_it_is_held_doesnt_move_it() {
        let mut layout = at(0.4);
        layout.press(px(758.), space());
        assert!(!layout.drag_to(px(758.), space()));
        assert!(!layout.drag_to(px(758.3), space()), "less than a pixel");
        assert_eq!(layout.pixels(space()), px(504.));
        assert_eq!(layout.release(space()), None, "nothing to keep");
    }

    #[test]
    fn a_drag_that_comes_back_keeps_the_width_it_began_with() {
        // 46% of 1260 is 579.6, shown as 580: a drag back to 580 px isn't a new width.
        let mut layout = PanelLayout::default();
        layout.press(px(680.), space());
        assert!(layout.drag_to(px(600.), space()));
        assert!(layout.drag_to(px(680.), space()));
        assert_eq!(layout.release(space()), None);
        assert_eq!(layout.width(), PanelWidth::DEFAULT);
    }

    #[test]
    fn a_drag_stops_at_the_bounds() {
        let mut layout = at(0.4);
        layout.press(px(756.), space());
        assert!(layout.drag_to(px(0.), space()));
        assert_eq!(layout.pixels(space()), px(640.), "the chat keeps 360 px");
        assert!(!layout.drag_to(px(-50.), space()), "held against the bound");
        assert!(layout.drag_to(px(1260.), space()));
        assert_eq!(layout.pixels(space()), px(380.), "the panel keeps 380 px");
        assert!(!layout.drag_to(px(2000.), space()));
    }

    #[test]
    fn a_drag_against_a_bound_keeps_nothing() {
        // 1100 px wide, 840 of content: 46% (506 px) is held at 480 by the chat's 360.
        let space = Space::new(px(1100.), px(840.));
        let mut layout = PanelLayout::default();
        layout.press(px(620.), space);
        assert!(!layout.drag_to(px(619.), space));
        assert!(!layout.drag_to(px(500.), space));
        assert_eq!(layout.release(space), None);
        assert_eq!(layout.width(), PanelWidth::DEFAULT);
    }

    #[test]
    fn a_drag_in_a_window_too_narrow_for_both_keeps_nothing() {
        // 900 px wide, 640 of content: the panel is 380 px whatever the share.
        let space = Space::new(px(900.), px(640.));
        let mut layout = PanelLayout::default();
        layout.press(px(520.), space);
        for x in [519., 400., 700.] {
            assert!(!layout.drag_to(px(x), space), "{x}");
        }
        assert_eq!(layout.release(space), None);
        assert_eq!(layout.width(), PanelWidth::DEFAULT);
    }

    #[test]
    fn a_move_without_a_press_is_no_drag() {
        let mut layout = at(0.4);
        assert!(!layout.drag_to(px(600.), space()));
        assert_eq!(layout.release(space()), None);
        layout.press(px(756.), space());
        layout.drag_to(px(700.), space());
        assert!(layout.release(space()).is_some());
        assert!(!layout.drag_to(px(600.), space()), "the drag is over");
    }

    #[test]
    fn a_reset_returns_to_the_default() {
        let mut layout = at(0.4);
        assert_eq!(layout.reset(), Some(PanelWidth::DEFAULT));
        assert_eq!(layout.width(), PanelWidth::DEFAULT);
        assert_eq!(layout.reset(), None, "already at the default");
    }

    #[test]
    fn a_reset_ends_the_drag_its_second_press_began() {
        // A double click's second press starts a drag; the reset wins over it.
        let mut layout = at(0.4);
        layout.press(px(756.), space());
        assert!(layout.drag_to(px(750.), space()));
        assert_eq!(layout.reset(), Some(PanelWidth::DEFAULT));
        assert_eq!(layout.release(space()), None);
        assert_eq!(layout.width(), PanelWidth::DEFAULT);

        // From the default, the drag's width was never kept: nothing new to keep.
        let mut layout = PanelLayout::default();
        layout.press(px(680.), space());
        assert!(layout.drag_to(px(670.), space()));
        assert_eq!(layout.reset(), None);
        assert_eq!(layout.width(), PanelWidth::DEFAULT);
    }

    #[test]
    fn expanding_fills_the_content_area_and_restoring_returns_to_the_width() {
        let mut layout = at(0.4);
        assert_eq!(layout.expand(space()), None);
        assert!(layout.is_expanded());
        assert_eq!(layout.pixels(space()), px(1000.));
        layout.restore();
        assert!(!layout.is_expanded());
        assert_eq!(layout.pixels(space()), px(504.));
        layout.restore();
        assert!(!layout.is_expanded(), "restoring twice is restoring once");
        assert_eq!(layout.pixels(space()), px(504.));
    }

    #[test]
    fn expanding_during_a_drag_keeps_what_the_drag_did() {
        let mut layout = at(0.4);
        layout.press(px(756.), space());
        assert!(layout.drag_to(px(700.), space()));
        let kept = layout.expand(space()).unwrap();
        assert_eq!(kept.pixels(space()), px(560.));
        assert!(!layout.drag_to(px(600.), space()), "the drag is over");
        layout.restore();
        assert_eq!(layout.pixels(space()), px(560.));
    }

    #[test]
    fn an_expanded_panel_has_no_edge_to_drag() {
        let mut layout = at(0.4);
        layout.expand(space());
        layout.press(px(260.), space());
        assert!(!layout.drag_to(px(200.), space()));
        assert_eq!(layout.release(space()), None);
        layout.restore();
        assert_eq!(layout.pixels(space()), px(504.));
    }

    #[test]
    fn the_layout_starts_at_its_width_not_expanded() {
        let layout = at(0.5);
        assert!(!layout.is_expanded());
        assert_eq!(layout.width(), PanelWidth::from_share(0.5).unwrap());
        assert_eq!(PanelLayout::default().width(), PanelWidth::DEFAULT);
    }
}
