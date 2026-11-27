//! Steps in the held app, in the background: pressing an element, setting a field, keys sent to
//! that app only, and its menus. Never into a password field.

use super::Hands;
use crate::desktop::{AxError, Desktop, Flag, Focus, Link, List, Text};
use crate::fail::{Fail, Reply, field, said};
use crate::keys::{check_length, chord, is_navigation, pieces, press, typing};
use crate::rules::{PASSWORD, Use};
use crate::text::{clip, menu_key};

/// How long the focus may take to move after a Tab or Return.
const FOCUS_MOVE: std::time::Duration = std::time::Duration::from_millis(300);
/// How many times it is looked at meanwhile.
const FOCUS_POLLS: u32 = 10;

/// The actions a press tries, in order, when none is named.
const PRESSES: [&str; 4] = ["AXPress", "AXPick", "AXConfirm", "AXShowMenu"];

/// What the agent is told when an accessibility call fails.
fn failed(what: &str, doing: &str, error: AxError) -> Fail {
    match error {
        AxError::Gone => Fail::bad(format!("{what} is gone: take a new snapshot.")),
        AxError::Code(code) => Fail::bad(format!("{doing} {what} failed ({code}).")),
    }
}

impl<D: Desktop> Hands<D> {
    /// `/press`: presses a ref (or performs its named `action`).
    pub(super) fn press(&self, body: &Reply) -> Result<Reply, Fail> {
        let target = self.target(body, Use::Act)?;
        let element = self.element(&field(body, "ref"))?;
        let actions = self.desktop.actions(&element);
        let what = format!("{} \"{}\"", self.kind(&element), self.label(&element));
        let named = field(body, "action");
        let action = if named.is_empty() {
            PRESSES
                .iter()
                .find(|press| actions.iter().any(|action| action == *press))
                .map(|press| (*press).to_owned())
                .ok_or_else(|| {
                    let can = if actions.is_empty() {
                        "nothing".to_owned()
                    } else {
                        actions.join(", ")
                    };
                    Fail::bad(format!("{what} can't be pressed (it can: {can})."))
                })?
        } else {
            let action = if named.starts_with("AX") {
                named
            } else {
                format!("AX{named}")
            };
            if !actions.contains(&action) {
                return Err(Fail::bad(format!(
                    "{what} has no {action} (it has {})",
                    actions.join(", ")
                )));
            }
            action
        };
        let ((), moved) = self.keep_front(&target, || {
            self.desktop
                .perform(&element, &action)
                .map_err(|error| failed(&what, "Pressing", error))
        })?;
        let how = if action == "AXPress" {
            String::new()
        } else {
            format!(" ({})", action.trim_start_matches("AX"))
        };
        tracing::info!(app = %target.name, %action, "pressed");
        Ok(said(format!(
            "Pressed {what} in {}{how}{moved}. Take a snapshot to see the result.",
            target.name
        )))
    }

    /// `/set`: sets a field's value to `text`. A password field is refused.
    pub(super) fn set(&self, body: &Reply) -> Result<Reply, Fail> {
        let target = self.target(body, Use::Act)?;
        let element = self.element(&field(body, "ref"))?;
        if self.secret(&element) {
            return Err(Fail::never(PASSWORD));
        }
        let text = field(body, "text");
        let what = format!("{} \"{}\"", self.kind(&element), self.label(&element));
        if !self.desktop.settable(&element) {
            // The server's tool falls back to keys on these words.
            return Err(Fail::bad(format!(
                "{what} can't be set directly: mac_app_keys with this ref types into it instead."
            )));
        }
        let ((), moved) = self.keep_front(&target, || {
            self.desktop
                .set_value(&element, &text)
                .map_err(|error| failed(&what, "Setting", error))
        })?;
        let now = self.desktop.text(&element, Text::Value).unwrap_or_default();
        tracing::info!(app = %target.name, "set a field");
        Ok(said(if now == text {
            format!(
                "Set {what} in {} to \"{}\"{moved}.",
                target.name,
                clip(&text, 80)
            )
        } else {
            format!(
                "Set {what}, but it now reads \"{}\": check with a snapshot{moved}.",
                clip(&now, 80)
            )
        }))
    }

    /// `/keys`: `text` typed and then `key` pressed, sent to the app only (focusing `ref` in it
    /// first). Never into a password field: the focus is checked before the text, again after
    /// each Tab, Return or new line in it (each may move the focus), and before a key that
    /// isn't only navigation.
    pub(super) fn keys(&self, body: &Reply) -> Result<Reply, Fail> {
        let target = self.target(body, Use::Act)?;
        let text = body
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or_default();
        check_length(text)?;
        let key = body.get("key").and_then(|k| k.as_str()).unwrap_or_default();
        let pressed = if key.is_empty() {
            None
        } else {
            Some(chord(key)?)
        };
        if body.get("ref").is_some_and(|r| !r.is_null()) {
            let element = self.element(&field(body, "ref"))?;
            if self.secret(&element) {
                return Err(Fail::never(PASSWORD));
            }
            if let Err(error) = self.desktop.set_flag(&element, Flag::Focused, true) {
                tracing::debug!(%error, "the ref was not focused");
            }
        }
        let check = || self.check_focus(self.desktop.focus_in(&target.root), &target.name);
        let ((), moved) = self.keep_front(&target, || {
            self.type_checked(
                text,
                &target.name,
                &|| self.desktop.focus_in(&target.root),
                &mut |piece| {
                    self.desktop
                        .post_to_app(target.pid, &typing(piece), &mut |_| true)
                },
            )?;
            if let Some(pressed) = pressed {
                if !is_navigation(key) {
                    check()?;
                }
                self.desktop
                    .post_to_app(target.pid, &press(pressed), &mut |_| true);
            }
            Ok(())
        })?;
        let mut sent = Vec::new();
        if body.get("text").is_some_and(serde_json::Value::is_string) {
            sent.push(format!("typed \"{}\"", clip(text, 60)));
        }
        if body.get("key").is_some_and(serde_json::Value::is_string) {
            sent.push(format!("pressed {key}"));
        }
        tracing::info!(app = %target.name, "sent keys");
        Ok(said(format!(
            "Sent to {} only: {}{moved}. Background apps sometimes ignore keys: check with a \
             snapshot.",
            target.name,
            sent.join(", ")
        )))
    }

    /// Types `text` (into `app`) piece by piece with `post`, reading what has the focus with
    /// `focus`: before each piece it is checked, and after a piece that ends in Tab or Return
    /// (or a new line) the focus must move within [`FOCUS_MOVE`] before the rest is typed (a
    /// new line in a multi-line text area stays there). Whether all was typed (`post` may stop).
    pub(super) fn type_checked(
        &self,
        text: &str,
        app: &str,
        focus: &dyn Fn() -> Focus<D::Element>,
        post: &mut dyn FnMut(&str) -> bool,
    ) -> Result<bool, Fail> {
        let mut typed_into: Option<(Focus<D::Element>, char)> = None;
        for piece in pieces(text) {
            let now = match typed_into.take() {
                None => focus(),
                Some((before, ended)) => self.moved_focus(&before, ended, app, focus)?,
            };
            self.check_focus(now.clone(), app)?;
            if !post(piece) {
                return Ok(false);
            }
            if let Some(ended) = piece
                .chars()
                .last()
                .filter(|c| ['\t', '\r', '\n'].contains(c))
            {
                typed_into = Some((now, ended));
            }
        }
        Ok(true)
    }

    /// The focus after a piece ending in `ended` typed while `before` had it: once it moved
    /// (polled for up to [`FOCUS_MOVE`]); where it was, for a new line in a multi-line text
    /// area.
    fn moved_focus(
        &self,
        before: &Focus<D::Element>,
        ended: char,
        app: &str,
        focus: &dyn Fn() -> Focus<D::Element>,
    ) -> Result<Focus<D::Element>, Fail> {
        if ended != '\t'
            && let Focus::On(element) = before
            && self.role(element) == "AXTextArea"
        {
            return Ok(focus());
        }
        for _ in 0..FOCUS_POLLS {
            // Moved means onto an element, another one: nothing focused, or unknown, isn't.
            let now = focus();
            if matches!(now, Focus::On(_)) && now != *before {
                return Ok(now);
            }
            self.desktop.pause(FOCUS_MOVE / FOCUS_POLLS);
        }
        let key = if ended == '\t' { "Tab" } else { "Return" };
        Err(Fail::never(format!(
            "After the {key}, {app} didn't show where the keyboard focus went, so the rest isn't \
             typed (it could be a password field). Take a snapshot and type by ref."
        )))
    }

    /// Refuses to type while a password field has the focus in `app`, or while what has it
    /// can't be told.
    pub(super) fn check_focus(&self, focus: Focus<D::Element>, app: &str) -> Result<(), Fail> {
        match focus {
            Focus::On(element) if self.secret(&element) => Err(Fail::never(format!(
                "{app} has a password field focused. {PASSWORD}"
            ))),
            Focus::On(_) | Focus::Nothing => Ok(()),
            Focus::Unknown => Err(Fail::never(format!(
                "{app} doesn't say what has the keyboard focus, so nothing is typed there (it \
                 could be a password field). Take a snapshot and type by ref, or hand it to the \
                 owner."
            ))),
        }
    }

    /// `/menu`: a menu by its path (`File > Export…`): an item is picked, a menu is listed.
    /// The Apple menu is never used.
    pub(super) fn menu(&self, body: &Reply) -> Result<Reply, Fail> {
        let target = self.target(body, Use::Act)?;
        let path: Vec<String> = field(body, "path")
            .split('>')
            .map(|step| step.trim().to_owned())
            .filter(|step| !step.is_empty())
            .collect();
        let bar = self.desktop.link(&target.root, Link::MenuBar);
        let (Some(bar), false) = (bar, path.is_empty()) else {
            return Err(Fail::bad(
                "path: a menu, like \"File\" or \"File > Export…\"",
            ));
        };
        let mut items: Vec<D::Element> = self
            .desktop
            .list(&bar, List::Children)
            .into_iter()
            .skip(1)
            .collect();
        let mut walked: Vec<String> = Vec::new();
        let mut picked = None;
        for (i, step) in path.iter().enumerate() {
            let titled: Vec<(D::Element, String)> = items
                .iter()
                .filter_map(|item| {
                    self.desktop
                        .text(item, Text::Title)
                        .filter(|title| !title.is_empty())
                        .map(|title| (item.clone(), title))
                })
                .collect();
            let wanted = menu_key(step);
            let Some((item, title)) = titled
                .iter()
                .find(|(_, title)| menu_key(title) == wanted)
                .or_else(|| {
                    titled
                        .iter()
                        .find(|(_, title)| menu_key(title).starts_with(&wanted))
                })
                .cloned()
            else {
                let place = if walked.is_empty() {
                    format!("{}'s menu bar", target.name)
                } else {
                    walked.join(" > ")
                };
                let has: Vec<&str> = titled.iter().map(|(_, title)| title.as_str()).collect();
                return Err(Fail::bad(format!(
                    "No \"{step}\" in {place}: it has {}.",
                    has.join(", ")
                )));
            };
            walked.push(title);
            let sub = self
                .desktop
                .list(&item, List::Children)
                .into_iter()
                .find(|child| self.role(child) == "AXMenu");
            if i + 1 < path.len() {
                let Some(sub) = sub else {
                    return Err(Fail::bad(format!(
                        "{} is an item, not a menu.",
                        walked.join(" > ")
                    )));
                };
                items = self.desktop.list(&sub, List::Children);
            } else if let Some(sub) = sub {
                return Ok(said(self.menu_lines(&target.name, &walked, &sub)));
            }
            picked = Some(item);
        }
        let item = picked.ok_or_else(|| Fail::bad("Nothing to pick"))?;
        let walked = walked.join(" > ");
        if self.desktop.flag(&item, Flag::Enabled) == Some(false) {
            return Err(Fail::bad(format!("{walked} is disabled right now.")));
        }
        let ((), moved) = self.keep_front(&target, || {
            self.desktop
                .perform(&item, "AXPress")
                .map_err(|error| failed(&walked, "Picking", error))
        })?;
        tracing::info!(app = %target.name, menu = %walked, "picked a menu item");
        Ok(said(format!(
            "Picked {walked} in {}{moved}. Take a snapshot to see what it opened.",
            target.name
        )))
    }

    /// What the menu `sub` holds, one item a line (`>` a submenu, `⌘S` its key).
    fn menu_lines(&self, app: &str, walked: &[String], sub: &D::Element) -> String {
        let lines: Vec<String> = self
            .desktop
            .list(sub, List::Children)
            .iter()
            .filter_map(|item| {
                let title = self.desktop.text(item, Text::Title)?;
                let off = if self.desktop.flag(item, Flag::Enabled) == Some(false) {
                    " (disabled)"
                } else {
                    ""
                };
                let more = if self
                    .desktop
                    .list(item, List::Children)
                    .iter()
                    .any(|child| self.role(child) == "AXMenu")
                {
                    " >"
                } else {
                    ""
                };
                let key = self
                    .desktop
                    .text(item, Text::MenuKey)
                    .map_or_default(|key| format!(" ⌘{key}"));
                Some(format!("- {title}{more}{key}{off}"))
            })
            .collect();
        let walked = walked.join(" > ");
        if lines.is_empty() {
            format!(
                "{app} fills in {walked} only when it opens on screen, so it can't be read in \
                 the background."
            )
        } else {
            format!("{app} › {walked}:\n{}", lines.join("\n"))
        }
    }
}
