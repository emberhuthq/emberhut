//! The user's Chrome profiles: the state carries them and their pairing links; here are the
//! changes (`POST /api/chrome`) and the sheet open over the list (a profile, adding one, a
//! link to pair). A change's answer holds the profiles as they are after: they replace the
//! state's at once, and the poll is kicked for the rest.

use std::sync::Arc;

use gpui_kit::AppContext as _;
use gpui_kit::Context;
use jiff::tz::TimeZone;

use super::chrome_install::{ChromeInstaller, InstallState, Installer};
use super::words;
use crate::backend::Error;
use crate::brand::APP_NAME;
use crate::store::{Store, update_or_log};
use emberhut_api::chrome::{ChromeAction, ChromeGrant, ChromePairing, ChromeProfile, ChromeReply};
use emberhut_api::ids::{ChromeCode, ChromeProfileId, TaskId};
use emberhut_api::state::State;
use emberhut_api::tasks::Issue;

/// The names offered for a new profile, minus the ones already taken.
pub(crate) const USUAL_NAMES: [&str; 4] = ["Work", "Personal", "Side project", "Shopping"];

/// What is open over the profiles' list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ChromeSheet {
    /// A paired profile's page.
    Profile(ChromeProfileId),
    /// Naming a new profile, to get its link.
    Add,
    /// A link to open in a profile, and its pairing as it happens.
    Pairing(ChromePairing),
}

/// The Chrome changes on their way and what they left.
#[derive(Debug, Default)]
pub(crate) struct ChromeFeed {
    /// Adding the extension to Chrome (`chrome_install`): written only on the user's OK.
    install: InstallState,
    /// What writes Chrome's entry; this computer's when `None`.
    installer: Option<Arc<dyn Installer>>,
    sheet: Option<ChromeSheet>,
    /// The profile a re-pair's link was asked from: closing the link goes back to its page.
    back_to: Option<ChromeProfileId>,
    /// A change on its way.
    busy: bool,
    problem: Option<String>,
    /// Counts the renames the server refused, so a name field can go back to the name kept.
    refused_renames: u64,
}

impl ChromeFeed {
    /// What is open over the list.
    #[must_use]
    pub(crate) fn sheet(&self) -> Option<&ChromeSheet> {
        self.sheet.as_ref()
    }

    /// Where adding the extension to Chrome is.
    #[must_use]
    pub(crate) fn install(&self) -> &InstallState {
        &self.install
    }

    /// Whether a change is on its way.
    #[must_use]
    pub(crate) fn is_busy(&self) -> bool {
        self.busy
    }

    /// Why the last change failed, worded for the user.
    #[must_use]
    pub(crate) fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// How many renames the server refused so far.
    #[must_use]
    pub(crate) fn refused_renames(&self) -> u64 {
        self.refused_renames
    }

    /// Takes a change's answer in; the profiles it carries, to put in the state.
    fn take(
        &mut self,
        action: &ChromeAction,
        reply: Result<ChromeReply, Error>,
    ) -> Option<ChromeReply> {
        self.busy = false;
        match reply {
            Ok(mut reply) => {
                self.problem = None;
                if let Some(pairing) = reply.pairing.take() {
                    self.back_to = match action {
                        ChromeAction::Repair { id } => Some(*id),
                        _ => None,
                    };
                    self.sheet = Some(ChromeSheet::Pairing(pairing));
                } else if matches!(action, ChromeAction::Unpair { .. }) {
                    self.sheet = None;
                }
                Some(reply)
            }
            Err(error) => {
                if matches!(action, ChromeAction::Rename { .. }) {
                    self.refused_renames += 1;
                }
                self.problem = Some(match &error {
                    Error::Unreachable(_) => format!("Can't reach {APP_NAME}"),
                    other => other
                        .server_message()
                        .map_or_else(|| other.to_string(), str::to_owned),
                });
                None
            }
        }
    }
}

/// How the profiles' row in Settings reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProfilesSummary {
    /// `None paired`, `1 of 2 online`.
    pub(crate) status: String,
    /// The profiles' names.
    pub(crate) detail: String,
    /// How it is doing.
    pub(crate) health: Health,
}

/// How a row is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Health {
    /// All well.
    Fine,
    /// Something wants a look.
    Warn,
    /// Something is down (a machine low or offline).
    Down,
    /// Nothing there, or turned off.
    Off,
}

/// The profiles' row in Settings.
#[must_use]
pub(crate) fn summary(profiles: &[ChromeProfile]) -> ProfilesSummary {
    let online = profiles.iter().filter(|profile| profile.connected).count();
    let status = if profiles.is_empty() {
        "None paired".to_owned()
    } else {
        format!("{online} of {} online", profiles.len())
    };
    let health = if profiles.is_empty() {
        Health::Off
    } else if online == profiles.len() {
        Health::Fine
    } else {
        Health::Warn
    };
    ProfilesSummary {
        status,
        detail: profiles
            .iter()
            .map(|profile| profile.label.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        health,
    }
}

/// The links still waiting for a new profile: not used yet, and not a re-pair.
#[must_use]
pub(crate) fn waiting(pairings: &[ChromePairing]) -> Vec<&ChromePairing> {
    pairings
        .iter()
        .filter(|pairing| !pairing.paired && pairing.device.is_none())
        .collect()
}

/// The names offered for a new profile: the usual ones nobody took yet (in any case).
#[must_use]
pub(crate) fn offered_names(profiles: &[ChromeProfile]) -> Vec<&'static str> {
    USUAL_NAMES
        .into_iter()
        .filter(|name| {
            !profiles
                .iter()
                .any(|profile| profile.label.eq_ignore_ascii_case(name))
        })
        .collect()
}

/// A task as the user knows it: its key when the state has it, else its number.
#[must_use]
pub(crate) fn task_name(task: TaskId, issues: &[Issue]) -> String {
    issues
        .iter()
        .find(|issue| issue.id == task)
        .map_or_else(|| format!("task {task}"), |issue| issue.key.clone())
}

/// `Emberhut may work here until 15:40 · for <task>`.
#[must_use]
pub(crate) fn grant_line(grant: &ChromeGrant, issues: &[Issue], zone: &TimeZone) -> String {
    let until = grant
        .until
        .as_ref()
        .and_then(|until| words::clock(until.as_str(), zone))
        .map_or_else(|| "any time".to_owned(), |clock| format!("until {clock}"));
    let task = grant
        .task
        .map_or_default(|task| format!(" · for {}", task_name(task, issues)));
    format!("{APP_NAME} may work here {until}{task}")
}

/// A machine's name as it reads: one with spaces stays, `example-laptop` reads `example laptop`.
#[must_use]
pub(crate) fn machine_name(machine: &str) -> String {
    if machine.contains(' ') {
        machine.to_owned()
    } else {
        machine.replace('-', " ")
    }
}

/// The profile's badge colour, steady for its name: an index into a palette of `colours`.
#[must_use]
pub(crate) fn badge_index(name: &str, colours: usize) -> usize {
    let sum = name
        .chars()
        .fold(0_u64, |sum, c| sum.wrapping_add(u64::from(u32::from(c))));
    u64::try_from(colours)
        .ok()
        .and_then(|colours| sum.checked_rem(colours))
        .and_then(|index| usize::try_from(index).ok())
        .unwrap_or_default()
}

/// The profile's initial, as its badge shows it.
#[must_use]
pub(crate) fn initial(label: &str) -> String {
    label
        .chars()
        .next()
        .map_or_default(|c| c.to_uppercase().collect())
}

impl Store {
    /// The Chrome changes and the sheet open over the list.
    #[must_use]
    pub(crate) fn chrome(&self) -> &ChromeFeed {
        &self.settings.chrome
    }

    /// The paired profiles, from the last state.
    #[must_use]
    pub(crate) fn chrome_profiles(&self) -> &[ChromeProfile] {
        self.state
            .as_ref()
            .map_or(&[], |state| state.chrome.as_slice())
    }

    /// A paired profile, by its id.
    #[must_use]
    pub(crate) fn chrome_profile(&self, id: ChromeProfileId) -> Option<&ChromeProfile> {
        self.chrome_profiles()
            .iter()
            .find(|profile| profile.id == id)
    }

    /// Opens a sheet over the list (`None` closes it).
    pub(crate) fn show_chrome_sheet(
        &mut self,
        sheet: Option<ChromeSheet>,
        cx: &mut Context<'_, Self>,
    ) {
        let feed = &mut self.settings.chrome;
        feed.sheet = sheet;
        feed.back_to = None;
        feed.problem = None;
        cx.notify();
    }

    /// Closes the open sheet (Done, Cancel, Esc): a re-pair's link goes back to its profile's
    /// page, where it was asked for; any other sheet shuts.
    pub(crate) fn close_chrome_sheet(&mut self, cx: &mut Context<'_, Self>) {
        let feed = &mut self.settings.chrome;
        let back = feed
            .back_to
            .take()
            .filter(|_| matches!(feed.sheet, Some(ChromeSheet::Pairing(_))));
        self.show_chrome_sheet(back.map(ChromeSheet::Profile), cx);
    }

    /// The user agreed to add the extension to Chrome: Chrome's entry for it is written, off the
    /// UI thread (on Linux the system asks for the user's password), once at a time. What comes
    /// next is the link to pair the profile, whose page waits for the extension.
    pub(crate) fn install_chrome_extension(&mut self, cx: &mut Context<'_, Self>) {
        let feed = &mut self.settings.chrome;
        if feed.install == InstallState::Writing {
            return;
        }
        feed.install = InstallState::Writing;
        let installer = feed
            .installer
            .clone()
            .unwrap_or_else(|| Arc::new(ChromeInstaller::here()));
        cx.spawn(async move |this, cx| {
            let written = cx
                .background_spawn(async move { installer.install() })
                .await;
            update_or_log(&this, cx, |store, cx| {
                store.settings.chrome.install = match written {
                    Ok(written) => InstallState::Written(written),
                    Err(why) => InstallState::Failed(why),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Writes Chrome's entry with `installer` instead of this computer's (the tests).
    #[cfg(test)]
    pub(crate) fn set_chrome_installer(&mut self, installer: Arc<dyn Installer>) {
        self.settings.chrome.installer = Some(installer);
    }

    /// Asks for a link for a new profile named `label`.
    pub(crate) fn add_chrome(&mut self, label: &str, cx: &mut Context<'_, Self>) {
        let label = label.trim();
        if !label.is_empty() {
            self.chrome_act(
                ChromeAction::Add {
                    label: label.to_owned(),
                },
                cx,
            );
        }
    }

    /// Cancels a link not used yet.
    pub(crate) fn cancel_chrome_link(&mut self, code: ChromeCode, cx: &mut Context<'_, Self>) {
        self.chrome_act(ChromeAction::Cancel { code }, cx);
    }

    /// Sends a Chrome change (one at a time).
    pub(crate) fn chrome_act(&mut self, action: ChromeAction, cx: &mut Context<'_, Self>) {
        let feed = &mut self.settings.chrome;
        if feed.busy {
            return;
        }
        feed.busy = true;
        feed.problem = None;
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let sent = action.clone();
            let reply = cx
                .background_spawn(async move { backend.chrome(&sent) })
                .await;
            update_or_log(&this, cx, |store, cx| {
                if let Some(reply) = store.settings.chrome.take(&action, reply) {
                    if let Some(state) = &mut store.state {
                        put_chrome(state, reply);
                    }
                    store.kicker.kick();
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

/// A change's answer into the state: the profiles and links as they are now.
fn put_chrome(state: &mut State, reply: ChromeReply) {
    state.chrome = reply.chrome;
    state.chrome_pairing = reply.chrome_pairing;
    state.chrome_latest = reply.chrome_latest;
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn profile(id: i64, label: &str, connected: bool) -> ChromeProfile {
        serde_json::from_value(
            json!({"id": id, "name": label.to_lowercase(), "label": label,
            "connected": connected, "created": "2026-11-01T09:00:00Z"}),
        )
        .unwrap()
    }

    fn pairing(paired: bool, device: Option<i64>) -> ChromePairing {
        serde_json::from_value(
            json!({"code": "AB-12", "link": "https://example.com/p#AB-12",
            "label": "Work", "name": "work", "device": device, "until": "2026-11-24T22:30:00Z",
            "paired": paired}),
        )
        .unwrap()
    }

    fn reply(pairing: Option<ChromePairing>) -> ChromeReply {
        ChromeReply {
            ok: true,
            chrome: vec![profile(1, "Work", true)],
            chrome_pairing: Vec::new(),
            chrome_latest: Some("1.0.0".into()),
            pairing,
        }
    }

    #[test]
    fn the_settings_row_counts_the_profiles_online() {
        assert_eq!(summary(&[]).status, "None paired");
        assert_eq!(summary(&[]).health, Health::Off);
        let two = [profile(1, "Work", true), profile(2, "Personal", false)];
        let row = summary(&two);
        assert_eq!(
            (row.status.as_str(), row.detail.as_str(), row.health),
            ("1 of 2 online", "Work, Personal", Health::Warn)
        );
        assert_eq!(summary(&two[..1]).health, Health::Fine);
    }

    #[test]
    fn only_new_unused_links_wait() {
        let links = [
            pairing(false, None),
            pairing(true, None),
            pairing(false, Some(2)),
        ];
        assert_eq!(waiting(&links).len(), 1);
    }

    #[test]
    fn taken_names_are_not_offered() {
        assert_eq!(
            offered_names(&[profile(1, "work", true)]),
            ["Personal", "Side project", "Shopping"]
        );
    }

    #[test]
    fn a_grant_says_until_when_and_for_what() {
        let grant = ChromeGrant {
            until: None,
            task: None,
            by: "paired".into(),
        };
        assert_eq!(
            grant_line(&grant, &[], &TimeZone::UTC),
            "Emberhut may work here any time"
        );
        let grant = ChromeGrant {
            until: Some(emberhut_api::time::Timestamp::new("2026-11-24T15:40:00Z")),
            task: Some(TaskId::new(12)),
            by: "paired".into(),
        };
        assert_eq!(
            grant_line(&grant, &[], &TimeZone::UTC),
            "Emberhut may work here until 15:40 · for task 12"
        );
    }

    #[test]
    fn names_badges_and_machines_read_plainly() {
        assert_eq!(machine_name("example-laptop"), "example laptop");
        assert_eq!(machine_name("Example Laptop"), "Example Laptop");
        assert_eq!(initial("work"), "W");
        assert_eq!(initial(""), "");
        assert_eq!(badge_index("work", 8), badge_index("work", 8));
        assert!(badge_index("work", 8) < 8);
        assert_eq!(badge_index("work", 0), 0);
    }

    #[test]
    fn a_new_link_opens_the_pairing_and_an_unpair_closes_the_sheet() {
        let mut feed = ChromeFeed {
            sheet: Some(ChromeSheet::Add),
            busy: true,
            ..ChromeFeed::default()
        };
        let add = ChromeAction::Add {
            label: "Work".into(),
        };
        let kept = feed.take(&add, Ok(reply(Some(pairing(false, None)))));
        assert!(kept.is_some() && !feed.is_busy());
        assert!(matches!(feed.sheet(), Some(ChromeSheet::Pairing(_))));
        feed.sheet = Some(ChromeSheet::Profile(ChromeProfileId::new(1)));
        let unpair = ChromeAction::Unpair {
            id: ChromeProfileId::new(1),
        };
        feed.take(&unpair, Ok(reply(None)));
        assert_eq!(feed.sheet(), None);
    }

    #[test]
    fn a_refused_rename_is_counted_and_said() {
        let mut feed = ChromeFeed::default();
        let rename = ChromeAction::Rename {
            id: ChromeProfileId::new(1),
            label: "Work".into(),
        };
        let refused = Error::Refused {
            status: Some(409),
            message: Some("That name is taken".into()),
        };
        assert!(feed.take(&rename, Err(refused)).is_none());
        assert_eq!(feed.refused_renames(), 1);
        assert_eq!(feed.problem(), Some("That name is taken"));
    }

    #[test]
    fn the_answer_replaces_the_states_profiles() {
        let mut state = crate::fixtures::state(json!({}));
        put_chrome(&mut state, reply(None));
        assert_eq!(state.chrome.len(), 1);
        assert_eq!(state.chrome_latest.as_deref(), Some("1.0.0"));
    }
}
