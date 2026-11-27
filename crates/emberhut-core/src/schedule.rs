//! The Scheduled page's words: how often something runs, read from a timer's own terms, and a
//! routine's name from its job.

/// How often, in words: `every 30 min`, `hourly`, `every 2 h`, `daily`, `every 3 days`.
#[must_use]
pub fn every(minutes: u64) -> String {
    if minutes >= 1440 && minutes.is_multiple_of(1440) {
        if minutes == 1440 {
            "daily".to_owned()
        } else {
            format!("every {} days", minutes / 1440)
        }
    } else if minutes >= 60 {
        if minutes.is_multiple_of(60) {
            if minutes == 60 {
                "hourly".to_owned()
            } else {
                format!("every {} h", minutes / 60)
            }
        } else {
            let tenths = (minutes * 10 + 30) / 60;
            if tenths.is_multiple_of(10) {
                format!("every {} h", tenths / 10)
            } else {
                format!("every {}.{} h", tenths / 10, tenths % 10)
            }
        }
    } else {
        format!("every {minutes} min")
    }
}

fn two_digits(text: &str) -> Option<&str> {
    (text.len() == 2 && text.bytes().all(|b| b.is_ascii_digit())).then_some(text)
}

/// A timer's schedule in words: `OnCalendar=*-*-* 08:00:30` is `daily 08:00`,
/// `Sat *-*-* 20:00` `weekly, Sat 20:00`, `*-*-* *:00:00` `hourly`, `*-*-* *:00/5` `every 5
/// min`, `OnUnitActiveUSec=5min` `every 5 min`. Anything else: the calendar as written.
#[must_use]
pub fn repeat(calendar: Option<&str>, monotonic: Option<&str>) -> Option<String> {
    let value = |text: Option<&str>, key: &str| {
        let text = text?;
        let start = text.find(key)? + key.len();
        let rest = &text[start..];
        let end = rest.find([';', '}']).unwrap_or(rest.len());
        Some(rest[..end].trim().to_owned())
    };
    if let Some(cal) = value(calendar, "OnCalendar=") {
        let words: Vec<&str> = cal.split_whitespace().collect();
        let clock = |time: &str| -> Option<(String, String, Option<String>)> {
            let mut parts = time.split(':');
            let hour = parts.next()?.to_owned();
            let minute = parts.next()?.to_owned();
            Some((hour, minute, parts.next().map(str::to_owned)))
        };
        match words.as_slice() {
            ["*-*-*", time] => {
                if let Some((hour, minute, seconds)) = clock(time) {
                    if let (Some(hour), Some(minute)) = (two_digits(&hour), two_digits(&minute)) {
                        return Some(format!("daily {hour}:{minute}"));
                    }
                    if hour == "*" {
                        if let Some((_, step)) = minute.split_once('/') {
                            return Some(format!("every {step} min"));
                        }
                        if two_digits(&minute).is_some() && seconds.is_some() {
                            return Some("hourly".to_owned());
                        }
                    }
                }
            }
            [day, "*-*-*", time]
                if ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].contains(day) =>
            {
                if let Some((hour, minute, _)) = clock(time)
                    && let (Some(hour), Some(minute)) = (two_digits(&hour), two_digits(&minute))
                {
                    return Some(format!("weekly, {day} {hour}:{minute}"));
                }
            }
            _ => {}
        }
        return Some(cal);
    }
    let mono = value(monotonic, "OnUnitActiveUSec=")?;
    let mut spaced = String::new();
    let mut previous: Option<char> = None;
    for c in mono.chars() {
        if previous.is_some_and(|p| p.is_ascii_digit()) && c.is_ascii_lowercase() {
            spaced.push(' ');
        }
        spaced.push(c);
        previous = Some(c);
    }
    Some(format!("every {spaced}"))
}

/// A routine by its first words: "Morning brief: ..." is `Morning brief`; "Standing job,
/// Thursday: draft three posts ..." is `Thursday: draft three posts ...` (its job, cut short).
#[must_use]
pub fn routine_name(what: &str) -> String {
    let head_end = what.find([':', '.']);
    let head = match head_end {
        Some(end) if (3..=60).contains(&what[..end].chars().count()) => {
            what[..end].trim().to_owned()
        }
        _ => what.chars().take(60).collect(),
    };
    let Some(job) = head.strip_prefix("Standing job, ") else {
        return head;
    };
    let Some(colon) = what.find(':') else {
        return head;
    };
    let rest: String = what[colon + 1..].trim().chars().take(70).collect();
    format!("{job}: {rest}")
}

/// When a routine at `at` (on `day`, or every day) runs next after `now`, a local time: the
/// first of the next eight days whose time is still to come; `now` itself when none is.
#[must_use]
pub fn next_routine(
    at: jiff::civil::Time,
    day: Option<jiff::civil::Weekday>,
    now: jiff::civil::DateTime,
) -> jiff::civil::DateTime {
    let today = now.date();
    for ahead in 0..8_i64 {
        let Ok(date) = today.checked_add(jiff::Span::new().days(ahead)) else {
            continue;
        };
        let then = date.to_datetime(at);
        if then > now && day.is_none_or(|day| date.weekday() == day) {
            return then;
        }
    }
    now
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_routine_runs_next_at_its_time_on_its_day() {
        let now = jiff::civil::date(2026, 11, 26).at(9, 0, 0, 0);
        let eight = jiff::civil::time(8, 0, 0, 0);
        let ten = jiff::civil::time(10, 0, 0, 0);
        assert_eq!(
            next_routine(ten, None, now),
            jiff::civil::date(2026, 11, 26).at(10, 0, 0, 0),
            "later today"
        );
        assert_eq!(
            next_routine(eight, None, now),
            jiff::civil::date(2026, 11, 27).at(8, 0, 0, 0),
            "tomorrow: today's has gone"
        );
        assert_eq!(
            next_routine(eight, Some(jiff::civil::Weekday::Saturday), now),
            jiff::civil::date(2026, 11, 28).at(8, 0, 0, 0),
            "its weekday"
        );
    }

    #[test]
    fn how_often_in_words() {
        assert_eq!(every(30), "every 30 min");
        assert_eq!(every(60), "hourly");
        assert_eq!(every(120), "every 2 h");
        assert_eq!(every(90), "every 1.5 h");
        assert_eq!(every(1440), "daily");
        assert_eq!(every(4320), "every 3 days");
    }

    #[test]
    fn a_timers_schedule_in_words() {
        let cal = |c: &str| repeat(Some(&format!("{{ OnCalendar={c} ; next_elapse=x }}")), None);
        assert_eq!(cal("*-*-* 08:00:30").as_deref(), Some("daily 08:00"));
        assert_eq!(
            cal("Sat *-*-* 20:00:00").as_deref(),
            Some("weekly, Sat 20:00")
        );
        assert_eq!(cal("*-*-* *:00:00").as_deref(), Some("hourly"));
        assert_eq!(cal("*-*-* *:00/5").as_deref(), Some("every 5 min"));
        assert_eq!(cal("Mon..Fri 09:00").as_deref(), Some("Mon..Fri 09:00"));
        assert_eq!(
            repeat(None, Some("{ OnUnitActiveUSec=5min ; next_elapse=x }")).as_deref(),
            Some("every 5 min")
        );
        assert_eq!(repeat(None, None), None);
    }

    #[test]
    fn a_routine_is_named_by_its_first_words() {
        assert_eq!(
            routine_name("Morning brief: read the news"),
            "Morning brief"
        );
        assert_eq!(
            routine_name("Standing job, Thursday: draft three posts for the week"),
            "Thursday: draft three posts for the week"
        );
    }
}
