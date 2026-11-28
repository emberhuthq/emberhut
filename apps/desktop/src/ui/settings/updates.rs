//! Settings' Updates page, a page of its own opened from Advanced's Updates row (and from the
//! app menu's Check for Updates…): this copy's version (its build in a small muted line), the channel
//! (Stable or Release candidates), Check for updates with what it is doing, Install and restart
//! once a build is ready, what's new (the offered build's verified notes, the ones it skips
//! folded; or this copy's own when nothing is offered), and the Check automatically switch. A
//! copy that doesn't update itself (a development build, Linux) shows its version and why.

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, FontWeight, div, px};

use super::SettingsView;
use super::appearance::{labelled, segment};
use super::parts::{nav_row, note, page_body, problem_line, section};
use crate::store::Store;
use crate::store::settings::SettingsPane;
use crate::store::settings::chrome::Health;
use crate::store::settings::entries::Fold;
use crate::store::updates::{Panel, WhatsNew};
use crate::ui::palette::{Pal, category_color};
use crate::updates::channel::Channel;

/// The Check for updates button, for tests to find.
pub(super) const CHECK: &str = "updates-check";
/// Advanced's Updates row.
pub(super) const ROW: &str = "updates-row";
/// The Install and restart button.
pub(super) const INSTALL: &str = "updates-install";
/// The Check automatically switch.
pub(super) const AUTOMATIC: &str = "updates-automatic";

/// The line under the channel: what release candidates are.
const CANDIDATES: &str = "Release candidates are previews of the next version, a few days early.";

impl SettingsView {
    /// The Updates page's scrolling body: the version, then the channel, the check and what's
    /// new in a copy that updates itself.
    pub(super) fn updates_page(&self, p: &Pal, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let panel = self
            .store
            .read(cx)
            .updates()
            .panel(std::time::SystemTime::now());
        let mut parts = vec![version(&panel, p).into_any_element()];
        if panel.updates {
            parts.push(self.channel(&panel, p).into_any_element());
            parts.push(self.check(&panel, p).into_any_element());
            if let Some(notes) = &panel.notes {
                parts.push(self.whats_new(notes, p, cx).into_any_element());
            }
        }
        page_body(
            "updates-scroll",
            div().flex().flex_col().gap(px(18.)).children(parts),
        )
    }

    /// Advanced's Updates row, which opens the page: `Emberhut 0.1.0 · up to date`.
    pub(super) fn updates_row(&self, p: &Pal, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let panel = self
            .store
            .read(cx)
            .updates()
            .panel(std::time::SystemTime::now());
        let (status, detail) = row_words(&panel);
        let row = nav_row(
            ROW,
            p,
            (IconName::RefreshCw, category_color(3)),
            "Updates",
            status,
            &detail,
        )
        .on_click(Self::opens(SettingsPane::Updates))
        .test_support();
        section(p, "", None, vec![row.into_any_element()], None)
    }

    /// The channel: Stable or Release candidates, and why a candidate on Stable stays put.
    fn channel(&self, panel: &Panel, p: &Pal) -> Div {
        let mut choice = div()
            .flex()
            .flex_none()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(9.))
            .bg(p.surface_raised);
        for (id, channel) in [
            ("updates-channel-stable", Channel::Stable),
            ("updates-channel-rc", Channel::Candidates),
        ] {
            let store = self.store.clone();
            // Seen by the headless tests (a no-op outside them).
            choice = choice.child(
                segment(id, channel.name(), panel.channel == channel, p)
                    .on_click(move |_, _, cx| {
                        store.update(cx, |store, cx| store.set_update_channel(channel, cx));
                    })
                    .test_support(),
            );
        }
        let row = labelled(p, "Channel", CANDIDATES).child(choice);
        let mut rows = vec![row.into_any_element()];
        if let Some(waiting) = &panel.waiting {
            rows.push(note(p, waiting.clone()).into_any_element());
        }
        section(p, "Channel", None, rows, None)
    }

    /// Check for updates and what it is doing, Install and restart, and Check automatically.
    fn check(&self, panel: &Panel, p: &Pal) -> Div {
        let status: AnyElement = match (&panel.problem, &panel.status) {
            (Some(problem), _) => problem_line(p, problem.clone()).into_any_element(),
            (None, Some(status)) => div()
                .text_size(px(12.5))
                .text_color(p.muted)
                .child(status.clone())
                .into_any_element(),
            (None, None) => div().into_any_element(),
        };
        let action: AnyElement = if panel.ready.is_some() {
            let store = self.store.clone();
            Button::new(INSTALL)
                .primary()
                .small()
                .label("Install and restart")
                .on_click(move |_, _, cx| store.update(cx, Store::restart_to_update))
                .into_any_element()
        } else {
            let store = self.store.clone();
            Button::new(CHECK)
                .small()
                .label("Check for updates")
                .disabled(!panel.can_check)
                .on_click(move |_, _, cx| store.update(cx, Store::check_updates_now))
                .into_any_element()
        };
        let title = panel.ready.as_ref().map_or_else(
            || "Updates".to_owned(),
            |version| format!("Emberhut {version} is ready"),
        );
        let check = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(div().text_size(px(13.)).child(title))
                    .child(status),
            )
            .child(action);
        let store = self.store.clone();
        let automatic = labelled(p, "Check automatically", "Once a day").child(
            Switch::new(AUTOMATIC)
                .accessibility_label("Check automatically")
                .checked(panel.automatic)
                .on_click(move |on, _, cx| {
                    store.update(cx, |store, cx| store.set_update_automatic(*on, cx));
                }),
        );
        section(
            p,
            "Check",
            None,
            vec![check.into_any_element(), automatic.into_any_element()],
            None,
        )
    }

    /// What's new: the notes, scrolling past a few lines, and the releases skipped folded.
    fn whats_new(&self, notes: &WhatsNew, p: &Pal, cx: &mut Context<'_, Self>) -> Div {
        let rows = vec![
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(notes.title.clone()),
                )
                .child(
                    div()
                        .id("updates-notes")
                        .max_h(px(220.))
                        .overflow_y_scroll()
                        .text_size(px(12.5))
                        .child(notes.text.clone()),
                )
                .into_any_element(),
        ];
        let mut whats_new =
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(section(p, "What's new", None, rows, None));
        if !notes.earlier.is_empty() {
            let earlier = notes
                .earlier
                .iter()
                .map(|release| {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .px(px(4.))
                        .child(
                            div()
                                .text_size(px(12.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!("In {}", release.version)),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(p.muted)
                                .child(release.notes.trim().to_owned()),
                        )
                        .into_any_element()
                })
                .collect();
            let count = notes.earlier.len();
            let summary = format!(
                "{count} skipped release{} · notes not verified",
                if count == 1 { "" } else { "s" }
            );
            whats_new = whats_new.child(self.fold(
                Fold::EarlierNotes,
                ("Earlier releases", summary),
                earlier,
                p,
                cx,
            ));
        }
        whats_new
    }
}

/// This copy's version, its build in a small muted line, and why it doesn't update itself
/// when it doesn't.
/// Advanced's Updates row's words: the version (with how it stands), and what an update is
/// doing, in a few words.
fn row_words(panel: &Panel) -> ((String, Health), String) {
    let version = format!("{} {}", crate::brand::APP_NAME, panel.version);
    if let Some(ready) = &panel.ready {
        return ((version, Health::Warn), format!("{ready} ready to install"));
    }
    if panel.problem.is_some() {
        return ((version, Health::Warn), "couldn't check".to_owned());
    }
    if !panel.updates {
        return ((version, Health::Off), "doesn't update itself".to_owned());
    }
    let detail = match panel.status.as_deref() {
        Some(status) if status.starts_with("You're up to date") => "up to date".to_owned(),
        Some(status) => status.to_owned(),
        None => String::new(),
    };
    ((version, Health::Fine), detail)
}

fn version(panel: &Panel, p: &Pal) -> Div {
    let mut rows = vec![
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .id("updates-version")
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(panel.version.clone())
                    .test_support(),
            )
            .child(note(p, panel.build.clone()))
            .into_any_element(),
    ];
    if let Some(why) = &panel.unavailable {
        rows.push(
            div()
                .text_size(px(12.5))
                .text_color(p.muted)
                .child(why.clone())
                .into_any_element(),
        );
    }
    section(p, "Version", None, rows, None)
}
