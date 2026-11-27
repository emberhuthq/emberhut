//! `theme`: the apps' colour theme, read or changed by the assistant.

use serde::Deserialize;
use serde_json::{Map, Value};

use super::{Tool, ToolContext, ToolError, ToolOutput};
use crate::services::theme::ThemeError;

/// Reads the theme choice, or changes the parts given.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Theme;

/// `theme`'s arguments: each one given changes, the rest stay; none given reads the choice.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct ThemeArgs {
    /// The preset (accent family): graphite, blue, teal, green, orange, rose or purple.
    #[garde(length(max = 32))]
    preset: Option<String>,
    /// Light, dark, or as the system is.
    #[garde(skip)]
    appearance: Option<ThemeAppearance>,
    /// A custom accent colour (`#3366FF`, `#36F`) in place of the preset's own; an empty
    /// string goes back to the preset's own.
    #[garde(length(max = 16))]
    accent: Option<String>,
}

/// An appearance, as the tool takes it.
#[derive(Clone, Copy, Debug, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
enum ThemeAppearance {
    /// As the system is.
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

impl ThemeAppearance {
    fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

impl Tool for Theme {
    type Args = ThemeArgs;

    fn name(&self) -> &'static str {
        "theme"
    }

    fn description(&self) -> &'static str {
        "The apps' colour theme, the same on every device of the user's. Without arguments: \
         the current choice (preset, appearance, custom accent) and whether its accent reads \
         well (WCAG AA). With preset, appearance or accent: changes those, keeps the rest, and \
         every app follows at once. A custom accent that reads badly is darkened or lightened \
         until it reads; the answer says so."
    }

    async fn call(&self, ctx: &ToolContext, args: ThemeArgs) -> Result<ToolOutput, ToolError> {
        let theme = ctx.app.theme();
        let mut change = Map::new();
        if let Some(preset) = args.preset {
            change.insert("preset".to_owned(), Value::from(preset));
        }
        if let Some(appearance) = args.appearance {
            change.insert("appearance".to_owned(), Value::from(appearance.as_str()));
        }
        if let Some(accent) = args.accent {
            change.insert("accent".to_owned(), Value::from(accent));
        }
        let choice = if change.is_empty() {
            theme.choice().await.map_err(ToolError::failed)?
        } else {
            theme
                .change(&Value::Object(change), false)
                .await
                .map_err(|error| match error {
                    ThemeError::Refused(refusal) => ToolError::refused(refusal.to_string()),
                    other => ToolError::failed(other),
                })?
        };
        let page = crate::services::theme::page(&choice);
        ToolOutput::json(&serde_json::json!({
            "choice": page.choice,
            "accent": { "light": page.tokens.light.accent, "dark": page.tokens.dark.accent },
            "adjusted": page.contrast.light.adjusted || page.contrast.dark.adjusted,
            "aa": page.contrast.aa,
        }))
    }
}
