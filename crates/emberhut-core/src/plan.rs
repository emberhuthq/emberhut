//! The person's plan around the tasks: goals ranked by what matters most, the watch list
//! (Focus) and the open loops. Each is a short list kept whole; these are its rules.

/// A Focus item's timeline keeps this many lines, newest first.
pub const TIMELINE_LINES: usize = 30;

/// The open loops kept, newest last.
pub const LOOPS_KEPT: usize = 80;

/// The ranks a list gets when the item at `moved` goes to `rank` (1 matters most; none: it
/// stays where it is): the others move down to make room, and every item is numbered again
/// from 1. Returns the new order, as indexes into the old list.
#[must_use]
pub fn reranked(len: usize, moved: usize, rank: Option<i64>) -> Vec<usize> {
    let mut order: Vec<usize> = (0..len).collect();
    if let Some(rank) = rank.filter(|rank| *rank != 0)
        && moved < len
    {
        order.retain(|index| *index != moved);
        let at = usize::try_from(rank.max(1) - 1)
            .unwrap_or(0)
            .min(order.len());
        order.insert(at, moved);
    }
    order
}

/// Whether something parked `until` (a day or an instant, as text) is still out of sight at
/// `now` (an instant as text): a date in the past means it is back.
#[must_use]
pub fn parked(until: Option<&str>, now: &str) -> bool {
    until.is_some_and(|until| !until.is_empty() && until > now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_rank_moves_the_others_down() {
        assert_eq!(reranked(3, 2, Some(1)), [2, 0, 1]);
        assert_eq!(reranked(3, 0, Some(9)), [1, 2, 0]);
        assert_eq!(reranked(3, 1, None), [0, 1, 2]);
        assert!(parked(Some("2026-12-01"), "2026-11-25T09:00:00Z"));
        assert!(!parked(Some("2026-11-01"), "2026-11-25T09:00:00Z"));
        assert!(!parked(None, "2026-11-25T09:00:00Z"));
    }
}
