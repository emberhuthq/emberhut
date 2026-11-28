//! What the macOS app is called, and where its build, bundle and zip live.

use std::path::{Path, PathBuf};

use crate::workspace::Workspace;

/// The name shown in Finder, the Dock and the menu bar (`CFBundleName`, `CFBundleDisplayName`).
pub(crate) const DISPLAY_NAME: &str = "Emberhut";
/// The bundle identifier (`CFBundleIdentifier`); it is what makes an installed Emberhut.app ours.
pub(crate) const BUNDLE_ID: &str = "com.emberhut.desktop";
/// The bundle directory's name.
pub(crate) const BUNDLE_DIR: &str = "Emberhut.app";
/// The cargo package that is the app.
pub(crate) const PACKAGE: &str = "emberhut-desktop";
/// The app's binary, in `Contents/MacOS` (`CFBundleExecutable`).
pub(crate) const EXECUTABLE: &str = "emberhut-desktop";
/// The cargo package of the hut, the server the app runs on this Mac (ADR 0030).
pub(crate) const SERVER_PACKAGE: &str = "emberhut-server";
/// The hut's binary, in `Contents/MacOS` beside the app's own, where the app looks for it.
pub(crate) const SERVER_EXECUTABLE: &str = "emberhut-server";
/// The cargo feature for the automation build.
pub(crate) const AUTOMATION_FEATURE: &str = "automation";
/// The app's own link scheme (`emberhut://…`), registered in `CFBundleURLTypes`.
pub(crate) const URL_SCHEME: &str = "emberhut";
/// The icon's file name in `Contents/Resources`, without `.icns` (`CFBundleIconFile`).
pub(crate) const ICON_NAME: &str = "AppIcon";
/// The 1024 px source of the icon, relative to the workspace root.
pub(crate) const ICON_SOURCE: &str = "apps/desktop/assets/AppIcon-1024.png";
/// The app's version: the workspace's, which xtask shares.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The paths `bundle-macos` reads and writes, all inside the workspace and its target dir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Layout {
    workspace: Workspace,
}

impl Layout {
    /// The layout of `bundle-macos` in `workspace`.
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

    /// The icon's 1024 px source image.
    #[must_use]
    pub(crate) fn icon_source(&self) -> PathBuf {
        self.root().join(ICON_SOURCE)
    }

    /// The directory the bundle, its zip and the scratch files go to.
    #[must_use]
    pub(crate) fn out_dir(&self) -> PathBuf {
        self.target().join("bundle")
    }

    /// The app bundle, `<target>/bundle/Emberhut.app`.
    #[must_use]
    pub(crate) fn app_bundle(&self) -> PathBuf {
        self.out_dir().join(BUNDLE_DIR)
    }

    /// The hut's binary inside the bundle, `Emberhut.app/Contents/MacOS/emberhut-server`.
    #[must_use]
    pub(crate) fn bundled_server(&self) -> PathBuf {
        self.app_bundle()
            .join("Contents")
            .join("MacOS")
            .join(SERVER_EXECUTABLE)
    }

    /// Scratch directory for the icon's sizes.
    #[must_use]
    pub(crate) fn iconset(&self) -> PathBuf {
        self.out_dir().join(format!("{ICON_NAME}.iconset"))
    }

    /// The zip uploaded to the notary service.
    #[must_use]
    pub(crate) fn notary_upload(&self) -> PathBuf {
        self.out_dir().join("notary-upload.zip")
    }

    /// The notary service's last answer, kept for reading after a failure.
    #[must_use]
    pub(crate) fn notary_log(&self) -> PathBuf {
        self.out_dir().join("notary-log.txt")
    }

    /// The zip to hand out, `<target>/bundle/Emberhut-<version>-macos-<arch>.zip`.
    #[must_use]
    pub(crate) fn zip(&self, version: &str, arch: &str) -> PathBuf {
        self.out_dir().join(zip_name(version, arch))
    }

    /// The disk image to hand out, `<target>/bundle/Emberhut-<version>-macos-<arch>.dmg`.
    #[must_use]
    pub(crate) fn dmg(&self, version: &str, arch: &str) -> PathBuf {
        self.out_dir().join(dmg_name(version, arch))
    }

    /// The folder the disk image is made from: the app and an /Applications link.
    #[must_use]
    pub(crate) fn dmg_staging(&self) -> PathBuf {
        self.out_dir().join("dmg")
    }

    /// Where a universal build's joined binaries go, `<target>/universal-apple-darwin/<profile>`,
    /// beside cargo's per-target directories.
    #[must_use]
    pub(crate) fn universal_dir(&self, profile: &str) -> PathBuf {
        self.target().join("universal-apple-darwin").join(profile)
    }

    /// The scratch folder the standalone server is signed and packed in.
    #[must_use]
    pub(crate) fn server_dir(&self) -> PathBuf {
        self.out_dir().join("server")
    }
}

/// The architecture as Apple names it in file names: `arm64` for Rust's `aarch64`.
#[must_use]
pub(crate) fn arch_label(rust_arch: &str) -> &str {
    match rust_arch {
        "aarch64" => "arm64",
        other => other,
    }
}

/// The handed-out zip's file name, `Emberhut-<version>-macos-<arch>.zip`.
#[must_use]
pub(crate) fn zip_name(version: &str, arch: &str) -> String {
    format!("{DISPLAY_NAME}-{version}-macos-{arch}.zip")
}

/// The disk image's file name, `Emberhut-<version>-macos-<arch>.dmg`.
#[must_use]
pub(crate) fn dmg_name(version: &str, arch: &str) -> String {
    format!("{DISPLAY_NAME}-{version}-macos-{arch}.dmg")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{BUNDLE_DIR, DISPLAY_NAME, Layout, arch_label, dmg_name, zip_name};
    use crate::workspace::Workspace;

    fn layout() -> Layout {
        Layout::new(Workspace::new(
            "/work/emberhut".into(),
            "/work/emberhut/target".into(),
        ))
    }

    #[test]
    fn the_bundle_is_named_after_the_app() {
        assert_eq!(BUNDLE_DIR, format!("{DISPLAY_NAME}.app"));
        assert_eq!(
            layout().app_bundle(),
            Path::new("/work/emberhut/target/bundle/Emberhut.app")
        );
    }

    #[test]
    fn the_zip_names_version_and_arch() {
        assert_eq!(zip_name("0.1.0", "arm64"), "Emberhut-0.1.0-macos-arm64.zip");
        assert_eq!(
            layout().zip("1.2.3", "x86_64"),
            Path::new("/work/emberhut/target/bundle/Emberhut-1.2.3-macos-x86_64.zip")
        );
    }

    #[test]
    fn the_disk_image_and_the_universal_binaries_have_fixed_places() {
        assert_eq!(
            dmg_name("0.1.0-rc.1", "universal"),
            "Emberhut-0.1.0-rc.1-macos-universal.dmg"
        );
        let layout = layout();
        assert_eq!(
            layout.dmg("0.1.0", "arm64"),
            Path::new("/work/emberhut/target/bundle/Emberhut-0.1.0-macos-arm64.dmg")
        );
        assert_eq!(
            layout.dmg_staging(),
            Path::new("/work/emberhut/target/bundle/dmg")
        );
        assert_eq!(
            layout.universal_dir("dist"),
            Path::new("/work/emberhut/target/universal-apple-darwin/dist")
        );
    }

    #[test]
    fn aarch64_is_called_arm64() {
        assert_eq!(arch_label("aarch64"), "arm64");
        assert_eq!(arch_label("x86_64"), "x86_64");
    }

    #[test]
    fn the_icon_and_scratch_files_have_fixed_places() {
        let layout = layout();
        assert_eq!(
            layout.icon_source(),
            Path::new("/work/emberhut/apps/desktop/assets/AppIcon-1024.png")
        );
        assert_eq!(
            layout.iconset(),
            Path::new("/work/emberhut/target/bundle/AppIcon.iconset")
        );
        assert_eq!(
            layout.bundled_server(),
            Path::new("/work/emberhut/target/bundle/Emberhut.app/Contents/MacOS/emberhut-server")
        );
        assert_eq!(
            layout.notary_upload(),
            Path::new("/work/emberhut/target/bundle/notary-upload.zip")
        );
    }
}
