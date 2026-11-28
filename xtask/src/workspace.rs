//! Where the workspace is, and where cargo puts what it builds.

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context as _;

use crate::cmd;

/// The workspace root and cargo's target directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Workspace {
    root: PathBuf,
    target: PathBuf,
}

impl Workspace {
    /// A workspace at `root` building into `target`.
    #[must_use]
    pub(crate) fn new(root: PathBuf, target: PathBuf) -> Self {
        Self { root, target }
    }

    /// This workspace: its root is xtask's parent directory, and its target directory is the
    /// one `cargo metadata` reports, so `CARGO_TARGET_DIR`, `--config` and `build.target-dir`
    /// in cargo's config files all count, exactly as they do for cargo itself.
    ///
    /// # Errors
    ///
    /// Fails when xtask's manifest directory has no parent, or `cargo metadata` fails or
    /// reports no target directory.
    pub(crate) fn discover() -> anyhow::Result<Self> {
        let root = Self::root_dir();
        // Run in the caller's directory, like the `cargo xtask` that started us: cargo finds
        // its config files from there and resolves a relative CARGO_TARGET_DIR against it.
        let mut metadata = Command::new(cargo());
        metadata
            .args([
                "metadata",
                "--format-version",
                "1",
                "--no-deps",
                "--manifest-path",
            ])
            .arg(root.join("Cargo.toml"));
        let json = cmd::stdout(&mut metadata)
            .context("asking cargo where the workspace's target directory is")?;
        let target = target_directory(&json).with_context(|| {
            format!(
                "`{}` reported no target_directory",
                cmd::describe(&metadata)
            )
        })?;
        Ok(Self::new(root, target))
    }

    /// The workspace root: xtask's parent directory, as compiled in.
    #[must_use]
    pub(crate) fn root_dir() -> PathBuf {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        manifest.parent().unwrap_or(manifest).to_path_buf()
    }

    /// The workspace root, where the top `Cargo.toml` is.
    #[must_use]
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// cargo's target directory.
    #[must_use]
    pub(crate) fn target(&self) -> &Path {
        &self.target
    }
}

/// The cargo that runs us (cargo sets `CARGO` for `cargo run`), else the one on `PATH`.
#[must_use]
pub(crate) fn cargo() -> OsString {
    env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}

/// The `target_directory` of `cargo metadata --format-version 1` output.
#[must_use]
pub(crate) fn target_directory(metadata_json: &str) -> Option<PathBuf> {
    let metadata: serde_json::Value = serde_json::from_str(metadata_json).ok()?;
    let dir = metadata.get("target_directory")?.as_str()?;
    (!dir.is_empty()).then(|| PathBuf::from(dir))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Workspace, target_directory};

    #[test]
    fn the_target_directory_comes_from_cargo_metadata() {
        let json = r#"{"packages":[],"target_directory":"/cache/target","version":1}"#;
        assert_eq!(
            target_directory(json).as_deref(),
            Some(Path::new("/cache/target"))
        );
    }

    #[test]
    fn metadata_without_a_target_directory_is_none() {
        assert_eq!(target_directory(r#"{"packages":[]}"#), None);
        assert_eq!(target_directory(r#"{"target_directory":""}"#), None);
        assert_eq!(target_directory(r#"{"target_directory":7}"#), None);
        assert_eq!(target_directory("not json"), None);
    }

    #[test]
    fn the_discovered_workspace_is_this_one() {
        let workspace = Workspace::discover().unwrap();
        assert!(workspace.root().join("Cargo.toml").is_file());
        assert!(workspace.root().join("xtask").is_dir());
        assert!(workspace.target().is_absolute());
    }
}
