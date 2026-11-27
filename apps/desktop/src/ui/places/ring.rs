//! A goal's ring: a faint circle, and over it the share done in the goal's colour, from the
//! top, clockwise. When the share changes the ring moves to it over 0.6 s, easing out; under
//! Reduce motion it jumps there.

use std::f32::consts::{FRAC_PI_2, TAU};
use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::{App, Bounds, ElementId, Hsla, PathBuilder, Pixels, Window, canvas, div, point, px};

/// How many straight pieces a whole circle is drawn with.
const PIECES: f32 = 64.;

/// How long the ring takes to move to a new share.
const MOVE: Duration = Duration::from_millis(600);

/// A ring `size` wide, its line `line` thick, `share` (0 to 1) of it in `color` over `track`;
/// `id` keeps its motion from one frame to the next.
pub(crate) fn ring(
    id: impl Into<ElementId>,
    size: f32,
    line: f32,
    share: f64,
    color: Hsla,
    track: Hsla,
) -> Ring {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a share from 0 to 1: well inside f32"
    )]
    let share = share.clamp(0.0, 1.0) as f32;
    Ring {
        id: id.into(),
        size,
        line,
        share,
        color,
        track,
    }
}

/// A goal's ring, as [`ring`] makes it.
#[derive(IntoElement)]
pub(crate) struct Ring {
    id: ElementId,
    size: f32,
    line: f32,
    share: f32,
    color: Hsla,
    track: Hsla,
}

/// Where a ring is on its way from one share to the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RingMotion {
    from: f32,
    to: f32,
    since: Instant,
}

impl RingMotion {
    /// A ring standing at `share`, as first drawn.
    #[must_use]
    pub(crate) fn at(share: f32, now: Instant) -> Self {
        Self {
            from: share,
            to: share,
            since: now,
        }
    }

    /// The share drawn at `now`.
    #[must_use]
    pub(crate) fn share(&self, now: Instant) -> f32 {
        let t = (now.saturating_duration_since(self.since).as_secs_f32() / MOVE.as_secs_f32())
            .clamp(0., 1.);
        let eased = 1. - (1. - t).powi(3);
        self.from + (self.to - self.from) * eased
    }

    /// Whether it is still moving at `now`.
    #[must_use]
    pub(crate) fn is_moving(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.since) < MOVE
            && (self.from - self.to).abs() > f32::EPSILON
    }

    /// Heads for `share` from where it is at `now` (nothing changes when it heads there
    /// already).
    pub(crate) fn head_for(&mut self, share: f32, now: Instant) {
        if (share - self.to).abs() > f32::EPSILON {
            *self = Self {
                from: self.share(now),
                to: share,
                since: now,
            };
        }
    }
}

impl RenderOnce for Ring {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            size,
            line,
            share,
            color,
            track,
        } = self;
        let now = Instant::now();
        let motion = window.use_keyed_state(id, cx, |_, _| RingMotion::at(share, now));
        let still = crate::motion::reduced(cx);
        let drawn = motion.update(cx, |motion, _| {
            if still {
                *motion = RingMotion::at(share, now);
            } else {
                motion.head_for(share, now);
            }
            (motion.share(now), motion.is_moving(now))
        });
        let (drawn, moving) = drawn;
        if moving {
            window.request_animation_frame();
        }
        div().flex_none().size(px(size)).child(
            canvas(
                |_, _, _| {},
                move |bounds, (), window, _| {
                    arc(bounds, line, 1.0, track, window);
                    if drawn > 0.0 {
                        arc(bounds, line, drawn, color, window);
                    }
                },
            )
            .size(px(size)),
        )
    }
}

/// Strokes `share` of the circle inside `bounds`, from the top, clockwise.
fn arc(bounds: Bounds<Pixels>, line: f32, share: f32, color: Hsla, window: &mut Window) {
    let side = f32::from(bounds.size.width.min(bounds.size.height));
    let radius = (side - line) / 2.;
    let center = bounds.center();
    let at = |turn: f32| {
        let angle = turn * TAU - FRAC_PI_2;
        point(
            center.x + px(radius * angle.cos()),
            center.y + px(radius * angle.sin()),
        )
    };
    let mut path = PathBuilder::stroke(px(line));
    path.move_to(at(0.));
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "at most 64 pieces"
    )]
    let pieces = (PIECES * share).ceil().max(1.) as u32;
    for piece in 1..=pieces {
        #[expect(clippy::cast_precision_loss, reason = "at most 64 pieces")]
        let turn = share * piece as f32 / pieces as f32;
        path.line_to(at(turn));
    }
    match path.build() {
        Ok(path) => window.paint_path(path, color),
        Err(error) => log::debug!("a ring didn't build: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ring_stands_until_its_share_changes_then_eases_there() {
        let start = Instant::now();
        let mut motion = RingMotion::at(0.25, start);
        assert!((motion.share(start) - 0.25).abs() < f32::EPSILON);
        assert!(!motion.is_moving(start), "first drawn: no motion");
        motion.head_for(0.75, start);
        assert!(motion.is_moving(start));
        let half = motion.share(start + MOVE / 2);
        assert!(
            half > 0.5 && half < 0.75,
            "past half way at half time: {half}"
        );
        assert!((motion.share(start + MOVE) - 0.75).abs() < f32::EPSILON);
        assert!(!motion.is_moving(start + MOVE));
        let before = motion;
        motion.head_for(0.75, start + MOVE / 4);
        assert_eq!(motion, before, "the same share again changes nothing");
    }
}
