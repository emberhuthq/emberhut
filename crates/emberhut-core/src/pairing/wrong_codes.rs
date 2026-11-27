//! The brake on guessing: after a few wrong codes, no code is taken for a while.
//!
//! The count is shared by every caller, so spreading guesses over many addresses does not
//! help. Five wrong codes within ten minutes stop every code, the right one too, until the
//! oldest of them is ten minutes old: at most 720 guesses a day against 31^8 codes that each
//! live ten minutes. The other side of a shared brake: anyone who can reach the pairing route
//! can keep pairing stopped with five requests every ten minutes.
//!
//! A try counts against the brake from the moment it starts, before its code is looked up, so
//! guesses sent all at once are braked like guesses sent one after another; a try whose code
//! turns out to exist is then [forgiven](WrongCodeBrake::forgive). A try that never finishes
//! (its request cancelled mid-lookup) keeps counting, which only errs on the side of the brake.
//! The count is kept in memory: a restart clears it, which a guesser cannot cause.

use std::collections::VecDeque;

use jiff::{SignedDuration, Timestamp};

/// Tries that may count as wrong within [`WINDOW`].
const ALLOWED: usize = 5;
/// How long a wrong code counts against every caller.
const WINDOW: SignedDuration = SignedDuration::from_mins(10);

/// The recent tries that count as wrong, and whether they are too many.
#[derive(Clone, Debug, Default)]
pub struct WrongCodeBrake {
    /// Each counted try, oldest first.
    tries: VecDeque<Try>,
    /// The number the next try gets.
    next: u64,
    /// Whether a try has been refused since the brake was last off.
    refusing: bool,
}

#[derive(Clone, Copy, Debug)]
struct Try {
    number: u64,
    at: Timestamp,
}

/// A try that counts against the brake until it is [forgiven](WrongCodeBrake::forgive).
#[derive(Debug)]
#[must_use = "a try whose code exists has to be forgiven"]
pub struct Attempt {
    number: u64,
}

/// The brake is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lockout {
    /// How long until a code is taken again.
    pub retry_after: SignedDuration,
    /// Whether this is the first try refused since the brake came on, to log it once.
    pub first: bool,
}

impl WrongCodeBrake {
    /// Starts a try at `now`, counted as wrong from now on; or the lockout while too many
    /// recent tries count as wrong.
    ///
    /// # Errors
    ///
    /// [`Lockout`] while the brake is on.
    pub fn begin(&mut self, now: Timestamp) -> Result<Attempt, Lockout> {
        let window_start = now - WINDOW;
        while self.tries.front().is_some_and(|t| t.at <= window_start) {
            self.tries.pop_front();
        }
        if let Some(oldest) = self.tries.front().filter(|_| self.tries.len() >= ALLOWED) {
            let first = !self.refusing;
            self.refusing = true;
            return Err(Lockout {
                retry_after: oldest.at - window_start,
                first,
            });
        }
        self.refusing = false;
        let number = self.next;
        self.next += 1;
        self.tries.push_back(Try { number, at: now });
        Ok(Attempt { number })
    }

    /// Stops counting `attempt`: its code existed (whether or not it still worked).
    #[expect(
        clippy::needless_pass_by_value,
        reason = "taken by value so that a try cannot be forgiven twice"
    )]
    pub fn forgive(&mut self, attempt: Attempt) {
        self.tries.retain(|t| t.number != attempt.number);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minutes: i64) -> Timestamp {
        Timestamp::UNIX_EPOCH + SignedDuration::from_mins(1_000 + minutes)
    }

    #[test]
    fn five_wrong_codes_lock_until_the_first_is_ten_minutes_old() {
        let mut brake = WrongCodeBrake::default();
        for minute in 0..5 {
            let _wrong = brake.begin(at(minute)).unwrap();
        }
        let lockout = brake.begin(at(4)).unwrap_err();
        assert_eq!(lockout.retry_after, SignedDuration::from_mins(6));
        assert!(lockout.first);
        let again = brake.begin(at(9)).unwrap_err();
        assert_eq!(again.retry_after, SignedDuration::from_mins(1));
        assert!(!again.first, "logged once");
        assert!(brake.begin(at(10)).is_ok(), "the first has left the window");
    }

    #[test]
    fn tries_count_from_the_start_so_a_burst_is_braked() {
        let mut brake = WrongCodeBrake::default();
        let started: Vec<_> = (0..20).map(|_| brake.begin(at(0))).collect();
        assert_eq!(started.iter().filter(|r| r.is_ok()).count(), ALLOWED);
    }

    #[test]
    fn a_forgiven_try_does_not_count() {
        let mut brake = WrongCodeBrake::default();
        for _ in 0..20 {
            let right = brake.begin(at(0)).unwrap();
            brake.forgive(right);
        }
        assert!(brake.begin(at(0)).is_ok());
    }

    #[test]
    fn wrong_codes_spread_out_never_lock() {
        let mut brake = WrongCodeBrake::default();
        for try_number in 0..20 {
            let _wrong = brake.begin(at(try_number * 3)).unwrap();
        }
    }
}
