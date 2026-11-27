//! Two-finger swipes on a trackpad, read from the scroll events of one gesture: a sideways
//! swipe far enough (and more sideways than up or down) counts once, left or right. A task row
//! swiped right is done, left put off (or archived); the task page swiped right goes back.

use std::cell::RefCell;
use std::rc::Rc;

use emberhut_api::ids::TaskId;
use gpui_kit::{ScrollEvent, TouchPhase, px};

/// How far sideways a swipe goes before it counts, in pixels.
const FAR_ENOUGH: f32 = 110.0;

/// Which way a swipe went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Swiped {
    /// The fingers moved right.
    Right,
    /// The fingers moved left.
    Left,
}

/// One gesture's travel so far.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Swipe {
    across: f32,
    down: f32,
    /// It counted already: the rest of the gesture is ignored.
    done: bool,
}

impl Swipe {
    /// Takes in one step of the gesture (`phase`, and how far it moved, `x` and `y`); the swipe
    /// when this step made it count.
    pub(crate) fn feed(&mut self, phase: TouchPhase, x: f32, y: f32) -> Option<Swiped> {
        // A gesture starts, ends or is taken by the system: start over.
        if phase != TouchPhase::Moved {
            *self = Self::default();
            return None;
        }
        if self.done {
            return None;
        }
        self.across += x;
        self.down += y;
        let sideways = self.across.abs() > 2.0 * self.down.abs();
        if !sideways || self.across.abs() < FAR_ENOUGH {
            return None;
        }
        self.done = true;
        Some(if self.across > 0.0 {
            Swiped::Right
        } else {
            Swiped::Left
        })
    }

    /// Takes in a scroll event of the gesture.
    pub(crate) fn feed_event(&mut self, event: &ScrollEvent) -> Option<Swiped> {
        let delta = event.delta.pixel_delta(px(16.));
        self.feed(event.touch_phase, f32::from(delta.x), f32::from(delta.y))
    }
}

/// The swipe over the rows of a list: one gesture at a time, on the row it started over.
#[derive(Clone, Debug, Default)]
pub(crate) struct RowSwipes {
    inner: Rc<RefCell<(Option<TaskId>, Swipe)>>,
}

impl RowSwipes {
    /// Takes in a scroll event over `task`'s row; the swipe when it counted.
    pub(crate) fn feed(&self, task: TaskId, event: &ScrollEvent) -> Option<Swiped> {
        let mut held = self.inner.borrow_mut();
        if held.0 != Some(task) {
            *held = (Some(task), Swipe::default());
        }
        held.1.feed_event(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_sideways_swipe_counts_once() {
        let mut swipe = Swipe::default();
        assert_eq!(swipe.feed(TouchPhase::Started, 0.0, 0.0), None);
        assert_eq!(swipe.feed(TouchPhase::Moved, 60.0, 5.0), None);
        assert_eq!(
            swipe.feed(TouchPhase::Moved, 60.0, 5.0),
            Some(Swiped::Right)
        );
        assert_eq!(
            swipe.feed(TouchPhase::Moved, 60.0, 0.0),
            None,
            "once a gesture"
        );
        swipe.feed(TouchPhase::Ended, 0.0, 0.0);
        swipe.feed(TouchPhase::Started, 0.0, 0.0);
        assert_eq!(
            swipe.feed(TouchPhase::Moved, -130.0, 10.0),
            Some(Swiped::Left)
        );
    }

    #[test]
    fn scrolling_up_and_down_is_no_swipe() {
        let mut swipe = Swipe::default();
        swipe.feed(TouchPhase::Started, 0.0, 0.0);
        assert_eq!(swipe.feed(TouchPhase::Moved, 120.0, 90.0), None);
        assert_eq!(swipe.feed(TouchPhase::Moved, 10.0, 200.0), None);
    }
}
