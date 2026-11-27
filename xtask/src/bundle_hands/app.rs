//! What the helper is called, and where its bundle, entitlements and zip are made.

use std::path::{Path, PathBuf};

use crate::workspace::Workspace;

/// The name shown in System Settings' privacy lists.
pub(crate) const DISPLAY_NAME: &str = "Emberhut Hands";
/// The bundle id: the one the Mac's grants belong to.
pub(crate) const BUNDLE_ID: &str = "com.emberhut.hands";
/// The bundle directory's name.
pub(crate) const BUNDLE_DIR: &str = "Emberhut Hands.app";
/// The cargo package.
pub(crate) const PACKAGE: &str = "emberhut-hands";
/// The binary in `Contents/MacOS`.
pub(crate) const EXECUTABLE: &str = "emberhut-hands";
/// The launch agent's label (and its plist's name).
pub(crate) const LABEL: &str = "com.emberhut.hands";
/// The helper's version: the workspace's, which xtask shares.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The paths `bundle-hands` writes, inside the target dir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Layout {
    workspace: Workspace,
}

impl Layout {
    /// The layout in `workspace`.
    #[must_use]
    pub(crate) fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }

    /// The workspace root.
    #[must_use]
    pub(crate) fn root(&self) -> &Path {
        self.workspace.root()
    }

    /// cargo's target directory.
    #[must_use]
    pub(crate) fn target(&self) -> &Path {
        self.workspace.target()
    }

    /// Where the bundle, its entitlements and its zip go.
    #[must_use]
    pub(crate) fn out_dir(&self) -> PathBuf {
        self.target().join("bundle")
    }

    /// `<target>/bundle/Emberhut Hands.app`.
    #[must_use]
    pub(crate) fn app_bundle(&self) -> PathBuf {
        self.out_dir().join(BUNDLE_DIR)
    }

    /// The entitlements file it is signed with.
    #[must_use]
    pub(crate) fn entitlements(&self) -> PathBuf {
        self.out_dir().join("emberhut-hands.entitlements")
    }

    /// `<target>/bundle/Emberhut-Hands-<version>-macos-<arch>.zip`.
    #[must_use]
    pub(crate) fn zip(&self) -> PathBuf {
        let arch = match std::env::consts::ARCH {
            "aarch64" => "arm64",
            other => other,
        };
        self.out_dir()
            .join(format!("Emberhut-Hands-{VERSION}-macos-{arch}.zip"))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{BUNDLE_DIR, DISPLAY_NAME, Layout};
    use crate::workspace::Workspace;

    #[test]
    fn the_bundle_and_its_files_sit_in_the_target_dir() {
        let layout = Layout::new(Workspace::new("/w".into(), "/w/target".into()));
        assert_eq!(BUNDLE_DIR, format!("{DISPLAY_NAME}.app"));
        assert_eq!(
            layout.app_bundle(),
            Path::new("/w/target/bundle/Emberhut Hands.app")
        );
        assert_eq!(
            layout.entitlements(),
            Path::new("/w/target/bundle/emberhut-hands.entitlements")
        );
        assert!(
            layout
                .zip()
                .to_string_lossy()
                .starts_with("/w/target/bundle/Emberhut-Hands-")
        );
    }
}
