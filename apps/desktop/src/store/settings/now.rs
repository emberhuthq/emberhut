//! What Settings' front door and group pages say about how things stand, gathered from the
//! store in one place: on duty, what the agent thinks with, what it knows, the appearance, the
//! devices, the connections that need a look. Search's live lines, the suggestions and the
//! pages' status lines are pure functions of it, so they are tested without a window.

use emberhut_api::environments::EnvGroupId;
use emberhut_api::switches::McpState;
use emberhut_api::theme::Appearance;

use super::about::listed_count;
use super::appearance::summary as theme_summary;
use super::chrome::{self, Health};
use crate::store::credentials::saved_line;
use crate::store::working::environments::health;
use crate::store::{AgentsPlace, Store};

/// The computer the app runs on, in the words of "as your Mac is".
#[must_use]
pub(crate) fn computer() -> &'static str {
    if cfg!(target_os = "macos") {
        "your Mac"
    } else {
        "your computer"
    }
}

/// How the things Settings speaks of stand now.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent facts about how things stand, one each"
)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Now {
    /// Whether the agent is on duty.
    pub(crate) on_duty: bool,
    /// What it is doing, as the sidebar says it.
    pub(crate) doing: String,
    /// Whether it is at work on something now.
    pub(crate) working: bool,
    /// How many background jobs it is running.
    pub(crate) jobs: usize,
    /// What it thinks with (`DeepSeek · key ending 0000`), once read.
    pub(crate) provider: Option<String>,
    /// The models it runs on, once read.
    pub(crate) models: Option<String>,
    /// How many facts it keeps about the user, once read.
    pub(crate) facts: Option<usize>,
    /// How many lessons it is trying out, waiting on the user.
    pub(crate) lessons: usize,
    /// The appearance chosen.
    pub(crate) appearance: Option<Appearance>,
    /// The theme in a few words: `Paper · System`.
    pub(crate) theme: String,
    /// Whether the window shows dark now.
    pub(crate) dark: bool,
    /// How many devices are signed in, when the server says.
    pub(crate) devices: Option<usize>,
    /// Whether one of them is a phone.
    pub(crate) phone_paired: bool,
    /// Whether this device has a session of its own (it can pair others).
    pub(crate) signed_in: bool,
    /// The Chrome profiles' line: `1 of 2 online · Personal, Work`.
    pub(crate) chrome: String,
    /// The Chrome profiles that aren't online.
    pub(crate) chrome_offline: Vec<String>,
    /// The tool servers that can't connect.
    pub(crate) failing_tools: Vec<String>,
    /// The data sources that need a look.
    pub(crate) sources_to_fix: Vec<String>,
    /// How many data sources are working.
    pub(crate) sources_on: usize,
    /// How many rooms are archived.
    pub(crate) archived: usize,
    /// The app's version line: `Emberhut 0.9.2 (build 12)`.
    pub(crate) version: String,
    /// The server's address.
    pub(crate) server: String,
    /// Whether phone push is on at the server.
    pub(crate) push: bool,
    /// Whether this build has the emberhut.com account.
    pub(crate) has_account: bool,
    /// Whether the sidebar lists the agents.
    pub(crate) agents_shown: bool,
    /// Where the sidebar lists them.
    pub(crate) agents_place: AgentsPlace,
}

impl Now {
    /// How many things on Connections need a look: Chrome profiles, tool servers, sources.
    #[must_use]
    pub(crate) fn needs_look(&self) -> usize {
        self.chrome_offline.len() + self.failing_tools.len() + self.sources_to_fix.len()
    }

    /// What the agent is doing, in the words of Your ember's head: `Ready`, `Working on 2
    /// things`, `Off duty`. The On duty switch says whether it is on duty; this doesn't repeat it.
    #[must_use]
    pub(crate) fn doing_word(&self) -> String {
        if !self.on_duty {
            return "Off duty".to_owned();
        }
        match self.jobs.max(usize::from(self.working)) {
            0 => "Ready".to_owned(),
            1 => "Working on 1 thing".to_owned(),
            count => format!("Working on {count} things"),
        }
    }

    /// The appearance chosen, in a word: `Light`, `Dark`, `As your Mac is`.
    #[must_use]
    pub(crate) fn appearance_word(&self) -> String {
        match self.appearance {
            Some(Appearance::Light) => "Light".to_owned(),
            Some(Appearance::Dark) => "Dark".to_owned(),
            _ => format!("As {} is", computer()),
        }
    }
}

/// Whether a device's model is a phone's (`iPhone 16`, `Pixel 8`, `Android phone`).
#[must_use]
pub(crate) fn is_phone(model: &str) -> bool {
    let model = model.to_lowercase();
    ["iphone", "android", "pixel", "galaxy", "phone"]
        .iter()
        .any(|word| model.contains(word))
}

impl Store {
    /// How Settings' things stand now; `dark` is whether the window shows dark, `has_account`
    /// whether this build has the emberhut.com account.
    #[must_use]
    pub(crate) fn settings_now(&self, dark: bool, has_account: bool) -> Now {
        let state = self.state();
        let auth = state.and_then(|state| state.auth.as_ref());
        let saved = self.credentials().saved();
        let facts = self.facts().page();
        let profiles = self.chrome_profiles();
        let summary = chrome::summary(profiles);
        let chrome = if summary.detail.is_empty() {
            summary.status
        } else {
            format!("{} · {}", summary.status, summary.detail)
        };
        let (mut sources_on, mut sources_to_fix) = (0, Vec::new());
        if let Some(page) = self.environments().page() {
            for env in page
                .rows()
                .filter(|env| env.group == EnvGroupId::Connections)
            {
                match health(&env.state) {
                    Health::Fine => sources_on += 1,
                    Health::Warn | Health::Down => sources_to_fix.push(env.name.clone()),
                    Health::Off => {}
                }
            }
        }
        let failing_tools = self.switches().page().map_or_else(Vec::new, |page| {
            page.mcp
                .iter()
                .filter(|server| server.on && server.state == McpState::Failed)
                .map(|server| server.name.clone())
                .collect()
        });
        Now {
            on_duty: self.is_on_duty(),
            doing: self.status().text,
            working: self.is_working(),
            jobs: state.map_or(0, |state| {
                state.tasks.iter().filter(|task| task.is_running()).count()
            }),
            provider: saved.map(saved_line),
            models: saved.and_then(|saved| {
                let model = saved.model.as_deref()?;
                Some(match saved.fast_model.as_deref() {
                    Some(fast) => format!("{model} · side rooms: {fast}"),
                    None => model.to_owned(),
                })
            }),
            facts: facts.map(listed_count),
            lessons: facts.map_or(0, |page| page.trials.len()),
            appearance: Some(self.theme().page().choice.appearance.clone()),
            theme: theme_summary(self.theme().page()),
            dark,
            devices: auth.map(|auth| auth.devices.len()),
            phone_paired: auth.is_some_and(|auth| {
                auth.devices
                    .iter()
                    .any(|device| device.model.as_deref().is_some_and(is_phone))
            }),
            signed_in: self.is_signed_in(),
            chrome,
            chrome_offline: profiles
                .iter()
                .filter(|profile| !profile.connected)
                .map(|profile| profile.label.clone())
                .collect(),
            failing_tools,
            sources_to_fix,
            sources_on,
            archived: self.switcher().archived().len(),
            version: format!(
                "{} {}",
                crate::brand::APP_NAME,
                self.updates().rows(std::time::SystemTime::now()).version
            ),
            server: self.server().to_string(),
            push: state.is_some_and(|state| state.push),
            has_account,
            agents_shown: self.agents_shown(),
            agents_place: self.agents_place(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phone_is_told_by_its_model() {
        assert!(is_phone("iPhone 16 Pro"));
        assert!(is_phone("Pixel 8"));
        assert!(!is_phone("MacBookPro18,1"));
    }

    #[test]
    fn your_embers_head_says_what_it_is_doing() {
        let ready = Now {
            on_duty: true,
            ..Now::default()
        };
        assert_eq!(ready.doing_word(), "Ready");
        let busy = Now {
            working: true,
            ..ready.clone()
        };
        assert_eq!(busy.doing_word(), "Working on 1 thing");
        let jobs = Now { jobs: 3, ..busy };
        assert_eq!(jobs.doing_word(), "Working on 3 things");
        assert_eq!(
            Now {
                on_duty: false,
                ..jobs
            }
            .doing_word(),
            "Off duty"
        );
    }

    #[test]
    fn connections_count_what_needs_a_look() {
        let now = Now {
            chrome_offline: vec!["Work".into()],
            failing_tools: vec!["linear".into()],
            ..Now::default()
        };
        assert_eq!(now.needs_look(), 2);
    }
}
