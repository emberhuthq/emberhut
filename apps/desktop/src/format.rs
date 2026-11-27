//! How numbers, sizes and times read in the app. Pure functions of their inputs ("now" is
//! passed in), so they are tested without a clock.

use jiff::{Timestamp, ZonedDateTime};

/// Bytes in a gigabyte as sizes are worded here: 1024³, a GiB, as the server counts them.
const GIGABYTE: f64 = 1024.0 * 1024.0 * 1024.0;

/// A size in bytes as `GB` the way the server's warnings word it, so a bar and the warning
/// under it agree: gigabytes of 1024³ bytes, one decimal under 100 GB and whole from there
/// (`63.9 GB`, `1863 GB`), halves rounded away from zero; `–` when unknown. The one place
/// sizes in GB are worded.
#[must_use]
pub(crate) fn gigabytes(bytes: Option<f64>) -> String {
    match bytes {
        Some(bytes) if bytes >= 100.0 * GIGABYTE => format!("{} GB", whole(bytes / GIGABYTE)),
        Some(bytes) => format!("{:.1} GB", (bytes / GIGABYTE * 10.0).round() / 10.0),
        None => "–".to_owned(),
    }
}

/// A size in binary units: `512 B`, `1.5 KiB`, `12 KiB`, `3.4 MiB`, `1.2 GiB` (one decimal
/// under 10).
#[must_use]
pub(crate) fn file_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut unit = 1024_u64;
    let mut name = UNITS[0];
    for next in &UNITS[1..] {
        if bytes < unit * 1024 {
            break;
        }
        unit *= 1024;
        name = next;
    }
    let (bytes, unit) = (u128::from(bytes), u128::from(unit));
    let tenths = (bytes * 10 + unit / 2) / unit;
    if tenths < 100 {
        format!("{}.{} {name}", tenths / 10, tenths % 10)
    } else {
        format!("{} {name}", (bytes + unit / 2) / unit)
    }
}

/// How far ahead `at` is from `now`: `in 3 days`, `in 5 hours`, `in 12 minutes`, `within a
/// minute` (and `now` once it has passed).
#[must_use]
pub(crate) fn until(at: Timestamp, now: Timestamp) -> String {
    let seconds = at.as_second() - now.as_second();
    let count = |n: i64, unit: &str| format!("in {n} {unit}{}", if n == 1 { "" } else { "s" });
    match seconds {
        ..=0 => "now".to_owned(),
        1..60 => "within a minute".to_owned(),
        60..3600 => count(seconds / 60, "minute"),
        3600..86_400 => count(seconds / 3600, "hour"),
        _ => count(seconds / 86_400, "day"),
    }
}

/// A share (0 to 1) as a whole percentage: `0.256` reads `26%`.
#[must_use]
pub(crate) fn percent(share: f64) -> String {
    format!("{}%", whole(share * 100.0))
}

/// A number rounded to a whole one, for display.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "display values (percentages, °C, W, minutes) are far inside i64's range; NaN reads 0"
)]
pub(crate) fn whole(value: f64) -> i64 {
    value.round() as i64
}

/// Minutes left as `1h 5m` or `45m`.
#[must_use]
pub(crate) fn minutes_left(minutes: f64) -> String {
    let minutes = whole(minutes);
    if minutes >= 60 {
        format!("{}h {}m left", minutes / 60, minutes % 60)
    } else {
        format!("{minutes}m left")
    }
}

/// A count for a badge: `99+` past 99.
#[must_use]
pub(crate) fn badge(count: usize) -> String {
    if count > 99 {
        "99+".to_owned()
    } else {
        count.to_string()
    }
}

/// The first letter in capitals: `idle` reads `Idle`.
#[must_use]
pub(crate) fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map_or_default(|first| first.to_uppercase().chain(chars).collect())
}

/// How long after the message before it a message gets a time stamp above it.
const STAMP_GAP_SECONDS: i64 = 15 * 60;

/// How recent a message is, in seconds, for its time stamp to name the weekday: within the
/// last six days, so the name is never the same as today's.
const WEEKDAY_WITHIN: i64 = 6 * 86_400;

/// The line above a message that comes more than 15 minutes after the one before it (or has
/// none before it): `Today 08:31`, `Yesterday 21:04`, the weekday within the last week
/// (`Monday 09:12`), else `Mon 2 Nov 09:12`, in `now`'s time zone.
#[must_use]
pub(crate) fn time_stamp(
    previous: Option<&str>,
    at: Option<&str>,
    now: &ZonedDateTime,
) -> Option<String> {
    let at: Timestamp = at?.parse().ok()?;
    if let Some(previous) = previous.and_then(|p| p.parse::<Timestamp>().ok())
        && at.as_second() - previous.as_second() <= STAMP_GAP_SECONDS
    {
        return None;
    }
    let within_week = now.timestamp().as_second() - at.as_second() < WEEKDAY_WITHIN;
    let at = at.to_zoned_datetime(now.time_zone().clone());
    let (day, today) = (at.date(), now.date());
    let day = if day == today {
        "Today".to_owned()
    } else if today.yesterday().ok() == Some(day) {
        "Yesterday".to_owned()
    } else if within_week {
        at.strftime("%A").to_string()
    } else if day.year() == today.year() {
        at.strftime("%a %-d %b").to_string()
    } else {
        at.strftime("%a %-d %b %Y").to_string()
    };
    Some(format!("{day} {}", at.strftime("%H:%M")))
}

#[cfg(test)]
mod tests {
    use jiff::tz::TimeZone;

    use super::*;

    fn utc(iso: &str) -> ZonedDateTime {
        iso.parse::<Timestamp>()
            .unwrap()
            .to_zoned_datetime(TimeZone::UTC)
    }

    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

    #[test]
    fn sizes_count_gigabytes_of_1024_cubed_bytes() {
        assert_eq!(
            gigabytes(Some(64.0 * GIB)),
            "64.0 GB",
            "a 64 GiB Mac reads as the server's 64 GB warning"
        );
        assert_eq!(gigabytes(Some(68.7e9)), "64.0 GB");
        assert_eq!(gigabytes(Some(64e9)), "59.6 GB");
        assert_eq!(gigabytes(None), "–");
    }

    #[test]
    fn small_and_fractional_sizes_keep_one_decimal() {
        assert_eq!(gigabytes(Some(0.0)), "0.0 GB");
        assert_eq!(gigabytes(Some(512.0 * 1024.0 * 1024.0)), "0.5 GB");
        assert_eq!(gigabytes(Some(40e6)), "0.0 GB", "under 50 MiB reads 0.0");
        assert_eq!(gigabytes(Some(2.5e9)), "2.3 GB");
        assert_eq!(gigabytes(Some(1.25 * GIB)), "1.3 GB", "a half rounds up");
        assert_eq!(gigabytes(Some(0.25 * GIB)), "0.3 GB", "a half rounds up");
        assert_eq!(gigabytes(Some(1.24 * GIB)), "1.2 GB");
    }

    #[test]
    fn sizes_from_100_gigabytes_read_whole() {
        assert_eq!(gigabytes(Some(99.9 * GIB)), "99.9 GB");
        assert_eq!(
            gigabytes(Some(99.96 * GIB)),
            "100.0 GB",
            "as the server says it"
        );
        assert_eq!(gigabytes(Some(100.0 * GIB)), "100 GB");
        assert_eq!(gigabytes(Some(100.5 * GIB)), "101 GB", "a half rounds up");
        assert_eq!(
            gigabytes(Some(2e12)),
            "1863 GB",
            "no terabytes, as in the warnings"
        );
    }

    #[test]
    fn small_numbers_read_whole() {
        assert_eq!(percent(0.256), "26%");
        assert_eq!(whole(41.5), 42);
        assert_eq!(minutes_left(125.0), "2h 5m left");
        assert_eq!(minutes_left(45.0), "45m left");
        assert_eq!(badge(7), "7");
        assert_eq!(badge(120), "99+");
        assert_eq!(capitalized("idle"), "Idle");
        assert_eq!(capitalized(""), "");
    }

    #[test]
    fn time_stamps_show_after_a_quarter_hour_gap() {
        let now = utc("2026-11-25T12:00:00Z");
        let stamp = |previous: Option<&str>, at: &str| time_stamp(previous, Some(at), &now);
        assert_eq!(
            stamp(None, "2026-11-25T08:31:00Z").as_deref(),
            Some("Today 08:31")
        );
        assert_eq!(
            stamp(Some("2026-11-25T08:16:00Z"), "2026-11-25T08:31:00Z"),
            None,
            "15 minutes is no gap"
        );
        assert_eq!(
            stamp(Some("2026-11-25T08:15:00Z"), "2026-11-25T08:31:00Z").as_deref(),
            Some("Today 08:31")
        );
        assert_eq!(
            stamp(Some("2026-11-24T20:00:00Z"), "2026-11-24T21:04:00Z").as_deref(),
            Some("Yesterday 21:04")
        );
        assert_eq!(
            stamp(None, "2026-11-23T09:12:00Z").as_deref(),
            Some("Monday 09:12"),
            "within the week: its weekday"
        );
        assert_eq!(
            stamp(None, "2026-11-12T09:12:00Z").as_deref(),
            Some("Thu 12 Nov 09:12")
        );
        assert_eq!(
            stamp(None, "2025-11-23T09:12:00Z").as_deref(),
            Some("Sun 23 Nov 2025 09:12")
        );
    }

    #[test]
    fn sizes_read_in_binary_units() {
        assert_eq!(file_size(0), "0 B");
        assert_eq!(file_size(1023), "1023 B");
        assert_eq!(file_size(1024), "1.0 KiB");
        assert_eq!(file_size(1536), "1.5 KiB");
        assert_eq!(file_size(12 * 1024 + 100), "12 KiB");
        assert_eq!(file_size(3_565_158), "3.4 MiB");
        assert_eq!(file_size(500 * 1024 * 1024), "500 MiB");
        assert_eq!(file_size(1_288_490_189), "1.2 GiB");
        assert_eq!(file_size(u64::MAX), "16777216 TiB");
    }

    #[test]
    fn times_ahead_read_short() {
        let at = |text: &str| text.parse::<Timestamp>().unwrap();
        let now = at("2026-11-27T12:00:00Z");
        assert_eq!(until(at("2026-11-27T12:00:30Z"), now), "within a minute");
        assert_eq!(until(at("2026-11-27T12:12:00Z"), now), "in 12 minutes");
        assert_eq!(until(at("2026-11-27T13:00:00Z"), now), "in 1 hour");
        assert_eq!(until(at("2026-11-27T11:00:00Z"), now), "now");
    }
}
