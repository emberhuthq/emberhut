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

/// A colour in CIE L*a*b* (D65): lightness, then green to red, then blue to yellow.
#[cfg(test)]
pub(crate) type Lab = (f64, f64, f64);

/// `colour` in CIE L*a*b* (D65), from sRGB.
#[cfg(test)]
#[must_use]
#[expect(
    clippy::many_single_char_names,
    reason = "the formula's own names: red, green, blue and the X, Y, Z between"
)]
pub(crate) fn lab(colour: Rgb) -> Lab {
    let [_, r, g, b] = colour.to_be_bytes();
    let linear = |channel: u8| {
        let c = f64::from(channel) / 255.0;
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (linear(r), linear(g), linear(b));
    let x = (0.412_456_4 * r + 0.357_576_1 * g + 0.180_437_5 * b) / 0.950_47;
    let y = 0.212_672_9 * r + 0.715_152_2 * g + 0.072_175 * b;
    let z = (0.019_333_9 * r + 0.119_192 * g + 0.950_304_1 * b) / 1.088_83;
    let f = |t: f64| {
        if t > 216.0 / 24_389.0 {
            t.cbrt()
        } else {
            (24_389.0 / 27.0 * t + 16.0) / 116.0
        }
    };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    (116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz))
}

/// How far apart two colours look, as CIEDE2000 measures it (Sharma, Wu and Dalal, 2005):
/// about 1 is the smallest difference an eye can see between two large patches side by side.
#[cfg(test)]
#[must_use]
pub(crate) fn delta_e_2000((l1, a1, b1): Lab, (l2, a2, b2): Lab) -> f64 {
    use std::f64::consts::PI;
    let pow7 = |x: f64| x.powi(7);
    let degrees = |radians: f64| radians * 180.0 / PI;
    let radians = |degrees: f64| degrees * PI / 180.0;
    let c_bar = f64::midpoint(a1.hypot(b1), a2.hypot(b2));
    let g = 0.5 * (1.0 - (pow7(c_bar) / (pow7(c_bar) + pow7(25.0))).sqrt());
    let (a1, a2) = (a1 * (1.0 + g), a2 * (1.0 + g));
    let (c1, c2) = (a1.hypot(b1), a2.hypot(b2));
    let hue = |b: f64, a: f64| {
        if a == 0.0 && b == 0.0 {
            0.0
        } else {
            degrees(b.atan2(a)).rem_euclid(360.0)
        }
    };
    let (h1, h2) = (hue(b1, a1), hue(b2, a2));
    let chromatic = c1 * c2 != 0.0;
    let dh = match h2 - h1 {
        _ if !chromatic => 0.0,
        d if d > 180.0 => d - 360.0,
        d if d < -180.0 => d + 360.0,
        d => d,
    };
    let (dl, dc) = (l2 - l1, c2 - c1);
    let dh = 2.0 * (c1 * c2).sqrt() * (radians(dh) / 2.0).sin();
    let l_bar = f64::midpoint(l1, l2);
    let c_bar = f64::midpoint(c1, c2);
    let h_bar = if !chromatic {
        h1 + h2
    } else if (h1 - h2).abs() <= 180.0 {
        f64::midpoint(h1, h2)
    } else if h1 + h2 < 360.0 {
        (h1 + h2 + 360.0) / 2.0
    } else {
        (h1 + h2 - 360.0) / 2.0
    };
    let t = 1.0 - 0.17 * radians(h_bar - 30.0).cos()
        + 0.24 * radians(2.0 * h_bar).cos()
        + 0.32 * radians(3.0 * h_bar + 6.0).cos()
        - 0.20 * radians(4.0 * h_bar - 63.0).cos();
    let theta = 30.0 * (-((h_bar - 275.0) / 25.0).powi(2)).exp();
    let rc = 2.0 * (pow7(c_bar) / (pow7(c_bar) + pow7(25.0))).sqrt();
    let sl = 1.0 + 0.015 * (l_bar - 50.0).powi(2) / (20.0 + (l_bar - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * c_bar;
    let sh = 1.0 + 0.015 * c_bar * t;
    let rt = -(radians(2.0 * theta)).sin() * rc;
    ((dl / sl).powi(2) + (dc / sc).powi(2) + (dh / sh).powi(2) + rt * (dc / sc) * (dh / sh)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_e_2000_matches_its_published_test_data() {
        // Pairs 1, 7, 9, 11, 13, 15, 16, 17 and 25 of Sharma, Wu and Dalal's table; 9 to 16
        // sit either side of the hue's wrap at 180 and 360 degrees.
        let pairs = [
            ((50.0, 2.6772, -79.7751), (50.0, 0.0, -82.7485), 2.0425),
            ((50.0, 0.0, 0.0), (50.0, -1.0, 2.0), 2.3669),
            ((50.0, 2.49, -0.001), (50.0, -2.49, 0.0009), 7.1792),
            ((50.0, 2.49, -0.001), (50.0, -2.49, 0.0011), 7.2195),
            ((50.0, -0.001, 2.49), (50.0, 0.0009, -2.49), 4.8045),
            ((50.0, -0.001, 2.49), (50.0, 0.0011, -2.49), 4.7461),
            ((50.0, 2.5, 0.0), (50.0, 0.0, -2.5), 4.3065),
            ((50.0, 2.5, 0.0), (73.0, 25.0, -18.0), 27.1492),
            (
                (60.2574, -34.0099, 36.2677),
                (60.4626, -34.1751, 39.4387),
                1.2644,
            ),
        ];
        for (a, b, expected) in pairs {
            let found = delta_e_2000(a, b);
            assert!((found - expected).abs() < 1e-4, "{a:?} {b:?}: {found}");
            assert!((delta_e_2000(b, a) - expected).abs() < 1e-4, "symmetric");
        }
        let white = lab(WHITE);
        assert!((white.0 - 100.0).abs() < 1e-3 && white.1.abs() < 1e-2 && white.2.abs() < 1e-2);
    }

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
