//! The theme: one choice for the person, the same on every device (`GET`/`POST /api/theme`,
//! and the state's `theme`). The choice is a setting; its colours are worked out by
//! `emberhut-core`'s theme rules each time they are asked for.

use std::sync::Arc;

use emberhut_api::theme::{
    Appearance, Contrast, ContrastFacts, EmberTint, Preset, ThemeChoice, ThemePage, TokenPair,
    Tokens,
};
use emberhut_core::Clock;
use emberhut_core::theme::{self, Palette, ThemeRefusal};
use emberhut_db::{DbError, LogStore, SettingKey, SettingStore};
use serde_json::Value;

use crate::convert::timestamp_to_api;

/// The setting the choice is kept under (`{ preset, appearance, accent, emberTint, at }`).
const THEME: SettingKey<ThemeChoice> = SettingKey::new("theme");

/// Why a theme change was not made.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ThemeError {
    /// A value the theme does not take; the text is what the app shows.
    #[error(transparent)]
    Refused(#[from] ThemeRefusal),
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
}

/// Keeps the theme choice and works out its colours.
#[derive(Clone, Debug)]
pub struct ThemeService {
    settings: SettingStore,
    log: LogStore,
    clock: Arc<dyn Clock>,
    key: SettingKey<ThemeChoice>,
}

impl ThemeService {
    /// A service keeping the choice in `settings`, noting changes in `log`.
    #[must_use]
    pub fn new(settings: SettingStore, log: LogStore, clock: Arc<dyn Clock>) -> Self {
        Self {
            settings,
            log,
            clock,
            key: THEME,
        }
    }

    /// The same service for a family member's own choice (`theme:<person>`, the reference
    /// server's key), apart from the owner's.
    #[must_use]
    pub fn for_person(&self, person: &str) -> Self {
        Self {
            key: SettingKey::named(format!("theme:{person}")),
            ..self.clone()
        }
    }

    /// The choice: the one last saved, or the default (paper, as the system is, the ember in its
    /// own colours, never set). A saved choice names its preset, so changing the default leaves
    /// it as it was; one saved before tints reads as the default tint.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn choice(&self) -> Result<ThemeChoice, DbError> {
        Ok(self.settings.get(&self.key).await?.unwrap_or_default())
    }

    /// The choice's page: its colours and contrast, and every preset's colours.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn page(&self) -> Result<ThemePage, DbError> {
        Ok(page(&self.choice().await?))
    }

    /// Changes the fields `change` gives (`{ preset?, appearance?, accent?, emberTint? }`;
    /// `accent: null` or blank goes back to the preset's own) and keeps the rest; with
    /// `preview`, nothing is saved. A change is read leniently: a preset, an appearance or a
    /// tint in any case, with spaces around; `accent` as `#RGB` or `#RRGGBB`, the `#` optional.
    ///
    /// # Errors
    ///
    /// [`ThemeError::Refused`] for a body that is not an object or a value the theme does not
    /// take (nothing is changed); [`ThemeError::Database`].
    pub async fn change(&self, change: &Value, preview: bool) -> Result<ThemeChoice, ThemeError> {
        let mut choice = self.choice().await?;
        let Value::Object(change) = change else {
            return Err(ThemeRefusal::NotAChange.into());
        };
        if let Some(preset) = change.get("preset") {
            theme::preset_id(preset.as_str().unwrap_or_default())?.clone_into(&mut choice.preset);
        }
        if let Some(appearance) = change.get("appearance") {
            choice.appearance = Appearance::from(
                theme::appearance(appearance.as_str().unwrap_or_default())?.as_str(),
            );
        }
        if let Some(accent) = change.get("accent") {
            choice.accent = match accent {
                Value::Null => None,
                Value::String(text) => theme::accent(Some(text))?,
                _ => return Err(ThemeRefusal::Accent.into()),
            };
        }
        if let Some(tint) = change.get("emberTint") {
            theme::tint_id(tint.as_str().unwrap_or_default())?.clone_into(&mut choice.ember_tint);
        }
        if preview {
            return Ok(choice);
        }
        let now = self.clock.now();
        choice.at = Some(timestamp_to_api(now));
        self.settings.set(&self.key, &choice).await?;
        let accent = choice
            .accent
            .as_deref()
            .map_or_else(String::new, |accent| format!(" with {accent}"));
        let tint = if choice.ember_tint == theme::DEFAULT_TINT {
            String::new()
        } else {
            format!(", the ember in {}", choice.ember_tint)
        };
        self.log
            .add(
                format!(
                    "The theme is now {}{accent}, {}{tint}",
                    choice.preset,
                    choice.appearance.as_str()
                ),
                now,
            )
            .await?;
        Ok(choice)
    }
}

/// The page of `choice`: as `GET /api/theme` answers it.
#[must_use]
pub fn page(choice: &ThemeChoice) -> ThemePage {
    let accent = choice.accent.as_deref();
    let (light, light_contrast) = theme::resolve(&choice.preset, accent, false);
    let (dark, dark_contrast) = theme::resolve(&choice.preset, accent, true);
    let aa = light_contrast.is_aa() && dark_contrast.is_aa();
    ThemePage {
        version: theme::VERSION,
        choice: choice.clone(),
        tokens: TokenPair {
            light: tokens(light),
            dark: tokens(dark),
        },
        contrast: Contrast {
            light: contrast(light_contrast),
            dark: contrast(dark_contrast),
            aa,
        },
        presets: theme::PRESETS
            .iter()
            .map(|preset| Preset {
                id: preset.id.to_owned(),
                name: preset.name.to_owned(),
                light: tokens(theme::resolve(preset.id, None, false).0),
                dark: tokens(theme::resolve(preset.id, None, true).0),
            })
            .collect(),
        names: theme::NAMES.iter().map(|name| (*name).to_owned()).collect(),
        tints: theme::TINTS
            .iter()
            .map(|tint| EmberTint {
                id: tint.id.to_owned(),
                name: tint.name.to_owned(),
            })
            .collect(),
    }
}

fn tokens(palette: Palette) -> Tokens {
    let Palette {
        accent,
        accent_soft,
        on_accent,
        background,
        surface,
        surface_raised,
        border,
        text,
        muted,
        faint,
        success,
        warn,
        danger,
        info,
    } = palette;
    Tokens {
        accent,
        accent_soft,
        on_accent,
        background,
        surface,
        surface_raised,
        border,
        text,
        muted,
        faint,
        success,
        warn,
        danger,
        info,
    }
}

fn contrast(facts: theme::Contrast) -> ContrastFacts {
    ContrastFacts {
        accent_on_background: facts.accent_on_background,
        accent_on_surface: facts.accent_on_surface,
        on_accent: facts.on_accent,
        adjusted: facts.adjusted,
        asked: facts.asked,
    }
}
