//! The window's material: a touch of macOS translucency behind the sidebar, and nowhere else.
//!
//! On macOS the window blurs what is behind it (GPUI's `WindowBackgroundAppearance::Blurred`)
//! and the sidebar is filled a little translucent, so a hint of the blur shows through it. The
//! page, the report, the title bar (the system's own) and the composer stay opaque: the blur is
//! of what is behind the window, so a translucent strip inside an opaque page would show the
//! desktop through a hole rather than frost the chat. Elsewhere, when the system asks to reduce
//! transparency, or when the command line asks for the opaque look, every surface is opaque.
//!
//! This module decides which ([`Material::choose`]) and how translucent the sidebar may be for
//! its text to read ([`alpha`]); `system` reads Reduce Transparency. `ui::palette` holds the
//! sidebar's tokens, and `ui::look` puts both on the window.

pub(crate) mod alpha;
mod system;

pub(crate) use system::Changes;

/// The platform the app runs on, as far as the material cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Platform {
    /// macOS, whose window blur GPUI draws with the system's own visual effect view.
    MacOs,
    /// Linux and Windows: blur there depends on the compositor, so the app stays opaque.
    Other,
}

impl Platform {
    /// The platform this build runs on.
    pub(crate) const CURRENT: Self = if cfg!(target_os = "macos") {
        Self::MacOs
    } else {
        Self::Other
    };
}

/// What the system's Reduce Transparency setting says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Transparency {
    /// Translucency is fine.
    Allowed,
    /// The system asks apps to reduce transparency.
    Reduced,
}

impl Transparency {
    /// The system's setting now (always [`Transparency::Allowed`] off macOS).
    #[must_use]
    pub(crate) fn current() -> Self {
        if system::reduce_transparency() {
            Self::Reduced
        } else {
            Self::Allowed
        }
    }

    /// The setting's changes from now on.
    #[must_use]
    pub(crate) fn changes() -> Changes {
        system::watch()
    }
}

/// The look the command line asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Look {
    /// The material where the platform and the system allow it.
    #[default]
    Material,
    /// Every surface opaque (`--opaque`, the look before the material).
    Opaque,
}

/// Which surfaces have material.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Material {
    /// Every surface opaque: the look without material.
    #[default]
    Opaque,
    /// The window blurs what is behind it, and the sidebar lets a little of it through.
    Sidebar,
}

impl Material {
    /// The material for `platform`: the sidebar's on macOS, unless the system reduces
    /// transparency or the command line asks for the opaque look.
    #[must_use]
    pub(crate) fn choose(platform: Platform, transparency: Transparency, look: Look) -> Self {
        match (platform, transparency, look) {
            (Platform::MacOs, Transparency::Allowed, Look::Material) => Self::Sidebar,
            _ => Self::Opaque,
        }
    }

    /// Whether the window blurs what is behind it (it is opaque otherwise).
    #[must_use]
    pub(crate) fn blurs_window(self) -> bool {
        self == Self::Sidebar
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_gives_the_sidebar_material() {
        let material = Material::choose(Platform::MacOs, Transparency::Allowed, Look::Material);
        assert_eq!(material, Material::Sidebar);
        assert!(material.blurs_window());
    }

    #[test]
    fn reduce_transparency_keeps_every_surface_opaque() {
        let material = Material::choose(Platform::MacOs, Transparency::Reduced, Look::Material);
        assert_eq!(material, Material::Opaque);
        assert!(!material.blurs_window());
    }

    #[test]
    fn the_opaque_look_can_be_asked_for() {
        for transparency in [Transparency::Allowed, Transparency::Reduced] {
            assert_eq!(
                Material::choose(Platform::MacOs, transparency, Look::Opaque),
                Material::Opaque
            );
        }
    }

    #[test]
    fn linux_and_windows_stay_opaque_whatever_is_asked() {
        for transparency in [Transparency::Allowed, Transparency::Reduced] {
            for look in [Look::Material, Look::Opaque] {
                assert_eq!(
                    Material::choose(Platform::Other, transparency, look),
                    Material::Opaque,
                    "{transparency:?} {look:?}"
                );
            }
        }
    }
}
