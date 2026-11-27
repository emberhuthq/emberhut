//! What Party draws its people with: each persona's colour, a round initial, the cast as
//! overlapping initials, and a status's dot.

use emberhut_api::party::{PartyStatus, Persona};
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::prelude::*;
use gpui_kit::{Div, FontWeight, Hsla, div, px};

use crate::store::party::words::{self, Tone};
use crate::ui::palette::{Pal, category_color};

/// A persona's colour, by the name the server gave it: blue, orange, green, purple or pink as
/// on the other apps (categorical, not the theme's); any other is the faint grey.
#[must_use]
pub(crate) fn persona_color(name: &str, p: &Pal) -> Hsla {
    // The places of these colours among the categorical ones.
    match name {
        "blue" => category_color(0),
        "green" => category_color(2),
        "orange" => category_color(3),
        "pink" => category_color(4),
        "purple" => category_color(6),
        _ => p.faint,
    }
}

/// A status's colour.
#[must_use]
pub(crate) fn status_color(status: &PartyStatus, p: &Pal) -> Hsla {
    match words::tone(status) {
        Tone::Live => p.success,
        Tone::Waiting => p.warn,
        Tone::Still => p.muted,
    }
}

/// `name`'s first letter on a round of `color`, `size` across.
pub(crate) fn initial(name: &str, color: Hsla, size: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(size))
        .rounded_full()
        .bg(color)
        .text_color(Pal::readable_on(color))
        .text_size(px(size * 0.48))
        .font_weight(FontWeight::BOLD)
        .child(words::initial(name))
}

/// The cast as overlapping initials, each ringed in the page's colour.
pub(crate) fn cast_dots(personas: &[Persona], p: &Pal) -> Div {
    const SIZE: f32 = 22.;
    let mut row = div().flex().flex_none();
    for (i, persona) in personas.iter().enumerate() {
        let dot = initial(&persona.name, persona_color(&persona.color, p), SIZE)
            .border_2()
            .border_color(p.surface);
        row = row.child(if i == 0 { dot } else { dot.ml(px(-SIZE * 0.3)) });
    }
    row
}

/// A Lucide icon in a colour and size.
pub(crate) fn glyph(name: IconName, color: Hsla, size: f32) -> Icon {
    Icon::new(name).size(px(size)).text_color(color)
}
