//! A project's status, set by the person: `POST /api/settings` with `{ projectSet }`.
//!
//! A project's room follows its status in the switcher ([`crate::rooms::Switcher`]): paused, it
//! folds away until its day; archived, it leaves the switcher and its open tasks are parked;
//! active again, it comes back (an archived project's tasks with it).
//!
//! The 200 answer is `{ ok: true }` with the whole state spread in ([`crate::error::OkReply`]
//! reads the `ok`); a project that doesn't exist is refused with 400 and an
//! [`crate::error::ErrorBody`].

use serde::{Deserialize, Serialize};

use crate::ids::ProjectId;
use crate::tasks::ProjectStatus;
use crate::time::Day;

/// `{ projectSet: { id, status?, pausedUntil? } }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct ProjectSetRequest {
    /// What changes.
    #[cfg_attr(feature = "validate", garde(dive))]
    pub project_set: ProjectChange,
}

/// What changes on a project: only the fields given are touched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct ProjectChange {
    /// The project.
    #[cfg_attr(feature = "validate", garde(dive))]
    pub id: ProjectId,
    /// Its new status. Any status but `paused` clears the day it was paused until.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub status: Option<ProjectStatus>,
    /// With `paused`: the day it comes back by itself (`YYYY-MM-DD`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub paused_until: Option<Day>,
}

impl ProjectSetRequest {
    /// Pauses `project` until `day`: its room folds away and comes back by itself then.
    #[must_use]
    pub fn pause_until(project: ProjectId, day: Day) -> Self {
        Self::status(project, ProjectStatus::Paused, Some(day))
    }

    /// Makes `project` active again (resumed, or unarchived).
    #[must_use]
    pub fn activate(project: ProjectId) -> Self {
        Self::status(project, ProjectStatus::Active, None)
    }

    /// Archives `project`: its room leaves the switcher.
    #[must_use]
    pub fn archive(project: ProjectId) -> Self {
        Self::status(project, ProjectStatus::Archived, None)
    }

    fn status(project: ProjectId, status: ProjectStatus, paused_until: Option<Day>) -> Self {
        Self {
            project_set: ProjectChange {
                id: project,
                status: Some(status),
                paused_until,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_change_sets_its_status() {
        let project = ProjectId::new(2);
        let pause = ProjectSetRequest::pause_until(project, Day::new("2026-12-01"));
        assert_eq!(pause.project_set.status, Some(ProjectStatus::Paused));
        assert_eq!(
            pause.project_set.paused_until.as_ref().map(Day::as_str),
            Some("2026-12-01")
        );
        let active = ProjectSetRequest::activate(project);
        assert_eq!(active.project_set.status, Some(ProjectStatus::Active));
        assert_eq!(active.project_set.paused_until, None);
        assert_eq!(
            ProjectSetRequest::archive(project).project_set.status,
            Some(ProjectStatus::Archived)
        );
    }
}
