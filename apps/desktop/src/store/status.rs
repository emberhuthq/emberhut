//! How the app words its own state: the assistant's status line, and what went wrong.

use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::backend::Error;
use emberhut_api::live::Live;
use emberhut_api::state::State;

/// The line under the assistant's name and in the window bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StatusLine {
    /// What it says.
    pub(crate) text: String,
    /// How much it asks for the user's attention (its colour).
    pub(crate) tone: Tone,
}

/// How a status line is coloured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    /// Muted.
    Quiet,
    /// The assistant is working.
    Working,
    /// Worth a look: offline, or most of a quota window used.
    Warn,
    /// Out of quota, or nearly.
    Danger,
}

/// Where the assistant stands, for its status line.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Standing<'a> {
    /// The state on hand.
    pub(crate) state: Option<&'a State>,
    /// Whether the state on hand is the open room's (busy is a room's).
    pub(crate) here: bool,
    /// Why polls fail, once they have failed for a while.
    pub(crate) offline: Option<Outage>,
    /// The live stream's step.
    pub(crate) live: Option<&'a Live>,
    /// Now.
    pub(crate) now: Timestamp,
    /// The time zone times are shown in.
    pub(crate) zone: &'a TimeZone,
}

/// Why the state polls fail, for the status line's word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outage {
    /// The server can't be reached (or this device failed the request): `Offline`.
    Unreachable,
    /// The server was reached but answered with an error, or with a state this app can't
    /// read: `Server error`.
    ServerError,
}

impl Outage {
    /// The outage a poll failing with `error` is part of.
    #[must_use]
    pub(crate) fn of(error: &Error) -> Self {
        match error {
            Error::Refused { .. } | Error::Unreadable(_) | Error::WrongMachine => Self::ServerError,
            Error::Unreachable(_) | Error::SignInRequired | Error::Local(_) => Self::Unreachable,
        }
    }
}

/// The status line: `Connecting…` before a state, `Offline` or `Server error` while polls fail,
/// then `Paused`, `Working…` or
/// `Watching` with how many background jobs run (`· 2 jobs`), and the quota: `Out of quota
/// until 3 PM` while the account is out, else the fuller window once 80% of it is used (`· 81%
/// of weekly used`, red from 95%).
#[must_use]
pub(crate) fn line(standing: Standing<'_>) -> StatusLine {
    let quiet = |text: &str, tone| StatusLine {
        text: text.to_owned(),
        tone,
    };
    let Some(state) = standing.state else {
        return quiet("Connecting…", Tone::Quiet);
    };
    match standing.offline {
        Some(Outage::Unreachable) => return quiet("Offline", Tone::Warn),
        Some(Outage::ServerError) => return quiet("Server error", Tone::Warn),
        None => {}
    }
    let working = standing.live.is_some() || standing.here && state.busy;
    let jobs = state.tasks.iter().filter(|task| task.is_running()).count();
    let mut text = if !state.on {
        "Paused"
    } else if working {
        "Working…"
    } else {
        "Watching"
    }
    .to_owned();
    if jobs > 0 {
        let plural = if jobs == 1 { "" } else { "s" };
        text = format!("{text} · {jobs} job{plural}");
    }
    let base_tone = if working && state.on {
        Tone::Working
    } else {
        Tone::Quiet
    };
    let account = state.account.as_ref();
    let limited = account
        .and_then(|account| account.limited_until.as_ref())
        .and_then(|until| until.as_str().parse::<Timestamp>().ok())
        .filter(|until| *until > standing.now);
    if let Some(until) = limited {
        let at = until.to_zoned_datetime(standing.zone.clone());
        return StatusLine {
            text: format!("Out of quota until {}", at.strftime("%-I:%M %p")),
            tone: Tone::Danger,
        };
    }
    let fullest = [
        ("5-hour", account.and_then(|account| account.used5h)),
        ("weekly", account.and_then(|account| account.used7d)),
    ]
    .into_iter()
    .filter_map(|(window, used)| Some((window, used?)))
    .filter(|(_, used)| *used >= 0.8)
    .max_by(|a, b| a.1.total_cmp(&b.1));
    let tone = match fullest {
        Some((window, used)) => {
            text = format!("{text} · {} of {window} used", crate::format::percent(used));
            if used >= 0.95 {
                Tone::Danger
            } else {
                Tone::Warn
            }
        }
        None => base_tone,
    };
    StatusLine { text, tone }
}

/// What a failed poll says to the user.
#[must_use]
pub(crate) fn problem(error: &Error) -> String {
    match error {
        Error::Unreachable(_) => "Can't reach the server · retrying".to_owned(),
        Error::SignInRequired => "This device has to sign in".to_owned(),
        Error::Refused {
            status: Some(401), ..
        } => "This device isn't let in".to_owned(),
        Error::Refused {
            message: Some(message),
            ..
        } => message.clone(),
        Error::Refused {
            status: Some(status),
            message: None,
        } => {
            format!("The server said {status} · retrying")
        }
        Error::Refused {
            status: None,
            message: None,
        } => "The server said no · retrying".to_owned(),
        Error::Unreadable(cause) => format!("The server sent a state this app can't read: {cause}"),
        Error::WrongMachine => error.to_string(),
        Error::Local(cause) => format!("Something went wrong on this device: {cause}"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixtures::state;

    fn on() -> State {
        state(json!({"on": true}))
    }

    fn step(doing: &str) -> Live {
        Live {
            text: String::new(),
            doing: Some(doing.into()),
        }
    }

    fn at(now: &str) -> Timestamp {
        now.parse().unwrap()
    }

    static UTC: TimeZone = TimeZone::UTC;

    fn standing(state: Option<&State>) -> Standing<'_> {
        Standing {
            state,
            here: true,
            offline: None,
            live: None,
            now: at("2026-01-10T12:00:00Z"),
            zone: &UTC,
        }
    }

    #[test]
    fn before_a_state_it_connects_and_offline_says_so() {
        assert_eq!(line(standing(None)).text, "Connecting…");
        let on = on();
        let offline = Standing {
            offline: Some(Outage::Unreachable),
            ..standing(Some(&on))
        };
        assert_eq!(
            line(offline),
            StatusLine {
                text: "Offline".into(),
                tone: Tone::Warn
            }
        );
        let failing = Standing {
            offline: Some(Outage::ServerError),
            ..standing(Some(&on))
        };
        assert_eq!(line(failing).text, "Server error");
    }

    #[test]
    fn it_watches_works_or_is_paused() {
        let watching = on();
        assert_eq!(line(standing(Some(&watching))).text, "Watching");
        let busy = State { busy: true, ..on() };
        assert_eq!(
            line(standing(Some(&busy))),
            StatusLine {
                text: "Working…".into(),
                tone: Tone::Working
            }
        );
        let elsewhere = Standing {
            here: false,
            ..standing(Some(&busy))
        };
        assert_eq!(
            line(elsewhere).text,
            "Watching",
            "busy in the room just left"
        );
        let live = step("Reading");
        let streaming = Standing {
            live: Some(&live),
            ..standing(Some(&watching))
        };
        assert_eq!(line(streaming).text, "Working…");
        let off = state(json!({"on": false}));
        assert_eq!(line(standing(Some(&off))).text, "Paused");
    }

    #[test]
    fn running_background_jobs_are_counted() {
        let task = |id: &str, status: &str| json!({"id": id, "status": status});
        let one = state(json!({"tasks": [task("a", "running"), task("b", "completed")]}));
        assert_eq!(line(standing(Some(&one))).text, "Watching · 1 job");
        let two = state(json!({"tasks": [task("a", "running"), task("b", "running")]}));
        assert_eq!(line(standing(Some(&two))).text, "Watching · 2 jobs");
    }

    #[test]
    fn the_quota_shows_once_most_of_a_window_is_used() {
        let used =
            |five: f64, week: f64| state(json!({"account": {"used5h": five, "used7d": week}}));
        let calm = used(0.5, 0.79);
        assert_eq!(line(standing(Some(&calm))).text, "Watching");
        let weekly = used(0.5, 0.81);
        assert_eq!(
            line(standing(Some(&weekly))),
            StatusLine {
                text: "Watching · 81% of weekly used".into(),
                tone: Tone::Warn
            }
        );
        let nearly_out = used(0.96, 0.85);
        let nearly_out = line(standing(Some(&nearly_out)));
        assert_eq!(nearly_out.text, "Watching · 96% of 5-hour used");
        assert_eq!(nearly_out.tone, Tone::Danger);
    }

    #[test]
    fn out_of_quota_says_until_when() {
        let out =
            state(json!({"account": {"used5h": 1.0, "limitedUntil": "2026-01-10T15:00:00Z"}}));
        assert_eq!(
            line(standing(Some(&out))),
            StatusLine {
                text: "Out of quota until 3:00 PM".into(),
                tone: Tone::Danger
            }
        );
        let back = state(json!({"account": {"limitedUntil": "2026-01-10T11:00:00Z"}}));
        assert_eq!(
            line(standing(Some(&back))).text,
            "Watching",
            "it has passed"
        );
    }

    #[test]
    fn a_server_that_answers_with_an_error_isnt_offline() {
        let refused = Error::Refused {
            status: Some(500),
            message: None,
        };
        assert_eq!(Outage::of(&refused), Outage::ServerError);
        assert_eq!(
            Outage::of(&Error::Unreadable("expected a map".into())),
            Outage::ServerError
        );
        assert_eq!(
            Outage::of(&Error::Unreachable("down".into())),
            Outage::Unreachable
        );
    }

    #[test]
    fn problems_are_worded_for_the_user() {
        let refused = |status, message: Option<&str>| Error::Refused {
            status,
            message: message.map(str::to_owned),
        };
        assert_eq!(
            problem(&refused(Some(401), Some("x"))),
            "This device isn't let in"
        );
        assert_eq!(
            problem(&refused(Some(503), Some("Restarting"))),
            "Restarting"
        );
        assert_eq!(
            problem(&refused(Some(502), None)),
            "The server said 502 · retrying"
        );
        assert_eq!(
            problem(&refused(None, None)),
            "The server said no · retrying"
        );
        assert_eq!(
            problem(&Error::Unreachable("down".into())),
            "Can't reach the server · retrying"
        );
        assert_eq!(
            problem(&Error::Unreadable("expected a map".into())),
            "The server sent a state this app can't read: expected a map"
        );
        assert_eq!(
            problem(&Error::SignInRequired),
            "This device has to sign in"
        );
    }
}
