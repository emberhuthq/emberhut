//! Colour arithmetic on `#RRGGBB` text, with no drawing library: the palette turns the results
//! into GPUI colours.

/// A colour as `0xRRGGBB`.
pub(crate) type Rgb = u32;

/// `#RRGGBB` or `RRGGBB` as a colour; `None` if it isn't one.
#[must_use]
pub(crate) fn parse_hex(text: &str) -> Option<Rgb> {
    let digits = text.trim().trim_start_matches('#');
    if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Rgb::from_str_radix(digits, 16).ok()
}

/// What the user typed as a colour, written the way the server takes it: `#36f` or `3366ff`
/// become `#3366FF`. `None` if it isn't a colour.
#[must_use]
pub(crate) fn normalize_hex(text: &str) -> Option<String> {
    let digits = text.trim().trim_start_matches('#');
    let digits: String = if digits.len() == 3 {
        digits.chars().flat_map(|c| [c, c]).collect()
    } else {
        digits.to_owned()
    };
    parse_hex(&digits).map(|_| format!("#{}", digits.to_uppercase()))
}

/// `share` of `a` mixed into `b`, channel by channel.
#[must_use]
pub(crate) fn mix(a: Rgb, b: Rgb, share: f32) -> Rgb {
    let share = share.clamp(0.0, 1.0);
    let [_, ar, ag, ab] = a.to_be_bytes();
    let [_, br, bg, bb] = b.to_be_bytes();
    let channel = |x: u8, y: u8| to_byte(f32::from(x) * share + f32::from(y) * (1.0 - share));
    Rgb::from_be_bytes([0, channel(ar, br), channel(ag, bg), channel(ab, bb)])
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "rounded and clamped to 0..=255 first, so the cast is exact"
)]
pub(crate) fn to_byte(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

/// The WCAG 2 contrast ratio between two colours: 1 (the same) to 21 (black on white).
#[must_use]
pub(crate) fn contrast_ratio(a: Rgb, b: Rgb) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    let (light, dark) = if a > b { (a, b) } else { (b, a) };
    (light + 0.05) / (dark + 0.05)
}

/// A colour's relative luminance, as WCAG 2 defines it.
#[must_use]
pub(crate) fn luminance(colour: Rgb) -> f64 {
    let [_, r, g, b] = colour.to_be_bytes();
    let linear = |channel: u8| {
        let c = f64::from(channel) / 255.0;
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// The near-black text and icons take on a light fill.
pub(crate) const INK: Rgb = 0x0B_0D_10;

/// White.
pub(crate) const WHITE: Rgb = 0xFF_FF_FF;

/// White or near-black ([`INK`]), whichever reads better on `fill` by its own luminance: for
/// counts and icons on coloured fills that aren't the accent (which has its own text colour).
#[must_use]
pub(crate) fn readable_on(fill: Rgb) -> Rgb {
    if contrast_ratio(WHITE, fill) >= contrast_ratio(INK, fill) {
        WHITE
    } else {
        INK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_on_a_fill_is_white_or_ink_by_its_luminance() {
        assert_eq!(readable_on(0x00_00_00), WHITE);
        assert_eq!(readable_on(0x25_63_EB), WHITE, "the light accent");
        assert_eq!(readable_on(0xFF_CC_00), INK, "a yellow");
        assert_eq!(readable_on(0xFF_FF_FF), INK);
    }

    #[test]
    fn hex_colours_read_with_or_without_the_hash() {
        assert_eq!(parse_hex("#2563EB"), Some(0x25_63_EB));
        assert_eq!(parse_hex("ffffff"), Some(0xFF_FF_FF));
        assert_eq!(parse_hex("#12345"), None);
        assert_eq!(parse_hex("#GGGGGG"), None);
        assert_eq!(parse_hex("+12345"), None);
    }

    #[test]
    fn typed_colours_are_normalized() {
        assert_eq!(normalize_hex("#36f").as_deref(), Some("#3366FF"));
        assert_eq!(normalize_hex(" 3366ff ").as_deref(), Some("#3366FF"));
        assert_eq!(normalize_hex("blue"), None);
    }

    #[test]
    fn mixing_takes_a_share_of_each_channel() {
        assert_eq!(mix(0xFF_FF_FF, 0x00_00_00, 1.0), 0xFF_FF_FF);
        assert_eq!(mix(0xFF_FF_FF, 0x00_00_00, 0.0), 0x00_00_00);
        assert_eq!(mix(0xFF_00_00, 0x00_00_FF, 0.5), 0x80_00_80);
    }

    #[test]
    fn contrast_follows_wcag() {
        assert!((contrast_ratio(0x00_00_00, 0xFF_FF_FF) - 21.0).abs() < 1e-9);
        assert!((contrast_ratio(0x12_34_56, 0x12_34_56) - 1.0).abs() < 1e-9);
        // The built-in light accent on white: just above AA.
        let ratio = contrast_ratio(0x25_63_EB, 0xFF_FF_FF);
        assert!((5.1..5.3).contains(&ratio), "{ratio}");
    }
}
