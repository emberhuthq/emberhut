//! `--self-check`: the release's smoke run of the exact app it ships. The app starts in a fresh,
//! private, empty data folder with no server (the session in memory, the address one where
//! nothing listens), opens its window without taking the focus, and exits 0 once the window has
//! drawn its first frame (the updater's "healthy"), or 1 after 60 seconds.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::{AnyWindowHandle, App};

use crate::config::{AppFiles, BadServerUrl, ServerUrl};

/// Where the self-check's server is: the discard port, where nothing answers.
const SERVER: &str = "http://127.0.0.1:9";

/// How long the window has to draw its first frame.
const WITHIN: Duration = Duration::from_mins(1);

/// A self-check under way: its data folder.
#[derive(Debug)]
pub(crate) struct SelfCheck {
    dir: PathBuf,
}

impl SelfCheck {
    /// Starts one: a new folder for its files.
    ///
    /// # Errors
    ///
    /// The folder couldn't be made.
    pub(crate) fn start() -> io::Result<Self> {
        let dir = std::env::temp_dir().join(format!("emberhut-self-check-{}", std::process::id()));
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(&dir)?;
        Ok(Self { dir })
    }

    /// The app's files, in the self-check's folder.
    #[must_use]
    pub(crate) fn files(&self) -> AppFiles {
        AppFiles::in_dir(&self.dir)
    }

    /// The server it talks to: none.
    ///
    /// # Errors
    ///
    /// Never, in practice: the address is a constant.
    pub(crate) fn server() -> Result<ServerUrl, BadServerUrl> {
        ServerUrl::parse(SERVER)
    }

    /// Exits 0 once the window `handle` has drawn its first frame, or 1 after a minute.
    pub(crate) fn exit_when_drawn(self, handle: AnyWindowHandle, cx: &mut App) {
        let dir = self.dir.clone();
        let watched = handle.update(cx, move |_, window, _| {
            window.on_next_frame(move |_, _| end(&dir, 0));
        });
        if let Err(error) = watched {
            log::error!("the self-check's window closed: {error}");
            end(&self.dir, 1);
        }
        cx.spawn(async move |cx| {
            cx.background_executor().timer(WITHIN).await;
            log::error!("No first frame in 60 s: is the display awake and the screen unlocked?");
            end(&self.dir, 1);
        })
        .detach();
    }
}

/// Removes the self-check's folder and exits with `code`.
fn end(dir: &std::path::Path, code: i32) -> ! {
    if let Err(error) = std::fs::remove_dir_all(dir) {
        log::warn!("couldn't remove the self-check's folder: {error}");
    }
    std::process::exit(code)
}
