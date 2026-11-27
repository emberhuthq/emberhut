//! The window server's list of an app's windows (`CGWindowListCopyWindowInfo`): their ids, for
//! a picture of one, and their titles.

use objc2_core_foundation::{CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_graphics::{
    CGWindowListCopyWindowInfo, CGWindowListOption, kCGNullWindowID, kCGWindowBounds,
    kCGWindowIsOnscreen, kCGWindowLayer, kCGWindowName, kCGWindowNumber, kCGWindowOwnerPID,
};

use crate::desktop::Window;

/// A window narrower than this is a sliver (a toolbar's shadow), not a window.
const MIN_WIDTH: f64 = 40.0;

/// A window's description, keyed by the window server's names.
type Info = CFDictionary<CFString, CFType>;

/// A number in `info` under `key`.
fn number(info: &Info, key: &CFString) -> Option<f64> {
    info.get(key)?.downcast_ref::<CFNumber>()?.as_f64()
}

/// The app `pid`'s ordinary windows (layer 0, wider than a sliver).
#[expect(
    unsafe_code,
    reason = "the window list's keys are extern statics, its items untyped"
)]
#[must_use]
pub(super) fn of(pid: i32) -> Vec<Window> {
    let options = CGWindowListOption::OptionAll | CGWindowListOption::ExcludeDesktopElements;
    let Some(list) = CGWindowListCopyWindowInfo(options, kCGNullWindowID) else {
        return Vec::new();
    };
    // SAFETY: the window list is an array of dictionaries keyed by strings.
    let list: CFRetained<CFArray<Info>> = unsafe { CFRetained::cast_unchecked(list) };
    // SAFETY: the keys are the framework's own constants.
    let (owner, layer, bounds, name, id, on_screen) = unsafe {
        (
            kCGWindowOwnerPID,
            kCGWindowLayer,
            kCGWindowBounds,
            kCGWindowName,
            kCGWindowNumber,
            kCGWindowIsOnscreen,
        )
    };
    let width = CFString::from_static_str("Width");
    list.iter()
        .filter(|info| number(info, owner) == Some(f64::from(pid)))
        .filter(|info| number(info, layer) == Some(0.0))
        .filter(|info| {
            info.get(bounds)
                .and_then(|b| b.downcast::<CFDictionary>().ok())
                .and_then(|b| {
                    // SAFETY: a window's bounds are a dictionary of numbers keyed by strings.
                    let b: CFRetained<Info> = unsafe { CFRetained::cast_unchecked(b) };
                    number(&b, &width)
                })
                .is_some_and(|w| w > MIN_WIDTH)
        })
        .filter_map(|info| {
            let id = number(&info, id)?;
            // A window id is a 32-bit number.
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a CGWindowID, a 32-bit unsigned number"
            )]
            let id = id as u32;
            Some(Window {
                id,
                title: info
                    .get(name)
                    .and_then(|n| n.downcast_ref::<CFString>().map(ToString::to_string))
                    .unwrap_or_default(),
                on_screen: number(&info, on_screen).is_some_and(|n| n != 0.0),
            })
        })
        .collect()
}
