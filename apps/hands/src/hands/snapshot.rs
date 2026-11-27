//! `/snapshot`: an app's window read as a list of elements with refs (`[a12] button "Save"`),
//! indented as they nest, with its menus and, if asked, a picture of that window only. A
//! password field is listed without its value, which is never read.

use std::time::Duration;

use base64::Engine as _;
use serde_json::Value;

use super::{Hands, lock};
use crate::desktop::{Desktop, Flag, Link, List, Text};
use crate::fail::{Fail, Reply, field, flag, number, said};
use crate::rules::{Use, is_secret};
use crate::text::{clip, kind, numbered, push};

/// Containers that are listed only when they have a label.
const BORING: &[&str] = &[
    "AXGroup",
    "AXSplitGroup",
    "AXScrollArea",
    "AXLayoutArea",
    "AXLayoutItem",
    "AXUnknown",
    "AXSplitter",
    "AXMatte",
    "AXGrowArea",
];
/// The actions every pressable element has, not listed.
const PLAIN_ACTIONS: &[&str] = &[
    "AXPress",
    "AXShowMenu",
    "AXCancel",
    "AXRaise",
    "AXPick",
    "AXConfirm",
];
/// How many elements a snapshot lists unless asked.
const LIMIT: usize = 400;
/// The most it lists when asked.
const MAX_LIMIT: usize = 1500;
/// How deep it goes.
const MAX_DEPTH: usize = 40;
/// A window picture's longest side, in pixels.
const PICTURE_MAX: u32 = 1600;
/// How long a restored window gets to draw.
const RESTORE: Duration = Duration::from_millis(400);

/// The `limit` a body asks for, as a whole count of elements in `1..=MAX_LIMIT`.
fn limit(body: &Reply) -> usize {
    number(body, "limit").map_or(LIMIT, |n| {
        // Clamped to 1..=1500 first, so the cast is exact.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to 1..=1500 first"
        )]
        let n = n.clamp(1.0, 1500.0) as usize;
        n.min(MAX_LIMIT)
    })
}

/// The walk's lines and refs so far.
struct Walk<E> {
    lines: Vec<String>,
    refs: Vec<E>,
    listed: usize,
    limit: usize,
}

impl<D: Desktop> Hands<D> {
    /// `/snapshot` (see the module).
    pub(super) fn snapshot(&self, body: &Reply) -> Result<Reply, Fail> {
        let target = self.target(body, Use::Look)?;
        let limit = limit(body);
        let mut head = format!(
            "{} (pid {}, {})",
            target.name,
            target.pid,
            self.state(target.pid, target.hidden)
        );
        let scope = field(body, "scope");
        if !scope.trim().is_empty() {
            return self.part(&head, &scope, limit);
        }
        let all = self.desktop.list(&target.root, List::Windows);
        let titles: Vec<String> = all
            .iter()
            .map(|window| {
                self.desktop
                    .text(window, Text::Title)
                    .unwrap_or_else(|| "untitled".to_owned())
            })
            .collect();
        let want = field(body, "window").to_lowercase();
        let windows: Vec<&D::Element> = if want.is_empty() {
            all.iter().take(1).collect()
        } else {
            all.iter()
                .zip(&titles)
                .enumerate()
                .filter(|(i, (_, title))| {
                    (i + 1).to_string() == want || title.to_lowercase().contains(&want)
                })
                .map(|(_, (window, _))| window)
                .collect()
        };
        if all.is_empty() {
            head.push_str(" · no windows open (or they're on another Space)");
        } else if windows.is_empty() {
            return Err(Fail::bad(format!(
                "No window of {} matches \"{want}\": its windows are {}.",
                target.name,
                numbered(&titles)
            )));
        } else if all.len() > 1 {
            push(
                &mut head,
                format_args!(" · windows: {} (window: to pick)", numbered(&titles)),
            );
        }
        let mut walk = Walk {
            lines: Vec::new(),
            refs: Vec::new(),
            listed: 0,
            limit,
        };
        for window in &windows {
            if self.desktop.flag(window, Flag::Minimized) == Some(true) {
                if let Err(error) = self.desktop.set_flag(window, Flag::Minimized, false) {
                    tracing::debug!(%error, "a minimized window was not restored");
                }
                self.desktop.pause(RESTORE);
            }
            self.walk(window, 0, &mut walk);
        }
        let listed = walk.listed;
        let mut lines = walk.lines;
        self.keep_refs(walk.refs);
        lines.extend(self.menus_line(&target.root));
        let cut = if listed >= limit {
            format!("\n… (cut at {limit}: scope a ref to read part of it)")
        } else {
            String::new()
        };
        let mut text = format!("{head}\n\n{}{cut}", lines.join("\n"));
        let mut image = None;
        if flag(body, "picture") == Some(true) {
            if self.desktop.screen_recording() {
                let title = windows
                    .first()
                    .and_then(|window| self.desktop.text(window, Text::Title));
                image = self.picture(target.pid, title.as_deref());
            } else {
                text.push_str(
                    "\n(no picture: Emberhut Hands has no Screen Recording permission yet)",
                );
            }
        }
        let mut reply = said(text);
        if let Some(image) = image {
            reply.insert("image".to_owned(), Value::String(image));
        }
        Ok(reply)
    }

    /// A part of the window, from the element `scope`: its refs go on from the last
    /// snapshot's.
    fn part(&self, head: &str, scope: &str, limit: usize) -> Result<Reply, Fail> {
        let element = self.element(scope)?;
        let refs = self.refs();
        let start = refs.len();
        let mut walk = Walk {
            lines: Vec::new(),
            refs,
            listed: 0,
            limit,
        };
        self.walk(&element, 0, &mut walk);
        let cut = if walk.listed >= limit {
            format!("\n… (cut at {limit})")
        } else {
            String::new()
        };
        let mut reply = said(format!(
            "{head} · part {scope}\n\n{}{cut}",
            walk.lines.join("\n")
        ));
        reply.insert("refs".to_owned(), Value::from(walk.refs.len() - start));
        self.keep_refs(walk.refs);
        Ok(reply)
    }

    /// The menus line: the menu bar's menus but the Apple menu, which is never used.
    fn menus_line(&self, root: &D::Element) -> Option<String> {
        let bar = self.desktop.link(root, Link::MenuBar)?;
        let menus: Vec<String> = self
            .desktop
            .list(&bar, List::Children)
            .iter()
            .skip(1)
            .filter_map(|menu| self.desktop.text(menu, Text::Title))
            .collect();
        let first = menus.first()?;
        Some(format!(
            "menus: {} (mac_app_menu \"{first}\" lists one, \"{first} > …\" picks an item)",
            menus.join(", ")
        ))
    }

    /// A picture of the app's window titled `title` (else its first), as base64.
    fn picture(&self, pid: i32, title: Option<&str>) -> Option<String> {
        let windows = self.desktop.windows(pid);
        let window = windows
            .iter()
            .find(|w| title == Some(w.title.as_str()))
            .or_else(|| windows.first())?;
        self.desktop
            .window_picture(window.id, PICTURE_MAX)
            .filter(|jpeg| jpeg.len() <= super::screen::MAX_PICTURE)
            .map(|jpeg| base64::engine::general_purpose::STANDARD.encode(jpeg))
    }

    /// The held app's refs.
    pub(super) fn refs(&self) -> Vec<D::Element> {
        lock(&self.session)
            .as_ref()
            .map_or_default(|session| session.refs.clone())
    }

    /// Keeps `refs` as the held app's refs.
    fn keep_refs(&self, refs: Vec<D::Element>) {
        if let Some(session) = lock(&self.session).as_mut() {
            session.refs = refs;
        }
    }

    /// Lists `element` and what it holds, `depth` levels in.
    fn walk(&self, element: &D::Element, depth: usize, walk: &mut Walk<D::Element>) {
        if walk.listed >= walk.limit || depth >= MAX_DEPTH {
            return;
        }
        let role = self.role(element);
        let subrole = self
            .desktop
            .text(element, Text::Subrole)
            .unwrap_or_default();
        let name = self.label(element);
        let secret = is_secret(&role, &subrole, &name);
        // A secret's value is never asked for, so never read.
        let value = if secret {
            None
        } else {
            self.desktop
                .text(element, Text::Value)
                .map(|value| clip(&value, 160))
        };
        // A nameless container, or an empty text, is skipped (its children are not).
        let shown = !name.is_empty()
            || !(BORING.contains(&role.as_str()) || (role == "AXStaticText" && value.is_none()));
        let mut inner = depth;
        if shown {
            walk.refs.push(element.clone());
            walk.listed += 1;
            let mut line = format!(
                "{}[a{}] {}",
                "  ".repeat(depth),
                walk.refs.len(),
                kind(&role)
            );
            if !name.is_empty() {
                push(&mut line, format_args!(" \"{name}\""));
            }
            if let Some(value) = value.filter(|value| *value != name) {
                if role == "AXCheckBox" || role == "AXRadioButton" {
                    line.push_str(if value == "1" { " (on)" } else { " (off)" });
                } else {
                    push(&mut line, format_args!(" = \"{value}\""));
                }
            }
            if self.desktop.flag(element, Flag::Enabled) == Some(false) {
                line.push_str(" (disabled)");
            }
            if self.desktop.flag(element, Flag::Focused) == Some(true) {
                line.push_str(" (focused)");
            }
            if secret {
                line.push_str(" (password: never read or typed; the owner types it)");
            }
            let extra: Vec<String> = self
                .desktop
                .actions(element)
                .into_iter()
                .filter(|action| {
                    action.starts_with("AX")
                        && !action.starts_with("AXScroll")
                        && !PLAIN_ACTIONS.contains(&action.as_str())
                })
                .map(|action| action.trim_start_matches("AX").to_owned())
                .collect();
            if !extra.is_empty() {
                push(&mut line, format_args!(" {{{}}}", extra.join(", ")));
            }
            walk.lines.push(line);
            inner += 1;
        }
        for child in self.desktop.list(element, List::Children) {
            self.walk(&child, inner, walk);
        }
    }
}
