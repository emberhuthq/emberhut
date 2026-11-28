//! The profile (`emberhut_core::memory::profile`): the few lines about the owner that every
//! session's prompt carries, and the slice of a project's facts its room's prompt carries too,
//! so the assistant knows the owner from its first word (ADR 0034). It is made from the facts
//! as a session starts, and kept in the context folder's `_profile.md` for the owner to read.

use std::sync::Arc;

use emberhut_core::Clock;
use emberhut_core::memory::profile::{self, PROFILE_TOKENS};
use emberhut_core::task::ProjectId;
use emberhut_db::ProjectStore;
use jiff::tz::TimeZone;

use super::facts::FactService;

/// Makes the profile.
#[derive(Clone, Debug)]
pub struct ProfileService {
    facts: FactService,
    projects: ProjectStore,
    clock: Arc<dyn Clock>,
    zone: TimeZone,
}

impl ProfileService {
    /// The profile of `facts`; a room's project is named from `projects`; the day is `clock`'s
    /// in `zone`.
    #[must_use]
    pub fn new(
        facts: FactService,
        projects: ProjectStore,
        clock: Arc<dyn Clock>,
        zone: TimeZone,
    ) -> Self {
        Self {
            facts,
            projects,
            clock,
            zone,
        }
    }

    /// Today, as the facts' `valid` ranges are written (`2026-11-28`).
    fn today(&self) -> String {
        self.clock
            .now()
            .to_zoned_datetime(self.zone.clone())
            .date()
            .to_string()
    }

    /// The profile now, written to `_profile.md` too when it changed. Empty: no fact fits.
    ///
    /// # Errors
    ///
    /// When the facts can't be read or the file written.
    pub async fn profile(&self) -> Result<String, super::facts::FactError> {
        let facts = self.facts.facts(None).await?;
        let made = profile::profile(&facts, &self.today());
        self.facts
            .write_profile(profile::file(&made, &self.today()))
            .await?;
        Ok(made)
    }

    /// What a session's prompt says about the owner in a room of `project` (none: a room of
    /// no project): the profile, then the project's slice.
    ///
    /// # Errors
    ///
    /// When the facts can't be read, or the project's name.
    pub async fn for_prompt(
        &self,
        project: Option<ProjectId>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let made = self.profile().await?;
        let mut out = if made.is_empty() {
            "About the user: nothing kept yet. Keep what will matter later with fact_set."
                .to_owned()
        } else {
            format!(
                "About the user (what you always know, made from your facts within \
                 {PROFILE_TOKENS} tokens; each line's fact id is in brackets, `facts` has the \
                 rest, and recall finds what was said):\n{made}"
            )
        };
        if let Some(project) = project
            && let Some(project) = self.projects.get(project).await?
        {
            let facts = self.facts.facts(None).await?;
            let slice = profile::slice(&facts, &project.name, &self.today());
            if !slice.is_empty() {
                out.push_str("\n\nThis room's project, ");
                out.push_str(&project.name);
                out.push_str(": the facts kept for it:\n");
                out.push_str(&slice);
            }
        }
        Ok(out)
    }
}
