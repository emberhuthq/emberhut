//! What the Sessions screen keeps on this device (`sessions.json` in the app's directory): its
//! last filter, so it opens as it was left, and the machine, model and effort the last new
//! session was opened with, so the sheet starts on them.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::filter::SessionFilter;
use crate::config::AppFiles;
use crate::json_file;

/// The new-session sheet's remembered choices.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SheetChoices {
    /// The runner.
    pub(crate) machine: String,
    /// The model's id.
    pub(crate) model: String,
    /// The effort's word.
    pub(crate) effort: String,
}

impl Default for SheetChoices {
    /// The Linux PC, Opus 5.5, high effort: the reference client's first choices.
    fn default() -> Self {
        Self {
            machine: emberhut_api::runner::accounts::LINUX_RUNNER.to_owned(),
            model: super::words::MODELS[0].0.to_owned(),
            effort: "high".to_owned(),
        }
    }
}

/// The file's contents.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Kept {
    /// The Sessions screen's filter.
    #[serde(default)]
    pub(crate) filter: SessionFilter,
    /// The sheet's choices.
    #[serde(default)]
    pub(crate) sheet: SheetChoices,
}

fn path(files: &AppFiles) -> PathBuf {
    files.dir().join("sessions.json")
}

/// What was kept: the defaults when there is no file, or it doesn't read (then it is logged,
/// and the next change writes a good one).
#[must_use]
pub(crate) fn load(files: Option<&AppFiles>) -> Kept {
    files
        .and_then(|files| json_file::read(&path(files), "the sessions' choices", "starting afresh"))
        .unwrap_or_default()
}

/// Keeps `kept` on this device, as write `number` of the file (skipped once a newer one is
/// written: `written` holds the last one's number), whole through a temporary file; a failure
/// is logged (the choice still holds for this launch). It touches the disk: call it off the
/// UI thread.
pub(crate) fn save(files: &AppFiles, kept: &Kept, number: u64, written: &Mutex<u64>) {
    json_file::write(&path(files), kept, number, written, "the sessions' choices");
}

#[cfg(test)]
mod tests {
    use emberhut_api::ids::RoomId;

    use super::*;

    #[test]
    fn what_is_kept_reads_back_and_a_bad_file_is_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(&dir.path().join("emberhut"));
        assert_eq!(load(Some(&files)), Kept::default());
        let kept = Kept {
            filter: SessionFilter {
                room: Some(RoomId::new(2)),
                ..SessionFilter::default()
            },
            sheet: SheetChoices {
                machine: "runner-b".into(),
                ..SheetChoices::default()
            },
        };
        save(&files, &kept, 2, &Mutex::new(0));
        assert_eq!(load(Some(&files)), kept);
        let older = Kept::default();
        save(&files, &older, 1, &Mutex::new(2));
        assert_eq!(
            load(Some(&files)),
            kept,
            "an older write never replaces a newer one"
        );
        std::fs::write(path(&files), b"not json").unwrap();
        assert_eq!(load(Some(&files)), Kept::default());
        assert_eq!(load(None), Kept::default());
    }
}
