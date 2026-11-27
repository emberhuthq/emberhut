//! How far the viewer is zoomed into a picture, a PDF or a note: a share of the size that fits
//! (1 fits the width), from its smallest (a note may get smaller than it starts, a picture not)
//! up to six times. ⌘+ and ⌘− step by a quarter, ⌘0 fits again, a double-click on a picture
//! goes to 2.5 and back, and a pinch follows the fingers.

use crate::store::files::Kind;

/// The most it zooms in.
const MOST: f32 = 6.;

/// What a double-click on a picture zooms to.
const CLOSE_UP: f32 = 2.5;

/// What ⌘+ multiplies by.
pub(crate) const IN: f32 = 1.25;

/// What ⌘− multiplies by.
pub(crate) const OUT: f32 = 0.8;

/// The viewer's zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Zoom {
    scale: f32,
    least: f32,
}

impl Zoom {
    /// Fitted, for a file of `kind`.
    #[must_use]
    pub(crate) fn fitted(kind: Kind) -> Self {
        let least = match kind {
            Kind::Note => 0.6,
            Kind::Pdf => 0.5,
            Kind::Picture | Kind::Video | Kind::Report | Kind::Document => 1.,
        };
        Self { scale: 1., least }
    }

    /// The share of the fitted size it shows at.
    #[must_use]
    pub(crate) fn scale(self) -> f32 {
        self.scale
    }

    fn set(&mut self, scale: f32) {
        if scale.is_finite() {
            self.scale = scale.clamp(self.least, MOST);
        }
    }

    /// ⌘+ (`by` [`IN`]) or ⌘− (`by` [`OUT`]).
    pub(crate) fn step(&mut self, by: f32) {
        self.set(self.scale * by);
    }

    /// ⌘0: back to the size that fits.
    pub(crate) fn fit(&mut self) {
        self.scale = 1.;
    }

    /// A double-click: a close-up, or back to fitted from any zoom.
    pub(crate) fn toggle(&mut self) {
        if self.scale > 1. {
            self.fit();
        } else {
            self.set(CLOSE_UP);
        }
    }

    /// A pinch moved by `delta` (0.1 is a tenth bigger, a negative one smaller).
    pub(crate) fn pinch(&mut self, delta: f32) {
        self.set(self.scale * (1. + delta));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn steps_stay_between_the_least_and_six() {
        let mut zoom = Zoom::fitted(Kind::Picture);
        assert!(close(zoom.scale(), 1.));
        zoom.step(OUT);
        assert!(
            close(zoom.scale(), 1.),
            "a picture never gets smaller than it fits"
        );
        zoom.step(IN);
        assert!(close(zoom.scale(), 1.25));
        for _ in 0..20 {
            zoom.step(IN);
        }
        assert!(close(zoom.scale(), 6.));
        zoom.fit();
        assert!(close(zoom.scale(), 1.));
        let mut note = Zoom::fitted(Kind::Note);
        for _ in 0..10 {
            note.step(OUT);
        }
        assert!(
            close(note.scale(), 0.6),
            "a note's text gets smaller, to a point"
        );
    }

    #[test]
    fn a_double_click_goes_close_and_back() {
        let mut zoom = Zoom::fitted(Kind::Picture);
        zoom.toggle();
        assert!(close(zoom.scale(), 2.5));
        zoom.toggle();
        assert!(close(zoom.scale(), 1.));
        zoom.step(IN);
        zoom.toggle();
        assert!(close(zoom.scale(), 1.), "from any zoom, back to fitted");
    }

    #[test]
    fn a_pinch_follows_the_fingers() {
        let mut zoom = Zoom::fitted(Kind::Pdf);
        zoom.pinch(0.5);
        assert!(close(zoom.scale(), 1.5));
        zoom.pinch(-0.9);
        assert!(close(zoom.scale(), 0.5), "no smaller than a PDF's least");
        zoom.pinch(f32::NAN);
        assert!(close(zoom.scale(), 0.5), "nonsense is ignored");
    }
}
