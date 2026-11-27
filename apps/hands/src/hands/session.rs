//! One app at a time: the helper holds at most one app in the background. Each step in an app
//! (a snapshot, a press, keys…) takes it, or keeps it; a step in another app is refused while
//! the held one was used in the last [`HOLD`], unless the held app has quit or the session was
//! let go (`/release`). The refs of a snapshot belong to the held app only.

use std::time::{Duration, Instant};

use crate::fail::Fail;

/// How long the held app stays held after its last step.
pub const HOLD: Duration = Duration::from_secs(30);

/// The app held now, and the refs of its last snapshot.
#[derive(Clone, Debug)]
pub struct Session<E> {
    /// Its process id.
    pub pid: i32,
    /// Its name.
    pub name: String,
    /// `a1`, `a2`…: the elements of its last snapshot, in order.
    pub refs: Vec<E>,
    /// When it was last used.
    pub used: Instant,
}

impl<E> Session<E> {
    /// A new session on the app `pid` named `name`, used `now`, with no refs yet.
    #[must_use]
    pub fn new(pid: i32, name: &str, now: Instant) -> Self {
        Self {
            pid,
            name: name.to_owned(),
            refs: Vec::new(),
            used: now,
        }
    }
}

/// Whether `name` (the app `pid`, when it runs) may be used `now` while `held` is held;
/// `held_runs` says whether the held app still runs.
///
/// # Errors
///
/// [`Fail`] (409) naming the held app and when it is let go.
pub fn admit<E>(
    held: Option<&Session<E>>,
    pid: Option<i32>,
    name: &str,
    held_runs: bool,
    now: Instant,
) -> Result<(), Fail> {
    let Some(held) = held else { return Ok(()) };
    if Some(held.pid) == pid || !held_runs {
        return Ok(());
    }
    let since = now.saturating_duration_since(held.used);
    if since >= HOLD {
        return Ok(());
    }
    let left = HOLD.saturating_sub(since).as_secs().max(1);
    Err(Fail::clash(format!(
        "Emberhut Hands is using {held_name} in the background, and uses one app at a time. Finish \
         there first: {name} can be used {left} s after the last step in {held_name}, or as \
         soon as {held_name} quits.",
        held_name = held.name
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_held_lets_any_app_in() {
        assert!(admit::<()>(None, Some(7), "Notes", true, Instant::now()).is_ok());
    }

    #[test]
    fn the_held_app_goes_on() {
        let now = Instant::now();
        let held: Session<()> = Session::new(7, "Notes", now);
        assert!(admit(Some(&held), Some(7), "Notes", true, now).is_ok());
    }

    #[test]
    fn another_app_waits_until_the_hold_ends() {
        let now = Instant::now();
        let held: Session<()> = Session::new(7, "Notes", now);
        let refused = admit(
            Some(&held),
            Some(9),
            "Pages",
            true,
            now + Duration::from_secs(10),
        )
        .unwrap_err();
        assert_eq!(refused.status, 409);
        assert!(refused.text.contains("using Notes"), "{}", refused.text);
        assert!(
            refused.text.contains("Pages can be used 20 s after"),
            "{}",
            refused.text
        );
        assert!(admit(Some(&held), Some(9), "Pages", true, now + HOLD).is_ok());
    }

    #[test]
    fn an_app_that_quit_is_not_held() {
        let now = Instant::now();
        let held: Session<()> = Session::new(7, "Notes", now);
        assert!(admit(Some(&held), Some(9), "Pages", false, now).is_ok());
    }

    #[test]
    fn an_app_not_running_yet_is_another_app() {
        let now = Instant::now();
        let held: Session<()> = Session::new(7, "Notes", now);
        assert!(admit(Some(&held), None, "Pages", true, now).is_err());
    }
}
