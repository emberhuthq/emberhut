//! The screen's lock: whether it is locked (`CGSessionCopyCurrentDictionary`) and locking it
//! now, as the menu bar's Lock Screen does. Nothing here ever unlocks it.

use std::ffi::CStr;

use objc2_core_foundation::{CFBoolean, CFDictionary, CFRetained, CFString, CFType};
use objc2_core_graphics::CGSessionCopyCurrentDictionary;

/// The login framework, which holds the lock call.
const LOGIN: &CStr = c"/System/Library/PrivateFrameworks/login.framework/Versions/Current/login";
/// The call that locks the screen at once.
const LOCK: &CStr = c"SACLockScreenImmediate";

/// Whether the screen is locked.
#[expect(unsafe_code, reason = "the session dictionary is untyped")]
#[must_use]
pub(super) fn locked() -> bool {
    let Some(session) = CGSessionCopyCurrentDictionary() else {
        return false;
    };
    // SAFETY: the session dictionary is keyed by strings.
    let session: CFRetained<CFDictionary<CFString, CFType>> =
        unsafe { CFRetained::cast_unchecked(session) };
    session
        .get(&CFString::from_static_str("CGSSessionScreenIsLocked"))
        .and_then(|value| value.downcast_ref::<CFBoolean>().map(CFBoolean::as_bool))
        .unwrap_or(false)
}

/// Locks the screen now. The only way without sending keys is the login framework's
/// `SACLockScreenImmediate`, a C function with no Objective-C or Swift surface, found at run
/// time.
///
/// # Errors
///
/// When this macOS has no such call.
#[expect(
    unsafe_code,
    reason = "dlopen and dlsym, and calling the function they find"
)]
pub(super) fn lock() -> Result<(), String> {
    // SAFETY: a valid C path; the handle is kept (never closed) as the framework stays loaded.
    let handle = unsafe { libc::dlopen(LOGIN.as_ptr(), libc::RTLD_NOW) };
    if handle.is_null() {
        return Err("this macOS has no login framework to lock with".to_owned());
    }
    // SAFETY: a valid handle and symbol name.
    let symbol = unsafe { libc::dlsym(handle, LOCK.as_ptr()) };
    if symbol.is_null() {
        return Err("this macOS has no SACLockScreenImmediate".to_owned());
    }
    // SAFETY: SACLockScreenImmediate takes nothing and returns an int.
    let lock_now: extern "C" fn() -> i32 = unsafe { std::mem::transmute(symbol) };
    let _status = lock_now();
    Ok(())
}
