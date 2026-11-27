//! The system's Reduce Motion setting: with it on, what moves on its own holds still (the
//! working spinner stands as a still icon). macOS: System Settings › Accessibility › Display ›
//! Reduce motion, read at launch and again on each change of the display options (the same
//! notification Reduce Transparency is watched by). Linux: GNOME's "enable-animations" off,
//! read once at launch through `gsettings` on a background thread (no `gsettings`, no
//! setting). Elsewhere it reads as off.

use gpui_kit::AppContext as _;
use gpui_kit::{App, Global};

use crate::material::Transparency;

/// Whether things on screen should hold still.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Motion {
    reduce: bool,
}

impl Global for Motion {}

/// Whether the system asks for reduced motion (off before [`follow`], as in the views' tests).
#[must_use]
pub(crate) fn reduced(cx: &App) -> bool {
    cx.try_global::<Motion>()
        .is_some_and(|motion| motion.reduce)
}

/// Reads the setting now and follows its changes while the app runs.
pub(crate) fn follow(cx: &mut App) {
    cx.set_global(Motion::default());
    let changes = Transparency::changes();
    cx.spawn(async move |cx| {
        loop {
            let reduce = cx
                .background_spawn(async { platform::reduce_motion() })
                .await;
            cx.update(|cx| {
                if reduced(cx) != reduce {
                    log::info!("reduce motion: {reduce}");
                    hold_still(reduce, cx);
                }
            });
            if !changes.next().await {
                break;
            }
        }
    })
    .detach();
}

/// Holds the app still as Reduce Motion does, or lets it move again (the views' tests).
#[cfg(test)]
pub(crate) fn reduce_for_tests(reduce: bool, cx: &mut App) {
    hold_still(reduce, cx);
}

/// Makes everything that moves on its own hold still (or move again): the app's own marks,
/// which read [`reduced`], and every animation GPUI and gpui-kit draw (a job's pulse, a room's
/// working dot, the kit's spinners), which stand at their first frame while
/// [`App::reduce_motion`] is set.
fn hold_still(reduce: bool, cx: &mut App) {
    cx.set_global(Motion { reduce });
    cx.set_reduce_motion(reduce);
    cx.refresh_windows();
}

#[cfg(target_os = "macos")]
mod platform {
    use objc2_app_kit::NSWorkspace;

    pub(super) fn reduce_motion() -> bool {
        NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::process::Command;

    /// GNOME's setting: animations off means reduce motion.
    pub(super) fn reduce_motion() -> bool {
        match Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "enable-animations"])
            .output()
        {
            Ok(output) if output.status.success() => {
                super::animations_off(&String::from_utf8_lossy(&output.stdout))
            }
            Ok(_) => false,
            Err(error) => {
                log::debug!("no gsettings to read reduce motion from: {error}");
                false
            }
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod platform {
    pub(super) fn reduce_motion() -> bool {
        false
    }
}

/// Whether `gsettings`' answer for `enable-animations` turns them off.
#[cfg(any(target_os = "linux", test))]
fn animations_off(answer: &str) -> bool {
    answer.trim() == "false"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gnome_animations_off_is_reduce_motion() {
        assert!(animations_off("false\n"));
        assert!(!animations_off("true\n"));
        assert!(!animations_off(""));
    }

    #[gpui_kit::test]
    fn motion_is_full_until_the_setting_is_read(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| assert!(!reduced(cx)));
    }

    #[gpui_kit::test]
    fn reduce_motion_holds_the_app_and_its_kit_still(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            hold_still(true, cx);
            assert!(reduced(cx));
            assert!(cx.reduce_motion());
            hold_still(false, cx);
            assert!(!reduced(cx));
            assert!(!cx.reduce_motion());
        });
    }
}
