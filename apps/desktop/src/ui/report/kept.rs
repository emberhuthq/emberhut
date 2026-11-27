//! The panel's width kept between launches: a share of the window in the app's `layout.json`
//! ([`AppFiles::layout`]), read and written as [`json_file`] does. The file is read once,
//! before the run loop starts, and written whenever the user changes the width. A file that is
//! missing, unreadable, not JSON, or holds a share out of range gives the default width (each
//! but a missing file is logged). Keys this build doesn't know are written back as they were
//! read, so a newer build's settings survive an older one.

use gpui_kit::{App, AppContext as _};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::width::PanelWidth;
use crate::config::AppFiles;
use crate::json_file::{self, Writes};

/// `layout.json`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct LayoutFile {
    /// The report panel's width: its share of the window.
    report_width: Option<f32>,
    /// The keys this build doesn't know.
    #[serde(flatten)]
    other: Map<String, Value>,
}

/// The kept width: the file as it was read, and the writes on their way to it.
#[derive(Debug)]
pub(super) struct KeptWidth {
    files: AppFiles,
    file: LayoutFile,
    writes: Writes,
}

impl KeptWidth {
    /// Reads the width kept in `files`. Reads a file: call it before the app's run loop starts.
    #[must_use]
    pub(super) fn load(files: AppFiles) -> Self {
        let file = json_file::read(
            &files.layout(),
            "a layout",
            "the report panel opens at its default width",
        )
        .unwrap_or_default();
        Self {
            files,
            file,
            writes: Writes::default(),
        }
    }

    /// The width kept, else [`PanelWidth::DEFAULT`].
    #[must_use]
    pub(super) fn width(&self) -> PanelWidth {
        let Some(share) = self.file.report_width else {
            return PanelWidth::DEFAULT;
        };
        PanelWidth::from_share(share).unwrap_or_else(|| {
            log::warn!(
                "{}'s report width ({share}) isn't a share of the window; the report panel opens \
                 at its default width",
                self.files.layout().display()
            );
            PanelWidth::DEFAULT
        })
    }

    /// Keeps `width`, written in the background.
    pub(super) fn keep(&mut self, width: PanelWidth, cx: &App) {
        self.file.report_width = Some(width.share());
        let (number, written) = self.writes.next();
        let (path, file) = (self.files.layout(), self.file.clone());
        cx.background_spawn(async move {
            json_file::write(&path, &file, number, &written, "the layout");
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    fn width(share: f32) -> PanelWidth {
        PanelWidth::from_share(share).unwrap()
    }

    /// Writes `file` to `files`' layout, as a write from [`KeptWidth::keep`] does.
    fn write(files: &AppFiles, file: &LayoutFile) {
        json_file::write(&files.layout(), file, 1, &Mutex::new(0), "the layout");
    }

    #[test]
    fn a_kept_width_is_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(&dir.path().join("emberhut"));
        assert_eq!(
            KeptWidth::load(files.clone()).width(),
            PanelWidth::DEFAULT,
            "nothing kept yet"
        );
        let file = LayoutFile {
            report_width: Some(0.6),
            other: Map::new(),
        };
        write(&files, &file);
        assert_eq!(KeptWidth::load(files).width(), width(0.6));
    }

    #[test]
    fn a_broken_file_gives_the_default_width() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(dir.path());
        for broken in [
            "",
            "{",
            "not json",
            r#"{"report_width": "wide"}"#,
            r#"{"report_width": null}"#,
            "0.5",
        ] {
            std::fs::write(files.layout(), broken).unwrap();
            assert_eq!(
                KeptWidth::load(files.clone()).width(),
                PanelWidth::DEFAULT,
                "{broken}"
            );
        }
    }

    #[test]
    fn a_width_out_of_range_gives_the_default_width() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(dir.path());
        for out_of_range in ["0", "-0.3", "1.5", "40", "1e39"] {
            let text = format!(r#"{{"report_width": {out_of_range}}}"#);
            std::fs::write(files.layout(), text).unwrap();
            assert_eq!(
                KeptWidth::load(files.clone()).width(),
                PanelWidth::DEFAULT,
                "{out_of_range}"
            );
        }
    }

    #[test]
    fn keys_this_build_doesnt_know_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(dir.path());
        let text = r#"{"sidebar": {"open": true}, "report_width": 0.25}"#;
        std::fs::write(files.layout(), text).unwrap();
        let kept = KeptWidth::load(files.clone());
        assert_eq!(kept.width(), width(0.25));

        // What `keep` writes: the new width, the other keys as they were.
        let mut file = kept.file.clone();
        file.report_width = Some(0.5);
        write(&files, &file);
        let written: Value =
            serde_json::from_slice(&std::fs::read(files.layout()).unwrap()).unwrap();
        assert_eq!(
            written,
            serde_json::json!({"sidebar": {"open": true}, "report_width": 0.5})
        );
    }

    #[gpui_kit::test]
    fn keeping_writes_the_width_and_the_other_keys(cx: &mut gpui_kit::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(dir.path());
        std::fs::write(files.layout(), r#"{"later": 1}"#).unwrap();
        let mut kept = KeptWidth::load(files.clone());
        cx.update(|cx| {
            kept.keep(width(0.3), cx);
            kept.keep(width(0.7), cx);
        });
        cx.run_until_parked();
        let reread = KeptWidth::load(files);
        assert_eq!(reread.width(), width(0.7), "the newest write is on disk");
        assert_eq!(reread.file.other.get("later"), Some(&Value::from(1)));
    }
}
