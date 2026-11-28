//! Building the app's binary with cargo, at low priority.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context as _;

use crate::bundle_macos::app::{
    AUTOMATION_FEATURE, EXECUTABLE, Layout, PACKAGE, SERVER_EXECUTABLE, SERVER_PACKAGE,
};
use crate::bundle_macos::plan::{BuildProfile, BundlePlan, UNIVERSAL_TARGETS};
use crate::bundle_macos::tool;
use crate::{cmd, say, workspace};

/// How much `nice` lowers the build's priority: the Mac is usually doing other work too.
const NICENESS: &str = "10";

/// The `nice … cargo build …` command for `plan`, run in the workspace root and building into
/// the layout's target directory: the app, and the hut's server with it unless `--no-hut` (one
/// cargo run, so the crates they share build once). Diagnostics go to the terminal; stdout
/// carries cargo's JSON messages, which name the binaries it built.
#[must_use]
pub(crate) fn build_command(cargo: OsString, plan: &BundlePlan, layout: &Layout) -> Command {
    // Named with its package: with the server in the same build, a bare name would be asked
    // of both packages.
    let feature = format!("{PACKAGE}/{AUTOMATION_FEATURE}");
    let feature = plan.automation.then_some(feature.as_str());
    let packages: &[&str] = if plan.hut {
        &[PACKAGE, SERVER_PACKAGE]
    } else {
        &[PACKAGE]
    };
    let mut command = cargo_build(
        cargo,
        &CargoBuild {
            packages,
            profile: plan.profile,
            jobs: plan.jobs,
            feature,
            targets: plan.targets(),
        },
        layout.root(),
        layout.target(),
    );
    // A release build: the app reads both with `option_env!`.
    if let Some((version, build)) = &plan.version.release {
        command
            .env("EMBERHUT_VERSION", version)
            .env("EMBERHUT_BUILD", build.to_string());
    }
    command
}

/// What a `cargo build` for a bundle builds.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CargoBuild<'a> {
    /// The packages.
    pub(crate) packages: &'a [&'a str],
    /// The profile.
    pub(crate) profile: BuildProfile,
    /// How many compile jobs at once.
    pub(crate) jobs: u16,
    /// A feature to turn on.
    pub(crate) feature: Option<&'a str>,
    /// The `--target`s to build for; none builds for the host.
    pub(crate) targets: &'a [&'a str],
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
    command.args(["-n", NICENESS]).arg(cargo).arg("build");
    for package in build.packages {
        command.args(["--package", package]);
    }
    command
        .args([
            "--profile",
            build.profile.name(),
            "--message-format=json-render-diagnostics",
        ])
        .args(["--jobs", &build.jobs.to_string()]);
    if let Some(feature) = build.feature {
        command.args(["--features", feature]);
    }
    for target in build.targets {
        command.args(["--target", target]);
    }
    command
        .arg("--locked")
        .current_dir(root)
        .env("CARGO_TARGET_DIR", target);
    command
}

/// The binaries a bundle is made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Built {
    /// The app's binary.
    pub(crate) app: PathBuf,
    /// The hut's server, unless `--no-hut`.
    pub(crate) server: Option<PathBuf>,
}

/// Builds the app (and the hut's server with it) as `plan` says and returns the binaries'
/// paths.
///
/// # Errors
///
/// Fails when cargo fails or doesn't report a binary.
pub(crate) fn build(plan: &BundlePlan, layout: &Layout) -> anyhow::Result<Built> {
    let features = if plan.automation {
        " with automation"
    } else {
        ""
    };
    let hut = if plan.hut {
        format!(" and {SERVER_PACKAGE}")
    } else {
        String::new()
    };
    say(&format!(
        "Building {PACKAGE}{hut} ({}{features}, {} jobs, low priority)",
        plan.profile, plan.jobs
    ));
    let messages = cmd::stdout(&mut build_command(workspace::cargo(), plan, layout))?;
    let binary = |name: &str, package: &str| {
        if plan.universal {
            universal(&messages, name, &layout.universal_dir(plan.profile.name()))
        } else {
            executable(&messages, name)
                .with_context(|| format!("cargo built {package} but named no `{name}` binary"))
        }
    };
    let app = binary(EXECUTABLE, PACKAGE)?;
    let server = if plan.hut {
        Some(binary(SERVER_EXECUTABLE, SERVER_PACKAGE)?)
    } else {
        None
    };
    Ok(Built { app, server })
}

/// Joins the binary `name` cargo built for each of [`UNIVERSAL_TARGETS`] into `dir/name` and
/// checks it holds both architectures.
fn universal(messages: &str, name: &str, dir: &Path) -> anyhow::Result<PathBuf> {
    let parts = UNIVERSAL_TARGETS
        .iter()
        .map(|target| {
            executable_for(messages, name, target)
                .with_context(|| format!("cargo named no `{name}` binary for {target}"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let out = dir.join(name);
    say(&format!("Joining {name} for arm64 and x86_64 with lipo"));
    cmd::run(&mut lipo_command(&parts, &out))?;
    cmd::run(
        Command::new(tool::LIPO)
            .arg(&out)
            .args(["-verify_arch", "arm64", "x86_64"]),
    )?;
    Ok(out)
}

/// The `lipo` command that joins `parts` into the universal binary `out`.
#[must_use]
pub(crate) fn lipo_command(parts: &[PathBuf], out: &Path) -> Command {
    let mut command = Command::new(tool::LIPO);
    command.arg("-create").arg("-output").arg(out).args(parts);
    command
}

/// The path of the binary target `name` cargo built for `target` (its output directory, under
/// `<target dir>/<target>/`), in cargo's `--message-format=json` output.
#[must_use]
pub(crate) fn executable_for(messages: &str, name: &str, target: &str) -> Option<PathBuf> {
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
        .filter_map(|message| message["executable"].as_str().map(PathBuf::from))
        .find(|path| path.components().any(|part| part.as_os_str() == target))
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

    use super::{build_command, executable, executable_for, lipo_command};
    use crate::bundle_macos::app::Layout;
    use crate::bundle_macos::args::BundleArgs;
    use crate::bundle_macos::plan::{BundlePlan, BundleVersion};
    use crate::cmd::describe;
    use crate::release::tag::Tag;
    use crate::workspace::Workspace;

    fn plan(dist: bool, automation: bool) -> BundlePlan {
        BundleArgs {
            dist,
            automation,
            jobs: 3,
            no_hut: dist,
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
            "-n 10 cargo build --package emberhut-desktop --package emberhut-server \
             --profile release --message-format=json-render-diagnostics --jobs 3 --locked"
        );
    }

    #[test]
    fn a_dist_automation_build_without_the_hut() {
        assert_eq!(
            args(&plan(true, true)).join(" "),
            "-n 10 cargo build --package emberhut-desktop --profile dist \
             --message-format=json-render-diagnostics --jobs 3 \
             --features emberhut-desktop/automation --locked"
        );
    }

    #[test]
    fn an_automation_build_with_the_hut_names_the_apps_feature() {
        let plan = BundleArgs {
            automation: true,
            jobs: 3,
            ..BundleArgs::bare()
        }
        .plan()
        .unwrap();
        assert_eq!(
            args(&plan).join(" "),
            "-n 10 cargo build --package emberhut-desktop --package emberhut-server \
             --profile release --message-format=json-render-diagnostics --jobs 3 \
             --features emberhut-desktop/automation --locked"
        );
    }

    #[test]
    fn a_universal_release_build_names_both_targets_and_the_version() {
        let mut plan = BundleArgs {
            dist: true,
            universal: true,
            jobs: 3,
            ..BundleArgs::bare()
        }
        .plan()
        .unwrap();
        plan.version = BundleVersion::release(&Tag::parse("v0.1.0-rc.1").unwrap());
        let layout = Layout::new(Workspace::new("/w".into(), "/t".into()));
        let command = build_command("cargo".into(), &plan, &layout);
        assert!(
            describe(&command).ends_with(
                "--profile dist --message-format=json-render-diagnostics --jobs 3 \
                 --target aarch64-apple-darwin --target x86_64-apple-darwin --locked"
            ),
            "{}",
            describe(&command)
        );
        let env = |key: &str| {
            command
                .get_envs()
                .find(|(name, _)| *name == key)
                .and_then(|(_, value)| value)
                .map(|value| value.to_string_lossy().into_owned())
        };
        assert_eq!(env("EMBERHUT_VERSION").as_deref(), Some("0.1.0-rc.1"));
        assert_eq!(env("EMBERHUT_BUILD").as_deref(), Some("10001"));
        let development = build_command("cargo".into(), &plan_without_version(), &layout);
        assert!(
            development
                .get_envs()
                .all(|(name, _)| name != "EMBERHUT_BUILD")
        );
    }

    fn plan_without_version() -> BundlePlan {
        BundleArgs::bare().plan().unwrap()
    }

    #[test]
    fn each_targets_binary_is_found_and_joined() {
        let messages = [
            r#"{"reason":"compiler-artifact","target":{"kind":["bin"],"name":"emberhut-server"},"executable":"/t/aarch64-apple-darwin/dist/emberhut-server"}"#,
            r#"{"reason":"compiler-artifact","target":{"kind":["bin"],"name":"emberhut-server"},"executable":"/t/x86_64-apple-darwin/dist/emberhut-server"}"#,
        ]
        .join("\n");
        let arm = executable_for(&messages, "emberhut-server", "aarch64-apple-darwin").unwrap();
        let intel = executable_for(&messages, "emberhut-server", "x86_64-apple-darwin").unwrap();
        assert_eq!(
            arm,
            Path::new("/t/aarch64-apple-darwin/dist/emberhut-server")
        );
        assert_eq!(
            intel,
            Path::new("/t/x86_64-apple-darwin/dist/emberhut-server")
        );
        assert_eq!(
            executable_for(&messages, "emberhut-desktop", "aarch64-apple-darwin"),
            None
        );
        assert_eq!(
            describe(&lipo_command(
                &[arm, intel],
                Path::new("/t/universal-apple-darwin/dist/emberhut-server")
            )),
            "/usr/bin/lipo -create -output /t/universal-apple-darwin/dist/emberhut-server \
             /t/aarch64-apple-darwin/dist/emberhut-server /t/x86_64-apple-darwin/dist/emberhut-server"
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
