//! `cargo xtask bundle-windows`: build the desktop app and the hut's server for 64-bit Windows and
//! package them as `target/bundle/Emberhut-<version>-windows-x64.msi` with the `WiX` Toolset v5
//! (`apps/desktop/wix/Emberhut.wxs`). Runs on Windows only.
//!
//! The version is `EMBERHUT_VERSION` (a release build; `EMBERHUT_MSI_VERSION` is then its
//! installer's `ProductVersion`, as `cargo xtask release version` prints both), else the
//! workspace's for a development build, whose `ProductVersion` is `X.Y.(Z*100)`: below every
//! release candidate of that version.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, bail};
use clap::Args;

use crate::bundle_macos::build::executable;
use crate::workspace::{self, Workspace};
use crate::{cmd, say};

/// The cargo package that is the app, and its binary.
const PACKAGE: &str = "emberhut-desktop";
/// The cargo package of the hut, the server the app runs beside itself (ADR 0030), and its
/// binary.
const SERVER_PACKAGE: &str = "emberhut-server";
/// The target the installer is for.
const TARGET: &str = "x86_64-pc-windows-msvc";
/// The installer's source, relative to the workspace root.
const WXS: &str = "apps/desktop/wix/Emberhut.wxs";
/// The icon of the Start menu shortcut and the installed programs list.
const ICON: &str = "apps/desktop/assets/AppIcon.ico";
/// The `WiX` Toolset major version the installer is written for. v5 is under the MS-RL; v6 adds
/// a maintenance fee to its terms, so it isn't used.
const WIX_MAJOR: &str = "5";
/// The workspace's version, which xtask shares.
const WORKSPACE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Options of `cargo xtask bundle-windows`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct WindowsArgs {
    /// Build with the `dist` profile (smaller, slower to build: a release's) instead of
    /// `release`.
    #[arg(long)]
    pub(crate) dist: bool,
}

/// The installer's versions: the app's (in the file name) and the MSI's `ProductVersion`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Versions {
    /// The full version, `X.Y.Z` or `X.Y.Z-rc.N`.
    pub(crate) version: String,
    /// The installer's `ProductVersion`, three numbers.
    pub(crate) msi: String,
}

impl Versions {
    /// The versions `EMBERHUT_VERSION` and `EMBERHUT_MSI_VERSION` (as `var` reads them) name,
    /// else the workspace's development build's.
    ///
    /// # Errors
    ///
    /// `EMBERHUT_VERSION` without `EMBERHUT_MSI_VERSION`, or an MSI version that isn't three
    /// numbers in MSI's ranges.
    pub(crate) fn from_vars(var: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let set = |name: &str| var(name).filter(|value| !value.is_empty());
        let (version, msi) = match (set("EMBERHUT_VERSION"), set("EMBERHUT_MSI_VERSION")) {
            (Some(version), Some(msi)) => (version, msi),
            (Some(version), None) => bail!(
                "EMBERHUT_VERSION is {version} but EMBERHUT_MSI_VERSION isn't set: \
                 `cargo xtask release version <tag>` prints both"
            ),
            (None, msi) => {
                let msi = match msi {
                    Some(msi) => msi,
                    None => development_msi_version(WORKSPACE_VERSION)?,
                };
                (WORKSPACE_VERSION.to_owned(), msi)
            }
        };
        check_msi_version(&msi)?;
        Ok(Self { version, msi })
    }
}

/// A development build's `ProductVersion`: `X.Y.(Z*100)`, below `X.Y.Z-rc.1`'s.
fn development_msi_version(version: &str) -> anyhow::Result<String> {
    let numbers: Vec<u32> = version
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .with_context(|| format!("the workspace version {version} isn't X.Y.Z"))?;
    let [major, minor, patch] = numbers[..] else {
        bail!("the workspace version {version} isn't X.Y.Z");
    };
    Ok(format!("{major}.{minor}.{}", patch * 100))
}

/// Refuses an MSI `ProductVersion` that isn't `X.Y.Z` with X and Y at most 255 and Z at most
/// 65535.
fn check_msi_version(msi: &str) -> anyhow::Result<()> {
    let fields: Vec<&str> = msi.split('.').collect();
    let limits = [255_u32, 255, 65_535];
    let fits = fields.len() == 3
        && fields.iter().zip(limits).all(|(field, limit)| {
            !field.is_empty()
                && field.bytes().all(|byte| byte.is_ascii_digit())
                && field.parse::<u32>().is_ok_and(|number| number <= limit)
        });
    if !fits {
        bail!("the MSI version {msi} isn't X.Y.Z with X, Y up to 255 and Z up to 65535");
    }
    Ok(())
}

/// The installer's file name.
#[must_use]
pub(crate) fn msi_name(version: &str) -> String {
    format!("Emberhut-{version}-windows-x64.msi")
}

/// Whether `wix --version`'s answer (`5.0.2+aa65968`) is the `WiX` Toolset v5.
#[must_use]
pub(crate) fn is_wix_5(answer: &str) -> bool {
    answer.trim().split('.').next() == Some(WIX_MAJOR)
}

/// What goes into the installer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Inputs {
    /// The installer's source.
    pub(crate) wxs: PathBuf,
    /// The app.
    pub(crate) desktop: PathBuf,
    /// The hut's server.
    pub(crate) server: PathBuf,
    /// The workspace root: the icon, the licences and the README.
    pub(crate) root: PathBuf,
}

/// The `wix build` command for `inputs`, writing the MSI of `versions` to `out`.
#[must_use]
pub(crate) fn wix_build(inputs: &Inputs, versions: &Versions, out: &Path) -> Command {
    let define = |name: &str, value: &Path| format!("{name}={}", value.display());
    let root = &inputs.root;
    let mut command = Command::new("wix");
    command
        .arg("build")
        .arg(&inputs.wxs)
        .args(["-arch", "x64", "-pdbtype", "none"])
        .args(["-d", &format!("Version={}", versions.msi)])
        .args(["-d", &define("DesktopExe", &inputs.desktop)])
        .args(["-d", &define("ServerExe", &inputs.server)])
        .args(["-d", &define("Icon", &root.join(ICON))])
        .args(["-d", &define("LicenseMit", &root.join("LICENSE-MIT"))])
        .args(["-d", &define("LicenseApache", &root.join("LICENSE-APACHE"))])
        .args(["-d", &define("Readme", &root.join("README.md"))])
        .arg("-o")
        .arg(out);
    command
}

/// The `cargo build` of the app and the server for [`TARGET`] with `profile`, in `root`.
#[must_use]
pub(crate) fn cargo_build(profile: &str, root: &Path, target: &Path) -> Command {
    let mut command = Command::new(workspace::cargo());
    command
        .args(["build", "--package", PACKAGE, "--package", SERVER_PACKAGE])
        .args(["--profile", profile, "--target", TARGET])
        .args(["--message-format=json-render-diagnostics", "--locked"])
        .current_dir(root)
        .env("CARGO_TARGET_DIR", target);
    command
}

/// Runs `bundle-windows` with `args`.
///
/// # Errors
///
/// Not on Windows, the versions are wrong, `WiX` v5 isn't installed, or a step fails.
pub(crate) fn run(args: &WindowsArgs) -> anyhow::Result<()> {
    if !cfg!(windows) {
        bail!(
            "bundle-windows builds the Windows installer and needs Windows, the MSVC build tools \
             and the WiX Toolset v5 (`dotnet tool install --global wix --version 5.0.2`); this \
             is {}",
            std::env::consts::OS
        );
    }
    let versions = Versions::from_vars(|name| std::env::var(name).ok())?;
    let wix = cmd::capture(Command::new("wix").arg("--version")).context(
        "the WiX Toolset isn't installed: `dotnet tool install --global wix --version 5.0.2`",
    )?;
    if !wix.status.success() || !is_wix_5(&wix.stdout) {
        bail!(
            "bundle-windows needs the WiX Toolset v5 (5.0.2), not `{}`: \
             `dotnet tool install --global wix --version 5.0.2`",
            wix.combined().trim()
        );
    }
    let workspace = Workspace::discover()?;
    let root = workspace.root();
    let profile = if args.dist { "dist" } else { "release" };
    say(&format!(
        "Building {PACKAGE} and {SERVER_PACKAGE} {} for {TARGET} ({profile})",
        versions.version
    ));
    let messages = cmd::stdout(&mut cargo_build(profile, root, workspace.target()))?;
    let built = |name: &str| {
        executable(&messages, name)
            .with_context(|| format!("cargo built {name} but named no `{name}` binary"))
    };
    let inputs = Inputs {
        wxs: root.join(WXS),
        desktop: built(PACKAGE)?,
        server: built(SERVER_PACKAGE)?,
        root: root.to_path_buf(),
    };
    let out_dir = workspace.target().join("bundle");
    std::fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    let msi = out_dir.join(msi_name(&versions.version));
    say(&format!(
        "Packaging {} (ProductVersion {})",
        msi.display(),
        versions.msi
    ));
    cmd::run(&mut wix_build(&inputs, &versions, &msi))?;
    say(&format!("Built {}", msi.display()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: BTreeMap<String, String> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn a_release_build_takes_both_versions_as_given() {
        let versions = Versions::from_vars(vars(&[
            ("EMBERHUT_VERSION", "0.1.0-rc.1"),
            ("EMBERHUT_MSI_VERSION", "0.1.1"),
        ]))
        .unwrap();
        assert_eq!(versions.version, "0.1.0-rc.1");
        assert_eq!(versions.msi, "0.1.1");
        assert_eq!(
            msi_name(&versions.version),
            "Emberhut-0.1.0-rc.1-windows-x64.msi"
        );
    }

    #[test]
    fn a_release_version_without_its_msi_version_is_refused() {
        let error = Versions::from_vars(vars(&[("EMBERHUT_VERSION", "0.1.0")])).unwrap_err();
        assert!(
            error.to_string().contains("EMBERHUT_MSI_VERSION"),
            "{error}"
        );
    }

    #[test]
    fn a_development_build_is_below_every_release_candidate() {
        let versions = Versions::from_vars(vars(&[])).unwrap();
        assert_eq!(versions.version, WORKSPACE_VERSION);
        assert_eq!(development_msi_version("0.1.0").unwrap(), "0.1.0");
        assert_eq!(development_msi_version("1.2.3").unwrap(), "1.2.300");
        assert!(development_msi_version("1.2").is_err());
        // An MSI version alone (the CI's upgrade check) is taken for a development build.
        let versions = Versions::from_vars(vars(&[("EMBERHUT_MSI_VERSION", "0.1.2")])).unwrap();
        assert_eq!(versions.msi, "0.1.2");
    }

    #[test]
    fn an_msi_version_outside_msis_ranges_is_refused() {
        for good in ["0.1.1", "255.255.65535", "1.0.99"] {
            assert!(check_msi_version(good).is_ok(), "{good}");
        }
        for bad in [
            "0.1",
            "0.1.1.1",
            "256.0.0",
            "0.256.0",
            "0.0.65536",
            "0.1.x",
            "0..1",
            "+1.0.0",
        ] {
            assert!(check_msi_version(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn only_wix_5_is_taken() {
        assert!(is_wix_5("5.0.2+aa65968\r\n"));
        assert!(!is_wix_5("6.0.0+1234"));
        assert!(!is_wix_5("4.0.5"));
        assert!(!is_wix_5(""));
    }

    #[test]
    fn wix_gets_every_variable_the_installer_names() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let inputs = Inputs {
            wxs: root.join(WXS),
            desktop: PathBuf::from("out/emberhut-desktop.exe"),
            server: PathBuf::from("out/emberhut-server.exe"),
            root: root.to_path_buf(),
        };
        let versions = Versions {
            version: "0.1.0-rc.1".to_owned(),
            msi: "0.1.1".to_owned(),
        };
        let command = wix_build(&inputs, &versions, Path::new("out/x.msi"));
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"Version=0.1.1".to_owned()), "{args:?}");
        let defined: Vec<&str> = args
            .iter()
            .filter_map(|arg| arg.split_once('=').map(|(name, _)| name))
            .collect();
        let wxs = std::fs::read_to_string(&inputs.wxs).unwrap();
        let named: Vec<&str> = wxs
            .split("$(var.")
            .skip(1)
            .filter_map(|rest| rest.split_once(')').map(|(name, _)| name))
            .collect();
        assert_ne!(named, [] as [&str; 0]);
        for name in &named {
            assert!(defined.contains(name), "{name} isn't passed to wix");
        }
        for name in &defined {
            assert!(
                named.contains(name),
                "{name} is passed but the installer doesn't use it"
            );
        }
        // Each file the installer takes from the workspace is there.
        for file in [ICON, "LICENSE-MIT", "LICENSE-APACHE", "README.md"] {
            assert!(root.join(file).is_file(), "{file}");
        }
    }

    #[test]
    fn the_installer_is_per_user_and_replaces_any_version() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let wxs = std::fs::read_to_string(root.join(WXS)).unwrap();
        for needed in [
            r#"Scope="perUser""#,
            r#"UpgradeCode="DDD826CF-51BB-471D-94A5-FA14619F2764""#,
            r#"AllowSameVersionUpgrades="yes""#,
            r#"<StandardDirectory Id="LocalAppDataFolder">"#,
            r#"Name="Programs""#,
            r#"Manufacturer="Emberhut""#,
        ] {
            assert!(wxs.contains(needed), "{needed}");
        }
    }

    #[test]
    fn the_build_is_for_64_bit_windows() {
        let command = cargo_build("dist", Path::new("/w"), Path::new("/w/target"));
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        for expected in [
            "emberhut-desktop",
            "emberhut-server",
            "dist",
            TARGET,
            "--locked",
        ] {
            assert!(
                args.iter().any(|arg| arg == expected),
                "{expected}: {args:?}"
            );
        }
    }
}
