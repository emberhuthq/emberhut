//! Where the window is for the user: the active window, on screen behind another app's, or out
//! of sight. The state poll and the machines' refresh go by it ([`next_poll`](super::poll::next_poll)):
//! they slow down or stop while no one can see what they would fetch, and fetch at once when
//! the window comes forward again.

/// Where the window is for the user, from out of sight to in front (the order is that).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Presence {
    /// Not on screen at all: minimised, the app hidden, on another Space or virtual desktop,
    /// fully covered, or on a display that is asleep (as far as the platform can tell).
    Hidden,
    /// On screen, but not the active window: the user is in another app (or the window opened
    /// without taking the focus).
    Behind,
    /// The active window: the user is in it.
    Front,
}

impl Presence {
    /// Whether coming here from `before` brings the window forward, so what it shows may be
    /// stale and is worth fetching at once.
    #[must_use]
    pub(crate) fn is_forward_of(self, before: Self) -> bool {
        self > before
    }
}

/// What the platform says of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WindowState {
    /// Whether it is the active window (the user is in it).
    pub(crate) active: bool,
    /// Whether the platform shows it: not minimised, hidden, on another Space or covered.
    pub(crate) visible: bool,
}

impl Default for WindowState {
    /// A window just opened: shown, not yet the active one.
    fn default() -> Self {
        Self {
            active: false,
            visible: true,
        }
    }
}

impl WindowState {
    /// Where the window is for the user. Out of sight wins: a window no one can see is hidden
    /// whatever else is true of it.
    #[must_use]
    pub(crate) fn presence(self) -> Presence {
        match self {
            Self { visible: false, .. } => Presence::Hidden,
            Self { active: false, .. } => Presence::Behind,
            Self { active: true, .. } => Presence::Front,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn presence(active: bool, visible: bool) -> Presence {
        WindowState { active, visible }.presence()
    }

    #[test]
    fn a_window_no_one_can_see_is_hidden_whether_or_not_it_is_active() {
        assert_eq!(presence(false, false), Presence::Hidden);
        assert_eq!(presence(true, false), Presence::Hidden);
        assert_eq!(presence(false, true), Presence::Behind);
        assert_eq!(presence(true, true), Presence::Front);
    }

    #[test]
    fn a_window_just_opened_is_behind() {
        assert_eq!(WindowState::default().presence(), Presence::Behind);
    }

    #[test]
    fn coming_into_sight_or_to_the_front_is_forward() {
        use Presence::{Behind, Front, Hidden};
        assert!(Behind.is_forward_of(Hidden), "shown again");
        assert!(Front.is_forward_of(Hidden), "shown again, in front");
        assert!(Front.is_forward_of(Behind), "brought to the front");
        for (before, now) in [(Front, Behind), (Front, Hidden), (Behind, Hidden)] {
            assert!(!now.is_forward_of(before), "{before:?} to {now:?}");
        }
        for same in [Hidden, Behind, Front] {
            assert!(!same.is_forward_of(same), "{same:?} stays");
        }
    }
}
