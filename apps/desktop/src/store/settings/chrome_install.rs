//! Adding the Emberhut extension to Chrome from the Chrome Web Store, with the user's OK: the
//! app writes Chrome's external-extension entry for it, which names the extension's id and the
//! store's update address, and Chrome installs it from the store. Where the entry goes is
//! Chrome's rule for each system:
//!
//! - macOS: a file in the user's own `~/Library/Application Support/Google/Chrome/External
//!   Extensions`, no administrator needed. Chrome shows "New extension added" the next time it
//!   starts, and the user turns it on with Enable.
//! - Windows: the user's registry key `HKCU\Software\Google\Chrome\Extensions\<id>` with the
//!   value `update_url`, written by `reg add`. Chrome asks the same way.
//! - Linux: Google Chrome reads only its root-owned folder `/opt/google/chrome/extensions` (a
//!   per-user folder is ignored), so the file goes in through `pkexec install`, which asks for
//!   the user's password. Chrome adds the extension by itself the next time it starts.
//!
//! Writing it again with the same content changes nothing; an entry already there is reported
//! as such. Without a way to write it (no `pkexec`, the password prompt closed), the Chrome Web
//! Store page ([`store_page`]) is the way to add it. The processes run through the hut's
//! [`Spawner`], so the tests use a fake one.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::hut::{Run, Spawner, SystemSpawner};
use emberhut_api::chrome::extension::ID;

/// The Chrome Web Store's update address, which the entry names: Chrome installs the extension
/// from the store.
pub(crate) const UPDATE_URL: &str = "https://clients2.google.com/service/update2/crx";

/// Google Chrome's own folder for external extensions on Linux.
pub(crate) const LINUX_FOLDER: &str = "/opt/google/chrome/extensions";

/// The user's registry key for Chrome's external extensions on Windows, without the id.
pub(crate) const WINDOWS_KEY: &str = r"HKCU\Software\Google\Chrome\Extensions";

/// The extension's page on the Chrome Web Store: adding it there works on every system, at once.
#[must_use]
pub(crate) fn store_page() -> String {
    format!("https://chromewebstore.google.com/detail/{ID}")
}

/// The entry file's content.
#[must_use]
pub(crate) fn entry_json() -> String {
    format!("{{\n  \"external_update_url\": \"{UPDATE_URL}\"\n}}\n")
}

/// The system Chrome runs on, for where its entry goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChromeOs {
    /// macOS.
    Mac,
    /// Windows.
    Windows,
    /// Linux.
    Linux,
    /// Anything else: no entry, the store page only.
    Other,
}

impl ChromeOs {
    /// The system this copy was built for.
    #[must_use]
    pub(crate) fn this() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Other
        }
    }
}

/// Where the entry goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    /// A file the user may write (macOS).
    File(PathBuf),
    /// A file only the administrator may write, put there through `pkexec` (Linux).
    Elevated(PathBuf),
    /// A key in the user's registry (Windows).
    Registry(String),
}

impl Entry {
    /// Where Chrome's entry goes on `os`, for the user whose home folder is `home`; `None` where
    /// Chrome has no such entry this app knows of.
    #[must_use]
    pub(crate) fn plan(os: ChromeOs, home: &Path) -> Option<Self> {
        let file = format!("{ID}.json");
        match os {
            ChromeOs::Mac => Some(Self::File(
                home.join("Library/Application Support/Google/Chrome/External Extensions")
                    .join(file),
            )),
            ChromeOs::Linux => Some(Self::Elevated(Path::new(LINUX_FOLDER).join(file))),
            ChromeOs::Windows => Some(Self::Registry(format!(r"{WINDOWS_KEY}\{ID}"))),
            ChromeOs::Other => None,
        }
    }

    /// Where it is, as the user reads it: the folder or the key.
    #[must_use]
    pub(crate) fn place(&self) -> String {
        match self {
            Self::File(path) | Self::Elevated(path) => {
                path.parent().unwrap_or(path).to_string_lossy().into_owned()
            }
            Self::Registry(key) => key.clone(),
        }
    }

    /// What comes next once it is written, in Chrome.
    #[must_use]
    pub(crate) fn next_step(&self) -> &'static str {
        match self {
            Self::File(_) | Self::Registry(_) => {
                "Chrome shows “New extension added” the next time it starts: click Enable."
            }
            Self::Elevated(_) => "Chrome adds it by itself the next time it starts.",
        }
    }
}

/// `reg add <key> /v update_url /t REG_SZ /d <update address> /f`: the Windows entry.
#[must_use]
pub(crate) fn reg_add(key: &str) -> Run {
    Run::new("reg").args([
        "add",
        key,
        "/v",
        "update_url",
        "/t",
        "REG_SZ",
        "/d",
        UPDATE_URL,
        "/f",
    ])
}

/// `reg query <key> /v update_url`: whether the Windows entry is there.
#[must_use]
pub(crate) fn reg_query(key: &str) -> Run {
    Run::new("reg").args(["query", key, "/v", "update_url"])
}

/// `pkexec install -D -m 0644 <from> <to>`: the Linux entry, copied in as the administrator
/// (the system asks for the user's password), its folder made when missing.
#[must_use]
pub(crate) fn pkexec_install(from: &Path, to: &Path) -> Run {
    Run::new("pkexec").args([
        "install".as_ref(),
        "-D".as_ref(),
        "-m".as_ref(),
        "0644".as_ref(),
        from.as_os_str(),
        to.as_os_str(),
    ])
}

/// How writing the entry went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Written {
    /// Where it is (the folder or the key).
    pub(crate) place: String,
    /// It was there already, with the same content.
    pub(crate) already: bool,
    /// What comes next in Chrome.
    pub(crate) next: &'static str,
}

/// Writes Chrome's entry for the extension. Blocks: call it off the UI thread.
pub(crate) trait Installer: Send + Sync + fmt::Debug {
    /// Writes the entry, or says why it couldn't, in the user's words.
    ///
    /// # Errors
    ///
    /// No entry on this system, or writing it failed.
    fn install(&self) -> Result<Written, String>;
}

/// The real [`Installer`]: Chrome's entry for `os`, under `home`, the processes run by
/// `spawner`, and the Linux file made first in `scratch`.
pub(crate) struct ChromeInstaller {
    /// The system.
    pub(crate) os: ChromeOs,
    /// The user's home folder.
    pub(crate) home: PathBuf,
    /// A folder for the Linux file before it is copied in.
    pub(crate) scratch: PathBuf,
    /// Runs `reg` and `pkexec`.
    pub(crate) spawner: Arc<dyn Spawner>,
}

impl fmt::Debug for ChromeInstaller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChromeInstaller")
            .field("os", &self.os)
            .field("home", &self.home)
            .finish_non_exhaustive()
    }
}

impl ChromeInstaller {
    /// This computer's: its system, `$HOME` (or `%USERPROFILE%`) and temporary folder.
    #[must_use]
    pub(crate) fn here() -> Self {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map_or_default(PathBuf::from);
        Self {
            os: ChromeOs::this(),
            home,
            scratch: std::env::temp_dir(),
            spawner: Arc::new(SystemSpawner),
        }
    }

    fn write_file(path: &Path) -> io::Result<bool> {
        let json = entry_json();
        if std::fs::read_to_string(path).is_ok_and(|kept| kept == json) {
            return Ok(true);
        }
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::write(path, json)?;
        Ok(false)
    }

    fn write_elevated(&self, path: &Path) -> Result<bool, String> {
        if std::fs::read_to_string(path).is_ok_and(|kept| kept == entry_json()) {
            return Ok(true);
        }
        let from = self.scratch.join(format!("emberhut-chrome-{ID}.json"));
        std::fs::write(&from, entry_json())
            .map_err(|error| format!("Couldn't make the file to copy: {error}"))?;
        let ran = self.spawner.run(&pkexec_install(&from, path));
        drop(std::fs::remove_file(&from));
        match ran {
            Ok(ran) if ran.ok => Ok(false),
            Ok(ran) => Err(format!(
                "Chrome's folder wasn't written ({}). Add it from the Chrome Web Store instead.",
                ran.said()
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(
                "This computer has no pkexec to ask for your password. Add it from the Chrome \
                 Web Store instead."
                    .to_owned(),
            ),
            Err(error) => Err(format!("Couldn't run pkexec: {error}")),
        }
    }

    fn write_registry(&self, key: &str) -> Result<bool, String> {
        if self
            .spawner
            .run(&reg_query(key))
            .is_ok_and(|ran| ran.ok && ran.stdout.contains(UPDATE_URL))
        {
            return Ok(true);
        }
        match self.spawner.run(&reg_add(key)) {
            Ok(ran) if ran.ok => Ok(false),
            Ok(ran) => Err(format!("The registry wasn't written ({}).", ran.said())),
            Err(error) => Err(format!("Couldn't run reg: {error}")),
        }
    }
}

impl Installer for ChromeInstaller {
    fn install(&self) -> Result<Written, String> {
        let entry = Entry::plan(self.os, &self.home).ok_or_else(|| {
            "Add it from the Chrome Web Store: this system has no entry to write.".to_owned()
        })?;
        let already = match &entry {
            Entry::File(path) => Self::write_file(path)
                .map_err(|error| format!("Couldn't write Chrome's folder: {error}"))?,
            Entry::Elevated(path) => self.write_elevated(path)?,
            Entry::Registry(key) => self.write_registry(key)?,
        };
        Ok(Written {
            place: entry.place(),
            already,
            next: entry.next_step(),
        })
    }
}

/// Where adding the extension is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum InstallState {
    /// Not requested yet: nothing is written before the user's OK.
    #[default]
    NotWritten,
    /// Being written.
    Writing,
    /// Written (or there already).
    Written(Written),
    /// It couldn't be written: why, in the user's words.
    Failed(String),
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;
    use crate::hut::{Ran, Running};

    /// Runs nothing: answers each run with the next of its answers, and keeps what was run.
    #[derive(Default)]
    struct Fake {
        answers: Mutex<VecDeque<io::Result<Ran>>>,
        ran: Mutex<Vec<Vec<String>>>,
    }

    impl Fake {
        fn answering(answers: Vec<io::Result<Ran>>) -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::new(answers.into()),
                ran: Mutex::default(),
            })
        }

        fn ran(&self) -> Vec<Vec<String>> {
            self.ran.lock().unwrap().clone()
        }
    }

    impl Spawner for Fake {
        fn spawn(&self, _: &Run) -> io::Result<Box<dyn Running>> {
            Err(io::Error::other("not in these tests"))
        }

        fn run(&self, run: &Run) -> io::Result<Ran> {
            let mut words = vec![run.program.to_string_lossy().into_owned()];
            words.extend(run.words());
            self.ran.lock().unwrap().push(words);
            self.answers
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Ok(Ran::default()))
        }

        fn terminate(&self, _: u32) -> io::Result<()> {
            Ok(())
        }
    }

    fn ok(stdout: &str) -> Ran {
        Ran {
            ok: true,
            stdout: stdout.to_owned(),
            stderr: String::new(),
        }
    }

    fn installer(os: ChromeOs, home: &Path, spawner: Arc<Fake>) -> ChromeInstaller {
        ChromeInstaller {
            os,
            home: home.to_owned(),
            scratch: home.to_owned(),
            spawner,
        }
    }

    #[test]
    fn each_system_has_chromes_own_place() {
        assert_eq!(
            Entry::plan(ChromeOs::Mac, Path::new("/Users/ada")),
            Some(Entry::File(PathBuf::from(format!(
                "/Users/ada/Library/Application Support/Google/Chrome/External Extensions/{ID}.json"
            ))))
        );
        assert_eq!(
            Entry::plan(ChromeOs::Linux, Path::new("/home/ada")),
            Some(Entry::Elevated(PathBuf::from(format!(
                "/opt/google/chrome/extensions/{ID}.json"
            ))))
        );
        assert_eq!(
            Entry::plan(ChromeOs::Windows, Path::new("/home/ada")),
            Some(Entry::Registry(format!(
                r"HKCU\Software\Google\Chrome\Extensions\{ID}"
            )))
        );
        assert_eq!(Entry::plan(ChromeOs::Other, Path::new("/home/ada")), None);
        assert_eq!(
            Entry::plan(ChromeOs::Linux, Path::new("/home/ada"))
                .unwrap()
                .place(),
            "/opt/google/chrome/extensions"
        );
    }

    #[test]
    fn the_entry_names_the_store_and_the_store_page_the_extension() {
        let json: serde_json::Value = serde_json::from_str(&entry_json()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "external_update_url": "https://clients2.google.com/service/update2/crx" })
        );
        assert_eq!(
            store_page(),
            "https://chromewebstore.google.com/detail/omnhmmnnbibfdohdhcbcomdhbgpehfnj"
        );
    }

    #[test]
    fn the_commands_are_regs_and_pkexecs_own() {
        let key = format!(r"{WINDOWS_KEY}\{ID}");
        assert_eq!(
            reg_add(&key).words(),
            [
                "add",
                key.as_str(),
                "/v",
                "update_url",
                "/t",
                "REG_SZ",
                "/d",
                UPDATE_URL,
                "/f"
            ]
        );
        assert_eq!(
            reg_query(&key).words(),
            ["query", key.as_str(), "/v", "update_url"]
        );
        let run = pkexec_install(Path::new("/tmp/a.json"), Path::new("/opt/x/b.json"));
        assert_eq!(run.program, PathBuf::from("pkexec"));
        assert_eq!(
            run.words(),
            [
                "install",
                "-D",
                "-m",
                "0644",
                "/tmp/a.json",
                "/opt/x/b.json"
            ]
        );
    }

    #[test]
    fn on_a_mac_the_file_is_written_once_its_folder_made() {
        let home = tempfile::tempdir().unwrap();
        let fake = Fake::answering(vec![]);
        let mac = installer(ChromeOs::Mac, home.path(), Arc::clone(&fake));
        let first = mac.install().unwrap();
        assert!(!first.already);
        assert!(first.next.contains("click Enable"));
        let file = home.path().join(format!(
            "Library/Application Support/Google/Chrome/External Extensions/{ID}.json"
        ));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), entry_json());
        assert!(first.place.ends_with("External Extensions"));
        assert!(mac.install().unwrap().already, "there already: unchanged");
        assert!(fake.ran().is_empty(), "no process on a Mac");
    }

    #[test]
    fn on_linux_the_file_goes_in_through_pkexec() {
        let home = tempfile::tempdir().unwrap();
        let fake = Fake::answering(vec![Ok(ok(""))]);
        let linux = installer(ChromeOs::Linux, home.path(), Arc::clone(&fake));
        let written = linux.install().unwrap();
        assert!(!written.already);
        assert!(written.next.contains("by itself"));
        let ran = fake.ran();
        assert_eq!(ran.len(), 1);
        assert_eq!(ran[0][..5], ["pkexec", "install", "-D", "-m", "0644"]);
        assert_eq!(
            ran[0][6],
            format!("/opt/google/chrome/extensions/{ID}.json")
        );
        assert!(
            !Path::new(&ran[0][5]).exists(),
            "the scratch file is cleaned up"
        );
    }

    #[test]
    fn on_linux_a_closed_prompt_or_no_pkexec_points_to_the_store() {
        let home = tempfile::tempdir().unwrap();
        let refused = Fake::answering(vec![Ok(Ran {
            ok: false,
            stdout: String::new(),
            stderr: "Error executing command as another user: Not authorized".to_owned(),
        })]);
        let error = installer(ChromeOs::Linux, home.path(), refused)
            .install()
            .unwrap_err();
        assert!(error.contains("Not authorized") && error.contains("Chrome Web Store"));
        let missing = Fake::answering(vec![Err(io::Error::from(io::ErrorKind::NotFound))]);
        let error = installer(ChromeOs::Linux, home.path(), missing)
            .install()
            .unwrap_err();
        assert!(error.contains("no pkexec") && error.contains("Chrome Web Store"));
    }

    #[test]
    fn on_windows_the_key_is_added_unless_its_there() {
        let home = tempfile::tempdir().unwrap();
        let fresh = Fake::answering(vec![Ok(Ran::default()), Ok(ok(""))]);
        let written = installer(ChromeOs::Windows, home.path(), Arc::clone(&fresh))
            .install()
            .unwrap();
        assert!(!written.already);
        assert!(written.place.ends_with(ID));
        let ran = fresh.ran();
        assert_eq!((ran[0][1].as_str(), ran[1][1].as_str()), ("query", "add"));

        let there = Fake::answering(vec![Ok(ok(&format!(
            "    update_url    REG_SZ    {UPDATE_URL}"
        )))]);
        assert!(
            installer(ChromeOs::Windows, home.path(), Arc::clone(&there))
                .install()
                .unwrap()
                .already
        );
        assert_eq!(there.ran().len(), 1, "no add when it's there");
    }

    #[test]
    fn elsewhere_only_the_store_page_is_offered() {
        let home = tempfile::tempdir().unwrap();
        let error = installer(ChromeOs::Other, home.path(), Fake::answering(vec![]))
            .install()
            .unwrap_err();
        assert!(error.contains("Chrome Web Store"));
    }
}
