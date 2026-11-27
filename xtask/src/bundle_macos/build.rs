//! Building the app's binary with cargo, at low priority.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context as _;

use crate::bundle_macos::app::{AUTOMATION_FEATURE, EXECUTABLE, Layout, PACKAGE};
use crate::bundle_macos::plan::{BuildProfile, BundlePlan};
use crate::bundle_macos::tool;
use crate::{cmd, say, workspace};

/// How much `nice` lowers the build's priority: the Mac is usually doing other work too.
const NICENESS: &str = "10";

/// The `nice … cargo build …` command for `plan`, run in the workspace root and building into
/// the layout's target directory. Diagnostics go to the terminal; stdout carries cargo's JSON
/// messages, which name the binary it built.
#[must_use]
pub(crate) fn build_command(cargo: OsString, plan: &BundlePlan, layout: &Layout) -> Command {
    let feature = plan.automation.then_some(AUTOMATION_FEATURE);
    cargo_build(
        cargo,
        &CargoBuild {
            package: PACKAGE,
            profile: plan.profile,
            jobs: plan.jobs,
            feature,
        },
        layout.root(),
        layout.target(),
    )
}

/// What a `cargo build` for a bundle builds.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CargoBuild<'a> {
    /// The package.
    pub(crate) package: &'a str,
    /// The profile.
    pub(crate) profile: BuildProfile,
    /// How many compile jobs at once.
    pub(crate) jobs: u16,
    /// A feature to turn on.
    pub(crate) feature: Option<&'a str>,
}

/// The `nice … cargo build …` command for `build`, run in `root` and building into `target`.
/// Diagnostics go to the terminal; stdout carries cargo's JSON messages, which name the binary
/// it built.
#[must_use]
pub(crate) fn cargo_build(
    cargo: OsString,
    build: &CargoBuild<'_>,
    root: &Path,
    target: &Path,
) -> Command {
    let mut command = Command::new(tool::NICE);
    command
        .args(["-n", NICENESS])
        .arg(cargo)
        .args([
            "build",
            "--package",
            build.package,
            "--profile",
            build.profile.name(),
            "--message-format=json-render-diagnostics",
        ])
        .args(["--jobs", &build.jobs.to_string()]);
    if let Some(feature) = build.feature {
        command.args(["--features", feature]);
    }
    command
        .arg("--locked")
        .current_dir(root)
        .env("CARGO_TARGET_DIR", target);
    command
}

/// Builds the app as `plan` says and returns the binary's path.
///
/// # Errors
///
/// Fails when cargo fails or doesn't report the binary.
pub(crate) fn build(plan: &BundlePlan, layout: &Layout) -> anyhow::Result<PathBuf> {
    let features = if plan.automation {
        " with automation"
    } else {
        ""
    };
    say(&format!(
        "Building {PACKAGE} ({}{features}, {} jobs, low priority)",
        plan.profile, plan.jobs
    ));
    let messages = cmd::stdout(&mut build_command(workspace::cargo(), plan, layout))?;
    executable(&messages, EXECUTABLE)
        .with_context(|| format!("cargo built {PACKAGE} but named no `{EXECUTABLE}` binary"))
}

/// The path of the binary target `name` in cargo's `--message-format=json` output: wherever
/// cargo put it, whatever `--target` or target directory its config chose.
#[must_use]
pub(crate) fn executable(messages: &str, name: &str) -> Option<PathBuf> {
    messages
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| message["reason"] == "compiler-artifact")
        .filter(|message| message["target"]["name"] == name)
        .filter(|message| {
            message["target"]["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin"))
        })
        .find_map(|message| message["executable"].as_str().map(PathBuf::from))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;

    use super::{build_command, executable};
    use crate::bundle_macos::app::Layout;
    use crate::bundle_macos::args::BundleArgs;
    use crate::bundle_macos::plan::BundlePlan;
    use crate::workspace::Workspace;

    fn plan(dist: bool, automation: bool) -> BundlePlan {
        BundleArgs {
            dist,
            automation,
            jobs: 3,
            ..BundleArgs::bare()
        }
        .plan()
        .unwrap()
    }

    fn args(plan: &BundlePlan) -> Vec<String> {
        let layout = Layout::new(Workspace::new(
            "/work/emberhut".into(),
            "/cache/target".into(),
        ));
        let command = build_command("cargo".into(), plan, &layout);
        assert_eq!(command.get_program(), "/usr/bin/nice");
        assert_eq!(command.get_current_dir(), Some(Path::new("/work/emberhut")));
        let target = command
            .get_envs()
            .find(|(key, _)| *key == "CARGO_TARGET_DIR")
            .and_then(|(_, value)| value);
        assert_eq!(target, Some(OsStr::new("/cache/target")));
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn a_release_build() {
        assert_eq!(
            args(&plan(false, false)).join(" "),
            "-n 10 cargo build --package emberhut-desktop --profile release \
             --message-format=json-render-diagnostics --jobs 3 --locked"
        );
    }

    #[test]
    fn a_dist_automation_build() {
        assert_eq!(
            args(&plan(true, true)).join(" "),
            "-n 10 cargo build --package emberhut-desktop --profile dist \
             --message-format=json-render-diagnostics --jobs 3 --features automation --locked"
        );
    }

    #[test]
    fn the_binary_is_read_from_cargos_messages() {
        let messages = [
            r#"{"reason":"compiler-artifact","target":{"kind":["lib"],"name":"emberhut_api"},"executable":null}"#,
            "not json",
            r#"{"reason":"compiler-artifact","target":{"kind":["bin"],"name":"other"},"executable":"/t/other"}"#,
            r#"{"reason":"compiler-artifact","target":{"kind":["bin"],"name":"emberhut-desktop"},"executable":"/t/aarch64-apple-darwin/release/emberhut-desktop"}"#,
            r#"{"reason":"build-finished","success":true}"#,
        ]
        .join("\n");
        assert_eq!(
            executable(&messages, "emberhut-desktop").as_deref(),
            Some(Path::new(
                "/t/aarch64-apple-darwin/release/emberhut-desktop"
            ))
        );
        assert_eq!(executable(&messages, "missing"), None);
        assert_eq!(executable("", "emberhut-desktop"), None);
    }
}
