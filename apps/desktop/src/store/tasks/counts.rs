//! A task's numbers in words: its count (`3/10 episodes`), a parent's rollup (`7/21 done ·
//! 160/300 episodes`), how far along it is, its size, its deadline chip, a token count.

use emberhut_api::tasks::{Issue, Rollup, TaskSize, TaskStatus};
use emberhut_api::time::Day;
use jiff::civil::Date;

use crate::store::lookup;

/// A counted task's count: `3/10 episodes` (all of them once it is done).
#[must_use]
pub(crate) fn count_text(task: &Issue) -> Option<String> {
    let total = task.total?;
    let done = if task.status == TaskStatus::Done {
        total
    } else {
        task.progress.unwrap_or(0).min(total)
    };
    let unit = task.unit.as_deref().unwrap_or_default();
    Some(format!("{done}/{total} {unit}").trim().to_owned())
}

/// A parent's subtasks at a glance: `7/21 done · 160/300 episodes`, or `12/21 done · 144
/// episodes left` when not every subtask has a count.
#[must_use]
pub(crate) fn rollup_summary(rollup: &Rollup) -> String {
    let unit = rollup.unit.as_deref().unwrap_or_default();
    let counts = match (rollup.of, rollup.left) {
        (Some(of), _) => format!(" · {}/{of} {unit}", rollup.progress.unwrap_or(0)),
        (None, Some(left)) => format!(" · {left} {unit} left"),
        (None, None) => String::new(),
    };
    format!("{}/{} done{}", rollup.done, rollup.total, counts.trim_end()).replace("  ", " ")
}

/// How far along it is, 0 to 1: a parent by its counts (or its subtasks done), a counted task
/// by its count; none for the rest.
#[must_use]
pub(crate) fn fraction(task: &Issue) -> Option<f64> {
    if let Some(rollup) = &task.sub {
        return Some(match rollup.of {
            Some(of) => f64::from(rollup.progress.unwrap_or(0)) / f64::from(of.max(1)),
            None => f64::from(rollup.done) / f64::from(rollup.total.max(1)),
        });
    }
    let total = task.total?;
    Some(if task.status == TaskStatus::Done {
        1.0
    } else {
        f64::from(task.progress.unwrap_or(0).min(total)) / f64::from(total.max(1))
    })
}

/// Whether its row draws a progress bar: a parent, or a counted task still under way.
#[must_use]
pub(crate) fn shows_bar(task: &Issue) -> bool {
    fraction(task).is_some() && (task.sub.is_some() || task.status != TaskStatus::Done)
}

/// Whether a subtask of it needs the user (its bar takes the accent).
#[must_use]
pub(crate) fn subtask_needs_you(task: &Issue) -> bool {
    task.sub
        .as_ref()
        .is_some_and(|rollup| !rollup.waiting.is_empty())
}

/// A size as a row says it: `M · 3 h`.
#[must_use]
pub(crate) fn size_short(size: &TaskSize) -> String {
    let time = match size {
        TaskSize::Small => "1 h",
        TaskSize::Medium => "3 h",
        TaskSize::Large => "1 day",
        TaskSize::ExtraLarge => "days",
        _ => return size.as_str().to_owned(),
    };
    format!("{} · {time}", size.as_str())
}

/// A size as its picker says it: `M · about 3 hours`.
#[must_use]
pub(crate) fn size_label(size: &TaskSize) -> String {
    let time = match size {
        TaskSize::Small => "about an hour",
        TaskSize::Medium => "about 3 hours",
        TaskSize::Large => "about a day",
        TaskSize::ExtraLarge => "several days",
        _ => return size.as_str().to_owned(),
    };
    format!("{} · {time}", size.as_str())
}

/// The sizes the picker offers, smallest first.
pub(crate) const SIZES: [TaskSize; 4] = [
    TaskSize::Small,
    TaskSize::Medium,
    TaskSize::Large,
    TaskSize::ExtraLarge,
];

/// How a deadline chip is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChipKind {
    /// Missed: red.
    Late,
    /// Someone else's: filled orange.
    Hard,
    /// Its own: outlined.
    Soft,
}

/// A deadline as its chip says it: `2d late`, `Today`, `Tomorrow`, `Fri` within the week,
/// `3 Dec` after.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Chip {
    /// The words.
    pub(crate) text: String,
    /// How it is drawn.
    pub(crate) kind: ChipKind,
}

/// The chip for `day`, `hard` when someone else holds the user to it, seen from `today`.
#[must_use]
pub(crate) fn deadline_chip(day: Option<&Day>, hard: bool, today: Date) -> Option<Chip> {
    let date = lookup::civil_day(day?.as_str())?;
    let days = lookup::days_from(today, date)?;
    let text = match days {
        ..0 => format!("{}d late", -days),
        0 => "Today".to_owned(),
        1 => "Tomorrow".to_owned(),
        2..=6 => date.strftime("%a").to_string(),
        _ => date.strftime("%-d %b").to_string(),
    };
    let kind = if days < 0 {
        ChipKind::Late
    } else if hard {
        ChipKind::Hard
    } else {
        ChipKind::Soft
    };
    Some(Chip { text, kind })
}

/// A token count: `12k`, `1.2M`, `950`.
#[must_use]
pub(crate) fn tokens(count: f64) -> String {
    if count >= 1_000_000.0 {
        let millions = (count / 100_000.0).round() / 10.0;
        if millions.fract() == 0.0 {
            format!("{millions:.0}M")
        } else {
            format!("{millions:.1}M")
        }
    } else if count >= 1000.0 {
        format!("{}k", crate::format::whole((count / 1000.0).floor()))
    } else {
        crate::format::whole(count).to_string()
    }
}

/// A row's grey line under its title, the parts that say something: archived, its project,
/// who does it, what it waits on, its count, its subtasks, its size.
#[must_use]
pub(crate) fn row_line(task: &Issue, project: Option<&str>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if task.status == TaskStatus::Canceled {
        parts.push("Archived".to_owned());
    }
    if let Some(project) = project {
        parts.push(project.to_owned());
    }
    if lookup::is_assistants(task) && task.parent.is_none() {
        parts.push(crate::brand::APP_NAME.to_owned());
    }
    if task.status == TaskStatus::Waiting {
        let asks = task.action.clone().or_else(|| {
            task.decision
                .as_ref()
                .map(|decision| decision.question.clone())
        });
        parts.extend(asks);
    }
    parts.extend(count_text(task));
    if let Some(rollup) = &task.sub {
        parts.push(rollup_summary(rollup));
        if !rollup.waiting.is_empty() {
            parts.push(format!("{} needs you", rollup.waiting.len()));
        }
    }
    parts.extend(task.size.as_ref().map(size_short));
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixtures::issue;
    use crate::wire::ASSISTANT_OWNER;

    const TODAY: Date = jiff::civil::date(2026, 11, 25);

    fn rollup(value: serde_json::Value) -> Rollup {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn counts_and_rollups_read_as_numbers_done() {
        let counted = issue(
            1,
            "working",
            json!({"progress": 12, "total": 10, "unit": "episodes"}),
        );
        assert_eq!(count_text(&counted).as_deref(), Some("10/10 episodes"));
        assert_eq!(fraction(&counted), Some(1.0));
        let done = issue(2, "done", json!({"progress": 1, "total": 4}));
        assert_eq!(count_text(&done).as_deref(), Some("4/4"));
        assert!(
            !shows_bar(&done),
            "a counted task that is done shows no bar"
        );
        assert_eq!(
            rollup_summary(&rollup(
                json!({"done": 7, "total": 21, "progress": 160, "of": 300, "unit": "episodes"})
            )),
            "7/21 done · 160/300 episodes"
        );
        assert_eq!(
            rollup_summary(&rollup(
                json!({"done": 12, "total": 21, "progress": null, "of": null, "left": 144, "unit": "episodes"})
            )),
            "12/21 done · 144 episodes left"
        );
        assert_eq!(
            rollup_summary(&rollup(
                json!({"done": 1, "total": 2, "progress": null, "of": null, "unit": null})
            )),
            "1/2 done"
        );
        let parent = issue(
            3,
            "todo",
            json!({"sub": {"done": 1, "total": 4, "waiting": [9], "progress": null, "of": null, "unit": null}}),
        );
        assert_eq!(fraction(&parent), Some(0.25));
        assert!(shows_bar(&parent) && subtask_needs_you(&parent));
        assert_eq!(fraction(&issue(4, "todo", json!({}))), None);
    }

    #[test]
    fn a_deadline_chip_says_how_far_off_it_is() {
        let chip = |day: &str, hard| deadline_chip(Some(&Day::new(day)), hard, TODAY).unwrap();
        assert_eq!(
            chip("2026-11-23", false),
            Chip {
                text: "2d late".into(),
                kind: ChipKind::Late
            }
        );
        assert_eq!(
            chip("2026-11-25", true),
            Chip {
                text: "Today".into(),
                kind: ChipKind::Hard
            }
        );
        assert_eq!(chip("2026-11-26", false).text, "Tomorrow");
        assert_eq!(
            chip("2026-11-28", false),
            Chip {
                text: "Sat".into(),
                kind: ChipKind::Soft
            }
        );
        assert_eq!(chip("2026-12-03", false).text, "3 Dec");
        assert_eq!(deadline_chip(None, false, TODAY), None);
    }

    #[test]
    fn sizes_and_tokens_read_short() {
        assert_eq!(size_short(&TaskSize::Medium), "M · 3 h");
        assert_eq!(size_label(&TaskSize::ExtraLarge), "XL · several days");
        assert_eq!(tokens(12_345.0), "12k");
        assert_eq!(tokens(1_250_000.0), "1.3M");
        assert_eq!(tokens(2_000_000.0), "2M");
        assert_eq!(tokens(950.0), "950");
    }

    #[test]
    fn a_rows_line_says_what_says_something() {
        let task = issue(
            1,
            "waiting",
            json!({"owner": ASSISTANT_OWNER, "action": "Pick a name",
            "size": "S", "progress": 1, "total": 3, "unit": "parts"}),
        );
        assert_eq!(
            row_line(&task, Some("Example")),
            format!(
                "Example · {} · Pick a name · 1/3 parts · S · 1 h",
                crate::brand::APP_NAME
            )
        );
        assert_eq!(row_line(&issue(2, "canceled", json!({})), None), "Archived");
    }
}
