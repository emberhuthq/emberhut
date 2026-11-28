//! Installing a verified payload on Windows: the release's `.msi`, a per-user install (no admin
//! prompt) into `%LOCALAPPDATA%\Programs\Emberhut`. Only a copy that runs from there updates
//! itself, since that is the copy the installer replaces.
//!
//! Staging keeps the verified file under its version's name and notes its SHA-256; before it
//! runs, the file is hashed again, so nothing that changed it in between is installed. Install
//! and restart then starts one detached PowerShell that waits for this process to exit, runs
//! `msiexec /i "<msi>" /passive /norestart` (a progress bar, no questions), and starts the
//! installed app again, whether or not the installer succeeded, so the user always gets an
//! Emberhut back. The app quits once it has started it.
//!
//! The checks before this are the ones every platform has: the size, the SHA-256 and the update
//! signature, whose trusted comment names `windows`. The `.msi` carries no Authenticode signature
//! yet; once releases are signed with a code-signing certificate, staging checks it too.

use std::fs::File;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, PoisonError};

use emberhut_api::releases::{Build, Release};
use sha2::{Digest as _, Sha256};

use super::Unavailable;
use super::installer::{InstallError, Installer, Staged, Verdict, Watch};
use super::version::Running;

/// The installer's folder under `%LOCALAPPDATA%`.
const INSTALL_DIR: [&str; 2] = ["Programs", "Emberhut"];

/// The installer of a per-user Emberhut on Windows.
#[derive(Debug)]
pub(crate) struct MsiInstaller {
    /// Where the verified `.msi` is kept.
    staging: PathBuf,
    /// The installed app's executable, started again after the install.
    exe: PathBuf,
    /// `%SystemRoot%`, where PowerShell and `msiexec` are.
    system_root: PathBuf,
    /// The `.msi` staged, if one is.
    kept: Mutex<Option<KeptMsi>>,
}

/// A staged `.msi`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct KeptMsi {
    build: Build,
    path: PathBuf,
    sha256: [u8; 32],
}

impl MsiInstaller {
    /// The installer for the running app, keeping the `.msi` in `staging`.
    ///
    /// # Errors
    ///
    /// Why this copy can't update itself: it doesn't run from where the installer puts it.
    #[cfg(windows)]
    pub(crate) fn for_running_app(staging: &Path) -> Result<Self, Unavailable> {
        let unreadable = |what: &str| Unavailable::Unreadable(what.to_owned());
        let exe = std::env::current_exe().map_err(|error| {
            Unavailable::Unreadable(format!("where Emberhut runs from: {error}"))
        })?;
        let local = std::env::var_os("LOCALAPPDATA").ok_or_else(|| unreadable("%LOCALAPPDATA%"))?;
        let system_root =
            std::env::var_os("SystemRoot").ok_or_else(|| unreadable("%SystemRoot%"))?;
        Self::new(staging, &exe, Path::new(&local), Path::new(&system_root))
    }

    /// The installer for `exe`, the per-user install being under `local` (`%LOCALAPPDATA%`)
    /// and the system's tools under `system_root`.
    ///
    /// # Errors
    ///
    /// [`Unavailable::Place`] when `exe` isn't in the per-user install's folder.
    pub(crate) fn new(
        staging: &Path,
        exe: &Path,
        local: &Path,
        system_root: &Path,
    ) -> Result<Self, Unavailable> {
        let dir = INSTALL_DIR
            .iter()
            .fold(local.to_path_buf(), |dir, part| dir.join(part));
        if !runs_from(exe, &dir) {
            return Err(Unavailable::Place(format!(
                "Emberhut updates itself when it runs from where its installer put it ({}); \
                 install it with the installer from emberhut.com",
                dir.display()
            )));
        }
        Ok(Self {
            staging: staging.to_path_buf(),
            exe: exe.to_path_buf(),
            system_root: system_root.to_path_buf(),
            kept: Mutex::new(None),
        })
    }

    fn kept(&self) -> std::sync::MutexGuard<'_, Option<KeptMsi>> {
        self.kept.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Whether `exe` is directly in `dir`, comparing as Windows does (any case, either slash).
fn runs_from(exe: &Path, dir: &Path) -> bool {
    // As text, so the tests read Windows paths the same on every system.
    let plain = |path: &Path| {
        path.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    };
    plain(exe)
        .rsplit_once('\\')
        .is_some_and(|(parent, _)| parent == plain(dir))
}

/// Whether `version` may name a file: digits, letters, dots and dashes only.
fn plain_version(version: &str) -> bool {
    !version.is_empty()
        && !version.starts_with('.')
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// The SHA-256 of the file at `path`, read in chunks.
fn sha256_of(path: &Path) -> io::Result<[u8; 32]> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 64 << 10];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => return Ok(hash.finalize().into()),
            Ok(read) => hash.update(&buffer[..read]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

impl Installer for MsiInstaller {
    fn stage(&self, payload: &Path, release: &Release) -> Result<Staged, InstallError> {
        if !plain_version(&release.version) {
            return Err(InstallError::Bundle(
                format!("the version {:?} can't name a file", release.version).into(),
            ));
        }
        let path = self
            .staging
            .join(format!("Emberhut-{}.msi", release.version));
        if let Some(old) = self.kept().take()
            && old.path != path
            && let Err(error) = std::fs::remove_file(&old.path)
        {
            log::debug!("no older update to remove: {error}");
        }
        std::fs::rename(payload, &path).map_err(InstallError::Local)?;
        let sha256 = sha256_of(&path).map_err(InstallError::Local)?;
        *self.kept() = Some(KeptMsi {
            build: release.build,
            path,
            sha256,
        });
        Ok(Staged {
            build: release.build,
            version: release.version.clone(),
        })
    }

    fn swap(&self, staged: &Staged, running: Running) -> Result<Box<dyn Watch>, InstallError> {
        if running.build.is_none() {
            return Err(InstallError::NotStaged);
        }
        let kept = self
            .kept()
            .clone()
            .filter(|kept| kept.build == staged.build)
            .ok_or(InstallError::NotStaged)?;
        // Hashed again right before it runs: it may have waited hours on disk since its checks.
        let now = sha256_of(&kept.path).map_err(InstallError::Local)?;
        if now != kept.sha256 {
            if let Err(error) = std::fs::remove_file(&kept.path) {
                log::warn!("couldn't remove the changed update: {error}");
            }
            *self.kept() = None;
            return Err(InstallError::Bundle(
                "the update changed on disk after it was checked".into(),
            ));
        }
        let chain = chain(&self.system_root, std::process::id(), &kept.path, &self.exe);
        spawn(&chain).map_err(InstallError::Local)?;
        log::info!(
            "the installer of build {} runs once Emberhut quits",
            staged.build
        );
        Ok(Box::new(Handed))
    }
}

/// The install is in the detached chain's hands: the app quits, nothing is watched.
struct Handed;

impl Watch for Handed {
    fn run(self: Box<Self>, _stop: &AtomicBool) -> Verdict {
        Verdict::Kept
    }
}

/// A program to start, and its arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Chain {
    /// The program.
    pub(crate) program: PathBuf,
    /// Its arguments.
    pub(crate) args: Vec<String>,
}

/// The chain that installs `msi` once process `pid` has exited, then starts `exe`: PowerShell
/// (from `system_root`, not from the search path) with the script encoded, so no path needs
/// quoting on its command line.
#[must_use]
pub(crate) fn chain(system_root: &Path, pid: u32, msi: &Path, exe: &Path) -> Chain {
    let root = system_root.display().to_string();
    let root = root.trim_end_matches(['\\', '/']);
    let msiexec = format!(r"{root}\System32\msiexec.exe");
    let arguments = format!("/i \"{}\" /passive /norestart", msi.display());
    let script = format!(
        "Wait-Process -Id {pid} -Timeout 120 -ErrorAction SilentlyContinue; \
         Start-Process -FilePath {msiexec} -ArgumentList {arguments} -Wait; \
         Start-Process -FilePath {exe}",
        msiexec = quoted(&msiexec),
        arguments = quoted(&arguments),
        exe = quoted(&exe.display().to_string()),
    );
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    Chain {
        program: PathBuf::from(format!(
            r"{root}\System32\WindowsPowerShell\v1.0\powershell.exe"
        )),
        args: [
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-EncodedCommand",
        ]
        .into_iter()
        .map(str::to_owned)
        .chain([data_encoding::BASE64.encode(&utf16)])
        .collect(),
    }
}

/// `text` as a PowerShell string that expands nothing.
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Starts `chain` detached from this process and its console, so it outlives the app.
#[cfg(windows)]
fn spawn(chain: &Chain) -> io::Result<()> {
    use std::os::windows::process::CommandExt as _;
    use std::process::{Command, Stdio};
    /// `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`.
    const DETACHED: u32 = 0x0000_0008 | 0x0000_0200;
    Command::new(&chain.program)
        .args(&chain.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(DETACHED)
        .spawn()
        .map(drop)
}

#[cfg(not(windows))]
fn spawn(chain: &Chain) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!("{} runs on Windows only", chain.program.display()),
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn release(version: &str, build: u64) -> Release {
        serde_json::from_value(json!({"version": version, "build": build, "date": "2026-12-08"}))
            .unwrap()
    }

    fn script(chain: &Chain) -> String {
        let bytes = data_encoding::BASE64
            .decode(chain.args.last().unwrap().as_bytes())
            .unwrap();
        let units: Vec<u16> = bytes
            .chunks(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        String::from_utf16(&units).unwrap()
    }

    #[test]
    fn the_chain_waits_for_the_app_installs_quietly_and_starts_it_again() {
        let chain = chain(
            Path::new(r"C:\Windows"),
            4242,
            Path::new(
                r"C:\Users\Sam O'Neil\AppData\Local\emberhut\cache\updates\Emberhut-0.1.0.msi",
            ),
            Path::new(r"C:\Users\Sam O'Neil\AppData\Local\Programs\Emberhut\emberhut-desktop.exe"),
        );
        assert_eq!(
            chain.program,
            PathBuf::from(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe")
        );
        assert_eq!(chain.args[chain.args.len() - 2], "-EncodedCommand");
        assert!(chain.args.contains(&"-NoProfile".to_owned()));
        assert_eq!(
            script(&chain),
            "Wait-Process -Id 4242 -Timeout 120 -ErrorAction SilentlyContinue; \
             Start-Process -FilePath 'C:\\Windows\\System32\\msiexec.exe' -ArgumentList \
             '/i \"C:\\Users\\Sam O''Neil\\AppData\\Local\\emberhut\\cache\\updates\\Emberhut-0.1.0.msi\" /passive /norestart' -Wait; \
             Start-Process -FilePath 'C:\\Users\\Sam O''Neil\\AppData\\Local\\Programs\\Emberhut\\emberhut-desktop.exe'"
        );
    }

    #[test]
    fn only_the_per_user_install_updates_itself() {
        let local = Path::new(r"C:\Users\sam\AppData\Local");
        let root = Path::new(r"C:\Windows");
        let staging = Path::new("staging");
        let installed =
            Path::new(r"C:\Users\sam\AppData\Local\Programs\Emberhut\emberhut-desktop.exe");
        let other_case =
            Path::new(r"c:\users\SAM\appdata\local\programs\emberhut\Emberhut-Desktop.exe");
        let elsewhere = Path::new(r"C:\Users\sam\Downloads\emberhut-desktop.exe");
        let deeper =
            Path::new(r"C:\Users\sam\AppData\Local\Programs\Emberhut\bin\emberhut-desktop.exe");
        for exe in [installed, other_case] {
            // Off Windows, `join` puts a `/` between the parts: compared as Windows does anyway.
            assert!(
                MsiInstaller::new(staging, exe, local, root).is_ok(),
                "{exe:?}"
            );
        }
        for exe in [elsewhere, deeper] {
            assert!(
                matches!(
                    MsiInstaller::new(staging, exe, local, root),
                    Err(Unavailable::Place(why)) if why.contains("Programs")
                ),
                "{exe:?}"
            );
        }
    }

    fn installer(dir: &Path) -> MsiInstaller {
        let local = dir.join("Local");
        let exe = local
            .join("Programs")
            .join("Emberhut")
            .join("emberhut-desktop.exe");
        MsiInstaller::new(&dir.join("staging"), &exe, &local, Path::new(r"C:\Windows")).unwrap()
    }

    #[test]
    fn a_staged_msi_is_kept_by_its_version_and_hashed_again_before_it_runs() {
        let dir = tempfile::tempdir().unwrap();
        let msi = installer(dir.path());
        std::fs::create_dir_all(dir.path().join("staging")).unwrap();
        let payload = dir.path().join("staging").join("Emberhut-build-10099.msi");
        std::fs::write(&payload, b"an installer").unwrap();
        let staged = msi.stage(&payload, &release("0.1.0", 10099)).unwrap();
        assert_eq!(staged.build, Build::new(10099));
        let kept = dir.path().join("staging").join("Emberhut-0.1.0.msi");
        assert_eq!(std::fs::read(&kept).unwrap(), b"an installer");
        assert!(!payload.exists());
        let running = Running::from_parts("0.1.0-rc.2", Some("10002"));
        let other = Staged {
            build: Build::new(10101),
            version: "0.1.1-rc.1".into(),
        };
        assert!(matches!(
            msi.swap(&other, running),
            Err(InstallError::NotStaged)
        ));
        std::fs::write(&kept, b"another installer").unwrap();
        let Err(error) = msi.swap(&staged, running) else {
            panic!("a changed installer ran");
        };
        assert!(error.is_verification(), "{error:?}");
        assert!(!kept.exists(), "the changed file is removed");
    }

    #[test]
    fn a_version_that_cant_name_a_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let msi = installer(dir.path());
        let payload = dir.path().join("x.msi");
        std::fs::write(&payload, b"x").unwrap();
        for version in ["..\\..\\evil", "0.1.0/x", "", ".hidden"] {
            assert!(
                matches!(
                    msi.stage(&payload, &release(version, 10099)),
                    Err(InstallError::Bundle(_))
                ),
                "{version:?}"
            );
        }
        assert!(plain_version("0.1.0-rc.1"));
    }
}
