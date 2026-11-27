//! `hut.json` in the hut's data folder: what the app remembers about the hut between runs.

use std::io::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::service::KeepLit;
use crate::private_files;

/// What the app remembers about its hut.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct HutState {
    /// The port it listens on, once chosen (another than the default when that one was taken
    /// by something that isn't this hut).
    pub(crate) port: Option<u16>,
    /// The server build it was last started with ([`super::LocalHut`]'s build stamp): a
    /// running hut of another build is restarted on the bundled one.
    pub(crate) server: Option<String>,
    /// Its process, when it runs as the app's child, so a later run of the app can stop it.
    pub(crate) pid: Option<u32>,
    /// Whether it stays lit while the app is closed, and from login.
    pub(crate) keep: KeepLit,
}

impl HutState {
    /// The state in `path`: the default when it is missing or doesn't read (logged).
    #[must_use]
    pub(crate) fn read(path: &Path) -> Self {
        crate::json_file::read(path, "the hut's state", "starting afresh").unwrap_or_default()
    }

    /// Writes the state to `path` whole, through a temporary file, for the user only.
    ///
    /// # Errors
    ///
    /// The file couldn't be written.
    pub(crate) fn write(&self, path: &Path) -> std::io::Result<()> {
        let temporary = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::from)?;
        private_files::file(&temporary)?.write_all(&bytes)?;
        std::fs::rename(&temporary, path)
    }
}
