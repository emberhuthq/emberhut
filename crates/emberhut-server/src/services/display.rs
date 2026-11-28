//! What the app shows: display switches the app reads from the state's `ui`, so a screen can be
//! hidden and brought back without a new build of the app. Only the screens hide: the goals
//! and projects themselves, and the assistant's use of them, stay.

use std::sync::Arc;

use emberhut_api::state::DisplayUi;
use emberhut_core::Clock;
use emberhut_db::{DbError, LogStore, SettingKey, SettingStore};
use serde_json::Value;

/// The setting the switches are kept under (`{ hideGoals, hideProjects }`).
const UI: SettingKey<DisplayUi> = SettingKey::new("ui");

/// The switches before any change: goals and projects hidden.
pub const DEFAULT_UI: DisplayUi = DisplayUi {
    hide_goals: true,
    hide_projects: true,
    task_plans: false,
};

/// Keeps the display switches.
#[derive(Clone, Debug)]
pub struct DisplayService {
    settings: SettingStore,
    log: LogStore,
    clock: Arc<dyn Clock>,
}

impl DisplayService {
    /// A service keeping the switches in `settings`, noting changes in `log`.
    #[must_use]
    pub fn new(settings: SettingStore, log: LogStore, clock: Arc<dyn Clock>) -> Self {
        Self {
            settings,
            log,
            clock,
        }
    }

    /// The switches: the ones last saved, or [`DEFAULT_UI`].
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn ui(&self) -> Result<DisplayUi, DbError> {
        Ok(self.settings.get(&UI).await?.unwrap_or(DEFAULT_UI))
    }

    /// Changes the switches `change` gives (`{ hideGoals?, hideProjects? }`); a switch left out,
    /// or not `true` or `false`, stays as it was. Saved, and noted in the log, either way.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn set_ui(&self, change: &Value) -> Result<DisplayUi, DbError> {
        let mut ui = self.ui().await?;
        if let Some(hide) = change.get("hideGoals").and_then(Value::as_bool) {
            ui.hide_goals = hide;
        }
        if let Some(hide) = change.get("hideProjects").and_then(Value::as_bool) {
            ui.hide_projects = hide;
        }
        self.settings.set(&UI, &ui).await?;
        let shown = |hidden: bool| if hidden { "hides" } else { "shows" };
        self.log
            .add(
                format!(
                    "The app now {} goals and {} projects",
                    shown(ui.hide_goals),
                    shown(ui.hide_projects)
                ),
                self.clock.now(),
            )
            .await?;
        Ok(ui)
    }
}
