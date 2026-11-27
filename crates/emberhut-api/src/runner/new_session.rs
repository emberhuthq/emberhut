//! A session of the person's own (`POST /api/runner/sessions`): on a machine they pick, in a
//! folder (optionally its own git worktree), on an account, model and effort, opened with their
//! first message. For its sheet, a machine's folders (`GET /api/runner/folders?machine=&path=`)
//! and a repository's branches (`GET /api/runner/branches?machine=&cwd=`). A refusal is 409 (not
//! opened), 400 (no such runner) or 502 (the runner didn't answer), with an
//! [`ErrorBody`](crate::error::ErrorBody).

use serde::{Deserialize, Serialize};

use crate::chat::ChatFile;
use crate::ids::{MachineId, SessionId, TurnId};
use crate::query::push_encoded;
use crate::runner::actions::Effort;
use crate::time::Timestamp;

/// `POST /api/runner/sessions`: the person's new session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OpenSession {
    /// The runner it runs on.
    pub machine: MachineId,
    /// The folder it works in.
    pub cwd: String,
    /// The first message.
    pub text: String,
    /// Files stored with `POST /api/uploads`, sent with it.
    #[serde(default)]
    pub files: Vec<ChatFile>,
    /// The Claude account, by id; none: the one with the most room.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The effort.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,
    /// Its own git worktree; none: it works in the folder itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<Worktree>,
    /// Its title; none: the runner makes one from the first message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// A new session's own git worktree.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Worktree {
    /// The branch: an existing one, or the name of a new one; none: a new branch named by the
    /// runner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

/// `POST /api/runner/sessions`'s answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OpenedSession {
    /// Always `true`: a refusal is an `ErrorBody`.
    pub ok: bool,
    /// Its id.
    pub id: SessionId,
    /// Its first turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<TurnId>,
    /// The session as the runner opened it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<OpenedHead>,
}

/// What the runner says of a session it just opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OpenedHead {
    /// Its id.
    pub id: SessionId,
    /// The account it runs on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The folder it works in (its worktree, when it has one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Its worktree's branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The model it runs on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Which folders of a machine to list: where to start (`path` none: the person's projects and
/// recent folders), or the folders inside one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderQuery {
    /// The runner.
    pub machine: MachineId,
    /// The folder to look inside.
    pub path: Option<String>,
}

impl FolderQuery {
    /// The query string, without the `?`.
    ///
    /// ```
    /// use emberhut_api::runner::new_session::FolderQuery;
    ///
    /// let inside = FolderQuery { machine: "runner".into(), path: Some("~/My work".into()) };
    /// assert_eq!(inside.to_query(), "machine=runner&path=~%2FMy%20work");
    /// ```
    #[must_use]
    pub fn to_query(&self) -> String {
        let mut query = String::from("machine=");
        push_encoded(&mut query, self.machine.as_str());
        if let Some(path) = &self.path {
            query.push_str("&path=");
            push_encoded(&mut query, path);
        }
        query
    }
}

/// `GET /api/runner/folders`' answer: where to start (the person's projects, recent folders and
/// Documents), or one folder's folders.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FolderList {
    /// The folder listed; none for where to start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// That folder is a git repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<bool>,
    /// The home folder on that machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
    /// The Documents folder, to browse from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documents: Option<String>,
    /// The folders inside `path`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<Folder>,
    /// Where sessions started lately, newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent: Vec<Folder>,
    /// The person's projects' folders on that machine.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<Folder>,
}

/// One folder of a [`FolderList`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Folder {
    /// Its path.
    pub path: String,
    /// Its name (a project's name for a project's folder).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// It is a git repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<bool>,
    /// When a session last started there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used: Option<Timestamp>,
}

impl Folder {
    /// Its name, else the last part of its path.
    #[must_use]
    pub fn label(&self) -> &str {
        self.name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| {
                self.path
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or(&self.path)
            })
    }
}

/// Which repository's branches to list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchQuery {
    /// The runner.
    pub machine: MachineId,
    /// A folder inside the repository.
    pub cwd: String,
}

impl BranchQuery {
    /// The query string, without the `?`.
    #[must_use]
    pub fn to_query(&self) -> String {
        let mut query = String::from("machine=");
        push_encoded(&mut query, self.machine.as_str());
        query.push_str("&cwd=");
        push_encoded(&mut query, &self.cwd);
        query
    }
}

/// `GET /api/runner/branches`' answer: a repository's branches, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BranchList {
    /// The repository's top folder.
    pub repo: String,
    /// The branch it is on.
    pub current: String,
    /// Its branches, local and the remote's, newest first.
    #[serde(default)]
    pub branches: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_without_a_name_goes_by_its_last_part() {
        let folder = Folder {
            path: "/work/example/".into(),
            name: None,
            git: None,
            used: None,
        };
        assert_eq!(folder.label(), "example");
        let named = Folder {
            name: Some("Example".into()),
            ..folder
        };
        assert_eq!(named.label(), "Example");
    }

    #[test]
    fn a_branch_query_encodes_its_folder() {
        let query = BranchQuery {
            machine: MachineId::from("runner-b"),
            cwd: "/work/a b".into(),
        };
        assert_eq!(query.to_query(), "machine=runner-b&cwd=%2Fwork%2Fa%20b");
    }
}
