//! A pairing link drawn as a QR code, for a phone's scanner: the link's modules, black on white
//! with the standard quiet zone of four modules. Each module is a whole number of the display's
//! pixels and the picture is shown at exactly its pixel size, so no edge is blurred by scaling.

use std::sync::Arc;

use emberhut_identity::PairingLink;
use gpui_kit::{App, RenderImage};
use image::{Frame, Rgba, RgbaImage};
use qrcode::{Color, EcLevel, QrCode};

use crate::store::devices::NewCode;

/// The quiet zone around the code, in modules (the standard's least).
const QUIET: u32 = 4;
/// About how wide the code is shown, in points.
const TARGET: f32 = 180.;

/// A QR code's picture and the size to show it at.
#[derive(Clone, Debug)]
pub(crate) struct Qr {
    /// The picture.
    pub(crate) picture: Arc<RenderImage>,
    /// Its side in points: its pixels over the display's scale factor.
    pub(crate) side: f32,
}

/// The QR code of a pairing code's link, drawn once a link and display scale (Settings ›
/// Devices, and the setup's "Let it reach you").
#[derive(Default)]
pub(crate) struct PairingQr {
    /// The link last drawn, for which scale (its bits), and its picture.
    drawn: Option<((String, u32), Qr)>,
}

impl PairingQr {
    /// The QR code of `code`'s link for a display of `scale`, once it is made and its link
    /// carries that code; the picture drawn before is let go when another is wanted.
    pub(crate) fn of(&mut self, code: Option<&NewCode>, scale: f32, cx: &mut App) -> Option<Qr> {
        let link = match code {
            Some(NewCode::Made(code)) => code.link.clone().filter(|link| {
                PairingLink::parse(link).is_ok_and(|read| same_code(read.code(), &code.code))
            }),
            _ => None,
        };
        let wanted = link.map(|link| (link, scale.to_bits()));
        if self.drawn.as_ref().map(|(drawn, _)| drawn) != wanted.as_ref() {
            if let Some((_, old)) = self.drawn.take() {
                cx.drop_image(old.picture, None);
            }
            self.drawn =
                wanted.and_then(|(link, bits)| qr(&link, scale).map(|drawn| ((link, bits), drawn)));
        }
        self.drawn.as_ref().map(|(_, drawn)| drawn.clone())
    }
}

/// Whether two pairing codes are the same as the server reads them: case, spaces and the dash
/// don't matter.
fn same_code(a: &str, b: &str) -> bool {
    let plain = |code: &str| {
        code.chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_uppercase())
            .collect::<String>()
    };
    plain(a) == plain(b)
}

/// `text` as a QR code for a display of `scale` pixels a point, or `None` when it is too long
/// for one.
#[must_use]
pub(crate) fn qr(text: &str, scale: f32) -> Option<Qr> {
    let (width, dark) = modules(text)?;
    let (pixels, side) = layout(width, scale);
    let picture = Arc::new(RenderImage::new(vec![Frame::new(draw(
        width, &dark, pixels,
    ))]));
    Some(Qr { picture, side })
}

/// Pixels a module for a code `width` modules wide on a display of `scale`, the most that keeps
/// it within about [`TARGET`] points (at least one), and the side that gives, in points.
fn layout(width: u32, scale: f32) -> (u32, f32) {
    let scale = if scale.is_finite() && scale > 0. {
        scale
    } else {
        1.
    };
    let across = width + 2 * QUIET;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "a small positive number of pixels, rounded down on purpose"
    )]
    let pixels = ((TARGET * scale / across as f32).floor() as u32).max(1);
    #[expect(
        clippy::cast_precision_loss,
        reason = "a picture's side in pixels, far below f32's exact range"
    )]
    let side = (across * pixels) as f32 / scale;
    (pixels, side)
}

/// The code's width in modules, and which modules are dark, row by row.
fn modules(text: &str) -> Option<(u32, Vec<bool>)> {
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M).ok()?;
    let width = u32::try_from(code.width()).ok()?;
    let dark = code
        .to_colors()
        .into_iter()
        .map(|color| color == Color::Dark)
        .collect();
    Some((width, dark))
}

/// The modules drawn at `pixels` a module, inside the quiet zone.
fn draw(width: u32, dark: &[bool], pixels: u32) -> RgbaImage {
    let side = (width + 2 * QUIET) * pixels;
    RgbaImage::from_fn(side, side, |x, y| {
        let (column, row) = (x / pixels, y / pixels);
        let modules = QUIET..QUIET + width;
        let is_dark = modules.contains(&column)
            && modules.contains(&row)
            && usize::try_from((row - QUIET) * width + (column - QUIET))
                .ok()
                .and_then(|index| dark.get(index).copied())
                .unwrap_or(false);
        // Black and white read the same in RGBA and in GPUI's BGRA.
        if is_dark {
            Rgba([0, 0, 0, 255])
        } else {
            Rgba([255, 255, 255, 255])
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINK: &str = "https://emberhut.com/pair#id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa&code=ABCD-EFGH\
        &route=http%3A%2F%2F127.0.0.1%3A8791";

    #[test]
    fn a_link_is_drawn_with_its_quiet_zone() {
        let (width, dark) = modules(LINK).expect("a code");
        assert_eq!(dark.len(), usize::try_from(width * width).unwrap());
        let drawn = draw(width, &dark, 3);
        assert_eq!(drawn.width(), (width + 2 * QUIET) * 3);
        assert_eq!(
            drawn.get_pixel(0, 0),
            &Rgba([255, 255, 255, 255]),
            "quiet zone"
        );
        let corner = QUIET * 3;
        assert_eq!(
            drawn.get_pixel(corner, corner),
            &Rgba([0, 0, 0, 255]),
            "the finder pattern's corner"
        );
        assert!(qr(LINK, 2.).is_some());
    }

    #[test]
    fn modules_are_whole_pixels_and_the_picture_is_shown_at_its_pixel_size() {
        for (width, scale) in [(33, 1.), (33, 2.), (57, 2.), (89, 1.5), (177, 1.)] {
            let (pixels, side) = layout(width, scale);
            let across = width + 2 * QUIET;
            assert!(pixels >= 1);
            let shown_pixels = side * scale;
            let drawn_pixels = f32::from(u16::try_from(across * pixels).unwrap());
            assert!(
                (shown_pixels - drawn_pixels).abs() < 0.01,
                "{width} at {scale}: a point of the picture is a pixel of the display"
            );
            assert!(side <= TARGET || pixels == 1, "{width} at {scale}: {side}");
        }
    }

    #[test]
    fn the_longest_link_still_makes_a_code() {
        assert!(modules(&"x".repeat(2_000)).is_some());
        assert!(
            modules(&"x".repeat(5_000)).is_none(),
            "past a QR code's capacity"
        );
    }
}
