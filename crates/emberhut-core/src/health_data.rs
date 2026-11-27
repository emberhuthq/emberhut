//! The owner's health and screen time, as their phone and Mac send them: one summary per day
//! (never raw samples), kept as it came. These are the rules read from a day: the walk it shows
//! (a walking, hiking or running workout, else the exercise minutes), and which walk task such
//! a walk closes ("First walk: 20 minutes").

/// The shortest walk, in minutes, that closes a walk task or is logged as one.
pub const WALK_MINUTES: f64 = 20.0;

/// What the phone's daily steps target is when none is set.
pub const STEPS_TARGET: f64 = 8000.0;

/// One day's summary, the parts the walk rules read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HealthDay {
    /// Steps that day.
    pub steps: Option<f64>,
    /// Exercise minutes that day.
    pub exercise_min: Option<f64>,
    /// Its workouts.
    pub workouts: Vec<Workout>,
}

/// A workout of a day.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Workout {
    /// Its type, as the phone names it (`Walking`, `Running`, ...).
    pub kind: String,
    /// How long, in minutes.
    pub min: Option<f64>,
    /// When it started (`07:40`).
    pub at: Option<String>,
}

/// What a day's walk is evidence of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkKind {
    /// A walking, hiking or running workout.
    Walk,
    /// The day's exercise minutes.
    Exercise,
}

/// A day's walk: its minutes, when, and what it was.
#[derive(Clone, Debug, PartialEq)]
pub struct Walk {
    /// Minutes, rounded.
    pub min: f64,
    /// When it started, for a workout.
    pub at: Option<String>,
    /// A workout or exercise minutes.
    pub kind: WalkKind,
}

/// Whether a workout of this type is a walk.
fn moves(kind: &str) -> bool {
    matches!(kind, "Walking" | "Hiking" | "Running")
}

/// A number that counts (not zero, not NaN), as JavaScript reads truth.
fn truthy(value: Option<f64>) -> Option<f64> {
    value.filter(|n| *n != 0.0 && !n.is_nan())
}

/// JavaScript's `Math.round`: halves go up.
fn round(n: f64) -> f64 {
    (n + 0.5).floor()
}

/// The day's best evidence of a walk: its longest walking workout, else its exercise minutes.
#[must_use]
pub fn walked(day: &HealthDay) -> Option<Walk> {
    let longest = day
        .workouts
        .iter()
        .filter(|w| moves(&w.kind))
        .max_by(|a, b| {
            let (a, b) = (a.min.unwrap_or(f64::NAN), b.min.unwrap_or(f64::NAN));
            a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
        });
    if let Some(workout) = longest
        && let Some(min) = truthy(workout.min)
    {
        return Some(Walk {
            min: round(min),
            at: workout.at.clone(),
            kind: WalkKind::Walk,
        });
    }
    truthy(day.exercise_min).map(|min| Walk {
        min: round(min),
        at: None,
        kind: WalkKind::Exercise,
    })
}

/// A walk task's target in minutes, from its title: a title that names a walk and a number of
/// minutes ("First walk: 20 minutes"); none for any other title.
#[must_use]
pub fn walk_target(title: &str) -> Option<f64> {
    let lower = title.to_lowercase();
    let names_a_walk = lower.match_indices("walk").any(|(at, _)| {
        lower[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_')
    });
    if !names_a_walk {
        return None;
    }
    let bytes = lower.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let digits = &lower[start..i];
            let rest = lower[i..].trim_start();
            if rest.starts_with("min") {
                return digits.parse::<f64>().ok().filter(|n| *n != 0.0);
            }
        } else {
            i += 1;
        }
    }
    None
}

/// `n` with thousands separators and up to three decimals (`12,345.5`).
#[must_use]
pub fn grouped(n: f64) -> String {
    let text = format!("{:.3}", n.abs());
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let mut out = String::new();
    for (i, digit) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    let fraction = fraction.trim_end_matches('0');
    if !fraction.is_empty() {
        out.push('.');
        out.push_str(fraction);
    }
    if n < 0.0 {
        out.insert(0, '-');
    }
    out
}

/// A whole number of minutes as text (`20`), or with its decimals when it has them.
fn minutes(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{n:.0}")
    } else {
        n.to_string()
    }
}

/// The task's title for a walk logged with no walk task open: `Walked 32 min`.
#[must_use]
pub fn walk_title(walk: &Walk) -> String {
    format!("{} {} min", verb(walk.kind), minutes(walk.min))
}

fn verb(kind: WalkKind) -> &'static str {
    match kind {
        WalkKind::Walk => "Walked",
        WalkKind::Exercise => "Exercised",
    }
}

/// The note that says what closed a walk task: `Walked 32 min, 07:40, 9,120 steps that day
/// (Apple Health, 2026-10-12)`.
#[must_use]
pub fn walk_evidence(walk: &Walk, day: &HealthDay, date: &str) -> String {
    let at = walk
        .at
        .as_deref()
        .filter(|at| !at.is_empty())
        .map_or_else(String::new, |at| format!(", {at}"));
    let steps = truthy(day.steps).map_or_else(String::new, |steps| {
        format!(", {} steps that day", grouped(steps))
    });
    format!(
        "{} {} min{at}{steps} (Apple Health, {date})",
        verb(walk.kind),
        minutes(walk.min)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workout(kind: &str, min: f64, at: &str) -> Workout {
        Workout {
            kind: kind.to_owned(),
            min: Some(min),
            at: Some(at.to_owned()),
        }
    }

    #[test]
    fn the_longest_walking_workout_counts_else_the_exercise_minutes() {
        let day = HealthDay {
            steps: Some(9120.0),
            exercise_min: Some(41.0),
            workouts: vec![
                workout("Walking", 22.4, "07:40"),
                workout("Yoga", 60.0, "18:00"),
                workout("Hiking", 31.6, "10:00"),
            ],
        };
        assert_eq!(
            walked(&day),
            Some(Walk {
                min: 32.0,
                at: Some("10:00".to_owned()),
                kind: WalkKind::Walk
            })
        );
        let no_walk = HealthDay {
            workouts: vec![workout("Yoga", 60.0, "18:00")],
            ..day.clone()
        };
        assert_eq!(
            walked(&no_walk),
            Some(Walk {
                min: 41.0,
                at: None,
                kind: WalkKind::Exercise
            })
        );
        assert_eq!(walked(&HealthDay::default()), None);
    }

    #[test]
    fn a_walk_task_names_a_walk_and_its_minutes() {
        assert_eq!(walk_target("First walk: 20 minutes"), Some(20.0));
        assert_eq!(walk_target("Walk 45min after lunch"), Some(45.0));
        assert_eq!(walk_target("Sidewalk repair, 20 min"), None, "not a walk");
        assert_eq!(walk_target("Walk to the shop"), None, "no minutes");
        assert_eq!(walk_target("Walk 0 min"), None);
        assert_eq!(walk_target("Read 20 minutes"), None);
    }

    #[test]
    fn the_evidence_says_what_when_and_the_day_s_steps() {
        let day = HealthDay {
            steps: Some(12_345.0),
            ..HealthDay::default()
        };
        let walk = Walk {
            min: 32.0,
            at: Some("07:40".to_owned()),
            kind: WalkKind::Walk,
        };
        assert_eq!(
            walk_evidence(&walk, &day, "2026-10-12"),
            "Walked 32 min, 07:40, 12,345 steps that day (Apple Health, 2026-10-12)"
        );
        assert_eq!(walk_title(&walk), "Walked 32 min");
        let exercise = Walk {
            min: 25.0,
            at: None,
            kind: WalkKind::Exercise,
        };
        assert_eq!(
            walk_evidence(&exercise, &HealthDay::default(), "2026-10-13"),
            "Exercised 25 min (Apple Health, 2026-10-13)"
        );
    }

    #[test]
    fn numbers_are_grouped_by_thousands() {
        assert_eq!(grouped(1_234_567.0), "1,234,567");
        assert_eq!(grouped(999.0), "999");
        assert_eq!(grouped(1_000.5), "1,000.5");
    }
}
