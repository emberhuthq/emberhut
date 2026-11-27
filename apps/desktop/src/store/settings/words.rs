//! How Settings' times read: a day (`20 Nov`), a clock time (`21:40`), how long ago a profile
//! was seen (`5m ago`). Pure functions of their inputs: the time zone and "now" are passed in.

use jiff::Timestamp;
use jiff::tz::TimeZone;

/// The day of `iso` in `zone`: `20 Nov`. `None` for text that isn't a time.
#[must_use]
pub(crate) fn day_month(iso: &str, zone: &TimeZone) -> Option<String> {
    let at: Timestamp = iso.parse().ok()?;
    Some(
        at.to_zoned_datetime(zone.clone())
            .strftime("%-d %b")
            .to_string(),
    )
}

/// The time of day of `iso` in `zone`: `21:40`.
#[must_use]
pub(crate) fn clock(iso: &str, zone: &TimeZone) -> Option<String> {
    let at: Timestamp = iso.parse().ok()?;
    Some(
        at.to_zoned_datetime(zone.clone())
            .strftime("%H:%M")
            .to_string(),
    )
}

/// The time of day of `millis` since the Unix epoch, in `zone`.
#[must_use]
pub(crate) fn clock_of_millis(millis: f64, zone: &TimeZone) -> Option<String> {
    if !millis.is_finite() {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a finite time in milliseconds, rounded: far inside i64"
    )]
    let at = Timestamp::from_millisecond(millis.round() as i64).ok()?;
    Some(
        at.to_zoned_datetime(zone.clone())
            .strftime("%H:%M")
            .to_string(),
    )
}

/// The day and time of `iso` in `zone`: `1 Nov 2026, 09:00`.
#[must_use]
pub(crate) fn day_and_time(iso: &str, zone: &TimeZone) -> Option<String> {
    let at: Timestamp = iso.parse().ok()?;
    Some(
        at.to_zoned_datetime(zone.clone())
            .strftime("%-d %b %Y, %H:%M")
            .to_string(),
    )
}

/// How long ago a profile was last online: `Just now`, `5m ago`, `3h ago`, `2d ago`, or
/// `Never` when it never was.
#[must_use]
pub(crate) fn seen_ago(iso: Option<&str>, now: Timestamp) -> String {
    let Some(at) = iso.and_then(|iso| iso.parse::<Timestamp>().ok()) else {
        return "Never".to_owned();
    };
    let minutes = ((now.as_second() - at.as_second()) / 60).max(0);
    match minutes {
        0 => "Just now".to_owned(),
        1..60 => format!("{minutes}m ago"),
        60..1440 => format!("{}h ago", minutes / 60),
        _ => format!("{}d ago", minutes / 1440),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_in_the_zone_given() {
        let utc = TimeZone::UTC;
        assert_eq!(
            day_month("2026-11-20T10:00:00Z", &utc).as_deref(),
            Some("20 Nov")
        );
        assert_eq!(
            clock("2026-11-24T22:30:00.000Z", &utc).as_deref(),
            Some("22:30")
        );
        assert_eq!(
            clock_of_millis(1_795_600_000_000.0, &utc).as_deref(),
            Some("09:46")
        );
        assert_eq!(clock_of_millis(f64::NAN, &utc), None);
        assert_eq!(
            day_and_time("2026-11-01T09:00:00Z", &utc).as_deref(),
            Some("1 Nov 2026, 09:00")
        );
        assert_eq!(day_month("soon", &utc), None);
    }

    #[test]
    fn seen_counts_minutes_hours_and_days() {
        let now: Timestamp = "2026-01-10T12:00:00Z".parse().unwrap();
        assert_eq!(seen_ago(Some("2026-01-10T11:59:40Z"), now), "Just now");
        assert_eq!(seen_ago(Some("2026-01-10T11:55:00Z"), now), "5m ago");
        assert_eq!(seen_ago(Some("2026-01-10T09:00:00Z"), now), "3h ago");
        assert_eq!(seen_ago(Some("2026-01-08T12:00:00Z"), now), "2d ago");
        assert_eq!(seen_ago(None, now), "Never");
    }
}
