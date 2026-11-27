//! A PDF's pages one after another in the viewer, fitted to its width and zoomed from there:
//! where each page sits, which are in sight (only those are drawn), which one the reader is on
//! (`3 / 12`), and how sharp a page's picture must be for the width it shows at.

use std::ops::Range;

/// The space between two pages, and around them, in pixels.
pub(crate) const GAP: f32 = 12.;

/// A page's size, in the PDF's points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PageSize {
    /// Its width.
    pub(crate) width: f32,
    /// Its height.
    pub(crate) height: f32,
}

impl PageSize {
    /// Its height over its width; a page with no width counts as A4.
    #[must_use]
    fn aspect(self) -> f32 {
        if self.width > 0. && self.height > 0. && self.height.is_finite() {
            self.height / self.width
        } else {
            297. / 210.
        }
    }
}

/// Where the pages sit for one width.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Layout {
    /// Each page's top and height, in pixels from the top of the first's gap.
    pages: Vec<(f32, f32)>,
    /// How wide each page shows.
    width: f32,
}

impl Layout {
    /// The pages of `sizes` at `width` pixels wide each.
    #[must_use]
    pub(crate) fn new(sizes: &[PageSize], width: f32) -> Self {
        let width = width.max(1.);
        let mut top = GAP;
        let pages = sizes
            .iter()
            .map(|size| {
                let height = width * size.aspect();
                let page = (top, height);
                top += height + GAP;
                page
            })
            .collect();
        Self { pages, width }
    }

    /// How wide each page shows.
    #[must_use]
    pub(crate) fn width(&self) -> f32 {
        self.width
    }

    /// Each page's height.
    pub(crate) fn heights(&self) -> impl Iterator<Item = f32> + '_ {
        self.pages.iter().map(|&(_, height)| height)
    }

    /// The pages in sight with the view scrolled `offset` pixels down and `viewport` tall, and
    /// one more each side, so a page is ready as it comes in.
    #[must_use]
    pub(crate) fn in_sight(&self, offset: f32, viewport: f32) -> Range<usize> {
        let bottom = offset + viewport;
        let first = self
            .pages
            .iter()
            .position(|&(top, height)| top + height >= offset)
            .unwrap_or(self.pages.len());
        let last = self
            .pages
            .iter()
            .rposition(|&(top, _)| top <= bottom)
            .map_or(first, |last| last + 1);
        first.saturating_sub(1)..(last + 1).min(self.pages.len()).max(first)
    }

    /// The page the reader is on: the one across the middle of the view (from 0).
    #[must_use]
    pub(crate) fn current(&self, offset: f32, viewport: f32) -> usize {
        let middle = offset + viewport / 2.;
        self.pages
            .iter()
            .rposition(|&(top, _)| top <= middle)
            .unwrap_or(0)
    }
}

/// `3 / 12`: the page the reader is on (from 0) of how many.
#[must_use]
pub(crate) fn label(current: usize, count: usize) -> String {
    format!("{} / {count}", (current + 1).min(count))
}

/// The steps a page's picture is drawn at, in pixels wide: the smallest one at least as wide as
/// it shows, so zooming redraws a page only when it crosses a step.
const SHARPNESS: [u32; 7] = [600, 900, 1300, 1800, 2600, 3600, 5000];

/// How wide a page's picture is drawn for a page `shown` pixels wide on a screen of `scale`.
#[must_use]
pub(crate) fn sharpness(shown: f32, scale: f32) -> u32 {
    let needed = (shown * scale.max(1.)).ceil();
    SHARPNESS
        .into_iter()
        .find(|&step| f64::from(step) >= f64::from(needed))
        .unwrap_or(SHARPNESS[SHARPNESS.len() - 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    const LETTER: PageSize = PageSize {
        width: 612.,
        height: 792.,
    };

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn pages_stack_with_a_gap_at_the_width() {
        let layout = Layout::new(&[LETTER, LETTER], 612.);
        let heights: Vec<f32> = layout.heights().collect();
        assert!(close(heights[0], 792.) && close(heights[1], 792.));
        assert!(close(layout.width(), 612.));
        let odd = Layout::new(
            &[PageSize {
                width: 0.,
                height: 5.,
            }],
            210.,
        );
        assert!(
            close(odd.heights().next().unwrap(), 297.),
            "a page with no width is A4"
        );
    }

    #[test]
    fn only_the_pages_in_sight_and_their_neighbours_are_drawn() {
        let layout = Layout::new(&[LETTER; 10], 612.);
        // Each page is 792 tall plus a 12 gap: page 3 (from 0) starts at 12 + 3 * 804.
        let top_of_3 = 12. + 3. * 804.;
        assert_eq!(layout.in_sight(top_of_3, 700.), 2..5);
        assert_eq!(layout.in_sight(0., 700.), 0..2);
        assert_eq!(layout.in_sight(100_000., 700.), 9..10);
        assert_eq!(Layout::new(&[], 600.).in_sight(0., 700.), 0..0);
    }

    #[test]
    fn the_current_page_is_across_the_middle_and_reads_n_of_count() {
        let layout = Layout::new(&[LETTER; 12], 612.);
        assert_eq!(layout.current(0., 700.), 0);
        assert_eq!(layout.current(12. + 2. * 804., 700.), 2);
        assert_eq!(label(2, 12), "3 / 12");
        assert_eq!(label(0, 0), "0 / 0");
    }

    #[test]
    fn a_page_is_drawn_sharp_enough_in_steps() {
        assert_eq!(sharpness(500., 1.), 600);
        assert_eq!(sharpness(500., 2.), 1300);
        assert_eq!(sharpness(4000., 2.), 5000, "no sharper than the last step");
        assert_eq!(sharpness(500., 0.5), 600, "a screen counts at least 1x");
    }
}
