//! What the app does with a file once it is here: opens it in the system's default app for its
//! type, shows it in its folder, saves it into Downloads (and shows it there) or where the user
//! picked, or copies it (a picture as a picture, text as text). Each says how it went on the
//! status line. A file that could run as a program ([`RemoteFile::runs_as_program`]) is never
//! opened, whoever asks: it is shown in its folder instead.

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::AppContext as _;
use gpui_kit::{ClipboardItem, Context};

use super::downloads::{downloads_dir, save_copy, save_to};
use super::{Kind, RemoteFile};
use crate::store::images;
use crate::store::{Store, update_or_log};

/// A use of a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Use {
    /// Open it in the system's default app for its type (one that could run as a program is
    /// shown in its folder instead).
    OpenInApp,
    /// Show it in its folder, in the system's file manager.
    ShowInFolder,
    /// Save it into Downloads under its own name (numbered when taken), and show it there.
    SaveToDownloads,
    /// Save it where the user picked in a save panel.
    SaveAs(PathBuf),
    /// Put it on the clipboard: a picture as a picture, text as text.
    Copy,
}

impl Store {
    /// Shows a file saved on this device in its folder, in the system's file manager.
    pub(crate) fn reveal_saved(&self, path: &std::path::Path, cx: &mut Context<'_, Self>) {
        let reveal = Rc::clone(&self.kept.reveal);
        reveal(path, cx);
    }

    /// Does `with` with `file` once it is here (a failed download said so already).
    pub(crate) fn use_file(&mut self, file: RemoteFile, with: Use, cx: &mut Context<'_, Self>) {
        let fetching = self.fetch_file(&file, cx);
        cx.spawn(async move |this, cx| {
            let Ok(kept) = fetching.kept().await else {
                return;
            };
            match with {
                Use::OpenInApp if file.runs_as_program() => {
                    log::info!("{} could run as a program: shown in its folder", file.path);
                    update_or_log(&this, cx, |store, cx| {
                        store.show_file_note(
                            format!("{} runs as a program: shown, not opened", file.name),
                            false,
                            cx,
                        );
                        let reveal = Rc::clone(&store.kept.reveal);
                        reveal(&kept, cx);
                    });
                }
                Use::OpenInApp => {
                    update_or_log(&this, cx, |store, cx| {
                        let open = Rc::clone(&store.kept.open);
                        open(&kept, cx);
                    });
                }
                Use::ShowInFolder => {
                    update_or_log(&this, cx, |store, cx| {
                        let reveal = Rc::clone(&store.kept.reveal);
                        reveal(&kept, cx);
                    });
                }
                Use::SaveToDownloads => {
                    let name = file.name.clone();
                    let saved = cx
                        .background_spawn(async move {
                            let dir = downloads_dir().ok_or("no Downloads folder")?;
                            let kept_name = kept.file_name().map_or_else(
                                || name.clone(),
                                |kept| kept.to_string_lossy().into_owned(),
                            );
                            save_copy(&kept, &dir, &kept_name).map_err(|error| {
                                log::warn!("couldn't save {name} into Downloads: {error}");
                                "couldn't write it"
                            })
                        })
                        .await;
                    update_or_log(&this, cx, |store, cx| match saved {
                        Ok(to) => {
                            let shown = to
                                .file_name()
                                .map_or_else(String::new, |name| name.to_string_lossy().into());
                            store.show_file_note(
                                format!("Saved to Downloads as {shown}"),
                                false,
                                cx,
                            );
                            cx.reveal_path(&to);
                        }
                        Err(why) => {
                            log::warn!("couldn't save {}: {why}", file.name);
                            store.show_file_note(format!("Couldn't save {}", file.name), true, cx);
                        }
                    });
                }
                Use::SaveAs(to) => {
                    let saved = cx
                        .background_spawn(async move { save_to(&kept, &to) })
                        .await;
                    if let Err(error) = saved {
                        log::warn!("couldn't save {}: {error}", file.name);
                        update_or_log(&this, cx, |store, cx| {
                            store.show_file_note(format!("Couldn't save {}", file.name), true, cx);
                        });
                    }
                }
                Use::Copy => {
                    let kind = file.kind();
                    let made = cx
                        .background_spawn(async move {
                            let bytes = std::fs::read(&kept).map_err(|error| {
                                log::warn!("couldn't read a file to copy it: {error}");
                                None
                            })?;
                            clipboard_item(kind, bytes).map_err(Some)
                        })
                        .await;
                    update_or_log(&this, cx, |store, cx| match made {
                        Ok(item) => {
                            cx.write_to_clipboard(item);
                            store.show_file_note("Copied".to_owned(), false, cx);
                        }
                        Err(why) => {
                            let said =
                                why.unwrap_or_else(|| format!("Couldn't copy {}", file.name));
                            store.show_file_note(said, true, cx);
                        }
                    });
                }
            }
        })
        .detach();
    }
}

/// What Copy puts on the clipboard for a file of `kind` made of `bytes`: a picture as one, else
/// its text. A HEIC picture is turned into one the clipboard's readers take
/// ([`images::readable`]), so it may decode the whole picture: call it off the UI thread.
///
/// # Errors
///
/// A picture this device can't read, worded for the user.
fn clipboard_item(kind: Kind, bytes: Vec<u8>) -> Result<ClipboardItem, String> {
    if kind == Kind::Picture {
        images::readable(bytes).map(|image| ClipboardItem::new_image(&image))
    } else {
        Ok(ClipboardItem::new_string(
            String::from_utf8_lossy(&bytes).into_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::sync::Arc;

    use gpui_kit::{ClipboardEntry, TestAppContext};

    use super::*;
    use crate::backend::Backend;
    use crate::backend::fake::FakeBackend;
    use crate::store::files::status::Note;

    #[gpui_kit::test]
    fn a_text_file_is_copied_as_text_and_the_line_says_so(cx: &mut TestAppContext) {
        let root = tempfile::tempdir().unwrap();
        let backend = Arc::new(FakeBackend::new([]));
        backend.push_download(Ok(b"# Plan\nShip it".to_vec()));
        let opened = Rc::new(RefCell::new(Vec::new()));
        let store = cx.new(|_| {
            let mut store = Store::for_tests(Arc::clone(&backend) as Arc<dyn Backend>);
            store.keep_files_in(root.path(), &opened);
            store
        });
        let file = RemoteFile::new("/uploads/plan.md", "plan.md");
        store.update(cx, |store, cx| store.use_file(file, Use::Copy, cx));
        cx.run_until_parked();
        assert_eq!(
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some("# Plan\nShip it")
        );
        store.read_with(cx, |store, _| {
            assert_eq!(
                store.file_status().map(Note::text).as_deref(),
                Some("Copied")
            );
        });
    }

    #[test]
    fn a_picture_is_copied_as_a_picture() {
        let png = b"\x89PNG\r\n\x1a\n rest".to_vec();
        let item = clipboard_item(Kind::Picture, png).unwrap();
        assert!(matches!(item.entries(), [ClipboardEntry::Image(_)]));
        let text = clipboard_item(Kind::Note, b"hi".to_vec()).unwrap();
        assert_eq!(text.text().as_deref(), Some("hi"));
    }

    #[test]
    fn a_heic_picture_is_never_copied_as_heic_bytes() {
        let heic = b"\0\0\0\x18ftypheic\0\0\0\0".to_vec();
        match clipboard_item(Kind::Picture, heic) {
            Ok(item) => match item.entries() {
                [ClipboardEntry::Image(image)] => {
                    assert_eq!(image.format, gpui_kit::ImageFormat::Tiff);
                    assert!(!image.bytes.windows(4).any(|w| w == b"ftyp"), "converted");
                }
                other => panic!("not a picture: {other:?}"),
            },
            Err(why) => assert_eq!(why, "This device can't show HEIC pictures"),
        }
    }
}
