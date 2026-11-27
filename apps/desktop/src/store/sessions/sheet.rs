//! A new session of the user's own: the sheet's choices (machine, folder, its own git worktree
//! on a new or an existing branch, the Claude account, model and effort), the folders it offers
//! (the user's projects' and recent ones as one-tap picks, a browser into the rest) and the
//! branches of the folder's repository; then the draft page, where the first message opens it.
//! The machine, model and effort are kept for next time.

use std::sync::Arc;

use emberhut_api::chat::ChatFile;
use emberhut_api::ids::{MachineId, SessionId};
use emberhut_api::runner::accounts::{ClaudeAccount, LINUX_RUNNER};
use emberhut_api::runner::actions::Effort;
use emberhut_api::runner::new_session::{
    BranchList, BranchQuery, Folder, FolderList, FolderQuery, OpenSession, Worktree,
};
use emberhut_api::state::State;
use gpui_kit::{AppContext as _, Context};
use jiff::tz::TimeZone;

use super::kept::SheetChoices;
use super::words::{machine_name, model_name, short_path};
use crate::backend::Backend;
use crate::store::gate::reason;

/// How many one-tap folders the sheet offers.
const QUICK_FOLDERS: usize = 6;

/// The machines a new session may go on: the runners the state's accounts are on, the runner an
/// account with no machine is on first.
#[must_use]
pub(crate) fn machines(state: Option<&State>) -> Vec<String> {
    let mut found: Vec<String> = state
        .into_iter()
        .flat_map(|state| &state.accounts)
        .map(|account| {
            account
                .machine
                .as_ref()
                .map_or(LINUX_RUNNER, MachineId::as_str)
                .to_owned()
        })
        .collect();
    found.push(LINUX_RUNNER.to_owned());
    found.sort_by_key(|machine| (machine != LINUX_RUNNER, machine.clone()));
    found.dedup();
    found
}

/// Where and on what a new session runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Setup {
    /// The runner.
    pub(crate) machine: MachineId,
    /// The folder it works in.
    pub(crate) cwd: String,
    /// It gets its own git worktree.
    pub(crate) worktree: bool,
    /// The worktree is on a branch that exists (else a new one).
    pub(crate) existing: bool,
    /// The branch: the existing one picked, or the new one's name (optional).
    pub(crate) branch: String,
    /// The account, by id; none: the one with the most room.
    pub(crate) account: Option<String>,
    /// The model's id.
    pub(crate) model: String,
    /// The effort.
    pub(crate) effort: Effort,
}

impl Setup {
    /// A setup starting on the kept choices.
    #[must_use]
    pub(crate) fn from_choices(choices: &SheetChoices) -> Self {
        Self {
            machine: MachineId::from(choices.machine.as_str()),
            cwd: String::new(),
            worktree: false,
            existing: false,
            branch: String::new(),
            account: None,
            model: choices.model.clone(),
            effort: Effort::from(choices.effort.as_str()),
        }
    }

    /// The choices to keep from it.
    #[must_use]
    pub(crate) fn choices(&self) -> SheetChoices {
        SheetChoices {
            machine: self.machine.to_string(),
            model: self.model.clone(),
            effort: self.effort.as_str().to_owned(),
        }
    }

    /// Whether Open may be pressed: a folder is picked, and a branch when it is an existing
    /// one.
    #[must_use]
    pub(crate) fn can_open(&self) -> bool {
        let needs_branch = self.worktree && self.existing && self.branch.trim().is_empty();
        !self.cwd.trim().is_empty() && !needs_branch
    }

    /// The request that opens it with the first message.
    #[must_use]
    pub(crate) fn request(&self, text: &str, files: Vec<ChatFile>) -> OpenSession {
        let branch = self.branch.trim();
        OpenSession {
            machine: self.machine.clone(),
            cwd: self.cwd.clone(),
            text: text.trim().to_owned(),
            files,
            account: self.account.clone(),
            model: Some(self.model.clone()),
            effort: Some(self.effort.clone()),
            worktree: self.worktree.then(|| Worktree {
                branch: (!branch.is_empty()).then(|| branch.to_owned()),
            }),
            title: None,
        }
    }

    /// Where: `Mac · ~/work · worktree on fix`.
    #[must_use]
    pub(crate) fn where_line(&self) -> String {
        let mut parts = vec![
            machine_name(self.machine.as_str()).to_owned(),
            short_path(&self.cwd),
        ];
        if self.worktree {
            let branch = self.branch.trim();
            parts.push(if branch.is_empty() {
                "new worktree".to_owned()
            } else {
                format!("worktree on {branch}")
            });
        }
        parts.join(" · ")
    }

    /// On what: `Account with most room · Opus 5.5 · High`.
    #[must_use]
    pub(crate) fn what_line(&self, state: Option<&State>) -> String {
        let account = self.account.as_deref().map_or_else(
            || "Account with most room".to_owned(),
            |id| {
                state
                    .and_then(|state| state.accounts.iter().find(|a| a.id == id))
                    .map_or_else(|| id.to_owned(), |a| a.name.clone())
            },
        );
        format!(
            "{account} · {} · {}",
            model_name(&self.model),
            self.effort.name()
        )
    }
}

/// The one-tap folders: the user's projects' first, then where sessions started lately, once
/// each, at most six, without the one picked.
#[must_use]
pub(crate) fn quick_folders<'a>(list: &'a FolderList, picked: &str) -> Vec<&'a Folder> {
    let mut seen: Vec<&str> = Vec::new();
    list.projects
        .iter()
        .chain(&list.recent)
        .filter(|folder| {
            let new = !seen.contains(&folder.path.as_str());
            seen.push(&folder.path);
            new
        })
        .take(QUICK_FOLDERS)
        .filter(|folder| folder.path != picked)
        .collect()
}

/// The folder a machine's sheet starts on: its most recent, else its first project's.
#[must_use]
pub(crate) fn first_folder(list: &FolderList) -> Option<&str> {
    list.recent
        .first()
        .or_else(|| list.projects.first())
        .map(|folder| folder.path.as_str())
}

/// The accounts a session on `machine` may run on.
#[must_use]
pub(crate) fn accounts_for<'a>(state: &'a State, machine: &MachineId) -> Vec<&'a ClaudeAccount> {
    state
        .accounts
        .iter()
        .filter(|account| account.is_on(machine) && account.can_be_picked())
        .collect()
}

/// An account with its room: `Work · 25% week · 40% 5 h`, `out until 18:00`, `not logged in`.
#[must_use]
pub(crate) fn room_line(account: &ClaudeAccount, zone: &TimeZone) -> String {
    let percent = |used: Option<f64>| format!("{}%", crate::format::whole(used.unwrap_or(0.)));
    let out = account
        .limited_until
        .as_ref()
        .and_then(crate::store::tasks::clock::instant)
        .map(|until| until.to_zoned_datetime(zone.clone()));
    let room = if account.logged_in == Some(false) {
        "not logged in".to_owned()
    } else if let Some(until) = out {
        format!("out until {}", until.strftime("%H:%M"))
    } else {
        format!(
            "{} week · {} 5 h",
            percent(account.used_7d),
            percent(account.used_5h)
        )
    };
    format!("{} · {room}", account.name)
}

/// Where a folder listing stands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Listing {
    /// Asked for.
    #[default]
    Loading,
    /// In hand.
    Ready(FolderList),
    /// The machine's runner didn't answer.
    Failed,
}

/// The new-session sheet, as a GPUI entity: its choices, and what it fetched for them.
pub(crate) struct SessionSheet {
    backend: Arc<dyn Backend>,
    setup: Setup,
    /// Where to start on the machine.
    roots: Listing,
    /// The folders being browsed, outermost first: each path and its listing.
    browsing: Vec<(String, Listing)>,
    branches: Option<BranchList>,
    /// Bumped on every change of machine or folder: an older answer is dropped.
    asked: u64,
}

impl SessionSheet {
    /// A sheet starting on `choices`, fetching where to start on their machine.
    pub(crate) fn new(
        backend: Arc<dyn Backend>,
        choices: &SheetChoices,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let mut sheet = Self {
            backend,
            setup: Setup::from_choices(choices),
            roots: Listing::Loading,
            browsing: Vec::new(),
            branches: None,
            asked: 0,
        };
        sheet.fetch_roots(cx);
        sheet
    }

    /// The choices so far.
    #[must_use]
    pub(crate) fn setup(&self) -> &Setup {
        &self.setup
    }

    /// Where to start on the machine.
    #[must_use]
    pub(crate) fn roots(&self) -> &Listing {
        &self.roots
    }

    /// The folder being browsed and its listing, if the browser is open.
    #[must_use]
    pub(crate) fn browsing(&self) -> Option<&(String, Listing)> {
        self.browsing.last()
    }

    /// Whether the browser is open (at its first level when nothing is browsed into yet).
    #[must_use]
    pub(crate) fn browse_depth(&self) -> usize {
        self.browsing.len()
    }

    /// The repository's branches, for a worktree on an existing one.
    #[must_use]
    pub(crate) fn branches(&self) -> Option<&BranchList> {
        self.branches.as_ref()
    }

    /// Picks the machine: the folder, branch and account start over.
    pub(crate) fn set_machine(&mut self, machine: &str, cx: &mut Context<'_, Self>) {
        if self.setup.machine.as_str() == machine {
            return;
        }
        self.setup.machine = MachineId::from(machine);
        self.setup.cwd.clear();
        self.setup.branch.clear();
        self.setup.account = None;
        self.browsing.clear();
        self.branches = None;
        self.fetch_roots(cx);
        cx.notify();
    }

    /// Picks the folder (the browser closes).
    pub(crate) fn set_cwd(&mut self, cwd: &str, cx: &mut Context<'_, Self>) {
        cwd.trim().clone_into(&mut self.setup.cwd);
        self.browsing.clear();
        self.fetch_branches(cx);
        cx.notify();
    }

    /// Turns the worktree on or off.
    pub(crate) fn set_worktree(&mut self, on: bool, cx: &mut Context<'_, Self>) {
        self.setup.worktree = on;
        self.fetch_branches(cx);
        cx.notify();
    }

    /// A new branch, or one that exists (the branch starts over).
    pub(crate) fn set_existing(&mut self, existing: bool, cx: &mut Context<'_, Self>) {
        if self.setup.existing != existing {
            self.setup.existing = existing;
            self.setup.branch.clear();
            cx.notify();
        }
    }

    /// The branch: picked, or typed.
    pub(crate) fn set_branch(&mut self, branch: &str, cx: &mut Context<'_, Self>) {
        branch.clone_into(&mut self.setup.branch);
        cx.notify();
    }

    /// The account (none: the one with the most room).
    pub(crate) fn set_account(&mut self, account: Option<String>, cx: &mut Context<'_, Self>) {
        self.setup.account = account;
        cx.notify();
    }

    /// The model.
    pub(crate) fn set_model(&mut self, model: &str, cx: &mut Context<'_, Self>) {
        model.clone_into(&mut self.setup.model);
        cx.notify();
    }

    /// The effort.
    pub(crate) fn set_effort(&mut self, effort: Effort, cx: &mut Context<'_, Self>) {
        self.setup.effort = effort;
        cx.notify();
    }

    /// Opens the browser into `path` (none: the first level, the projects and recent ones).
    pub(crate) fn browse(&mut self, path: Option<String>, cx: &mut Context<'_, Self>) {
        let Some(path) = path else {
            self.browsing.clear();
            self.browsing.push((String::new(), self.roots.clone()));
            cx.notify();
            return;
        };
        self.browsing.push((path.clone(), Listing::Loading));
        let depth = self.browsing.len();
        let query = FolderQuery {
            machine: self.setup.machine.clone(),
            path: Some(path),
        };
        let (backend, asked) = (Arc::clone(&self.backend), self.asked);
        cx.spawn(async move |this, cx| {
            let answer = cx
                .background_spawn(async move { backend.runner_folders(&query) })
                .await;
            this.update(cx, |sheet, cx| {
                if sheet.asked != asked {
                    return;
                }
                if let Some((_, listing)) = sheet.browsing.get_mut(depth - 1) {
                    *listing = answer.map_or(Listing::Failed, Listing::Ready);
                    cx.notify();
                }
            })
            .inspect_err(|error| log::debug!("the sheet closed: {error}"))
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Back out of the folder being browsed (from the first level, the browser closes).
    pub(crate) fn browse_back(&mut self, cx: &mut Context<'_, Self>) {
        self.browsing.pop();
        cx.notify();
    }

    fn fetch_roots(&mut self, cx: &mut Context<'_, Self>) {
        self.asked += 1;
        self.roots = Listing::Loading;
        let query = FolderQuery {
            machine: self.setup.machine.clone(),
            path: None,
        };
        let (backend, asked) = (Arc::clone(&self.backend), self.asked);
        cx.spawn(async move |this, cx| {
            let answer = cx
                .background_spawn(async move { backend.runner_folders(&query) })
                .await;
            this.update(cx, |sheet, cx| {
                if sheet.asked != asked {
                    return;
                }
                sheet.roots = match answer {
                    Ok(list) => {
                        if sheet.setup.cwd.is_empty()
                            && let Some(first) = first_folder(&list)
                        {
                            first.clone_into(&mut sheet.setup.cwd);
                        }
                        Listing::Ready(list)
                    }
                    Err(error) => {
                        log::info!("no folders from the runner: {}", reason(&error));
                        Listing::Failed
                    }
                };
                sheet.fetch_branches(cx);
                cx.notify();
            })
            .inspect_err(|error| log::debug!("the sheet closed: {error}"))
            .ok();
        })
        .detach();
    }

    fn fetch_branches(&mut self, cx: &mut Context<'_, Self>) {
        self.branches = None;
        if !self.setup.worktree || self.setup.cwd.is_empty() {
            return;
        }
        let query = BranchQuery {
            machine: self.setup.machine.clone(),
            cwd: self.setup.cwd.clone(),
        };
        let backend = Arc::clone(&self.backend);
        let want = (self.setup.machine.clone(), self.setup.cwd.clone());
        cx.spawn(async move |this, cx| {
            let answer = cx
                .background_spawn(async move { backend.runner_branches(&query) })
                .await;
            this.update(cx, |sheet, cx| {
                if (&sheet.setup.machine, &sheet.setup.cwd) != (&want.0, &want.1) {
                    return;
                }
                sheet.branches = answer
                    .inspect_err(|error| log::info!("no branches: {}", reason(error)))
                    .ok();
                cx.notify();
            })
            .inspect_err(|error| log::debug!("the sheet closed: {error}"))
            .ok();
        })
        .detach();
    }
}

/// A session about to open, as a GPUI entity: its setup, and the first message that opens it.
pub(crate) struct DraftSession {
    backend: Arc<dyn Backend>,
    setup: Setup,
    opening: bool,
    problem: Option<String>,
    /// The words and files that didn't open it, for the box to have them back.
    unsent: Option<(String, Vec<ChatFile>)>,
    opened: Option<SessionId>,
}

impl DraftSession {
    /// A draft of `setup`.
    #[must_use]
    pub(crate) fn new(backend: Arc<dyn Backend>, setup: Setup) -> Self {
        Self {
            backend,
            setup,
            opening: false,
            problem: None,
            unsent: None,
            opened: None,
        }
    }

    /// Its setup.
    #[must_use]
    pub(crate) fn setup(&self) -> &Setup {
        &self.setup
    }

    /// Whether the first message is on its way.
    #[must_use]
    pub(crate) fn is_opening(&self) -> bool {
        self.opening
    }

    /// Why it didn't open.
    #[must_use]
    pub(crate) fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// The session it opened.
    #[must_use]
    pub(crate) fn opened(&self) -> Option<&SessionId> {
        self.opened.as_ref()
    }

    /// The words and files that didn't open it, taken once.
    pub(crate) fn take_unsent(&mut self) -> Option<(String, Vec<ChatFile>)> {
        self.unsent.take()
    }

    /// Opens it with the first message (and files stored already); nothing while it is being
    /// opened or once it is. Whether it took the message.
    pub(crate) fn open(
        &mut self,
        text: &str,
        files: Vec<ChatFile>,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if self.opening || self.opened.is_some() || (text.trim().is_empty() && files.is_empty()) {
            return false;
        }
        let words = (text.to_owned(), files.clone());
        let request = self.setup.request(text, files);
        self.opening = true;
        self.problem = None;
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let answer = cx
                .background_spawn(async move { backend.open_session(&request) })
                .await;
            this.update(cx, |draft, cx| {
                draft.opening = false;
                match answer {
                    Ok(opened) => draft.opened = Some(opened.id),
                    Err(error) => {
                        draft.problem = Some(format!("Not opened: {}", reason(&error)));
                        draft.unsent = Some(words);
                    }
                }
                cx.notify();
            })
            .inspect_err(|error| log::debug!("the draft closed: {error}"))
            .ok();
        })
        .detach();
        cx.notify();
        true
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn setup() -> Setup {
        Setup::from_choices(&SheetChoices::default())
    }

    #[test]
    fn open_needs_a_folder_and_an_existing_branch_once_asked_for() {
        let mut setup = setup();
        assert!(!setup.can_open());
        setup.cwd = "/work".into();
        assert!(setup.can_open());
        setup.worktree = true;
        setup.existing = true;
        assert!(!setup.can_open());
        setup.branch = "fix".into();
        assert!(setup.can_open());
        let request = setup.request(" Go ", Vec::new());
        assert_eq!(request.text, "Go");
        assert_eq!(request.worktree.unwrap().branch.as_deref(), Some("fix"));
        assert_eq!(setup.where_line(), "Linux PC · /work · worktree on fix");
        assert_eq!(
            setup.what_line(None),
            "Account with most room · Opus 5.5 · High"
        );
        setup.branch.clear();
        setup.existing = false;
        assert_eq!(
            setup.request("Go", Vec::new()).worktree,
            Some(Worktree::default())
        );
        assert_eq!(setup.choices(), SheetChoices::default());
    }

    #[test]
    fn the_quick_folders_are_the_projects_then_the_recent_ones_once_each() {
        let list: FolderList = serde_json::from_value(json!({
            "projects": [{"path": "/a", "name": "A"}, {"path": "/b"}],
            "recent": [{"path": "/b"}, {"path": "/c"}]
        }))
        .unwrap();
        let paths: Vec<&str> = quick_folders(&list, "/a")
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        assert_eq!(paths, ["/b", "/c"]);
        assert_eq!(first_folder(&list), Some("/b"));
    }

    #[test]
    fn an_account_shows_its_room() {
        let account = |extra: serde_json::Value| -> ClaudeAccount {
            let mut base = json!({"id": "a", "name": "Work", "used5h": 40.4, "used7d": 25.0});
            if let (Some(base), serde_json::Value::Object(extra)) = (base.as_object_mut(), extra) {
                base.extend(extra);
            }
            serde_json::from_value(base).unwrap()
        };
        let utc = TimeZone::UTC;
        assert_eq!(
            room_line(&account(json!({})), &utc),
            "Work · 25% week · 40% 5 h"
        );
        assert_eq!(
            room_line(
                &account(json!({"limitedUntil": "2026-11-25T18:00:00Z"})),
                &utc
            ),
            "Work · out until 18:00"
        );
        assert_eq!(
            room_line(&account(json!({"loggedIn": false})), &utc),
            "Work · not logged in"
        );
        let state = crate::fixtures::state(json!({"accounts": [
            {"id": "a", "name": "Work", "machine": "runner"},
            {"id": "b", "name": "Mac's", "machine": "runner-b"},
            {"id": "c", "name": "Kept", "scope": "example"}
        ]}));
        let names: Vec<&str> = accounts_for(&state, &MachineId::from("runner"))
            .iter()
            .map(|a| a.name.as_str())
            .collect();
        assert_eq!(names, ["Work"]);
    }
}
