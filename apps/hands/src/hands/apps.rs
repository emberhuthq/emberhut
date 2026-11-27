//! Which app a name means, the apps that run, and opening one in the background.

use std::time::Duration;

use serde_json::json;

use super::{Hands, object};
use crate::desktop::{Desktop, RunningApp};
use crate::fail::{Fail, Reply, field, flag, said};
use crate::rules::{Use, is_chromium, never_used, refusal};
use crate::text::clip;

/// How long a just-opened app gets to show its first window.
const FIRST_WINDOW: Duration = Duration::from_millis(1500);

/// The running apps `want` names: by pid, bundle id or name (case aside).
pub(super) fn matching(running: &[RunningApp], want: &str) -> Vec<RunningApp> {
    let lower = want.to_lowercase();
    let pid: Option<i32> = want.parse().ok();
    running
        .iter()
        .filter(|app| {
            Some(app.pid) == pid
                || app.bundle.to_lowercase() == lower
                || app.name.to_lowercase() == lower
        })
        .cloned()
        .collect()
}

impl<D: Desktop> Hands<D> {
    /// `/which`: the app a name means, running or on disk, by its own name and bundle id (the
    /// server asks the owner about an app by these, not by the agent's words).
    pub(super) fn which(&self, body: &Reply) -> Result<Reply, Fail> {
        let want = field(body, "app").trim().to_owned();
        if want.is_empty() {
            return Err(Fail::bad("Which app? (app: its name, like Calculator)"));
        }
        if let [app] = matching(&self.desktop.running(), &want).as_slice() {
            if let Some(refused) = refusal(&app.name, &app.bundle, Use::Look) {
                return Err(refused);
            }
            return Ok(object(
                json!({ "name": app.name, "bundle": app.bundle, "running": true }),
            ));
        }
        let installed = self
            .desktop
            .installed(&want)
            .ok_or_else(|| Fail::missing(format!("No app called {want} on this Mac.")))?;
        if let Some(refused) = refusal(&installed.name, &installed.bundle, Use::Look) {
            return Err(refused);
        }
        Ok(object(json!({
            "name": installed.name,
            "bundle": installed.bundle,
            "running": false,
        })))
    }

    /// `/apps`: every running app with its windows, marked in front, hidden, or never used.
    pub(super) fn apps(&self) -> Reply {
        let front = self.desktop.front();
        let lines: Vec<String> = self
            .desktop
            .running()
            .iter()
            .map(|app| {
                let windows: Vec<String> = self
                    .desktop
                    .windows(app.pid)
                    .iter()
                    .map(|window| {
                        let name = if window.title.is_empty() {
                            "untitled".to_owned()
                        } else {
                            format!("\"{}\"", clip(&window.title, 50))
                        };
                        if window.on_screen {
                            name
                        } else {
                            format!("{name} (not on screen)")
                        }
                    })
                    .collect();
                let mut marks = Vec::new();
                if Some(app.pid) == front {
                    marks.push("in front");
                }
                if app.hidden {
                    marks.push("hidden");
                }
                if never_used(&app.bundle) || is_chromium(&app.name, &app.bundle) {
                    marks.push("never used by the assistant");
                }
                let marks = if marks.is_empty() {
                    String::new()
                } else {
                    format!(", {}", marks.join(", "))
                };
                let bundle = if app.bundle.is_empty() {
                    "?"
                } else {
                    app.bundle.as_str()
                };
                let windows = if windows.is_empty() {
                    "no windows".to_owned()
                } else {
                    windows.join(", ")
                };
                format!(
                    "- {} ({bundle}, pid {}{marks}): {windows}",
                    app.name, app.pid
                )
            })
            .collect();
        said(format!("Running apps on this Mac:\n{}", lines.join("\n")))
    }

    /// `/open`: opens an app without bringing it forward (hidden unless `hidden: false`); one
    /// already running is left as it is.
    pub(super) fn open(&self, body: &Reply) -> Result<Reply, Fail> {
        let want = field(body, "app").trim().to_owned();
        if want.is_empty() {
            return Err(Fail::bad("Which app? (app: its name, like Calculator)"));
        }
        if let [app] = matching(&self.desktop.running(), &want).as_slice() {
            if let Some(refused) = refusal(&app.name, &app.bundle, Use::Look) {
                return Err(refused);
            }
            self.may_use(Some(app.pid), &app.name)?;
            return Ok(said(format!(
                "{} is already running ({}).",
                app.name,
                self.state(app.pid, app.hidden)
            )));
        }
        let installed = self
            .desktop
            .installed(&want)
            .ok_or_else(|| Fail::missing(format!("No app called {want} on this Mac.")))?;
        if let Some(refused) = refusal(&installed.name, &installed.bundle, Use::Look) {
            return Err(refused);
        }
        self.may_use(None, &installed.name)?;
        let hidden = flag(body, "hidden").unwrap_or(true);
        self.desktop
            .open(&installed, hidden)
            .map_err(|why| Fail::bad(format!("Couldn't open {want}: {why}")))?;
        self.desktop.pause(FIRST_WINDOW);
        tracing::info!(app = %installed.name, hidden, "opened an app");
        Ok(said(format!(
            "Opened {want} in the background{}. Take mac_app_snapshot to read it.",
            if hidden { ", hidden" } else { "" }
        )))
    }
}
