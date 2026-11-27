//! What a running turn is drawn with: the spark, a four-point star that breathes and turns with
//! a smaller one behind it turning the other way, so it twinkles while the turn works; and the
//! live word with a slow light passing over it. Both hold still under Reduce motion.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::component::shimmer::ShimmerText;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Bounds, ElementId, Hsla, PathBuilder, Pixels, Point, canvas};
use gpui_kit::{div, point, px};

/// How strongly the star behind shows through.
const BACK_OPACITY: f32 = 0.35;

/// How long the light takes to pass over the word once.
const SWEEP: Duration = Duration::from_millis(2400);

/// How wide the light is, against the word's width.
const SWEEP_SPREAD: f32 = 0.35;

/// How far the sides of a star are drawn in towards its middle, against its reach.
const WAIST: f32 = 0.16;

/// A point from the spark's middle: right, then down.
pub(crate) type Spot = (f32, f32);

/// One star of the spark at a moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Star {
    /// How far it is turned, in degrees, clockwise.
    pub(crate) turn: f32,
    /// How far its points reach, against the spark's half width.
    pub(crate) reach: f32,
}

/// The two stars `t` seconds into the spark's life (none: held still): the one behind, then
/// the one in front. The front one breathes about twice a second and turns once in 24 s; the
/// one behind breathes the other way and turns back once in 12 s.
#[must_use]
pub(crate) fn stars(t: Option<f64>) -> (Star, Star) {
    let Some(t) = t else {
        return (
            Star {
                turn: 45.,
                reach: 0.72 * 0.8,
            },
            Star {
                turn: 0.,
                reach: 1.,
            },
        );
    };
    let breath = 0.82 + 0.18 * (t * 3.4).sin();
    let back = 45. - t.rem_euclid(12.) * 30.;
    let front = t.rem_euclid(24.) * 15.;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "angles within a turn and sizes near 1: well inside f32"
    )]
    (
        Star {
            turn: back as f32,
            reach: (0.72 * (1.8 - breath)) as f32,
        },
        Star {
            turn: front as f32,
            reach: breath as f32,
        },
    )
}

/// A star's outline around its middle, `radius` to each point and turned `turn` degrees: where
/// it starts (its top point), then each of the next points clockwise with the control point of
/// the curve that reaches it.
#[must_use]
pub(crate) fn outline(radius: f32, turn: f32) -> (Spot, [(Spot, Spot); 4]) {
    let (sin, cos) = turn.to_radians().sin_cos();
    let at = |x: f32, y: f32| (x * cos - y * sin, x * sin + y * cos);
    let (r, k) = (radius, radius * WAIST);
    (
        at(0., -r),
        [
            (at(r, 0.), at(k, -k)),
            (at(0., r), at(k, k)),
            (at(-r, 0.), at(-k, k)),
            (at(0., -r), at(-k, -k)),
        ],
    )
}

/// The spark, `size` wide, in `color` (the one behind fainter).
pub(crate) fn spark(color: Hsla, size: f32, cx: &App) -> impl IntoElement {
    let still = crate::motion::reduced(cx);
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            let (back, front) = stars((!still).then(seconds));
            paint_star(bounds, back, color.opacity(BACK_OPACITY), window);
            paint_star(bounds, front, color, window);
            if !still {
                window.request_animation_frame();
            }
        },
    )
    .size(px(size))
    .flex_none()
}

/// The live word with the light passing over it; plain under Reduce motion.
pub(crate) fn shimmer(id: impl Into<ElementId>, word: String, cx: &App) -> AnyElement {
    if crate::motion::reduced(cx) {
        return div().min_w_0().truncate().child(word).into_any_element();
    }
    ShimmerText::new(word)
        .id(id)
        .duration(SWEEP)
        .spread(SWEEP_SPREAD)
        .truncate()
        .into_any_element()
}

/// Seconds on the wall clock: the spark's moment, the same in every window.
fn seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0., |since| since.as_secs_f64())
}

fn paint_star(bounds: Bounds<Pixels>, star: Star, color: Hsla, window: &mut gpui_kit::Window) {
    let half = f32::from(bounds.size.width.min(bounds.size.height)) / 2.;
    let center = bounds.center();
    let at = |(x, y): (f32, f32)| -> Point<Pixels> { point(center.x + px(x), center.y + px(y)) };
    let (start, curves) = outline(half * star.reach, star.turn);
    let mut path = PathBuilder::fill();
    path.move_to(at(start));
    for (to, control) in curves {
        path.curve_to(at(to), at(control));
    }
    path.close();
    match path.build() {
        Ok(path) => window.paint_path(path, color),
        Err(error) => log::debug!("a spark didn't build: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4
    }

    #[test]
    fn held_still_the_front_star_is_whole_and_upright() {
        let (back, front) = stars(None);
        assert!((front.reach - 1.).abs() < f32::EPSILON);
        assert!(front.turn.abs() < f32::EPSILON);
        assert!((back.turn - 45.).abs() < f32::EPSILON);
        assert!(back.reach < front.reach);
    }

    #[test]
    fn it_breathes_and_the_two_stars_turn_opposite_ways() {
        let (back0, front0) = stars(Some(0.));
        let (back1, front1) = stars(Some(1.));
        assert!(front1.turn > front0.turn, "the front one turns clockwise");
        assert!(back1.turn < back0.turn, "the one behind turns back");
        let reaches: Vec<f32> = (0..20)
            .map(|i| stars(Some(f64::from(i) * 0.1)).1.reach)
            .collect();
        let (low, high) = reaches
            .iter()
            .fold((f32::MAX, f32::MIN), |(l, h), r| (l.min(*r), h.max(*r)));
        assert!(low < 0.7 && high > 0.95, "between {low} and {high}");
        let (back, front) = stars(Some(24.));
        assert!(front.turn.abs() < 1e-3, "a whole turn in 24 s");
        assert!((back.turn - 45.).abs() < 1e-3, "two turns back in 24 s");
    }

    #[test]
    fn a_star_has_four_points_and_a_narrow_waist() {
        let (start, curves) = outline(10., 0.);
        assert!(near(start, (0., -10.)));
        let points: Vec<(f32, f32)> = curves.iter().map(|(to, _)| *to).collect();
        assert!(near(points[0], (10., 0.)));
        assert!(near(points[1], (0., 10.)));
        assert!(near(points[2], (-10., 0.)));
        assert!(near(points[3], start), "it closes where it started");
        assert!(
            curves
                .iter()
                .all(|(_, c)| c.0.abs() <= 1.6 && c.1.abs() <= 1.6)
        );
        let (turned, _) = outline(10., 90.);
        assert!(
            near(turned, (10., 0.)),
            "a quarter turn puts the top point right"
        );
    }
}
