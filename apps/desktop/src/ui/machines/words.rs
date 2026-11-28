//! What a hut's card says: its kind (from the platform and names its runner reports), whether
//! it is online or asleep and when it was last seen, what runs there now, and each of its
//! accounts with the room left. Plain functions of their inputs ("now" is passed in), so they
//! are tested without a window.

use emberhut_api::machines::{Machine, MachineState};
use emberhut_api::runner::accounts::ClaudeAccount;
use gpui_kit::assets::IconName;
use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::format::{gigabytes, percent, whole};

/// What kind of computer a hut is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HutKind {
    /// A Mac.
    Mac,
    /// A PC running Linux.
    Linux,
    /// A PC running Windows.
    Windows,
    /// A server or a box kept on in a corner (its name says so).
    Server,
    /// A computer rented in the cloud.
    Cloud,
    /// A computer whose runner didn't say what it runs.
    Computer,
}

/// Words in a hut's name or host name that make it a server.
const SERVER_WORDS: [&str; 4] = ["server", "vps", "srv", "nas"];

impl HutKind {
    /// The kind of `machine`: from the platform its runner reports, and a server when its name
    /// or host name says so.
    #[must_use]
    pub(crate) fn of(machine: &Machine) -> Self {
        let platform = machine
            .platform
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let named = format!(
            "{} {}",
            machine.name,
            machine.host.as_deref().unwrap_or_default()
        )
        .to_ascii_lowercase();
        match platform.as_str() {
            "darwin" | "macos" => Self::Mac,
            "win32" | "windows" => Self::Windows,
            "cloud" => Self::Cloud,
            _ if SERVER_WORDS.iter().any(|word| named.contains(word)) => Self::Server,
            "linux" => Self::Linux,
            _ => Self::Computer,
        }
    }

    /// Its name, as the card says it.
    #[must_use]
    pub(crate) const fn word(self) -> &'static str {
        match self {
            Self::Mac => "Mac",
            Self::Linux => "Linux PC",
            Self::Windows => "Windows PC",
            Self::Server => "Server",
            Self::Cloud => "Cloud",
            Self::Computer => "Computer",
        }
    }

    /// Its icon: a laptop for one with a battery, else a desktop, a tower, a rack or a cloud.
    #[must_use]
    pub(crate) const fn icon(self, laptop: bool) -> IconName {
        match self {
            Self::Mac | Self::Linux | Self::Windows if laptop => IconName::Laptop,
            Self::Mac | Self::Windows => IconName::Monitor,
            Self::Linux => IconName::PcCase,
            Self::Server => IconName::Server,
            Self::Cloud => IconName::Cloud,
            Self::Computer => IconName::Computer,
        }
    }
}

/// Whether a hut is there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Presence {
    /// Its runner reports.
    Online,
    /// No report for a minute: still counted on.
    Quiet,
    /// No report for two minutes: off, asleep or away.
    Asleep,
    /// It never reported.
    NotYet,
}

impl Presence {
    /// How `state` reads.
    #[must_use]
    pub(crate) fn of(state: &MachineState) -> Self {
        match state {
            MachineState::Ok | MachineState::Warn | MachineState::Low => Self::Online,
            MachineState::Stale => Self::Quiet,
            MachineState::Offline => Self::Asleep,
            // Unknown, and a state this app doesn't know yet.
            _ => Self::NotYet,
        }
    }

    /// Its word, for the pill.
    #[must_use]
    pub(crate) const fn word(self) -> &'static str {
        match self {
            Self::Online => "Online",
            Self::Quiet => "Quiet",
            Self::Asleep => "Asleep",
            Self::NotYet => "Not reported yet",
        }
    }
}

/// How long ago, short: `just now`, `5 min ago`, `2 h ago`, `3 d ago`.
#[must_use]
pub(crate) fn ago(seconds: f64) -> String {
    let seconds = whole(seconds.max(0.));
    match seconds {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..86_400 => format!("{} h ago", seconds / 3600),
        _ => format!("{} d ago", seconds / 86_400),
    }
}

/// Whether `machine` is there, and when it was last seen when it is quiet or asleep, as of
/// `now`: `Online`, `Asleep · last seen 2 h ago`.
#[must_use]
pub(crate) fn presence_line(machine: &Machine, now: Timestamp) -> String {
    let presence = Presence::of(&machine.state);
    if matches!(presence, Presence::Online | Presence::NotYet) {
        return presence.word().to_owned();
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "seconds since a sample: far inside f64's whole numbers"
    )]
    let seconds = machine
        .seen
        .as_ref()
        .and_then(crate::store::tasks::clock::instant)
        .map(|seen| (now.as_second() - seen.as_second()) as f64)
        .or(machine.age);
    match seconds {
        Some(seconds) => format!("{} · last seen {}", presence.word(), ago(seconds)),
        None => presence.word().to_owned(),
    }
}

/// What runs on `machine` now: `3 sessions · 1 running · 2 queued`, `Nothing running`.
#[must_use]
pub(crate) fn running_line(machine: &Machine) -> String {
    let sessions = machine.sessions.len();
    let running = machine.sessions.iter().filter(|s| s.running).count();
    let queued = machine
        .runner
        .as_ref()
        .and_then(|runner| runner.queued)
        .unwrap_or(0);
    if sessions == 0 && queued == 0 {
        return "Nothing running".to_owned();
    }
    let mut parts = vec![if sessions == 1 {
        "1 session".to_owned()
    } else {
        format!("{sessions} sessions")
    }];
    if running > 0 {
        parts.push(format!("{running} running"));
    }
    if queued > 0 {
        parts.push(format!("{queued} queued"));
    }
    parts.join(" · ")
}

/// Its processor and memory at a glance: `CPU 12% · Memory 18.0 of 64.0 GB · 1 GPU`; none when
/// its runner measures neither.
#[must_use]
pub(crate) fn resources_line(machine: &Machine) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(usage) = machine.cpu.as_ref().and_then(|cpu| cpu.usage) {
        parts.push(format!("CPU {}", percent(usage)));
    }
    if let Some(memory) = &machine.memory {
        let used = match (memory.total, memory.available) {
            (Some(total), Some(free)) => Some(total - free),
            _ => memory.used,
        };
        if used.is_some() && memory.total.is_some() {
            let used = gigabytes(used);
            let total = gigabytes(memory.total);
            parts.push(format!(
                "Memory {} of {total}",
                used.trim_end_matches(" GB")
            ));
        }
    }
    match machine.gpus.len() {
        0 => {}
        1 => parts.push("1 GPU".to_owned()),
        n => parts.push(format!("{n} GPUs")),
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// The accounts kept on the hut `machine`, in the server's order.
#[must_use]
pub(crate) fn accounts_on<'a>(
    accounts: &'a [ClaudeAccount],
    machine: &Machine,
) -> Vec<&'a ClaudeAccount> {
    accounts
        .iter()
        .filter(|account| account.is_on(&machine.id))
        .collect()
}

/// How an account's room reads, for its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RoomTone {
    /// Plenty left.
    Fine,
    /// Little left.
    Low,
    /// It can't take work now (out, off, not logged in, or none left).
    Out,
    /// Its use isn't known.
    Unknown,
}

/// An account's room on its hut.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Room {
    /// What the row says on the right: `72% left`, `Not checked yet`, `Out until 18:00`.
    pub(crate) value: String,
    /// The line under it: `5-hour 28% used · week 10% used`, `5-hour not reported · week 40%
    /// used`; empty when there is nothing more to say.
    pub(crate) detail: String,
    /// The room left, 0 to 1, for its bar; none when it isn't known.
    pub(crate) share: Option<f64>,
    /// Its colour.
    pub(crate) tone: RoomTone,
}

/// Less room than this (%) is low.
const LOW_ROOM: f64 = 20.;

/// `account`'s room as of `now` (out-until times in `zone`): 100 less the higher of its known
/// 5-hour and week use. An unknown 5-hour use is said as not reported, never read as 0% used,
/// and an account with no use known at all hasn't been checked yet (never 100% left).
#[must_use]
pub(crate) fn room(account: &ClaudeAccount, now: Timestamp, zone: &TimeZone) -> Room {
    let used = |label: &str, used: Option<f64>| {
        used.map_or_else(
            || format!("{label} not reported"),
            |used| format!("{label} {}% used", whole(used)),
        )
    };
    let detail = if account.used_5h.is_none() && account.used_7d.is_none() {
        String::new()
    } else {
        format!(
            "{} · {}",
            used("5-hour", account.used_5h),
            used("week", account.used_7d)
        )
    };
    let out = account
        .limited_until
        .as_ref()
        .and_then(crate::store::tasks::clock::instant)
        .filter(|until| *until > now);
    let said = |value: &str, tone| Room {
        value: value.to_owned(),
        detail: detail.clone(),
        share: None,
        tone,
    };
    if account.logged_in == Some(false) {
        return said("Not logged in", RoomTone::Out);
    }
    if account.enabled == Some(false) {
        return said("Off", RoomTone::Out);
    }
    if let Some(until) = out {
        let until = until
            .to_zoned_datetime(zone.clone())
            .strftime("%H:%M")
            .to_string();
        return said(&format!("Out until {until}"), RoomTone::Out);
    }
    let Some(most) = [account.used_5h, account.used_7d]
        .into_iter()
        .flatten()
        .reduce(f64::max)
    else {
        return said("Not checked yet", RoomTone::Unknown);
    };
    let left = (100. - most).clamp(0., 100.);
    Room {
        value: format!("{}% left", whole(left)),
        detail,
        share: Some(left / 100.),
        tone: if left <= 0. {
            RoomTone::Out
        } else if left < LOW_ROOM {
            RoomTone::Low
        } else {
            RoomTone::Fine
        },
    }
}

/// The page's line over the cards: `2 huts · 1 asleep`, `1 hut`.
#[must_use]
pub(crate) fn count_line(machines: &[Machine]) -> String {
    let asleep = machines
        .iter()
        .filter(|machine| Presence::of(&machine.state) == Presence::Asleep)
        .count();
    let huts = if machines.len() == 1 {
        "1 hut".to_owned()
    } else {
        format!("{} huts", machines.len())
    };
    if asleep == 0 {
        huts
    } else {
        format!("{huts} · {asleep} asleep")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn machine(extra: serde_json::Value) -> Machine {
        let mut base = json!({"id": "studio", "name": "Studio", "state": "ok", "status": "ok"});
        if let (Some(base), serde_json::Value::Object(extra)) = (base.as_object_mut(), extra) {
            base.extend(extra);
        }
        serde_json::from_value(base).unwrap()
    }

    fn account(extra: serde_json::Value) -> ClaudeAccount {
        let mut base = json!({"id": "a", "name": "Work", "machine": "studio", "loggedIn": true});
        if let (Some(base), serde_json::Value::Object(extra)) = (base.as_object_mut(), extra) {
            base.extend(extra);
        }
        serde_json::from_value(base).unwrap()
    }

    const NOW: &str = "2026-11-28T12:00:00Z";

    fn now() -> Timestamp {
        NOW.parse().unwrap()
    }

    #[test]
    fn a_hut_is_named_for_what_it_runs() {
        let kind = |extra| HutKind::of(&machine(extra));
        assert_eq!(kind(json!({"platform": "darwin"})), HutKind::Mac);
        assert_eq!(kind(json!({"platform": "linux"})), HutKind::Linux);
        assert_eq!(kind(json!({"platform": "win32"})), HutKind::Windows);
        assert_eq!(
            kind(json!({"platform": "linux", "host": "build-server.lan"})),
            HutKind::Server
        );
        assert_eq!(kind(json!({"platform": "cloud"})), HutKind::Cloud);
        assert_eq!(kind(json!({})), HutKind::Computer, "an older runner");
        assert_eq!(HutKind::Linux.word(), "Linux PC");
        assert_eq!(HutKind::Mac.icon(true), IconName::Laptop);
        assert_eq!(HutKind::Mac.icon(false), IconName::Monitor);
        assert_eq!(HutKind::Server.icon(false), IconName::Server);
    }

    #[test]
    fn an_offline_hut_is_asleep_and_says_when_it_was_last_seen() {
        let online = machine(json!({"state": "warn"}));
        assert_eq!(presence_line(&online, now()), "Online");
        let asleep = machine(json!({"state": "offline", "seen": "2026-11-28T10:00:00Z"}));
        assert_eq!(presence_line(&asleep, now()), "Asleep · last seen 2 h ago");
        let quiet = machine(json!({"state": "stale", "age": 90.0}));
        assert_eq!(presence_line(&quiet, now()), "Quiet · last seen 1 min ago");
        let never = machine(json!({"state": "unknown"}));
        assert_eq!(presence_line(&never, now()), "Not reported yet");
        assert_eq!(ago(30.), "just now");
        assert_eq!(ago(3. * 86_400.), "3 d ago");
    }

    #[test]
    fn what_runs_there_is_counted() {
        assert_eq!(running_line(&machine(json!({}))), "Nothing running");
        let busy = machine(json!({
            "runner": {"queued": 2},
            "sessions": [
                {"id": "r-1", "running": true}, {"id": "r-2", "running": false},
                {"id": "r-3", "running": false}
            ]
        }));
        assert_eq!(running_line(&busy), "3 sessions · 1 running · 2 queued");
    }

    #[test]
    fn its_resources_read_in_one_line() {
        let gb = 1024.0 * 1024.0 * 1024.0;
        let hut = machine(json!({
            "cpu": {"usage": 0.12},
            "memory": {"total": 64.0 * gb, "available": 46.0 * gb},
            "gpus": [{"name": "Example GPU"}]
        }));
        assert_eq!(
            resources_line(&hut).as_deref(),
            Some("CPU 12% · Memory 18.0 of 64.0 GB · 1 GPU")
        );
        assert_eq!(resources_line(&machine(json!({}))), None);
    }

    #[test]
    fn room_is_what_the_higher_known_use_leaves() {
        let utc = TimeZone::UTC;
        let both = room(
            &account(json!({"used5h": 28.0, "used7d": 10.0})),
            now(),
            &utc,
        );
        assert_eq!(both.value, "72% left");
        assert_eq!(both.detail, "5-hour 28% used · week 10% used");
        assert_eq!(both.tone, RoomTone::Fine);
        assert_eq!(both.share, Some(0.72));
        let low = room(
            &account(json!({"used5h": 91.0, "used7d": 40.0})),
            now(),
            &utc,
        );
        assert_eq!((low.value.as_str(), low.tone), ("9% left", RoomTone::Low));
    }

    #[test]
    fn an_unknown_five_hour_use_is_not_reported_never_zero() {
        let hut = room(
            &account(json!({"used5h": null, "used7d": 40.0})),
            now(),
            &TimeZone::UTC,
        );
        assert_eq!(hut.value, "60% left", "from the week alone");
        assert_eq!(hut.detail, "5-hour not reported · week 40% used");
        assert!(!hut.detail.contains("5-hour 0%"), "{}", hut.detail);
    }

    #[test]
    fn an_account_with_no_use_known_is_not_checked_yet_never_full() {
        let fresh = room(&account(json!({})), now(), &TimeZone::UTC);
        assert_eq!(fresh.value, "Not checked yet");
        assert_eq!(fresh.share, None);
        assert_eq!(fresh.tone, RoomTone::Unknown);
        assert!(!fresh.value.contains("100"));
    }

    #[test]
    fn an_account_that_cant_work_says_why() {
        let utc = TimeZone::UTC;
        let say = |extra| room(&account(extra), now(), &utc).value;
        assert_eq!(say(json!({"loggedIn": false})), "Not logged in");
        assert_eq!(say(json!({"enabled": false})), "Off");
        assert_eq!(
            say(json!({"used5h": 100.0, "limitedUntil": "2026-11-28T18:00:00Z"})),
            "Out until 18:00"
        );
        assert_eq!(
            say(json!({"used5h": 20.0, "limitedUntil": "2026-11-28T08:00:00Z"})),
            "80% left",
            "a limit that has passed"
        );
    }

    #[test]
    fn a_huts_accounts_are_the_ones_kept_on_it() {
        let accounts: Vec<ClaudeAccount> = serde_json::from_value(json!([
            {"id": "a", "name": "Work", "machine": "studio"},
            {"id": "b", "name": "Home", "machine": "runner"},
            {"id": "c", "name": "Unplaced"}
        ]))
        .unwrap();
        let names = |m: &Machine| -> Vec<String> {
            accounts_on(&accounts, m)
                .iter()
                .map(|a| a.name.clone())
                .collect()
        };
        assert_eq!(names(&machine(json!({}))), ["Work"]);
        assert_eq!(
            names(&machine(json!({"id": "runner"}))),
            ["Home", "Unplaced"],
            "an account placed nowhere is the Linux runner's"
        );
    }

    #[test]
    fn the_page_counts_its_huts_and_the_asleep_ones() {
        assert_eq!(count_line(&[machine(json!({}))]), "1 hut");
        assert_eq!(
            count_line(&[machine(json!({})), machine(json!({"state": "offline"}))]),
            "2 huts · 1 asleep"
        );
    }
}
