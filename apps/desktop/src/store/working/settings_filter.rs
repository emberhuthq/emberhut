//! Settings' filter: one part of the page (MCP servers, Skills, Data sources, Where the
//! assistant works) and one state (Needs a look, Off, Fine). It narrows those parts' rows; the
//! rest of the page shows only with no filter on. Kept on this device
//! (`settings-filter.json`), so the page opens as it was left.

use std::path::PathBuf;
use std::sync::Mutex;

use emberhut_api::switches::{McpServer, McpState, Skill};
use gpui_kit::{AppContext as _, Context};
use serde::{Deserialize, Serialize};

use crate::brand::APP_NAME;
use crate::config::AppFiles;
use crate::json_file;
use crate::store::Store;
use crate::store::settings::chrome::Health;

/// A part of Settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SettingsPart {
    /// The MCP servers.
    Mcp,
    /// The skills.
    Skills,
    /// The data sources.
    Sources,
    /// Where the assistant works: the machines, runners, workers, its browser, external threads.
    Machines,
}

impl SettingsPart {
    /// Every part, in the page's order.
    pub(crate) const ALL: [Self; 4] = [Self::Mcp, Self::Skills, Self::Sources, Self::Machines];

    /// Its name.
    #[must_use]
    pub(crate) fn name(self) -> String {
        match self {
            Self::Mcp => "MCP servers".to_owned(),
            Self::Skills => "Skills".to_owned(),
            Self::Sources => "Data sources".to_owned(),
            Self::Machines => format!("Where {APP_NAME} works"),
        }
    }
}

/// How a row is doing, as the filter picks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum HealthPick {
    /// It wants a look, or is down.
    Problem,
    /// It is off.
    Off,
    /// It is fine.
    Fine,
}

impl HealthPick {
    /// Every pick, in the bar's order.
    pub(crate) const ALL: [Self; 3] = [Self::Problem, Self::Off, Self::Fine];

    /// Its name.
    #[must_use]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Problem => "Needs a look",
            Self::Off => "Off",
            Self::Fine => "Fine",
        }
    }

    /// The pick a row's health falls under.
    #[must_use]
    pub(crate) fn of(health: Health) -> Self {
        match health {
            Health::Fine => Self::Fine,
            Health::Off => Self::Off,
            Health::Warn | Health::Down => Self::Problem,
        }
    }
}

/// An MCP server's health: connected is fine, off here or there is off, the rest want a look.
#[must_use]
pub(crate) fn server_health(server: &McpServer) -> Health {
    match server.state {
        McpState::Connected => Health::Fine,
        McpState::Off | McpState::Disabled => Health::Off,
        _ => Health::Warn,
    }
}

/// A skill's health: on is fine, else off.
#[must_use]
pub(crate) fn skill_health(skill: &Skill) -> Health {
    if skill.on { Health::Fine } else { Health::Off }
}

/// The filter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SettingsFilter {
    /// The part shown alone.
    #[serde(default)]
    pub(crate) part: Option<SettingsPart>,
    /// The state its rows are in.
    #[serde(default)]
    pub(crate) health: Option<HealthPick>,
}

impl SettingsFilter {
    /// Whether no filter is on: the whole page shows.
    #[must_use]
    pub(crate) fn is_empty(self) -> bool {
        self.part.is_none() && self.health.is_none()
    }

    /// How many filters are on.
    #[must_use]
    pub(crate) fn count(self) -> usize {
        usize::from(self.part.is_some()) + usize::from(self.health.is_some())
    }

    /// Whether `part` shows.
    #[must_use]
    pub(crate) fn shows(self, part: SettingsPart) -> bool {
        self.part.is_none_or(|shown| shown == part)
    }

    /// Whether a row in `health` shows.
    #[must_use]
    pub(crate) fn keeps(self, health: Health) -> bool {
        self.health
            .is_none_or(|pick| pick == HealthPick::of(health))
    }
}

/// Where the filter is kept.
fn kept_path(files: &AppFiles) -> PathBuf {
    files.dir().join("settings-filter.json")
}

/// Settings' last filter on this device: none when there is no file, or it doesn't read (then
/// it is logged, and the next pick writes a good one).
#[must_use]
pub(crate) fn load(files: Option<&AppFiles>) -> SettingsFilter {
    files
        .and_then(|files| {
            json_file::read(
                &kept_path(files),
                "the Settings filter",
                "showing every part",
            )
        })
        .unwrap_or_default()
}

/// Keeps `filter` on this device as write `number` of the file (skipped once a newer one is
/// written: `written` holds the last one's number), whole through a temporary file; a failure
/// is logged (the pick still holds for this launch). It touches the disk: call it off the UI
/// thread.
fn save(files: &AppFiles, filter: SettingsFilter, number: u64, written: &Mutex<u64>) {
    json_file::write(
        &kept_path(files),
        &filter,
        number,
        written,
        "the Settings filter",
    );
}

impl Store {
    /// Settings' filter.
    #[must_use]
    pub(crate) fn settings_filter(&self) -> SettingsFilter {
        self.working.settings_filter
    }

    /// Sets Settings' filter, and keeps it on this device.
    pub(crate) fn set_settings_filter(
        &mut self,
        filter: SettingsFilter,
        cx: &mut Context<'_, Self>,
    ) {
        if self.working.settings_filter == filter {
            return;
        }
        self.working.settings_filter = filter;
        // Off the UI thread, numbered: a newer pick's write always wins.
        if let Some(files) = self.files.clone() {
            let (number, written) = self.working.settings_filter_writes.next();
            cx.background_spawn(async move { save(&files, filter, number, &written) })
                .detach();
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filter_narrows_its_part_and_state() {
        let all = SettingsFilter::default();
        assert!(all.is_empty() && all.shows(SettingsPart::Mcp) && all.keeps(Health::Down));
        let sources = SettingsFilter {
            part: Some(SettingsPart::Sources),
            health: Some(HealthPick::Problem),
        };
        assert_eq!(sources.count(), 2);
        assert!(sources.shows(SettingsPart::Sources));
        assert!(!sources.shows(SettingsPart::Skills));
        assert!(sources.keeps(Health::Warn) && sources.keeps(Health::Down));
        assert!(!sources.keeps(Health::Fine));
        assert_eq!(SettingsPart::Machines.name(), "Where Emberhut works");
    }

    #[test]
    fn the_filter_is_kept_across_launches() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(dir.path());
        assert_eq!(load(Some(&files)), SettingsFilter::default());
        let filter = SettingsFilter {
            part: Some(SettingsPart::Machines),
            health: None,
        };
        save(&files, filter, 2, &Mutex::new(0));
        assert_eq!(load(Some(&files)), filter);
        save(&files, SettingsFilter::default(), 1, &Mutex::new(2));
        assert_eq!(
            load(Some(&files)),
            filter,
            "an older write never replaces a newer one"
        );
        std::fs::write(kept_path(&files), "not json").unwrap();
        assert_eq!(load(Some(&files)), SettingsFilter::default());
    }
}
