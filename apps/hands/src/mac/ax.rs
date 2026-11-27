//! Accessibility through `AXUIElement`: an app's elements read, pressed and set by pid. The
//! calls are unsafe in objc2 (out-pointers), so they live here, each checked for its error and
//! a null value.

use std::ptr::{self, NonNull};

use objc2_application_services::{
    AXError, AXIsProcessTrusted, AXIsProcessTrustedWithOptions, AXUIElement,
    kAXTrustedCheckOptionPrompt,
};
use objc2_core_foundation::{
    CFArray, CFAttributedString, CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType,
};

use crate::desktop::{AxError, Focus};

/// How long an app may take to answer one accessibility call, in seconds.
const TIMEOUT: f32 = 3.0;

/// An accessibility element.
#[derive(Clone, Debug)]
pub struct Element(CFRetained<AXUIElement>);

impl PartialEq for Element {
    /// The same element (`CFEqual`).
    fn eq(&self, other: &Self) -> bool {
        *self.0 == *other.0
    }
}

// SAFETY: an `AXUIElement` is an immutable reference to an element in another process; the
// accessibility API takes it on any thread, and the helper runs one request at a time.
#[expect(
    unsafe_code,
    reason = "AXUIElement is used from the request worker's thread"
)]
unsafe impl Send for Element {}
// SAFETY: as above; no method mutates the reference.
#[expect(
    unsafe_code,
    reason = "AXUIElement is used from the request worker's thread"
)]
unsafe impl Sync for Element {}

/// Whether the helper holds the Accessibility grant.
#[expect(unsafe_code, reason = "AXIsProcessTrusted is unsafe in objc2")]
#[must_use]
pub(super) fn trusted() -> bool {
    // SAFETY: takes no arguments.
    unsafe { AXIsProcessTrusted() }
}

/// Whether the helper holds the grant, asking the Mac to list it in System Settings (the
/// first start) when it doesn't.
#[expect(
    unsafe_code,
    reason = "AXIsProcessTrustedWithOptions is unsafe in objc2"
)]
#[must_use]
pub(super) fn ask_trust() -> bool {
    // SAFETY: the key is the framework's own constant.
    let key: &CFString = unsafe { kAXTrustedCheckOptionPrompt };
    let options: CFRetained<CFDictionary<CFString, CFBoolean>> =
        CFDictionary::from_slices(&[key], &[CFBoolean::new(true)]);
    // SAFETY: a dictionary of the documented key and a boolean.
    unsafe { AXIsProcessTrustedWithOptions(Some(options.as_opaque())) }
}

/// An error code as the trait says it.
fn result(error: AXError) -> Result<(), AxError> {
    if error == AXError::Success {
        Ok(())
    } else if error == AXError::InvalidUIElement {
        Err(AxError::Gone)
    } else {
        Err(AxError::Code(error.0))
    }
}

impl Element {
    /// The app `pid`, answering within the timeout; an Electron app is asked to build its
    /// tree (it does only when asked).
    #[expect(unsafe_code, reason = "AXUIElement's calls are unsafe in objc2")]
    #[must_use]
    pub(super) fn application(pid: i32) -> Self {
        // SAFETY: any pid makes an element; a wrong one fails each call.
        let element = Self(unsafe { AXUIElement::new_application(pid) });
        // SAFETY: a valid element and a positive timeout.
        let _timeout = unsafe { element.0.set_messaging_timeout(TIMEOUT) };
        let manual = CFString::from_static_str("AXManualAccessibility");
        // SAFETY: a valid element, attribute name and boolean value.
        let _manual = unsafe { element.0.set_attribute_value(&manual, CFBoolean::new(true)) };
        element
    }

    /// The Mac as a whole (its focus, the element at a point).
    #[expect(unsafe_code, reason = "AXUIElement's calls are unsafe in objc2")]
    fn system() -> Self {
        // SAFETY: takes no arguments.
        let system = Self(unsafe { AXUIElement::new_system_wide() });
        // SAFETY: a valid element and a positive timeout.
        let _timeout = unsafe { system.0.set_messaging_timeout(TIMEOUT) };
        system
    }

    /// What has the keyboard focus, whichever app has it.
    #[must_use]
    pub(super) fn focused() -> Focus<Self> {
        Self::system().focus()
    }

    /// What has the keyboard focus in this app (or the Mac): an element, nothing (only when
    /// the app answers `kAXErrorNoValue`), or unknown (any other answer: an unsupported
    /// attribute, a timeout, an error).
    #[must_use]
    pub(super) fn focus(&self) -> Focus<Self> {
        match self.copy_checked("AXFocusedUIElement") {
            Ok(Some(value)) => value
                .downcast::<AXUIElement>()
                .map_or(Focus::Unknown, |e| Focus::On(Self(e))),
            Err(error) if error == AXError::NoValue => Focus::Nothing,
            Ok(None) | Err(_) => Focus::Unknown,
        }
    }

    /// The process that owns the element at the point `x`, `y` (screen points).
    #[expect(
        unsafe_code,
        reason = "AXUIElementCopyElementAtPosition is unsafe in objc2"
    )]
    #[must_use]
    pub(super) fn pid_at(x: f64, y: f64) -> Option<i32> {
        let system = Self::system();
        let mut element: *const AXUIElement = ptr::null();
        // Points on a screen fit an f32 well enough to find an element.
        #[expect(clippy::cast_possible_truncation, reason = "screen points")]
        let (x, y) = (x as f32, y as f32);
        // SAFETY: `element` is a valid out-pointer; on success it holds a +1 reference.
        let error = unsafe {
            system
                .0
                .copy_element_at_position(x, y, NonNull::from(&mut element))
        };
        let element = NonNull::new(element.cast_mut())?;
        // SAFETY: a copied element is owned by the caller.
        let element = unsafe { CFRetained::from_raw(element) };
        if error != AXError::Success {
            return None;
        }
        let mut pid: i32 = 0;
        // SAFETY: `pid` is a valid out-pointer.
        let error = unsafe { element.pid(NonNull::from(&mut pid)) };
        (error == AXError::Success && pid > 0).then_some(pid)
    }

    /// The attribute `name`'s value.
    #[expect(
        unsafe_code,
        reason = "AXUIElementCopyAttributeValue is unsafe in objc2"
    )]
    fn copy_checked(&self, name: &str) -> Result<Option<CFRetained<CFType>>, AXError> {
        let attribute = CFString::from_str(name);
        let mut value: *const CFType = ptr::null();
        // SAFETY: `value` is a valid out-pointer; on success it holds a +1 reference.
        let error = unsafe {
            self.0
                .copy_attribute_value(&attribute, NonNull::from(&mut value))
        };
        // SAFETY: a copied value is owned by the caller.
        let value =
            NonNull::new(value.cast_mut()).map(|value| unsafe { CFRetained::from_raw(value) });
        if error == AXError::Success {
            Ok(value)
        } else {
            Err(error)
        }
    }

    /// The attribute `name`'s value, when it has one.
    fn copy(&self, name: &str) -> Option<CFRetained<CFType>> {
        self.copy_checked(name).ok().flatten()
    }

    /// A text attribute: a string, an attributed string, a number or a boolean as text; never
    /// a point or a size; `None` when empty.
    #[must_use]
    pub(super) fn text(&self, name: &str) -> Option<String> {
        let value = self.copy(name)?;
        let text = if let Some(s) = value.downcast_ref::<CFString>() {
            s.to_string()
        } else if let Some(s) = value.downcast_ref::<CFAttributedString>() {
            s.string()?.to_string()
        } else if let Some(b) = value.downcast_ref::<CFBoolean>() {
            if b.as_bool() { "1" } else { "0" }.to_owned()
        } else {
            number_text(value.downcast_ref::<CFNumber>()?.as_f64()?)
        };
        (!text.is_empty()).then_some(text)
    }

    /// A yes-or-no attribute.
    #[must_use]
    pub(super) fn flag(&self, name: &str) -> Option<bool> {
        let value = self.copy(name)?;
        if let Some(b) = value.downcast_ref::<CFBoolean>() {
            return Some(b.as_bool());
        }
        value
            .downcast_ref::<CFNumber>()
            .and_then(CFNumber::as_f64)
            .map(|n| n != 0.0)
    }

    /// An attribute naming another element.
    #[must_use]
    pub(super) fn link(&self, name: &str) -> Option<Self> {
        self.copy(name)?.downcast::<AXUIElement>().ok().map(Self)
    }

    /// An attribute listing elements.
    #[expect(
        unsafe_code,
        reason = "an untyped CFArray is cast to its items' base type"
    )]
    #[must_use]
    pub(super) fn list(&self, name: &str) -> Vec<Self> {
        let Some(array) = self.copy(name).and_then(|v| v.downcast::<CFArray>().ok()) else {
            return Vec::new();
        };
        // SAFETY: every item of a CFArray is a CF object.
        let array: CFRetained<CFArray<CFType>> = unsafe { CFRetained::cast_unchecked(array) };
        array
            .iter()
            .filter_map(|item| item.downcast::<AXUIElement>().ok().map(Self))
            .collect()
    }

    /// The actions it takes.
    #[expect(unsafe_code, reason = "AXUIElementCopyActionNames is unsafe in objc2")]
    #[must_use]
    pub(super) fn actions(&self) -> Vec<String> {
        let mut names: *const CFArray = ptr::null();
        // SAFETY: `names` is a valid out-pointer; on success it holds a +1 reference.
        let error = unsafe { self.0.copy_action_names(NonNull::from(&mut names)) };
        let Some(names) = NonNull::new(names.cast_mut()) else {
            return Vec::new();
        };
        // SAFETY: a copied array is owned by the caller.
        let names = unsafe { CFRetained::from_raw(names) };
        if error != AXError::Success {
            return Vec::new();
        }
        // SAFETY: every item of a CFArray is a CF object.
        let names: CFRetained<CFArray<CFType>> = unsafe { CFRetained::cast_unchecked(names) };
        names
            .iter()
            .filter_map(|name| name.downcast_ref::<CFString>().map(ToString::to_string))
            .collect()
    }

    /// Performs `action`.
    ///
    /// # Errors
    ///
    /// [`AxError`].
    #[expect(unsafe_code, reason = "AXUIElementPerformAction is unsafe in objc2")]
    pub(super) fn perform(&self, action: &str) -> Result<(), AxError> {
        let action = CFString::from_str(action);
        // SAFETY: a valid element and action name.
        result(unsafe { self.0.perform_action(&action) })
    }

    /// Whether the attribute `name` can be set.
    #[expect(
        unsafe_code,
        reason = "AXUIElementIsAttributeSettable is unsafe in objc2"
    )]
    #[must_use]
    pub(super) fn settable(&self, name: &str) -> bool {
        let attribute = CFString::from_str(name);
        let mut settable: u8 = 0;
        // SAFETY: `settable` is a valid out-pointer.
        let error = unsafe {
            self.0
                .is_attribute_settable(&attribute, NonNull::from(&mut settable))
        };
        error == AXError::Success && settable != 0
    }

    /// Sets the attribute `name` to `value`.
    ///
    /// # Errors
    ///
    /// [`AxError`].
    #[expect(
        unsafe_code,
        reason = "AXUIElementSetAttributeValue is unsafe in objc2"
    )]
    pub(super) fn set(&self, name: &str, value: &CFType) -> Result<(), AxError> {
        let attribute = CFString::from_str(name);
        // SAFETY: a valid element, attribute name and CF value.
        result(unsafe { self.0.set_attribute_value(&attribute, value) })
    }
}

/// A number as the reference helper writes one: whole numbers without a point.
fn number_text(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{n:.0}")
    } else {
        n.to_string()
    }
}
