//! Where the assistant works and what it uses: the environments (the places it works and the
//! connections it reads through), the Claude accounts' use and the account it runs on, the
//! background jobs on the machines. Each submodule hides one of them; their words are pure
//! functions, tested on their own.

pub(crate) mod environments;
pub(in crate::store) mod follow;
pub(crate) mod jobs;
pub(in crate::store) mod keys;
pub(crate) mod usage;

use gpui_kit::Context;

use self::environments::EnvironmentsFeed;
use self::jobs::JobsFeed;
use self::usage::UsageCard;
use super::settings::SettingsPane;
use super::{Page, Store};

/// What the store holds of them; nothing fetched at first.
#[derive(Debug, Default)]
pub(crate) struct WorkingData {
    envs: EnvironmentsFeed,
    jobs: JobsFeed,
    usage: UsageCard,
}

impl Store {
    /// Starts their loops (once each): the environments', the jobs' and the usage checks'.
    pub(super) fn start_working(&mut self, cx: &mut Context<'_, Self>) {
        self.start_environments(cx);
        self.start_jobs(cx);
        self.start_usage_checks(cx);
    }

    /// A column shows `page` afresh: what it shows is fetched now.
    pub(super) fn working_page_shown(&self, page: Option<Page>) {
        match page {
            Some(
                Page::Settings
                | Page::SettingsPane(SettingsPane::Connections | SettingsPane::Advanced)
                | Page::Environment(_),
            ) => self.working.envs.kick(),
            Some(Page::Jobs | Page::Job(_)) => self.working.jobs.kick(),
            _ => {}
        }
    }

    /// The window came to the front: what shows is fetched at once, and the accounts' use is
    /// checked.
    pub(super) fn working_forward(&self) {
        self.working.envs.kick();
        self.working.jobs.kick();
        self.working.usage.kick();
    }
}

#[cfg(test)]
mod tests;
