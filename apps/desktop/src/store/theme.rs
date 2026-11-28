//! The theme as the app holds it: the server's (cached on disk to start in it offline), a pick
//! shown at once and sent after, a custom accent's preview, and a theme fixed from the command
//! line (shown, never saved).

use std::sync::{Arc, Mutex};

use gpui_kit::AppContext as _;
use gpui_kit::Context;

use super::{Store, update_or_log};
use crate::backend::Error;
use crate::config::AppFiles;
use crate::json_file::{self, Writes};
use crate::theme::{built_in, with_choice};
use emberhut_api::theme::{AccentChange, Appearance, ThemeChange, ThemeChoice, ThemePage};

/// The theme state.
#[derive(Debug)]
pub(crate) struct ThemeState {
    page: ThemePage,
    /// Bumped on every change, so the window knows to apply it.
    revision: u64,
    /// Why the last change didn't reach the server.
    problem: Option<String>,
    /// A custom accent as the server would make it (not saved).
    preview: Option<ThemePage>,
    /// Picked while the server was away: sent when it answers.
    pending: Option<ThemeChoice>,
    /// Shown, never saved (`--theme`, `--appearance`).
    fixed: Option<ThemeChoice>,
    loading: bool,
    /// The number of the last save sent; only the newest save's reply is taken.
    pub(super) sent: u64,
    /// The number of the last save whose reply came back.
    pub(super) answered: u64,
    /// The number of the last preview asked for; only the newest preview's reply is taken.
    pub(super) previews: u64,
    /// The writes to the cache: an older one that comes later is skipped, so the newest theme
    /// is the one on disk.
    writes: Writes,
}

impl ThemeState {
    /// The last theme kept in `files`, else the built-in one. Reads a file: call it before the
    /// app's run loop starts.
    #[must_use]
    pub(crate) fn load(files: Option<&AppFiles>) -> Self {
        let page = files.and_then(read_cache).unwrap_or_else(built_in);
        Self {
            page,
            revision: 0,
            problem: None,
            preview: None,
            pending: None,
            fixed: None,
            loading: false,
            sent: 0,
            answered: 0,
            previews: 0,
            writes: Writes::default(),
        }
    }

    /// The theme to draw.
    #[must_use]
    pub(crate) fn page(&self) -> &ThemePage {
        &self.page
    }

    /// Changes whenever the theme does.
    #[must_use]
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// Why the last change didn't reach the server.
    #[must_use]
    pub(crate) fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// The custom accent's preview.
    #[must_use]
    pub(crate) fn preview(&self) -> Option<&ThemePage> {
        self.preview.as_ref()
    }
}

impl Store {
    /// The theme.
    #[must_use]
    pub(crate) fn theme(&self) -> &ThemeState {
        &self.theme
    }

    /// Shows a preset and appearance from the command line without ever saving them.
    pub(crate) fn fix_theme(&mut self, preset: Option<String>, appearance: Option<Appearance>) {
        let mut choice = self.theme.page.choice.clone();
        if let Some(preset) = preset {
            choice.preset = preset;
            choice.accent = None;
        }
        if let Some(appearance) = appearance {
            choice.appearance = appearance;
        }
        if let Some(page) = with_choice(&self.theme.page, &choice) {
            self.theme.page = page;
        }
        self.theme.fixed = Some(choice);
    }

    /// Holds `page` as the theme the server sent, for the views' tests.
    #[cfg(test)]
    pub(crate) fn put_theme_for_tests(&mut self, page: ThemePage) {
        self.theme.page = page;
    }

    /// Fetches the theme from the server.
    pub(crate) fn load_theme(&mut self, cx: &mut Context<'_, Self>) {
        if self.theme.loading {
            return;
        }
        self.theme.loading = true;
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let page = cx.background_spawn(async move { backend.theme() }).await;
            update_or_log(&this, cx, |store, cx| {
                store.theme.loading = false;
                match page {
                    Ok(page) => match store.theme.fixed.clone() {
                        Some(fixed) => {
                            let shown = with_choice(&page, &fixed).unwrap_or(page);
                            store.set_theme(shown, false, cx);
                        }
                        None => store.set_theme(page, true, cx),
                    },
                    Err(error) => log::warn!("couldn't fetch the theme: {error}"),
                }
            });
        })
        .detach();
    }

    /// A preset or an appearance: shown at once from the presets at hand, then saved on the
    /// server.
    pub(crate) fn pick_theme(
        &mut self,
        preset: Option<String>,
        appearance: Option<Appearance>,
        cx: &mut Context<'_, Self>,
    ) {
        let mut choice = self
            .theme
            .pending
            .clone()
            .unwrap_or_else(|| self.theme.page.choice.clone());
        if let Some(preset) = preset {
            choice.preset = preset;
            choice.accent = None;
        }
        if let Some(appearance) = appearance {
            choice.appearance = appearance;
        }
        if let Some(page) = with_choice(&self.theme.page, &choice) {
            self.set_theme(page, false, cx);
        }
        self.theme.preview = None;
        self.send_theme(choice, true, cx);
    }

    /// The ember's tint: shown at once, then saved on the server like a preset.
    pub(crate) fn pick_ember_tint(&mut self, tint: String, cx: &mut Context<'_, Self>) {
        let mut choice = self
            .theme
            .pending
            .clone()
            .unwrap_or_else(|| self.theme.page.choice.clone());
        choice.ember_tint = tint;
        if let Some(page) = with_choice(&self.theme.page, &choice) {
            self.set_theme(page, false, cx);
        }
        self.send_theme(choice, true, cx);
    }

    /// A custom accent (`None`: the preset's own). The server holds it to WCAG AA, so it needs
    /// the server; with `preview`, only what it would come out as.
    pub(crate) fn custom_accent(
        &mut self,
        accent: Option<String>,
        preview: bool,
        cx: &mut Context<'_, Self>,
    ) {
        if preview {
            self.theme.previews += 1;
            let asked = self.theme.previews;
            let backend = Arc::clone(&self.backend);
            cx.spawn(async move |this, cx| {
                let change = ThemeChange {
                    accent: accent_change(accent),
                    preview: true,
                    ..ThemeChange::default()
                };
                let page = cx
                    .background_spawn(async move { backend.change_theme(&change) })
                    .await;
                update_or_log(&this, cx, |store, cx| store.take_preview(asked, page, cx));
            })
            .detach();
            return;
        }
        let mut choice = self.theme.page.choice.clone();
        choice.accent = accent;
        self.theme.preview = None;
        self.send_theme(choice, false, cx);
    }

    /// Drops the custom accent's preview (and any still on its way): the typed colour went.
    pub(crate) fn clear_accent_preview(&mut self, cx: &mut Context<'_, Self>) {
        self.theme.previews += 1;
        if self.theme.preview.take().is_some() {
            cx.notify();
        }
    }

    /// The state's brief of the theme: another device's choice means fetching the whole theme;
    /// a pick made while the server was away goes now.
    pub(super) fn theme_seen(&mut self, brief: &ThemeChoice, cx: &mut Context<'_, Self>) {
        if self.theme.fixed.is_some() {
            return;
        }
        if let Some(pending) = self.theme.pending.clone() {
            // Sent again only once the last save is back: one save on the wire at a time.
            if self.theme.answered == self.theme.sent {
                self.send_theme(pending, true, cx);
            }
        } else if *brief != self.theme.page.choice {
            self.load_theme(cx);
        }
    }

    fn set_theme(&mut self, page: ThemePage, keep: bool, cx: &mut Context<'_, Self>) {
        if keep && let Some(files) = self.files.clone() {
            let (number, written) = self.theme.writes.next();
            let kept = page.clone();
            cx.background_spawn(async move { write_cache(&files, &kept, number, &written) })
                .detach();
        }
        if page != self.theme.page {
            self.theme.page = page;
            self.theme.revision += 1;
            cx.notify();
        }
    }

    /// Saves `choice` on the server; `local` picks are kept to send again if it is away.
    fn send_theme(&mut self, choice: ThemeChoice, local: bool, cx: &mut Context<'_, Self>) {
        if self.theme.fixed.is_some() {
            return;
        }
        if local {
            self.theme.pending = Some(choice.clone());
        }
        self.theme.sent += 1;
        let number = self.theme.sent;
        // A preview still on its way is of a choice this save replaces: its reply is dropped.
        self.theme.previews += 1;
        self.theme.preview = None;
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let change = ThemeChange {
                preset: Some(choice.preset),
                appearance: Some(choice.appearance),
                accent: accent_change(choice.accent),
                ember_tint: Some(choice.ember_tint),
                preview: false,
            };
            let page = cx
                .background_spawn(async move { backend.change_theme(&change) })
                .await;
            update_or_log(&this, cx, |store, cx| {
                store.take_saved(number, local, page, cx);
            });
        })
        .detach();
    }
}

impl Store {
    /// The reply to preview number `asked`: taken only if no newer preview or save came since.
    pub(super) fn take_preview(
        &mut self,
        asked: u64,
        page: Result<ThemePage, Error>,
        cx: &mut Context<'_, Self>,
    ) {
        if asked != self.theme.previews {
            return;
        }
        match page {
            Ok(page) => {
                self.theme.preview = Some(page);
                self.theme.problem = None;
            }
            Err(error) => self.theme.problem = Some(refusal(&error)),
        }
        cx.notify();
    }

    /// The reply to save number `number` (`local`: a pick kept to send again if the server is
    /// away): taken only if it is the newest save's.
    pub(super) fn take_saved(
        &mut self,
        number: u64,
        local: bool,
        page: Result<ThemePage, Error>,
        cx: &mut Context<'_, Self>,
    ) {
        self.theme.answered = self.theme.answered.max(number);
        if number != self.theme.sent {
            return; // a newer save is on its way: its reply is the one that counts
        }
        match page {
            Ok(page) => {
                self.theme.pending = None;
                self.theme.problem = None;
                self.set_theme(page, true, cx);
            }
            Err(error) if local && error.is_unreachable() => {
                self.theme.problem =
                    Some("Saved on this device; the server gets it when it's back.".to_owned());
            }
            Err(error) => {
                self.theme.pending = None;
                self.theme.problem = Some(refusal(&error));
            }
        }
        cx.notify();
    }
}

/// A custom accent as a change: a colour, or `None` for the preset's own.
fn accent_change(accent: Option<String>) -> AccentChange {
    accent.map_or(AccentChange::Reset, AccentChange::Set)
}

/// Why a theme change didn't take: the server's words, else what went wrong.
fn refusal(error: &Error) -> String {
    error
        .server_message()
        .map_or_else(|| format!("Not saved: {error}"), str::to_owned)
}

fn read_cache(files: &AppFiles) -> Option<ThemePage> {
    json_file::read(&files.theme(), "a theme", "using the built-in one")
}

/// Writes `page` as theme number `number`, unless a newer one was written already; through a
/// temporary file and a rename, so a reader never sees half a theme.
fn write_cache(files: &AppFiles, page: &ThemePage, number: u64, written: &Mutex<u64>) {
    json_file::write(&files.theme(), page, number, written, "the theme");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cache_starts_the_next_launch_in_the_last_theme() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(&dir.path().join("emberhut"));
        assert_eq!(
            ThemeState::load(Some(&files)).page(),
            &built_in(),
            "nothing kept yet"
        );
        let mut page = built_in();
        page.choice.appearance = Appearance::Dark;
        write_cache(&files, &page, 1, &Mutex::new(0));
        assert_eq!(ThemeState::load(Some(&files)).page(), &page);
    }

    #[test]
    fn a_broken_cache_falls_back_to_the_built_in_theme() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(dir.path());
        std::fs::write(files.theme(), "{").unwrap();
        assert_eq!(ThemeState::load(Some(&files)).page(), &built_in());
    }

    #[test]
    fn an_older_cache_write_that_comes_late_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(dir.path());
        let written = Mutex::new(0);
        let mut newer = built_in();
        newer.choice.preset = "green".into();
        write_cache(&files, &newer, 2, &written);
        write_cache(&files, &built_in(), 1, &written);
        assert_eq!(ThemeState::load(Some(&files)).page(), &newer);
        assert!(!files.theme().with_extension("json.tmp").exists());
    }
}
