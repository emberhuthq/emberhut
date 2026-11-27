//! The new copy's side of an update: once its window has drawn its first frame, whatever it
//! shows (the chat, sign-in, offline), it is healthy, and says so in the marker. No network is
//! needed, so a Mac that relaunches offline keeps a good build.

use emberhut_api::releases::Build;
use gpui_kit::{AnyWindowHandle, App, AppContext as _, Task};

use super::version::Running;
use crate::config::AppFiles;

/// Reports to an update waiting on this copy once the window `handle` has drawn its first
/// frame.
pub(crate) fn report_when_drawn(handle: AnyWindowHandle, files: Option<AppFiles>, cx: &mut App) {
    let watched = handle.update(cx, |_, window, _| {
        window.on_next_frame(move |_, cx| report(files, Running::this().build, cx).detach());
    });
    if let Err(error) = watched {
        log::warn!("the window closed before its first frame: {error}");
    }
}

/// Reports the first frame of build `me` off the UI thread: the marker is read and written,
/// and the old bundle (a whole app of many files) may be removed.
fn report(files: Option<AppFiles>, me: Option<Build>, cx: &App) -> Task<()> {
    cx.background_spawn(async move { first_frame(files.as_ref(), me) })
}

/// This copy's window drew its first frame: report it to an update waiting on it.
fn first_frame(files: Option<&AppFiles>, me: Option<Build>) {
    let (Some(files), Some(build)) = (files, me) else {
        return;
    };
    #[cfg(unix)]
    super::install::report_first_frame(files.dir(), build);
    #[cfg(not(unix))]
    log::debug!(
        "no update to report to ({}, build {build})",
        files.dir().display()
    );
}

#[cfg(all(test, unix))]
mod tests {
    use gpui_kit::TestAppContext;

    use super::*;
    use crate::updates::marker::{self, Marker, State};

    #[gpui_kit::test]
    fn the_first_frame_is_reported_off_the_ui_thread(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(marker::FILE);
        // An update to build 15 whose old copy (this process) still watches.
        let mut pending = marker::tests::pending();
        pending.watchdog = std::process::id();
        pending.write(&path).unwrap();
        let files = AppFiles::in_dir(dir.path());

        let reporting = cx.update(|cx| report(Some(files), Some(Build::new(15)), cx));
        assert_eq!(
            Marker::read(&path).unwrap().state,
            State::Pending,
            "nothing read or written on the UI thread"
        );
        cx.run_until_parked();
        assert_eq!(Marker::read(&path).unwrap().state, State::Healthy);
        drop(reporting);
    }
}
