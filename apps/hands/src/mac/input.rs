//! Input events through `CGEvent`: to one app's process (`CGEventPostToPid`), or as the real
//! mouse and keyboard (the HID tap).

use std::time::Duration;

use objc2_core_foundation::CGPoint;
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation,
    CGEventType, CGMouseButton, CGScrollEventUnit,
};

use crate::desktop::Posting;
use crate::keys::{Button, Chord, Input};

/// Where events go.
#[derive(Clone, Copy, Debug)]
pub(super) enum To {
    /// One app's process only.
    App(i32),
    /// The real mouse and keyboard.
    Screen,
}

/// The flags for a chord's modifiers.
fn flags(chord: Chord) -> CGEventFlags {
    let mut flags = CGEventFlags::empty();
    let m = chord.modifiers;
    for (on, flag) in [
        (m.command, CGEventFlags::MaskCommand),
        (m.shift, CGEventFlags::MaskShift),
        (m.option, CGEventFlags::MaskAlternate),
        (m.control, CGEventFlags::MaskControl),
    ] {
        if on {
            flags |= flag;
        }
    }
    flags
}

/// The event for one input, or `None` for a pause.
#[expect(
    unsafe_code,
    reason = "CGEventKeyboardSetUnicodeString takes a raw pointer"
)]
fn event(
    input: &Input,
    source: Option<&CGEventSource>,
) -> Option<objc2_core_foundation::CFRetained<CGEvent>> {
    match input {
        Input::Key { chord, down } => {
            let event = CGEvent::new_keyboard_event(source, chord.code, *down)?;
            CGEvent::set_flags(Some(&event), flags(*chord));
            Some(event)
        }
        Input::Text { units, down } => {
            let event = CGEvent::new_keyboard_event(source, 0, *down)?;
            // SAFETY: `units` is a live slice of UTF-16 units of that length.
            unsafe {
                CGEvent::keyboard_set_unicode_string(
                    Some(&event),
                    units.len() as _,
                    units.as_ptr(),
                );
            }
            Some(event)
        }
        Input::Move { x, y } => CGEvent::new_mouse_event(
            source,
            CGEventType::MouseMoved,
            CGPoint { x: *x, y: *y },
            CGMouseButton::Left,
        ),
        Input::Click {
            button,
            down,
            x,
            y,
            count,
        } => {
            let (kind, which) = match (button, down) {
                (Button::Left, true) => (CGEventType::LeftMouseDown, CGMouseButton::Left),
                (Button::Left, false) => (CGEventType::LeftMouseUp, CGMouseButton::Left),
                (Button::Right, true) => (CGEventType::RightMouseDown, CGMouseButton::Right),
                (Button::Right, false) => (CGEventType::RightMouseUp, CGMouseButton::Right),
            };
            let event = CGEvent::new_mouse_event(source, kind, CGPoint { x: *x, y: *y }, which)?;
            CGEvent::set_integer_value_field(
                Some(&event),
                CGEventField::MouseEventClickState,
                *count,
            );
            Some(event)
        }
        // The wheel moves up for a positive value: down the page is negative.
        Input::Scroll { dy } => CGEvent::new_scroll_wheel_event2(
            source,
            CGScrollEventUnit::Pixel,
            1,
            dy.saturating_neg(),
            0,
            0,
        ),
        Input::Pause(_) => None,
    }
}

/// Posts `inputs` in order from an event source of the helper's own, pausing where they say
/// and asking `go_on` before each event (telling it after each one posted): whether all were
/// posted.
pub(super) fn post(to: To, inputs: &[Input], go_on: &mut dyn FnMut(Posting<'_>) -> bool) -> bool {
    let source = CGEventSource::new(CGEventSourceStateID::Private);
    for input in inputs {
        if let Input::Pause(time) = input {
            std::thread::sleep(*time);
            continue;
        }
        if !go_on(Posting::Before(input)) {
            return false;
        }
        let Some(event) = event(input, source.as_deref()) else {
            tracing::warn!(?input, "an input event could not be made");
            continue;
        };
        match to {
            To::App(pid) => CGEvent::post_to_pid(pid, Some(&event)),
            To::Screen => CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event)),
        }
        go_on(Posting::After(input));
    }
    true
}

/// The kinds of input the helper counts (see `Input::counted`).
const COUNTED: [CGEventType; 6] = [
    CGEventType::MouseMoved,
    CGEventType::LeftMouseDown,
    CGEventType::RightMouseDown,
    CGEventType::KeyDown,
    CGEventType::ScrollWheel,
    CGEventType::LeftMouseDragged,
];

/// How many of those the Mac has counted in this login session, from every source.
#[must_use]
pub(super) fn count() -> u64 {
    COUNTED
        .into_iter()
        .map(|kind| {
            u64::from(CGEventSource::counter_for_event_type(
                CGEventSourceStateID::CombinedSessionState,
                kind,
            ))
        })
        .sum()
}

/// Seconds since the last mouse move, click, key or scroll, whoever made it.
#[must_use]
pub(super) fn idle_seconds() -> f64 {
    [
        CGEventType::MouseMoved,
        CGEventType::LeftMouseDown,
        CGEventType::RightMouseDown,
        CGEventType::KeyDown,
        CGEventType::ScrollWheel,
        CGEventType::LeftMouseDragged,
    ]
    .into_iter()
    .map(|kind| {
        CGEventSource::seconds_since_last_event_type(CGEventSourceStateID::HIDSystemState, kind)
    })
    .fold(f64::INFINITY, f64::min)
    .min(Duration::from_hours(99).as_secs_f64())
}
