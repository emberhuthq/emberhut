//! How opaque the sidebar must stay over the material for its text to read.
//!
//! The sidebar's fill is mixed with the blur behind the window channel by channel, and
//! luminance rises with every channel, so the fill over a black and over a white backdrop
//! bound the fill over any backdrop. An ink outside that range of luminance reads worst at one
//! of the two ends; an ink inside it matches the fill over some backdrop between them (1:1),
//! so it doesn't read at all. At both ends each colour the sidebar draws text or icons in
//! keeps WCAG AA (4.5:1) if it had it on the opaque fill, and an ink that never had it (the
//! faint one) keeps four fifths of its contrast. The same holds on a row under the pointer.
//! Selected rows are filled with an opaque token, so the backdrop never reaches their text.

use crate::color::{Rgb, contrast_ratio, luminance, mix};

/// WCAG AA for normal text.
pub(crate) const AA: f64 = 4.5;

/// The share of its contrast an ink below AA on the opaque fill keeps over any backdrop.
pub(crate) const KEEP_BELOW_AA: f64 = 0.8;

/// The darkest and the lightest backdrop, which bound all others.
pub(crate) const BACKDROPS: [Rgb; 2] = [0x00_00_00, 0xFF_FF_FF];

/// The sidebar's colours, as far as reading it goes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sidebar<'a> {
    /// Its fill, opaque.
    pub(crate) fill: Rgb,
    /// Every colour its text and icons are drawn in.
    pub(crate) inks: &'a [Rgb],
    /// A row under the pointer: this colour mixed into the fill, at this share.
    pub(crate) hover: (Rgb, f32),
}

impl Sidebar<'_> {
    /// The sidebar's alpha: the lowest from `floor` up, in hundredths, at which every ink
    /// still reads; 1 (opaque) when nothing below does.
    #[must_use]
    pub(crate) fn alpha(&self, floor: f32) -> f32 {
        (0..100_u8)
            .map(|hundredths| f32::from(hundredths) / 100.0)
            .filter(|&alpha| alpha >= floor)
            .find(|&alpha| self.reads_at(alpha))
            .unwrap_or(1.0)
    }

    /// Whether every ink reads with the fill at `alpha` over the material, on a plain row and
    /// on a row under the pointer: outside the range the fill spans over all backdrops, and
    /// with enough contrast at both of its ends.
    #[must_use]
    pub(crate) fn reads_at(&self, alpha: f32) -> bool {
        let rows = [None, Some(self.hover)];
        self.inks.iter().all(|&ink| {
            rows.iter().all(|&row| {
                let on = |fill: Rgb| row.map_or(fill, |(colour, share)| mix(colour, fill, share));
                let needed = needed(contrast_ratio(ink, on(self.fill)));
                let ends = BACKDROPS.map(|backdrop| on(mix(self.fill, backdrop, alpha)));
                let [darkest, lightest] = ends.map(luminance);
                let inside =
                    (darkest.min(lightest)..=darkest.max(lightest)).contains(&luminance(ink));
                !inside && ends.iter().all(|&end| contrast_ratio(ink, end) >= needed)
            })
        })
    }
}

/// The contrast an ink must keep over any backdrop, given what it has on the opaque fill.
#[must_use]
pub(crate) fn needed(opaque: f64) -> f64 {
    if opaque >= AA {
        AA
    } else {
        opaque * KEEP_BELOW_AA
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIGHT: Rgb = 0xF5_F6_F8;
    const INK: Rgb = 0x14_17_1C;

    #[test]
    fn an_opaque_sidebar_always_reads() {
        let sidebar = Sidebar {
            fill: LIGHT,
            inks: &[INK, 0x8A_90_9B],
            hover: (INK, 0.05),
        };
        assert!(sidebar.reads_at(1.0));
    }

    #[test]
    fn an_ink_right_at_aa_leaves_the_sidebar_opaque() {
        // Grey on the light fill at 4.5:1 or a hair above: any backdrop through it drops it.
        let at_aa = (0x60_u32..0xA0)
            .map(|v| v << 16 | v << 8 | v)
            .rfind(|&grey| contrast_ratio(grey, LIGHT) >= AA)
            .unwrap();
        assert!(contrast_ratio(at_aa, LIGHT) < AA + 0.1);
        let sidebar = Sidebar {
            fill: LIGHT,
            inks: &[at_aa],
            hover: (INK, 0.0),
        };
        assert!((sidebar.alpha(0.5) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_strong_ink_lets_the_floor_through() {
        let sidebar = Sidebar {
            fill: LIGHT,
            inks: &[INK],
            hover: (INK, 0.05),
        };
        assert!((sidebar.alpha(0.9) - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn the_alpha_found_reads_and_the_one_below_does_not() {
        let sidebar = Sidebar {
            fill: LIGHT,
            inks: &[INK, 0x5B_62_70, 0x25_63_EB],
            hover: (INK, 0.05),
        };
        let alpha = sidebar.alpha(0.5);
        assert!(alpha > 0.5 && alpha < 1.0, "{alpha}");
        assert!(sidebar.reads_at(alpha));
        assert!(!sidebar.reads_at(alpha - 0.01));
    }

    #[test]
    fn an_ink_the_fill_can_match_over_some_backdrop_never_reads() {
        // Over black and over white the grey keeps what it needs (2.23 and 1.82 against 1.41),
        // but a mid-grey backdrop makes the half-clear fill the grey itself.
        let sidebar = Sidebar {
            fill: LIGHT,
            inks: &[0xBC_BC_BC],
            hover: (INK, 0.0),
        };
        let needed = needed(contrast_ratio(0xBC_BC_BC, LIGHT));
        for backdrop in BACKDROPS {
            assert!(contrast_ratio(0xBC_BC_BC, mix(LIGHT, backdrop, 0.5)) >= needed);
        }
        assert!(contrast_ratio(0xBC_BC_BC, mix(LIGHT, 0x82_82_82, 0.5)) < 1.05);
        assert!(!sidebar.reads_at(0.5));
    }

    #[test]
    fn an_ink_below_aa_keeps_four_fifths() {
        assert!((needed(3.0) - 2.4).abs() < 1e-9);
        assert!((needed(7.0) - AA).abs() < 1e-9);
    }
}
