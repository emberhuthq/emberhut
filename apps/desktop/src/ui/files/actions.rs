//! A file's uses, as data and as the menu and buttons that offer them: Share… (where the
//! platform has a share menu), Save to Downloads, Save As…, Open in its app, and Copy for a
//! picture or a text file. A file that could run as a program is never opened: Show in Finder
//! (Show in Folder elsewhere) takes Open in its app's place. Each fetches the file first
//! ([`Store::fetch_file`]); the status line says how far it is and how it went.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, Window};

use super::default_app;
use crate::store::Store;
use crate::store::files::{RemoteFile, Use, downloads_dir};
use crate::ui::share::{self, Shared};

/// A use of a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileAction {
    /// The system's share menu, with the file itself.
    Share,
    /// Into Downloads under its own name, then shown there.
    SaveToDownloads,
    /// Where the user picks, in a save panel starting in Downloads.
    SaveAs,
    /// In the system's default app for its type.
    OpenInApp,
    /// In its folder, in the system's file manager: for a file that could run as a program.
    ShowInFolder,
    /// On the clipboard: a picture as a picture, text as text.
    Copy,
}

impl FileAction {
    /// The uses offered for `file`, in the menu's order.
    #[must_use]
    pub(crate) fn offered(file: &RemoteFile) -> Vec<Self> {
        let mut actions = Vec::with_capacity(5);
        if share::HAS_MENU {
            actions.push(Self::Share);
        }
        let open = if file.runs_as_program() {
            Self::ShowInFolder
        } else {
            Self::OpenInApp
        };
        actions.extend([Self::SaveToDownloads, Self::SaveAs, open]);
        if file.kind().copies() {
            actions.push(Self::Copy);
        }
        actions
    }

    /// What it says for `file`.
    #[must_use]
    pub(crate) fn label(self, file: &RemoteFile) -> String {
        match self {
            Self::Share => "Share…".to_owned(),
            Self::SaveToDownloads => "Save to Downloads".to_owned(),
            Self::SaveAs => "Save As…".to_owned(),
            Self::OpenInApp => default_app::open_in(&file.name),
            Self::ShowInFolder => show_in_folder().to_owned(),
            Self::Copy => "Copy".to_owned(),
        }
    }

    /// Its icon.
    #[must_use]
    pub(crate) fn icon(self) -> IconName {
        match self {
            Self::Share => IconName::Share,
            Self::SaveToDownloads => IconName::Download,
            Self::SaveAs => IconName::Folder,
            Self::OpenInApp => IconName::ExternalLink,
            Self::ShowInFolder => IconName::FolderSearch,
            Self::Copy => IconName::Copy,
        }
    }

    /// Does it with `file`, from `window`.
    pub(crate) fn run(
        self,
        file: RemoteFile,
        store: &Entity<Store>,
        window: &Window,
        cx: &mut App,
    ) {
        match self {
            Self::Share => share_file(&file, store, window, cx),
            Self::SaveToDownloads => {
                store.update(cx, |store, cx| {
                    store.use_file(file, Use::SaveToDownloads, cx);
                });
            }
            Self::SaveAs => save_as(file, store, cx),
            Self::OpenInApp => {
                store.update(cx, |store, cx| store.use_file(file, Use::OpenInApp, cx));
            }
            Self::ShowInFolder => {
                store.update(cx, |store, cx| store.use_file(file, Use::ShowInFolder, cx));
            }
            Self::Copy => store.update(cx, |store, cx| store.use_file(file, Use::Copy, cx)),
        }
    }
}

/// What showing a file in its folder is called on this platform.
#[must_use]
pub(crate) fn show_in_folder() -> &'static str {
    if cfg!(target_os = "macos") {
        "Show in Finder"
    } else {
        "Show in Folder"
    }
}

/// The share menu with `file` once it is here, at the pointer in `window`.
fn share_file(file: &RemoteFile, store: &Entity<Store>, window: &Window, cx: &mut App) {
    let fetching = store.update(cx, |store, cx| store.fetch_file(file, cx));
    let handle = window.window_handle();
    cx.spawn(async move |cx| {
        let Ok(path) = fetching.kept().await else {
            return;
        };
        if let Err(error) = handle.update(cx, |_, window, cx| {
            share::share(Shared::File(path), window, cx);
        }) {
            log::debug!("the window to share from closed: {error}");
        }
    })
    .detach();
}

/// Asks where to save `file` (Downloads to start with, its own name), then saves it there.
fn save_as(file: RemoteFile, store: &Entity<Store>, cx: &mut App) {
    let folder = downloads_dir()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    let picked = cx.prompt_for_new_path(&folder, Some(&file.name));
    let store = store.clone();
    cx.spawn(async move |cx| {
        let path = match picked.await {
            Ok(Ok(Some(path))) => path,
            Ok(Ok(None)) => return,
            Ok(Err(error)) => {
                log::warn!("the save panel failed: {error}");
                return;
            }
            Err(error) => {
                log::warn!("the save panel went away: {error}");
                return;
            }
        };
        store.update(cx, |store, cx| store.use_file(file, Use::SaveAs(path), cx));
    })
    .detach();
}

/// `file`'s uses as a right-click menu.
pub(crate) fn menu(
    file: &RemoteFile,
    store: &Entity<Store>,
) -> impl Fn(PopupMenu, &mut Window, &mut Context<'_, PopupMenu>) -> PopupMenu + 'static {
    let (file, store) = (file.clone(), store.clone());
    move |menu, _, _| {
        FileAction::offered(&file)
            .into_iter()
            .fold(menu, |menu, action| {
                let (file, store) = (file.clone(), store.clone());
                menu.item(
                    PopupMenuItem::new(action.label(&file))
                        .icon(Icon::new(action.icon()))
                        .on_click(move |_, window, cx| {
                            action.run(file.clone(), &store, window, cx);
                        }),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_is_offered_for_pictures_and_text_only() {
        let offered = |name: &str| FileAction::offered(&RemoteFile::new("/uploads/x", name));
        let pdf = offered("review-pack.pdf");
        assert!(pdf.contains(&FileAction::SaveToDownloads));
        assert!(pdf.contains(&FileAction::SaveAs));
        assert!(pdf.contains(&FileAction::OpenInApp));
        assert!(!pdf.contains(&FileAction::Copy));
        assert!(offered("shot.png").contains(&FileAction::Copy));
        assert!(offered("notes.md").contains(&FileAction::Copy));
        assert_eq!(pdf.contains(&FileAction::Share), share::HAS_MENU);
    }

    #[test]
    fn a_file_that_could_run_is_shown_in_its_folder_never_opened() {
        for name in ["x.command", "x.exe", "x.terminal", "x.app", "run.sh"] {
            let offered = FileAction::offered(&RemoteFile::new("/uploads/x", name));
            assert!(!offered.contains(&FileAction::OpenInApp), "{name}");
            assert!(offered.contains(&FileAction::ShowInFolder), "{name}");
        }
        let by_address = FileAction::offered(&RemoteFile::new("/uploads/x.exe", "invoice"));
        assert!(!by_address.contains(&FileAction::OpenInApp), "its address");
        let pdf = FileAction::offered(&RemoteFile::new("/uploads/x", "a.pdf"));
        assert!(!pdf.contains(&FileAction::ShowInFolder));
    }

    #[test]
    fn the_uses_read_as_the_reference_client_words_them() {
        let file = RemoteFile::new("/uploads/x", "a.pdf");
        assert_eq!(FileAction::Share.label(&file), "Share…");
        assert_eq!(
            FileAction::SaveToDownloads.label(&file),
            "Save to Downloads"
        );
        assert_eq!(FileAction::SaveAs.label(&file), "Save As…");
        assert!(FileAction::OpenInApp.label(&file).starts_with("Open in "));
    }
}
