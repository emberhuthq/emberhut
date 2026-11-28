//! What a hut's card says: its kind (from the platform and names its runner reports), whether
//! it is online or asleep and since when, its sessions and resources, and each of its accounts
//! with the room left. Plain functions of their inputs ("now" is passed in), so they are
//! tested without a window.

use emberhut_api::machines::{Machine, MachineState};
use emberhut_api::runner::accounts::ClaudeAccount;
use gpui_kit::assets::IconName;
use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::format::{percent, whole};
use crate::store::tasks::clock::instant;

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

    /// Its icon: a laptop for one with a battery, else a desktop's screen, a rack or a cloud.
    #[must_use]
    pub(crate) const fn icon(self, laptop: bool) -> IconName {
        match self {
            Self::Mac | Self::Linux | Self::Windows if laptop => IconName::Laptop,
            Self::Mac | Self::Linux | Self::Windows => IconName::Monitor,
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

    /// Its word.
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

/// What an asleep hut's head says under its kind, quietly: being asleep isn't a fault.
pub(crate) const ASLEEP_NOTE: &str = "Won't take new sessions until it wakes";

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

/// Seconds from `at` to `now`.
#[expect(
    clippy::cast_precision_loss,
    reason = "seconds between two moments: far inside f64's whole numbers"
)]
fn since(at: Timestamp, now: Timestamp) -> f64 {
    (now.as_second() - at.as_second()) as f64
}

/// How long since `machine`'s last report, as of `now`: from its last sample's time, else the
/// age the server gave.
fn silent_for(machine: &Machine, now: Timestamp) -> Option<f64> {
    machine
        .seen
        .as_ref()
        .and_then(instant)
        .map(|seen| since(seen, now))
        .or(machine.age)
}

/// The pill's words: whether `machine` is there, and for how long it hasn't reported when it
/// is quiet or asleep, as of `now`: `Online`, `Asleep · 2 h ago`.
#[must_use]
pub(crate) fn pill_line(machine: &Machine, now: Timestamp) -> String {
    let presence = Presence::of(&machine.state);
    if matches!(presence, Presence::Online | Presence::NotYet) {
        return presence.word().to_owned();
    }
    match silent_for(machine, now) {
        Some(seconds) => format!("{} · {}", presence.word(), ago(seconds)),
        None => presence.word().to_owned(),
    }
}

/// Its sessions: `2 open · 1 working · 2 queued`, `None`.
#[must_use]
pub(crate) fn sessions_line(machine: &Machine) -> String {
    let open = machine.sessions.len();
    let working = machine.sessions.iter().filter(|s| s.running).count();
    let queued = machine
        .runner
        .as_ref()
        .and_then(|runner| runner.queued)
        .unwrap_or(0);
    if open == 0 && queued == 0 {
        return "None".to_owned();
    }
    let mut parts = vec![format!("{open} open")];
    if working > 0 {
        parts.push(format!("{working} working"));
    }
    if queued > 0 {
        parts.push(format!("{queued} queued"));
    }
    parts.join(" · ")
}

/// Bytes in a gigabyte, as the server counts them (1024³).
const GIGABYTE: f64 = 1024.0 * 1024.0 * 1024.0;

/// A GPU busier than this share isn't idle.
const IDLE: f64 = 0.03;

/// Its processor, memory and graphics at a glance, in whole numbers: `CPU 12% · RAM 18 / 64
/// GB · GPU idle`; none when its runner measures none of them.
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
        if let (Some(used), Some(total)) = (used, memory.total) {
            parts.push(format!(
                "RAM {} / {} GB",
                whole(used / GIGABYTE),
                whole(total / GIGABYTE)
            ));
        }
    }
    let busiest = machine
        .gpus
        .iter()
        .filter_map(|gpu| gpu.util)
        .reduce(f64::max);
    let gpu = if machine.gpus.len() > 1 {
        "GPUs"
    } else {
        "GPU"
    };
    match busiest {
        Some(util) if util < IDLE => parts.push(format!("{gpu} idle")),
        Some(util) => parts.push(format!("{gpu} {}", percent(util))),
        None => {}
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// How old an asleep hut's figures are, as of `now`: `as of 2 h ago`; none for a hut that is
/// there (its figures are live) or when the age isn't known.
#[must_use]
pub(crate) fn as_of(machine: &Machine, now: Timestamp) -> Option<String> {
    (Presence::of(&machine.state) == Presence::Asleep)
        .then(|| silent_for(machine, now))
        .flatten()
        .map(|seconds| format!("as of {}", ago(seconds)))
}

/// The page's subtitle: when the huts were last fetched, as of `now`: `Updated just now`,
/// `Updated 2 min ago`.
#[must_use]
pub(crate) fn updated_line(fetched: Timestamp, now: Timestamp) -> String {
    let seconds = since(fetched, now);
    if seconds < 60. {
        "Updated just now".to_owned()
    } else {
        format!("Updated {}", ago(seconds))
    }
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
    /// More than 40% left.
    Fine,
    /// 16 to 40% left.
    Low,
    /// 15% left or less, or it can't take work now (out, off, not logged in).
    Out,
    /// Its use isn't known.
    Unknown,
}

/// An account's room on its hut.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Room {
    /// What the row says on the right: `72% left`, `60% left · week only`, `9% left · back in
    /// 1 h 20 m`, `Not checked yet`, `Out until 18:00`.
    pub(crate) value: String,
    /// The line under it, in the same direction: `5-hour 72% left · week 90% left`, `5-hour
    /// not reported · week 60% left`, with `checked 2 h ago` on an asleep hut; empty when
    /// there is nothing more to say.
    pub(crate) detail: String,
    /// The room left, 0 to 1, for its bar; none when it isn't known.
    pub(crate) share: Option<f64>,
    /// Its colour, the bar's and the value's.
    pub(crate) tone: RoomTone,
}

/// This much room left (%) or less is red.
const RED_ROOM: f64 = 15.;
/// This much room left (%) or less is amber.
const AMBER_ROOM: f64 = 40.;

/// How long until `at`, as of `now`: `back in 20 m`, `back in 1 h 20 m`, `back in 3 d`.
fn back_in(at: Timestamp, now: Timestamp) -> String {
    let minutes = whole(-since(at, now) / 60.).max(1);
    match minutes {
        0..60 => format!("back in {minutes} m"),
        60..1440 if minutes % 60 == 0 => format!("back in {} h", minutes / 60),
        60..1440 => format!("back in {} h {} m", minutes / 60, minutes % 60),
        _ => format!("back in {} d", minutes / 1440),
    }
}

/// `account`'s room as of `now` (out-until times in `zone`): 100 less the higher of its known
/// 5-hour and week use. An unknown 5-hour use is said as not reported and its room as from
/// the week only, never read as 0% used; an account with no use known at all hasn't been
/// checked yet (never 100% left). Low room says when the fuller window starts again, when
/// known. `asleep`: its hut is asleep, so the line says how old the figures are.
#[must_use]
pub(crate) fn room(account: &ClaudeAccount, now: Timestamp, zone: &TimeZone, asleep: bool) -> Room {
    let left = |label: &str, used: Option<f64>| {
        used.map_or_else(
            || format!("{label} not reported"),
            |used| format!("{label} {}% left", whole((100. - used).clamp(0., 100.))),
        )
    };
    let mut lines = Vec::new();
    if account.used_5h.is_some() || account.used_7d.is_some() {
        lines.push(left("5-hour", account.used_5h));
        lines.push(left("week", account.used_7d));
    }
    if asleep && let Some(checked) = account.checked.as_ref().and_then(instant) {
        lines.push(format!("checked {}", ago(since(checked, now))));
    }
    let detail = lines.join(" · ");
    let said = |value: String, tone| Room {
        value,
        detail: detail.clone(),
        share: None,
        tone,
    };
    if account.logged_in == Some(false) {
        return said("Not logged in".to_owned(), RoomTone::Out);
    }
    if account.enabled == Some(false) {
        return said("Off".to_owned(), RoomTone::Out);
    }
    let out = account
        .limited_until
        .as_ref()
        .and_then(instant)
        .filter(|until| *until > now);
    if let Some(until) = out {
        let until = until.to_zoned_datetime(zone.clone()).strftime("%H:%M");
        return said(format!("Out until {until}"), RoomTone::Out);
    }
    // The fuller window, which sets the room, and when it starts again.
    let (most, resets) = match (account.used_5h, account.used_7d) {
        (Some(hour), Some(week)) if week > hour => (week, &account.resets_7d),
        (Some(hour), _) => (hour, &account.resets_5h),
        (None, Some(week)) => (week, &account.resets_7d),
        (None, None) => return said("Not checked yet".to_owned(), RoomTone::Unknown),
    };
    let room = (100. - most).clamp(0., 100.);
    let tone = if room <= RED_ROOM {
        RoomTone::Out
    } else if room <= AMBER_ROOM {
        RoomTone::Low
    } else {
        RoomTone::Fine
    };
    let mut value = format!("{}% left", whole(room));
    if account.used_5h.is_none() {
        value.push_str(" · week only");
    }
    let back = resets
        .as_ref()
        .and_then(instant)
        .filter(|at| *at > now && tone != RoomTone::Fine);
    if let Some(back) = back {
        value = format!("{value} · {}", back_in(back, now));
    }
    Room {
        value,
        detail,
        share: Some(room / 100.),
        tone,
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

    fn now() -> Timestamp {
        "2026-11-28T12:00:00Z".parse().unwrap()
    }

    fn room_of(extra: serde_json::Value) -> Room {
        room(&account(extra), now(), &TimeZone::UTC, false)
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
        assert_eq!(HutKind::Linux.icon(false), IconName::Monitor);
        assert_eq!(HutKind::Server.icon(false), IconName::Server);
    }

    #[test]
    fn the_pill_says_whether_it_is_there_and_since_when() {
        let online = machine(json!({"state": "warn"}));
        assert_eq!(pill_line(&online, now()), "Online");
        let asleep = machine(json!({"state": "offline", "seen": "2026-11-28T10:00:00Z"}));
        assert_eq!(pill_line(&asleep, now()), "Asleep · 2 h ago");
        let quiet = machine(json!({"state": "stale", "age": 90.0}));
        assert_eq!(pill_line(&quiet, now()), "Quiet · 1 min ago");
        let never = machine(json!({"state": "unknown"}));
        assert_eq!(pill_line(&never, now()), "Not reported yet");
        assert_eq!(ago(30.), "just now");
        assert_eq!(ago(3. * 86_400.), "3 d ago");
    }

    #[test]
    fn its_sessions_are_counted() {
        assert_eq!(sessions_line(&machine(json!({}))), "None");
        let busy = machine(json!({
            "runner": {"queued": 2},
            "sessions": [
                {"id": "r-1", "running": true}, {"id": "r-2", "running": false},
                {"id": "r-3", "running": false}
            ]
        }));
        assert_eq!(sessions_line(&busy), "3 open · 1 working · 2 queued");
    }

    #[test]
    fn its_resources_read_in_whole_numbers_with_the_gpu_as_a_state() {
        let hut = |gpus| {
            machine(json!({
                "cpu": {"usage": 0.12},
                "memory": {"total": 64.0 * GIGABYTE, "available": 46.2 * GIGABYTE},
                "gpus": gpus
            }))
        };
        assert_eq!(
            resources_line(&hut(json!([{"util": 0.01}]))).as_deref(),
            Some("CPU 12% · RAM 18 / 64 GB · GPU idle")
        );
        assert_eq!(
            resources_line(&hut(json!([{"util": 0.1}, {"util": 0.45}]))).as_deref(),
            Some("CPU 12% · RAM 18 / 64 GB · GPUs 45%")
        );
        assert_eq!(
            resources_line(&hut(json!([{"name": "unmeasured"}]))).as_deref(),
            Some("CPU 12% · RAM 18 / 64 GB")
        );
        assert_eq!(resources_line(&machine(json!({}))), None);
    }

    #[test]
    fn an_asleep_huts_figures_say_how_old_they_are() {
        let asleep = machine(json!({"state": "offline", "seen": "2026-11-28T10:00:00Z"}));
        assert_eq!(as_of(&asleep, now()).as_deref(), Some("as of 2 h ago"));
        assert_eq!(as_of(&machine(json!({})), now()), None, "live figures");
        let checked = room(
            &account(json!({"used5h": 28.0, "used7d": 10.0, "checked": "2026-11-28T10:00:00Z"})),
            now(),
            &TimeZone::UTC,
            true,
        );
        assert_eq!(
            checked.detail,
            "5-hour 72% left · week 90% left · checked 2 h ago"
        );
    }

    #[test]
    fn the_page_says_when_it_was_updated() {
        let at = |s: &str| s.parse::<Timestamp>().unwrap();
        assert_eq!(
            updated_line(at("2026-11-28T11:59:50Z"), now()),
            "Updated just now"
        );
        assert_eq!(
            updated_line(at("2026-11-28T11:57:00Z"), now()),
            "Updated 3 min ago"
        );
    }

    #[test]
    fn room_is_what_the_higher_known_use_leaves_said_as_left() {
        let both = room_of(json!({"used5h": 28.0, "used7d": 10.0}));
        assert_eq!(both.value, "72% left");
        assert_eq!(both.detail, "5-hour 72% left · week 90% left");
        assert_eq!(both.tone, RoomTone::Fine);
        assert_eq!(both.share, Some(0.72));
    }

    #[test]
    fn room_is_red_at_15_amber_to_40_and_says_when_it_comes_back() {
        let tone = |used: f64| room_of(json!({"used5h": used})).tone;
        assert_eq!(tone(85.), RoomTone::Out);
        assert_eq!(tone(84.), RoomTone::Low);
        assert_eq!(tone(60.), RoomTone::Low);
        assert_eq!(tone(59.), RoomTone::Fine);
        let low = room_of(json!({"used5h": 91.0, "used7d": 40.0,
            "resets5h": "2026-11-28T13:20:00Z", "resets7d": "2026-12-01T00:00:00Z"}));
        assert_eq!(low.value, "9% left · back in 1 h 20 m");
        let fine = room_of(json!({"used5h": 10.0, "resets5h": "2026-11-28T13:20:00Z"}));
        assert_eq!(fine.value, "90% left", "plenty left: no reset said");
    }

    #[test]
    fn an_unknown_five_hour_use_is_not_reported_and_the_room_is_from_the_week_only() {
        let hut = room_of(json!({"used5h": null, "used7d": 40.0}));
        assert_eq!(hut.value, "60% left · week only");
        assert_eq!(hut.detail, "5-hour not reported · week 60% left");
    }

    #[test]
    fn an_account_with_no_use_known_is_not_checked_yet_never_full() {
        let fresh = room_of(json!({}));
        assert_eq!(fresh.value, "Not checked yet");
        assert_eq!(fresh.share, None);
        assert_eq!(fresh.tone, RoomTone::Unknown);
    }

    #[test]
    fn an_account_that_cant_work_says_why() {
        let say = |extra| room_of(extra).value;
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
