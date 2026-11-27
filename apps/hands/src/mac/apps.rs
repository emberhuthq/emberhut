//! The apps through `NSWorkspace`: those running, the one in front, finding one on disk and
//! opening it in the background.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2_app_kit::{
    NSApplicationActivationOptions, NSApplicationActivationPolicy, NSRunningApplication,
    NSWorkspace, NSWorkspaceOpenConfiguration,
};
use objc2_foundation::{NSBundle, NSError, NSString, NSURL};

use crate::desktop::{InstalledApp, RunningApp};

/// How long opening an app may take.
const OPENING: Duration = Duration::from_secs(20);

/// The folders apps are looked for in, by name, after the bundle id.
const FOLDERS: [&str; 4] = [
    "/Applications",
    "/System/Applications",
    "/System/Applications/Utilities",
    "/Applications/Utilities",
];

/// An `NSRunningApplication` as the trait has it.
fn running_app(app: &NSRunningApplication) -> RunningApp {
    RunningApp {
        pid: app.processIdentifier(),
        name: app
            .localizedName()
            .map_or_else(|| "?".to_owned(), |name| name.to_string()),
        bundle: app.bundleIdentifier().map_or_default(|id| id.to_string()),
        hidden: app.isHidden(),
    }
}

/// The apps that run with a Dock icon.
#[must_use]
pub(super) fn running() -> Vec<RunningApp> {
    NSWorkspace::sharedWorkspace()
        .runningApplications()
        .iter()
        .filter(|app| {
            app.activationPolicy() == NSApplicationActivationPolicy::Regular
                && !app.isTerminated()
                && app.processIdentifier() > 0
        })
        .map(|app| running_app(&app))
        .collect()
}

/// The running copies of the app with `bundle`.
#[must_use]
pub(super) fn with_bundle(bundle: &str) -> Vec<RunningApp> {
    NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(bundle))
        .iter()
        .filter(|app| !app.isTerminated())
        .map(|app| running_app(&app))
        .collect()
}

/// The app in front.
#[must_use]
pub(super) fn front() -> Option<i32> {
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map(|app| app.processIdentifier())
}

/// The running app `pid`, Dock icon or not.
#[must_use]
pub(super) fn app(pid: i32) -> Option<RunningApp> {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .filter(|app| !app.isTerminated())
        .map(|app| running_app(&app))
}

/// Brings the app `pid` to the front.
pub(super) fn activate(pid: i32) {
    if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
        let _sent = app.activateWithOptions(NSApplicationActivationOptions::empty());
    }
}

/// The installed app `want` names: its bundle id, else its name in the usual folders and
/// `~/Applications`.
#[must_use]
pub(super) fn installed(want: &str, home: &Path) -> Option<InstalledApp> {
    let path = NSWorkspace::sharedWorkspace()
        .URLForApplicationWithBundleIdentifier(&NSString::from_str(want))
        .and_then(|url| url.path())
        .map(|path| PathBuf::from(path.to_string()))
        .or_else(|| by_name(want, home))?;
    let bundle = NSBundle::bundleWithPath(&NSString::from_str(&path.to_string_lossy()))?;
    let id = bundle
        .bundleIdentifier()
        .map_or_default(|id| id.to_string());
    let name = ["CFBundleDisplayName", "CFBundleName"]
        .iter()
        .find_map(|key| {
            bundle
                .objectForInfoDictionaryKey(&NSString::from_str(key))
                .and_then(|value| value.downcast::<NSString>().ok())
                .map(|value| value.to_string())
        })
        .unwrap_or_else(|| {
            path.file_stem()
                .map_or_default(|stem| stem.to_string_lossy().into_owned())
        });
    Some(InstalledApp {
        path: path.to_string_lossy().into_owned(),
        bundle: id,
        name,
    })
}

/// The app bundle named `want` (`Pages`, `Pages.app`) in the usual folders.
fn by_name(want: &str, home: &Path) -> Option<PathBuf> {
    let name = format!(
        "{}.app",
        want.strip_suffix(".app").unwrap_or(want).to_lowercase()
    );
    let user = home.join("Applications");
    FOLDERS
        .iter()
        .map(PathBuf::from)
        .chain([user])
        .filter_map(|dir| fs::read_dir(dir).ok())
        .flat_map(|entries| entries.filter_map(Result::ok))
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .is_some_and(|file| file.to_string_lossy().to_lowercase() == name)
        })
}

/// Opens `app` without bringing it forward.
///
/// # Errors
///
/// The Mac's words when it doesn't open in time.
pub(super) fn open(app: &InstalledApp, hidden: bool) -> Result<(), String> {
    let url = NSURL::fileURLWithPath(&NSString::from_str(&app.path));
    let configuration = NSWorkspaceOpenConfiguration::new();
    configuration.setActivates(false);
    configuration.setHides(hidden);
    configuration.setAddsToRecentItems(false);
    let (done, opened) = mpsc::channel::<Option<String>>();
    let handler = RcBlock::new(
        move |_app: *mut NSRunningApplication, error: *mut NSError| {
            // SAFETY: the error is null or a valid NSError for the block's call.
            #[expect(
                unsafe_code,
                reason = "the completion handler's error is a raw pointer"
            )]
            let error = unsafe { error.as_ref() };
            let _sent = done.send(error.map(|e| e.localizedDescription().to_string()));
        },
    );
    NSWorkspace::sharedWorkspace().openApplicationAtURL_configuration_completionHandler(
        &url,
        &configuration,
        Some(&handler),
    );
    match opened.recv_timeout(OPENING) {
        Ok(None) => Ok(()),
        Ok(Some(error)) => Err(error),
        Err(_) => Err("it didn't open within 20 seconds".to_owned()),
    }
}
