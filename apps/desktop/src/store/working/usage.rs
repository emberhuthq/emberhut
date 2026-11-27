//! The Claude accounts' use, for Settings' usage card: the state's rows worded (each window's
//! bar colour, when it resets, what the account is for, when it was checked), the account the
//! assistant runs on and the user's choice, the row open in full, and the checks: at launch,
//! each time the window comes to the front, and on Check now. A check asks the server to look
//! at every account; the state's poll, kicked at once, brings the numbers.

use std::sync::Arc;

use emberhut_api::usage::{AssistantAccount, SCOPE_ANY, UsageRow, UseAccountRequest};
use gpui_kit::AppContext as _;
use gpui_kit::Context;
use jiff::tz::TimeZone;
use jiff::{Timestamp, Unit};

use crate::store::devices::relative;
use crate::store::poll::{self, Kicked, Kicker};
use crate::store::{Store, update_or_log};

/// How full a window is, as its bar's colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fill {
    /// Room left.
    Green,
    /// Over 70% used.
    Amber,
    /// Over 90% used.
    Red,
}

/// The bar colour for `used` percent of a window: green, amber over 70%, red over 90%.
#[must_use]
pub(crate) fn fill(used: Option<f64>) -> Fill {
    match used.unwrap_or(0.) {
        u if u > 90. => Fill::Red,
        u if u > 70. => Fill::Amber,
        _ => Fill::Green,
    }
}

/// A window's use in words: `82%`, or `–` with no data.
#[must_use]
pub(crate) fn percent(used: Option<f64>) -> String {
    used.map_or_else(
        || "–".to_owned(),
        |used| format!("{}%", crate::format::whole(used)),
    )
}

/// A time of day as the card says it, to the minute: `4:50 am` today, `Tue 4 pm` another day.
#[must_use]
pub(crate) fn when(at: Timestamp, now: Timestamp, zone: &TimeZone) -> String {
    let at = at
        .round(Unit::Minute)
        .unwrap_or(at)
        .to_zoned_datetime(zone.clone());
    let hour = at.hour();
    let twelve = match hour % 12 {
        0 => 12,
        other => other,
    };
    let minute = match at.minute() {
        0 => String::new(),
        minute => format!(":{minute:02}"),
    };
    let half = if hour < 12 { "am" } else { "pm" };
    let time = format!("{twelve}{minute} {half}");
    if at.date() == now.to_zoned_datetime(zone.clone()).date() {
        time
    } else {
        format!("{} {time}", at.strftime("%a"))
    }
}

/// How long until `at`, in words: `in 5 minutes`, `in 2 hours`, `tomorrow`, `in 3 days`; a
/// time gone by reads as [`relative`] does.
#[must_use]
pub(crate) fn until(at: Timestamp, now: Timestamp) -> String {
    let seconds = at.as_second() - now.as_second();
    if seconds <= 0 {
        return relative(at, now);
    }
    let count = |n: i64, unit: &str| format!("in {n} {unit}{}", if n == 1 { "" } else { "s" });
    match seconds {
        0..60 => "in under a minute".to_owned(),
        60..3600 => count(seconds / 60, "minute"),
        3600..86_400 => count(seconds / 3600, "hour"),
        86_400..172_800 => "tomorrow".to_owned(),
        _ => count(seconds / 86_400, "day"),
    }
}

/// `text` as a time, if it reads as one.
fn time(text: Option<&emberhut_api::time::Timestamp>) -> Option<Timestamp> {
    text?.as_str().parse().ok()
}

/// What an account is for, as its details say it.
#[must_use]
pub(crate) fn scope_words(row: &UsageRow) -> Option<String> {
    match row.scope.as_str() {
        "" => None,
        SCOPE_ANY => Some("For any work".to_owned()),
        // The server's own word names the work; the app has no name of its own for it.
        _ => Some("Kept for other work; your ember can't be pinned to it".to_owned()),
    }
}

/// A row's lines when it is open: what it is for, where it runs, each reset with how long
/// until then, and when it was checked.
#[must_use]
pub(crate) fn details(row: &UsageRow, now: Timestamp, zone: &TimeZone) -> Vec<String> {
    let mut lines: Vec<String> = scope_words(row).into_iter().collect();
    if !row.places.is_empty() {
        lines.push(format!("On the {}", row.places.join(" and ")));
    }
    let windows = [
        ("5 hours", row.resets_5h.as_ref()),
        ("Week", row.resets_7d.as_ref()),
    ];
    for (name, resets) in windows {
        if let Some(at) = time(resets) {
            lines.push(format!(
                "{name}: resets {}, {}",
                when(at, now, zone),
                until(at, now)
            ));
        }
    }
    if let Some(at) = time(row.checked.as_ref()) {
        lines.push(format!("Checked {}", relative(at, now)));
    }
    lines
}

/// A row's line in red instead of its bars: `not logged in`, `out until 4 pm`; or `no data`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RowLine {
    /// It isn't logged in.
    NotLoggedIn,
    /// The server has no numbers for it.
    NoData,
    /// Its bars, under a red `out until…` while it is out.
    Bars {
        /// `out until 4 pm`, while it is out.
        out: Option<String>,
    },
}

/// What a row shows under its name.
#[must_use]
pub(crate) fn row_line(row: &UsageRow, now: Timestamp, zone: &TimeZone) -> RowLine {
    if !row.logged_in {
        RowLine::NotLoggedIn
    } else if !row.has_numbers() {
        RowLine::NoData
    } else {
        RowLine::Bars {
            out: time(row.limited_until.as_ref())
                .map(|until| format!("out until {}", when(until, now, zone))),
        }
    }
}

/// A window's `resets Tue 4 pm`, or nothing.
#[must_use]
pub(crate) fn resets(
    at: Option<&emberhut_api::time::Timestamp>,
    now: Timestamp,
    zone: &TimeZone,
) -> String {
    time(at).map_or_default(|at| format!("resets {}", when(at, now, zone)))
}

/// `Checked 3 minutes ago`, by the oldest check; nothing before any.
#[must_use]
pub(crate) fn checked(rows: &[UsageRow], now: Timestamp) -> Option<String> {
    let oldest = rows
        .iter()
        .filter_map(|row| time(row.checked.as_ref()))
        .min()?;
    Some(format!("Checked {}", relative(oldest, now)))
}

/// The line under the account picker, and whether it is a warning (the pinned one is out).
#[must_use]
pub(crate) fn runs_on(account: &AssistantAccount, rows: &[UsageRow]) -> (String, bool) {
    let name = |id: Option<&str>| {
        rows.iter()
            .find(|row| Some(row.id.as_str()) == id)
            .map_or_else(|| id.unwrap_or("?").to_owned(), |row| row.name.clone())
    };
    let on = name(account.on.as_deref());
    match &account.pin {
        None => (
            format!("Now on {on} · Auto picks the account with room"),
            false,
        ),
        Some(pin) if account.pin_out => (
            format!(
                "Now on {on}: {} is out, your ember goes back when it's free",
                name(Some(pin))
            ),
            true,
        ),
        Some(_) => (
            format!("Now on {on} · stays there until you pick Auto"),
            false,
        ),
    }
}

/// The picker's choices: Auto, then each account for any work, each with its request.
#[must_use]
pub(crate) fn choices(rows: &[UsageRow]) -> Vec<(String, UseAccountRequest)> {
    std::iter::once(("Auto".to_owned(), UseAccountRequest::auto()))
        .chain(
            rows.iter()
                .filter(|row| row.can_be_pinned())
                .map(|row| (row.name.clone(), UseAccountRequest::pin(row.id.clone()))),
        )
        .collect()
}

/// The picker's label: Auto, or the pinned account's name.
#[must_use]
pub(crate) fn picked(account: &AssistantAccount, rows: &[UsageRow]) -> String {
    account.pin.as_ref().map_or_else(
        || "Auto".to_owned(),
        |pin| {
            rows.iter()
                .find(|row| &row.id == pin)
                .map_or_else(|| pin.clone(), |row| row.name.clone())
        },
    )
}

/// The usage card's own state.
#[derive(Debug)]
pub(crate) struct UsageCard {
    /// The account shown in full.
    open: Option<String>,
    /// A pick of the assistant's account is on its way.
    picking: bool,
    /// Wakes the check; the check's end of it waits here until it starts.
    kicker: Kicker,
    kicked: Option<Kicked>,
}

impl Default for UsageCard {
    fn default() -> Self {
        let (kicker, waiting) = poll::kicker();
        Self {
            open: None,
            picking: false,
            kicker,
            kicked: Some(waiting),
        }
    }
}

impl UsageCard {
    /// Whether `id`'s row is open in full.
    #[must_use]
    pub(crate) fn is_open(&self, id: &str) -> bool {
        self.open.as_deref() == Some(id)
    }

    /// Whether a pick of the assistant's account is on its way.
    #[must_use]
    pub(crate) fn is_picking(&self) -> bool {
        self.picking
    }

    /// Asks for a check now.
    pub(super) fn kick(&self) {
        self.kicker.kick();
    }
}

impl Store {
    /// The usage card's state.
    #[must_use]
    pub(crate) fn usage_card(&self) -> &UsageCard {
        &self.working.usage
    }

    /// Opens `id`'s row in full, or folds it again.
    pub(crate) fn toggle_usage_row(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let card = &mut self.working.usage;
        card.open = (!card.is_open(id)).then(|| id.to_owned());
        cx.notify();
    }

    /// Check now: the server checks every account, the poll brings the numbers.
    pub(crate) fn check_usage_now(&self) {
        self.working.usage.kick();
    }

    /// Pins the assistant to an account, or sets it to Auto. The card shows the answer at
    /// once and the poll goes; a refusal says `Account not switched`.
    pub(crate) fn use_account(&mut self, request: UseAccountRequest, cx: &mut Context<'_, Self>) {
        if self.working.usage.picking {
            return;
        }
        self.working.usage.picking = true;
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let reply = cx
                .background_spawn(async move { backend.use_account(&request) })
                .await;
            update_or_log(&this, cx, |store, cx| {
                store.working.usage.picking = false;
                match reply {
                    Ok(reply) => {
                        if let Some(state) = store.state.as_mut() {
                            state.assistant_account = Some(reply.account);
                            if !reply.usage.is_empty() {
                                state.usage = reply.usage;
                            }
                        }
                    }
                    Err(error) => store.action_failed("Account not switched", &error, cx),
                }
                store.kicker.kick();
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Starts the checks' loop (once): a check at launch, then one per kick.
    pub(super) fn start_usage_checks(&mut self, cx: &mut Context<'_, Self>) {
        let Some(kicked) = self.working.usage.kicked.take() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            loop {
                let Some(backend) = update_or_log(&this, cx, |store, _| Arc::clone(&store.backend))
                else {
                    return;
                };
                let checked = cx
                    .background_spawn(async move { backend.check_usage() })
                    .await;
                if let Err(error) = checked {
                    log::info!("couldn't ask for the accounts' use: {error}");
                }
                if update_or_log(&this, cx, |store, _| store.kicker.kick()).is_none() {
                    return;
                }
                kicked.wait().await;
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn row(value: &serde_json::Value) -> UsageRow {
        let mut base = json!({"id": "a-1", "name": "Example", "scope": "any", "loggedIn": true,
            "used5h": 25, "resets5h": "2026-11-25T01:00:00Z", "used7d": 82,
            "resets7d": "2026-11-28T14:00:00Z", "limitedUntil": null,
            "checked": "2026-11-24T21:57:00Z", "places": ["Linux PC", "Mac"], "best": true});
        if let (Some(base), Some(more)) = (base.as_object_mut(), value.as_object()) {
            base.extend(more.clone());
        }
        serde_json::from_value(base).unwrap()
    }

    fn now() -> Timestamp {
        "2026-11-24T22:00:00Z".parse().unwrap()
    }

    #[test]
    fn bars_go_amber_over_70_and_red_over_90() {
        assert_eq!(fill(Some(70.)), Fill::Green);
        assert_eq!(fill(Some(71.)), Fill::Amber);
        assert_eq!(fill(Some(91.)), Fill::Red);
        assert_eq!(fill(None), Fill::Green);
        assert_eq!(percent(Some(81.6)), "82%");
        assert_eq!(percent(None), "–");
    }

    #[test]
    fn a_time_today_is_its_hour_and_another_days_has_its_weekday() {
        let utc = TimeZone::UTC;
        let today: Timestamp = "2026-11-24T04:50:10Z".parse().unwrap();
        assert_eq!(when(today, now(), &utc), "4:50 am");
        let tuesday: Timestamp = "2026-11-25T16:00:00Z".parse().unwrap();
        assert_eq!(when(tuesday, now(), &utc), "Wed 4 pm");
        let noon: Timestamp = "2026-11-24T12:00:00Z".parse().unwrap();
        assert_eq!(when(noon, now(), &utc), "12 pm");
    }

    #[test]
    fn how_long_until_a_reset() {
        let later = |seconds: i64| Timestamp::from_second(now().as_second() + seconds).unwrap();
        assert_eq!(until(later(30), now()), "in under a minute");
        assert_eq!(until(later(300), now()), "in 5 minutes");
        assert_eq!(until(later(7200), now()), "in 2 hours");
        assert_eq!(until(later(90_000), now()), "tomorrow");
        assert_eq!(until(later(3 * 86_400), now()), "in 3 days");
        assert_eq!(until(later(-120), now()), "2 minutes ago");
    }

    #[test]
    fn a_row_says_why_it_has_no_bars() {
        let utc = TimeZone::UTC;
        assert_eq!(
            row_line(&row(&json!({"loggedIn": false})), now(), &utc),
            RowLine::NotLoggedIn
        );
        assert_eq!(
            row_line(&row(&json!({"used5h": null, "used7d": null})), now(), &utc),
            RowLine::NoData
        );
        assert_eq!(
            row_line(
                &row(&json!({"limitedUntil": "2026-11-24T23:00:00Z"})),
                now(),
                &utc
            ),
            RowLine::Bars {
                out: Some("out until 11 pm".into())
            }
        );
    }

    #[test]
    fn an_open_row_tells_its_resets_and_its_check() {
        let lines = details(&row(&json!({})), now(), &TimeZone::UTC);
        assert_eq!(
            lines,
            [
                "For any work",
                "On the Linux PC and Mac",
                "5 hours: resets Wed 1 am, in 3 hours",
                "Week: resets Sat 2 pm, in 3 days",
                "Checked 3 minutes ago",
            ]
        );
        assert_eq!(
            checked(&[row(&json!({}))], now()).as_deref(),
            Some("Checked 3 minutes ago")
        );
        assert_eq!(checked(&[], now()), None);
    }

    #[test]
    fn the_picker_says_where_the_assistant_runs() {
        let rows = [
            row(&json!({})),
            row(&json!({"id": "a-2", "name": "Second", "scope": "example"})),
        ];
        let account = |pin: Option<&str>, out: bool| AssistantAccount {
            on: Some("a-1".into()),
            pin: pin.map(str::to_owned),
            mode: String::new(),
            pin_out: out,
            rooms: Vec::new(),
            waits: None,
        };
        assert_eq!(
            runs_on(&account(None, false), &rows),
            (
                "Now on Example · Auto picks the account with room".into(),
                false
            )
        );
        assert_eq!(
            runs_on(&account(Some("a-1"), false), &rows).0,
            "Now on Example · stays there until you pick Auto"
        );
        assert_eq!(
            runs_on(&account(Some("a-2"), true), &rows),
            (
                "Now on Example: Second is out, your ember goes back when it's free".into(),
                true
            )
        );
        let names: Vec<String> = choices(&rows).into_iter().map(|(name, _)| name).collect();
        assert_eq!(
            names,
            ["Auto", "Example"],
            "one project's account isn't offered"
        );
        assert_eq!(picked(&account(Some("a-2"), false), &rows), "Second");
        assert_eq!(picked(&account(None, false), &rows), "Auto");
    }
}
