//! How an environment's row looks: its icon (the server names a symbol of the reference
//! client's; each maps to the nearest of the icons built into the app) and its colour (a
//! colour's name, or the theme's accent), and a state's colour.

use emberhut_api::environments::{EnvState, Environment};
use gpui_kit::Hsla;
use gpui_kit::assets::IconName;

use crate::ui::palette::{Pal, category_color};

/// The icon for a symbol name the server sends.
#[must_use]
pub(crate) fn icon(symbol: &str) -> IconName {
    match symbol {
        "server.rack" | "desktopcomputer" => IconName::HardDrive,
        "laptopcomputer" => IconName::Frame,
        "gearshape.2.fill" | "gearshape.2" => IconName::Settings2,
        "globe" | "safari.fill" | "safari" => IconName::Globe,
        "building.2.fill" | "building.2" => IconName::Building2,
        "envelope.fill" | "envelope" => IconName::Inbox,
        "message.fill" | "message.badge.fill" | "bubble.left.and.bubble.right.fill" => {
            IconName::Bell
        }
        "number" => IconName::Network,
        "heart.fill" | "heart" => IconName::Heart,
        "calendar" => IconName::Calendar,
        "hourglass" => IconName::Eye,
        "person.2.circle.fill" | "person.crop.circle" => IconName::CircleUser,
        "cpu" => IconName::Cpu,
        _ => IconName::Settings,
    }
}

/// The colour a tint's name stands for: the theme's accent, a colour of the categorical set,
/// or grey.
#[must_use]
pub(crate) fn tint(name: &str, p: &Pal) -> Hsla {
    match name {
        "accent" => p.accent,
        "blue" => category_color(0),
        "indigo" => category_color(1),
        "green" => category_color(2),
        "orange" => category_color(3),
        "pink" => category_color(4),
        "teal" | "cyan" => category_color(5),
        "purple" => category_color(6),
        "red" => category_color(7),
        _ => p.faint,
    }
}

/// A row's icon and its colour, dimmed for one kept only while work moves off it.
#[must_use]
pub(crate) fn badge(env: &Environment, p: &Pal) -> (IconName, Hsla) {
    let color = tint(&env.tint, p);
    let color = if env.legacy == Some(true) {
        color.opacity(0.6)
    } else {
        color
    };
    (icon(&env.icon), color)
}

/// A state's colour: fine green, a look amber, down red, off muted.
#[must_use]
pub(crate) fn state_color(state: &EnvState, p: &Pal) -> Hsla {
    match state {
        EnvState::Ok => p.success,
        EnvState::Warn => p.warn,
        EnvState::Off => p.muted,
        // Down, and a state this app doesn't know yet.
        _ => p.danger,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_symbol_has_an_icon() {
        assert_eq!(icon("server.rack"), IconName::HardDrive);
        assert_eq!(icon("envelope.fill"), IconName::Inbox);
        assert_eq!(icon("something.new"), IconName::Settings);
    }
}
