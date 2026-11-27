//! The ember beside the setup's steps: the app's own face, asleep and grey before it has a
//! home, then warmer with each step done, a soft glow around it that grows with it and breathes
//! slowly (still under Reduce Motion).

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, BoxShadow, Role, TestSupportExt as _, div, img,
    pulsating_between, px,
};

use crate::assets::FACE;
use crate::store::setup::{BRIGHTEST, NOTES};
use crate::ui::palette::Pal;

/// How long one breath of the glow takes, in and out: slow, so it reads as warmth.
const BREATH: Duration = Duration::from_millis(3_600);

/// The glow at each level, for an ember 100 points wide: its blur, its spread and how much of
/// the palette's glow colour shows.
const HALO: [(f32, f32, f32); 5] = [
    (0., 0., 0.),
    (14., 2., 0.35),
    (26., 6., 0.6),
    (38., 10., 0.8),
    (54., 16., 1.),
];

/// The id of the glow while it breathes, and while it holds still (the tests tell them apart).
pub(crate) const BREATHING: &str = "setup-ember-breathing";
/// See [`BREATHING`].
pub(crate) const STILL: &str = "setup-ember-still";

/// The ember at `glow` (0 to [`BRIGHTEST`]), `size` points wide; its glow holds still when the
/// system asks for `reduced` motion.
pub(crate) fn picture(glow: u8, size: f32, p: &Pal, reduced: bool) -> AnyElement {
    let level = glow.min(BRIGHTEST);
    let (blur, spread, share) = HALO[usize::from(level)];
    let scale = size / 100.;
    let face = img(FACE).size_full().map(|face| match level {
        0 => face.grayscale(true).opacity(0.55),
        1 => face.opacity(0.85),
        _ => face,
    });
    let halo = (level > 0).then(|| {
        let halo = div()
            .absolute()
            .inset(px(size * 0.12))
            .rounded_full()
            .shadow(vec![
                BoxShadow::new(px(0.), px(0.), p.glow.opacity(share))
                    .blur_radius(px(blur * scale))
                    .spread_radius(px(spread * scale)),
            ]);
        if reduced {
            div()
                .id(STILL)
                .absolute()
                .inset_0()
                .child(halo)
                .test_support()
                .into_any_element()
        } else {
            div()
                .id(BREATHING)
                .absolute()
                .inset_0()
                .child(
                    halo.with_animation(
                        "setup-ember-breath",
                        Animation::new(BREATH)
                            .repeat()
                            .with_easing(pulsating_between(0.8, 1.)),
                        Styled::opacity,
                    ),
                )
                .test_support()
                .into_any_element()
        }
    });
    div()
        .id("setup-ember")
        .role(Role::Image)
        .aria_label(format!("Your ember. {}", NOTES[usize::from(level)]))
        .relative()
        .flex_none()
        .size(px(size))
        .children(halo)
        .child(face)
        .test_support()
        .into_any_element()
}
