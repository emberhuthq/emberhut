//! The user's goals and their week: each goal's ring (its list this week, done of all), its
//! label, its page's lists, and the weekly report's bars of tasks finished per week.

use emberhut_api::ids::GoalId;
use emberhut_api::plan::{Todo, WeekReport};
use emberhut_api::tasks::Goal;
use jiff::Timestamp;

use super::tasks::clock;

/// A week is this long.
const WEEK_SECONDS: i64 = 7 * 86_400;

/// A goal's list this week: the done ones of all of them (a parked task is off it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Progress {
    /// Done this week.
    pub(crate) done: usize,
    /// On the list in all.
    pub(crate) total: usize,
}

impl Progress {
    /// How full its ring is, 0 to 1.
    #[must_use]
    pub(crate) fn share(self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            f64::from(u32::try_from(self.done).unwrap_or(u32::MAX))
                / f64::from(u32::try_from(self.total).unwrap_or(u32::MAX))
        }
    }

    /// `3 of 5`, or `nothing yet`.
    #[must_use]
    pub(crate) fn words(self) -> String {
        if self.total == 0 {
            "nothing yet".to_owned()
        } else {
            format!("{} of {}", self.done, self.total)
        }
    }
}

/// `goal`'s list this week in `todos`.
#[must_use]
pub(crate) fn progress(goal: GoalId, todos: &[Todo]) -> Progress {
    let list = todos.iter().filter(|todo| todo.goal == Some(goal));
    let (done, total) = list.fold((0, 0), |(done, total), todo| {
        (done + usize::from(todo.is_done()), total + 1)
    });
    Progress { done, total }
}

/// A goal's label under its ring: its own short one, else the first two words of its title.
#[must_use]
pub(crate) fn label(goal: &Goal) -> String {
    goal.short.clone().unwrap_or_else(|| {
        goal.title
            .split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ")
    })
}

/// One week's bar in the weekly report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Bar {
    /// Tasks finished that week.
    pub(crate) count: u32,
    /// Under it: `This week`, else the day it began (`3 Nov`).
    pub(crate) label: String,
    /// It is this week's.
    pub(crate) now: bool,
}

/// The tasks finished per week: the bars (weeks before tracking began left out: they're
/// unknown, not zero), the busiest week's count (the bars scale to it), and the line under
/// them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WeekBars {
    /// The bars, oldest first.
    pub(crate) bars: Vec<Bar>,
    /// The busiest week's count, at least 1.
    pub(crate) top: u32,
    /// `3 finished this week · last 8 weeks`, or `… · tracked since 10 Oct`.
    pub(crate) caption: String,
}

/// The weekly report's bars, in `zone`'s days.
#[must_use]
pub(crate) fn week_bars(report: &WeekReport, zone: &jiff::tz::TimeZone) -> Option<WeekBars> {
    let last = report.weeks.len().checked_sub(1)?;
    let since = report.since.as_ref().and_then(clock::instant);
    let starts = report.week_starts.as_deref().unwrap_or_default();
    let tracked = |i: usize| -> bool {
        let from = starts.get(i).and_then(clock::instant);
        match (since, from) {
            (Some(since), Some(from)) => from.as_second() + WEEK_SECONDS > since.as_second(),
            _ => true,
        }
    };
    let day = |at: Timestamp| {
        at.to_zoned_datetime(zone.clone())
            .strftime("%-d %b")
            .to_string()
    };
    let bars: Vec<Bar> = report
        .weeks
        .iter()
        .enumerate()
        .filter(|&(i, _)| tracked(i))
        .map(|(i, &count)| Bar {
            count,
            label: if i == last {
                "This week".to_owned()
            } else {
                starts.get(i).and_then(clock::instant).map_or_default(day)
            },
            now: i == last,
        })
        .collect();
    let this_week = report.weeks[last];
    let caption = if bars.len() < report.weeks.len() {
        format!(
            "{this_week} finished this week · tracked since {}",
            since.map_or_default(day)
        )
    } else {
        format!(
            "{this_week} finished this week · last {} weeks",
            report.weeks.len()
        )
    };
    Some(WeekBars {
        top: bars.iter().map(|bar| bar.count).max().unwrap_or(1).max(1),
        bars,
        caption,
    })
}

#[cfg(test)]
mod tests {
    use jiff::tz::TimeZone;
    use serde_json::json;

    use super::*;

    fn todo(id: i64, goal: Option<i64>, done: bool) -> Todo {
        serde_json::from_value(json!({"id": id, "goal": goal, "text": format!("Step {id}"),
            "who": "owner", "today": false, "done": done.then_some("2026-11-24T10:00:00Z"),
            "parkedUntil": null, "at": "2026-11-24T10:00:00Z"}))
        .unwrap()
    }

    #[test]
    fn a_goals_ring_is_its_list_this_week_done_of_all() {
        let todos = [
            todo(1, Some(1), true),
            todo(2, Some(1), false),
            todo(3, Some(2), true),
            todo(4, None, true),
        ];
        let ring = progress(GoalId::new(1), &todos);
        assert_eq!((ring.done, ring.total), (1, 2));
        assert!((ring.share() - 0.5).abs() < f64::EPSILON);
        assert_eq!(ring.words(), "1 of 2");
        assert_eq!(progress(GoalId::new(9), &todos).words(), "nothing yet");
    }

    #[test]
    fn a_goal_is_labelled_by_its_short_name_or_two_words() {
        let goal = |short: Option<&str>| -> Goal {
            serde_json::from_value(json!({"id": 1, "title": "Ship the example app", "why": "",
                "rank": 1, "status": null, "short": short}))
            .unwrap()
        };
        assert_eq!(label(&goal(None)), "Ship the");
        assert_eq!(label(&goal(Some("Ship"))), "Ship");
    }

    #[test]
    fn weeks_before_tracking_are_left_out_and_this_week_is_last() {
        let report: WeekReport = serde_json::from_value(json!({
            "weeks": [0, 0, 2, 3],
            "weekStarts": ["2026-11-01T00:00:00Z", "2026-11-08T00:00:00Z", "2026-11-15T00:00:00Z",
                           "2026-11-22T00:00:00Z"],
            "since": "2026-11-10T00:00:00Z",
            "goals": [], "done": []
        }))
        .unwrap();
        let bars = week_bars(&report, &TimeZone::UTC).unwrap();
        let labels: Vec<&str> = bars.bars.iter().map(|bar| bar.label.as_str()).collect();
        assert_eq!(labels, ["8 Nov", "15 Nov", "This week"]);
        assert_eq!(bars.top, 3);
        assert!(bars.bars[2].now);
        assert_eq!(bars.caption, "3 finished this week · tracked since 10 Nov");
        let untracked = WeekReport {
            since: None,
            ..report
        };
        assert_eq!(
            week_bars(&untracked, &TimeZone::UTC).unwrap().caption,
            "3 finished this week · last 4 weeks"
        );
        assert_eq!(week_bars(&WeekReport::default(), &TimeZone::UTC), None);
    }
}
