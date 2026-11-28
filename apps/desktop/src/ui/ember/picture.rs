//! The look's small pictures, painted: a preset's page (its paper, a sheet with two lines of
//! ink and an accent button, and an accent dot) in light, in dark, or in both split on a
//! diagonal for System, as macOS draws "Auto"; and the first-run setup's swatch, a circle of the
//! preset's paper split with its accent. Shapes are polygons, so the dark half can be cut along
//! the diagonal exactly.

use gpui_kit::prelude::*;
use gpui_kit::{Bounds, Hsla, PathBuilder, Pixels, Window, canvas, point, px};

use crate::ui::palette::Pal;
use emberhut_api::theme::{Appearance, Preset, Tokens};

/// A preset's page's height.
pub(crate) const PAGE_HEIGHT: f32 = 40.;

/// Which of a preset's appearances a picture shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shown {
    /// Its light colours.
    Light,
    /// Its dark colours.
    Dark,
    /// Light above the diagonal, dark below it (System).
    Both,
}

impl Shown {
    /// What a picture shows under `appearance`: System (and one this app doesn't know) both.
    pub(crate) fn of(appearance: &Appearance) -> Self {
        match appearance {
            Appearance::Light => Self::Light,
            Appearance::Dark => Self::Dark,
            _ => Self::Both,
        }
    }
}

/// A point, in pixels from the picture's top left.
type At = (f32, f32);

/// A filled shape and its colour.
type Shape = (Vec<At>, Hsla);

/// A preset's page, as wide as its place and [`PAGE_HEIGHT`] high, showing `shown`.
pub(crate) fn page(preset: &Preset, shown: Shown) -> impl IntoElement {
    let light = Colours::of(&preset.light);
    let dark = Colours::of(&preset.dark);
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            let (width, height) = size(bounds);
            match shown {
                Shown::Light => paint(window, bounds, &page_shapes(&light, width, height), None),
                Shown::Dark => paint(window, bounds, &page_shapes(&dark, width, height), None),
                Shown::Both => {
                    // The whole light page, then the dark one over it below the diagonal, so
                    // no seam shows between the halves.
                    paint(window, bounds, &page_shapes(&light, width, height), None);
                    let cut = Diagonal::steep(width, height);
                    paint(
                        window,
                        bounds,
                        &page_shapes(&dark, width, height),
                        Some(cut),
                    );
                }
            }
        },
    )
    .w_full()
    .h(px(PAGE_HEIGHT))
}

/// A preset's swatch for the first-run setup: a circle `diameter` across of its paper, the
/// lower right half its accent, in the light or `dark` colours, edged in `edge`.
pub(crate) fn swatch(preset: &Preset, dark: bool, edge: Hsla, diameter: f32) -> impl IntoElement {
    let colours = Colours::of(if dark { &preset.dark } else { &preset.light });
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            let r = diameter / 2.;
            let paper = vec![
                (circle(r, r, r), edge),
                (circle(r, r, r - 1.), colours.paper),
            ];
            paint(window, bounds, &paper, None);
            let accent = vec![(circle(r, r, r - 1.), colours.accent)];
            paint(
                window,
                bounds,
                &accent,
                Some(Diagonal::through_centre(diameter)),
            );
        },
    )
    .size(px(diameter))
    .flex_none()
}

/// The colours a picture is drawn in.
#[derive(Clone, Copy)]
struct Colours {
    paper: Hsla,
    sheet: Hsla,
    ink: Hsla,
    muted: Hsla,
    accent: Hsla,
}

impl Colours {
    fn of(tokens: &Tokens) -> Self {
        Self {
            paper: Pal::token(&tokens.background),
            sheet: Pal::token(&tokens.surface),
            ink: Pal::token(&tokens.text),
            muted: Pal::token(&tokens.muted),
            accent: Pal::token(&tokens.accent),
        }
    }
}

/// The page's shapes, back to front, for a page `width` by `height`.
fn page_shapes(c: &Colours, width: f32, height: f32) -> Vec<Shape> {
    let sheet_width = (width * 0.6).max(30.);
    vec![
        (rounded(0., 0., width, height, 6.), c.paper),
        (rounded(7., 7., sheet_width, height - 14., 4.), c.sheet),
        (rounded(13., 13., sheet_width * 0.62, 3., 1.5), c.ink),
        (rounded(13., 19., sheet_width * 0.4, 3., 1.5), c.muted),
        (rounded(13., 25., 16., 5., 2.5), c.accent),
        (circle(width - 15., height / 2., 5.), c.accent),
    ]
}

/// A line that cuts a picture in two: what lies to its right (below it, for a line drawn
/// top to bottom) is kept.
#[derive(Clone, Copy, Debug)]
struct Diagonal {
    from: At,
    to: At,
}

impl Diagonal {
    /// System's split on a page: steeper than corner to corner, through the middle.
    fn steep(width: f32, height: f32) -> Self {
        let lean = (height * 0.6).min(width * 0.25);
        Self {
            from: (width / 2. + lean, 0.),
            to: (width / 2. - lean, height),
        }
    }

    /// Corner to corner through a square `size` across, top right to bottom left.
    fn through_centre(size: f32) -> Self {
        Self {
            from: (size, 0.),
            to: (0., size),
        }
    }

    /// How far `at` lies on the kept side (negative: the other side).
    fn side(self, at: At) -> f32 {
        let (dx, dy) = (self.to.0 - self.from.0, self.to.1 - self.from.1);
        dx * (self.from.1 - at.1) - dy * (self.from.0 - at.0)
    }
}

/// `shape` cut along `line`: the part on its kept side (Sutherland–Hodgman, one edge).
fn cut(shape: &[At], line: Diagonal) -> Vec<At> {
    let mut kept = Vec::with_capacity(shape.len() + 2);
    for (n, &at) in shape.iter().enumerate() {
        let before = shape[(n + shape.len() - 1) % shape.len()];
        let (here, there) = (line.side(at), line.side(before));
        let crossing = || {
            let t = there / (there - here);
            (
                before.0 + t * (at.0 - before.0),
                before.1 + t * (at.1 - before.1),
            )
        };
        if here >= 0. {
            if there < 0. {
                kept.push(crossing());
            }
            kept.push(at);
        } else if there >= 0. {
            kept.push(crossing());
        }
    }
    kept
}

/// Paints `shapes` in `bounds`, each cut along `line` when there is one.
fn paint(window: &mut Window, bounds: Bounds<Pixels>, shapes: &[Shape], line: Option<Diagonal>) {
    for (shape, colour) in shapes {
        let points = match line {
            Some(line) => cut(shape, line),
            None => shape.clone(),
        };
        let Some((&first, rest)) = points.split_first() else {
            continue;
        };
        if rest.len() < 2 {
            continue;
        }
        let at = |(x, y): At| bounds.origin + point(px(x), px(y));
        let mut path = PathBuilder::fill();
        path.move_to(at(first));
        for &next in rest {
            path.line_to(at(next));
        }
        path.close();
        if let Ok(path) = path.build() {
            window.paint_path(path, *colour);
        }
    }
}

/// `bounds`' width and height in pixels.
fn size(bounds: Bounds<Pixels>) -> (f32, f32) {
    (f32::from(bounds.size.width), f32::from(bounds.size.height))
}

/// The points around a circle at (`x`, `y`) of radius `r`.
fn circle(x: f32, y: f32, r: f32) -> Vec<At> {
    const STEPS: u16 = 40;
    (0..STEPS)
        .map(|n| {
            let angle = f32::from(n) / f32::from(STEPS) * std::f32::consts::TAU;
            (x + r * angle.cos(), y + r * angle.sin())
        })
        .collect()
}

/// The points around a rectangle with corners rounded to `r`.
fn rounded(x: f32, y: f32, width: f32, height: f32, r: f32) -> Vec<At> {
    const STEPS: u16 = 6;
    let r = r.min(width / 2.).min(height / 2.);
    // Each corner's centre and the angle its quarter starts at, clockwise from the top left.
    let corners = [
        (x + r, y + r, 180_f32),
        (x + width - r, y + r, 270.),
        (x + width - r, y + height - r, 0.),
        (x + r, y + height - r, 90.),
    ];
    corners
        .iter()
        .flat_map(|&(cx, cy, start)| {
            (0..=STEPS).map(move |n| {
                let angle = (start + 90. * f32::from(n) / f32::from(STEPS)).to_radians();
                (cx + r * angle.cos(), cy + r * angle.sin())
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A polygon's area (positive whichever way round it goes).
    fn area(shape: &[At]) -> f32 {
        let twice: f32 = shape
            .iter()
            .zip(shape.iter().cycle().skip(1))
            .map(|(a, b)| a.0 * b.1 - b.0 * a.1)
            .sum();
        twice.abs() / 2.
    }

    #[test]
    fn the_diagonal_keeps_the_lower_right_half() {
        let square = rounded(0., 0., 10., 10., 0.);
        let half = cut(&square, Diagonal::through_centre(10.));
        assert!((area(&half) - 50.).abs() < 0.01, "{}", area(&half));
        let line = Diagonal::through_centre(10.);
        assert!(line.side((9., 9.)) > 0., "the bottom right is kept");
        assert!(line.side((1., 1.)) < 0., "the top left is not");
    }

    #[test]
    fn systems_split_cuts_a_page_through_its_middle() {
        let (width, height) = (104., PAGE_HEIGHT);
        let line = Diagonal::steep(width, height);
        let page = rounded(0., 0., width, height, 0.);
        let dark = area(&cut(&page, line));
        assert!((dark - width * height / 2.).abs() < 0.5, "{dark}");
        // Steeper than corner to corner.
        assert!(line.from.0 - line.to.0 < width / 2.);
    }

    #[test]
    fn a_shape_wholly_on_one_side_is_kept_whole_or_dropped() {
        let line = Diagonal::through_centre(100.);
        let corner = rounded(80., 80., 10., 10., 2.);
        assert_eq!(cut(&corner, line).len(), corner.len());
        assert_eq!(cut(&rounded(5., 5., 10., 10., 2.), line), Vec::<At>::new());
    }

    #[test]
    fn rounded_corners_stay_inside_the_rectangle() {
        for (x, y) in rounded(2., 3., 20., 8., 4.) {
            assert!(
                (2. ..=22.0001).contains(&x) && (3. ..=11.0001).contains(&y),
                "{x},{y}"
            );
        }
    }

    #[test]
    fn what_a_picture_shows_follows_the_appearance() {
        assert_eq!(Shown::of(&Appearance::Light), Shown::Light);
        assert_eq!(Shown::of(&Appearance::Dark), Shown::Dark);
        assert_eq!(Shown::of(&Appearance::System), Shown::Both);
    }
}
