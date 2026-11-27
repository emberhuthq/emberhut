//! Runner machines: what each runner's sample of its machine says is left, the warnings and the
//! state the Machines view shows, whether a new session may start there, and the 5-minute points
//! of its last day.
//!
//! A runner samples its machine every 20 seconds and posts it to the server. The sample's figures
//! are optional: a runner that doesn't measure something leaves it out. Bytes are bytes; shares
//! (`usage`, `util`) are 0 to 1.

use jiff::{SignedDuration, Timestamp};

/// One gibibyte, in bytes: the unit the limits are given in.
pub const GB: f64 = 1024.0 * 1024.0 * 1024.0;

/// One history point per this many milliseconds (5 minutes).
pub const BUCKET_MS: i64 = 5 * 60_000;

/// How long history is kept, in milliseconds (a day).
pub const KEEP_MS: i64 = 24 * 3_600_000;

/// The limits a machine's state and warnings are judged by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thresholds {
    /// Less available memory than this (GB) is low.
    pub min_available_gb: f64,
    /// Less free disk than this (GB) is low.
    pub min_free_disk_gb: f64,
    /// More 5-minute load per core than this is low.
    pub max_load_per_core: f64,
    /// Less free disk than this (GB) is critical, and tells the assistant.
    pub alert_disk_gb: f64,
    /// A disk at least this full (%) is a warning.
    pub warn_disk_pct: f64,
    /// Less room under the runner's memory cap than this (GB) is low.
    pub min_cap_room_gb: f64,
    /// No sample for this long (s) is stale.
    pub stale_seconds: f64,
    /// No sample for this long (s) is offline.
    pub offline_seconds: f64,
}

impl Default for Thresholds {
    /// The limits the server runs with.
    fn default() -> Self {
        Self {
            min_available_gb: 4.0,
            min_free_disk_gb: 15.0,
            max_load_per_core: 1.5,
            alert_disk_gb: 10.0,
            warn_disk_pct: 95.0,
            min_cap_room_gb: 2.0,
            stale_seconds: 60.0,
            offline_seconds: 120.0,
        }
    }
}

/// The figures of a sample the rules read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sample {
    /// The processors.
    pub cpu: Option<CpuSample>,
    /// The memory.
    pub memory: Option<MemorySample>,
    /// The GPUs.
    pub gpus: Vec<GpuSample>,
    /// The disks.
    pub disks: Vec<DiskSample>,
    /// The battery, on a laptop.
    pub battery: Option<BatterySample>,
    /// The runner's own figures.
    pub runner: Option<RunnerSample>,
}

/// Processor figures.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CpuSample {
    /// Its cores.
    pub cores: Option<f64>,
    /// The 1-minute load.
    pub load1: Option<f64>,
    /// The 5-minute load.
    pub load5: Option<f64>,
    /// How busy, 0 to 1.
    pub usage: Option<f64>,
}

/// Memory figures, in bytes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MemorySample {
    /// All of it.
    pub total: Option<f64>,
    /// What new work can have.
    pub available: Option<f64>,
    /// Swap, all of it.
    pub swap_total: Option<f64>,
    /// Swap in use.
    pub swap_used: Option<f64>,
    /// The system's memory pressure (`normal`, `warn`, `critical`).
    pub pressure: Option<String>,
}

/// One GPU.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GpuSample {
    /// Its name.
    pub name: Option<String>,
    /// How busy, 0 to 1.
    pub util: Option<f64>,
    /// Its memory in use, in bytes.
    pub mem_used: Option<f64>,
    /// Its memory, in bytes.
    pub mem_total: Option<f64>,
    /// Its cores (Apple GPUs).
    pub cores: Option<f64>,
    /// It shares the system's memory.
    pub shared: bool,
    /// The processes holding its memory: name, service unit, bytes.
    pub procs: Vec<(String, Option<String>, f64)>,
}

/// One disk, in bytes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiskSample {
    /// Where it is mounted.
    pub path: Option<String>,
    /// Its size.
    pub total: Option<f64>,
    /// What is free.
    pub free: Option<f64>,
}

/// A battery.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BatterySample {
    /// Its charge, 0 to 100.
    pub percent: Option<f64>,
    /// On the charger.
    pub plugged: bool,
    /// `charging`, `discharging`, `charged`.
    pub state: Option<String>,
    /// Minutes left on battery.
    pub minutes_left: Option<f64>,
}

/// The runner's own figures.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunnerSample {
    /// Its turns running.
    pub sessions: Option<f64>,
    /// Its agent processes.
    pub claude: Option<f64>,
    /// Its memory cap (a cgroup's), where it runs under one.
    pub cap: Option<CapSample>,
}

/// A memory cap, in bytes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CapSample {
    /// The cap's name.
    pub name: Option<String>,
    /// The hard limit.
    pub max: Option<f64>,
    /// Where it starts to throttle.
    pub high: Option<f64>,
    /// What counts against the cap.
    pub cap_used: Option<f64>,
    /// What the cgroup holds now.
    pub current: Option<f64>,
    /// What sets the hard limit.
    pub max_by: Option<String>,
    /// What sets the throttle.
    pub high_by: Option<String>,
}

impl CapSample {
    /// The lower of its limits, when it has one.
    #[must_use]
    pub fn limit(&self) -> Option<f64> {
        match (self.max, self.high) {
            (Some(max), high) => Some(high.map_or(max, |high| max.min(high))),
            (None, high) => high,
        }
    }

    /// What it holds.
    #[must_use]
    pub fn used(&self) -> Option<f64> {
        self.cap_used.or(self.current)
    }

    /// What sets it, in words.
    #[must_use]
    pub fn by(&self) -> Option<&str> {
        self.max_by.as_deref().or(self.high_by.as_deref())
    }
}

/// How serious a warning is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// Act now; no new sessions.
    Critical,
    /// Too full for new sessions.
    Low,
    /// Look at it.
    Warn,
}

impl Level {
    /// The level's word: `critical`, `low`, `warn`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::Low => "low",
            Self::Warn => "warn",
        }
    }

    /// Whether it keeps new sessions off the machine.
    #[must_use]
    pub const fn blocks(self) -> bool {
        matches!(self, Self::Critical | Self::Low)
    }
}

/// A warning about a machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    /// How serious it is.
    pub level: Level,
    /// What it is, in words.
    pub text: String,
}

/// How a machine is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MachineState {
    /// Fine.
    Ok,
    /// Look at it.
    Warn,
    /// Too full: no new sessions start there.
    Low,
    /// No sample for a minute.
    Stale,
    /// No sample for two minutes.
    Offline,
    /// Never reported.
    Unknown,
}

impl MachineState {
    /// The state's word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Low => "low",
            Self::Stale => "stale",
            Self::Offline => "offline",
            Self::Unknown => "unknown",
        }
    }
}

/// `bytes` in gigabytes: `12.3 GB`, or `512 GB` from 100 GB.
#[must_use]
pub fn gb(bytes: f64) -> String {
    let value = bytes / GB;
    if bytes >= 100.0 * GB {
        format!("{value:.0} GB")
    } else {
        format!("{value:.1} GB")
    }
}

/// A number as JavaScript prints it: no `.0` on a whole one.
#[must_use]
pub fn num(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{value:.0}")
    } else {
        format!("{value}")
    }
}

/// A span of `ms` milliseconds in words: `now`, `5m`, `2h 10m`, `3h`, `2d`.
#[must_use]
pub fn span(ms: f64) -> String {
    let minutes = (ms / 60_000.0).round().max(0.0);
    if minutes < 1.0 {
        "now".to_owned()
    } else if minutes < 60.0 {
        format!("{minutes:.0}m")
    } else if minutes < 1440.0 {
        let hours = (minutes / 60.0).floor();
        let rest = minutes % 60.0;
        if rest == 0.0 {
            format!("{hours:.0}h")
        } else {
            format!("{hours:.0}h {rest:.0}m")
        }
    } else {
        format!("{:.0}d", (minutes / 1440.0).floor())
    }
}

/// What is wrong with `sample` by `limits`, worst first. A [`Level::blocks`] one means no new
/// sessions there.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "one check per figure, in a fixed order"
)]
pub fn warnings(sample: &Sample, limits: &Thresholds) -> Vec<Warning> {
    let mut found = Vec::new();
    let mut add = |level, text: String| found.push(Warning { level, text });
    if let Some(memory) = &sample.memory {
        if let Some(available) = memory.available
            && available < limits.min_available_gb * GB
        {
            add(
                Level::Low,
                format!(
                    "only {} memory available (new sessions need {} GB)",
                    gb(available),
                    num(limits.min_available_gb)
                ),
            );
        }
        match memory.pressure.as_deref() {
            Some("critical") => add(Level::Critical, "memory pressure is critical".to_owned()),
            Some("warn") => add(Level::Warn, "memory pressure is raised".to_owned()),
            _ => {}
        }
    }
    for disk in &sample.disks {
        let (Some(free), Some(total)) = (disk.free, disk.total) else {
            continue;
        };
        if total <= 0.0 {
            continue;
        }
        let path = disk.path.as_deref().unwrap_or("/");
        let pct = ((1.0 - free / total) * 100.0).round_ties_even();
        if free < limits.alert_disk_gb * GB {
            add(
                Level::Critical,
                format!("{path} has only {} free ({}% full)", gb(free), num(pct)),
            );
        } else if free < limits.min_free_disk_gb * GB {
            add(
                Level::Low,
                format!(
                    "{path} has only {} free (new sessions need {} GB)",
                    gb(free),
                    num(limits.min_free_disk_gb)
                ),
            );
        } else if pct >= limits.warn_disk_pct {
            add(
                Level::Warn,
                format!("{path} is {}% full ({} free)", num(pct), gb(free)),
            );
        }
    }
    // The 5-minute load, so a build's minute-long spike doesn't flip a machine.
    if let Some(cpu) = &sample.cpu
        && let (Some(load), Some(cores)) = (cpu.load5.or(cpu.load1), cpu.cores)
        && cores > 0.0
        && load > limits.max_load_per_core * cores
    {
        add(
            Level::Low,
            format!(
                "5-minute load {load:.1} on {} cores (over {}× its cores)",
                num(cores),
                num(limits.max_load_per_core)
            ),
        );
    }
    if let Some(cap) = sample
        .runner
        .as_ref()
        .and_then(|runner| runner.cap.as_ref())
        && let (Some(limit), Some(used)) = (cap.limit(), cap.used())
        && limit - used < limits.min_cap_room_gb * GB
    {
        add(
            Level::Low,
            format!(
                "the runner's memory cap is nearly reached ({} of {}, {})",
                gb(used),
                gb(limit),
                cap.by().or(cap.name.as_deref()).unwrap_or("")
            ),
        );
    }
    if let Some(battery) = &sample.battery
        && !battery.plugged
        && let Some(percent) = battery.percent
        && percent < 20.0
    {
        let left = battery
            .minutes_left
            .map_or_default(|minutes| format!(", about {} left", span(minutes * 60_000.0)));
        add(
            Level::Warn,
            format!("on battery at {}%{left}", num(percent)),
        );
    }
    if let Some(memory) = &sample.memory
        && let (Some(total), Some(used)) = (memory.swap_total, memory.swap_used)
        && total > 0.0
        && used / total > 0.9
    {
        add(
            Level::Warn,
            format!(
                "swap is {}% used",
                num((used / total * 100.0).round_ties_even())
            ),
        );
    }
    found.sort_by_key(|warning| warning.level);
    found
}

/// A machine's last sample: what it said, when it came, and since when the server has been up
/// (so a machine isn't offline before it had the time to send again after a restart).
#[derive(Clone, Debug, PartialEq)]
pub struct Seen<'a> {
    /// The sample.
    pub sample: &'a Sample,
    /// When it came.
    pub at: Timestamp,
}

/// A machine's state at `now`, and its last sample's age in seconds (`None` without one).
#[must_use]
pub fn state_of(
    seen: Option<&Seen<'_>>,
    started: Timestamp,
    now: Timestamp,
    limits: &Thresholds,
) -> (MachineState, Option<f64>) {
    let Some(seen) = seen else {
        return (MachineState::Unknown, None);
    };
    let age = seconds(now.duration_since(seen.at));
    let silent = seconds(now.duration_since(seen.at.max(started)));
    if silent > limits.offline_seconds {
        return (MachineState::Offline, Some(age));
    }
    if age > limits.stale_seconds {
        return (MachineState::Stale, Some(age));
    }
    let found = warnings(seen.sample, limits);
    let state = if found.iter().any(|warning| warning.level.blocks()) {
        MachineState::Low
    } else if found.is_empty() {
        MachineState::Ok
    } else {
        MachineState::Warn
    };
    (state, Some(age))
}

fn seconds(duration: SignedDuration) -> f64 {
    duration.as_secs_f64()
}

/// Whether a new session may start on a machine in `state` with `sample`, and why not.
/// Offline or low: no. Unknown (a runner that sends no samples) and stale: yes.
///
/// # Errors
///
/// Why not, in words, when it may not.
pub fn can_place(
    state: MachineState,
    age: Option<f64>,
    sample: Option<&Sample>,
    limits: &Thresholds,
) -> Result<(), String> {
    match state {
        MachineState::Offline => Err(format!(
            "it's offline (no sample from its runner for {})",
            span(age.unwrap_or(0.0) * 1000.0)
        )),
        MachineState::Low => Err(sample.map_or_default(|sample| {
            warnings(sample, limits)
                .into_iter()
                .filter(|warning| warning.level.blocks())
                .map(|warning| warning.text)
                .collect::<Vec<_>>()
                .join("; ")
        })),
        _ => Ok(()),
    }
}

/// A state in words, as the app shows it.
#[must_use]
pub fn status(state: MachineState, age: Option<f64>) -> String {
    let age = age.unwrap_or(0.0);
    match state {
        MachineState::Unknown => {
            "No samples yet (its runner is on older code, or hasn't started)".to_owned()
        }
        MachineState::Offline => format!("Offline · last sample {} ago", span(age * 1000.0)),
        MachineState::Stale => format!("Late · last sample {} s ago", num(age.round())),
        _ => format!("Online · {} s ago", num(age.round())),
    }
}

/// One 5-minute point: what matters for "how much is left".
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    /// The start of its 5 minutes, in milliseconds since the epoch.
    pub bucket: i64,
    /// Processor use, 0 to 1.
    pub cpu: Option<f64>,
    /// The 1-minute load.
    pub load: Option<f64>,
    /// Available memory, in bytes.
    pub mem_avail: Option<f64>,
    /// The busiest GPU's use, 0 to 1.
    pub gpu: Option<f64>,
    /// GPU memory in use, all GPUs together, in bytes.
    pub vram: Option<f64>,
    /// The fullest disk's free space, in bytes.
    pub disk_free: Option<f64>,
}

/// The bucket a sample taken at `at_ms` falls in.
#[must_use]
pub const fn bucket_of(at_ms: i64) -> i64 {
    at_ms.div_euclid(BUCKET_MS) * BUCKET_MS
}

impl Point {
    /// `sample`'s point, taken at `at_ms`.
    #[must_use]
    pub fn of(sample: &Sample, at_ms: i64) -> Self {
        let gpus = &sample.gpus;
        let disks = &sample.disks;
        Self {
            bucket: bucket_of(at_ms),
            cpu: sample.cpu.and_then(|cpu| cpu.usage),
            load: sample.cpu.and_then(|cpu| cpu.load1),
            mem_avail: sample.memory.as_ref().and_then(|memory| memory.available),
            gpu: (!gpus.is_empty()).then(|| {
                gpus.iter()
                    .map(|gpu| gpu.util.unwrap_or(0.0))
                    .fold(f64::MIN, f64::max)
            }),
            vram: (!gpus.is_empty())
                .then(|| gpus.iter().map(|gpu| gpu.mem_used.unwrap_or(0.0)).sum()),
            disk_free: (!disks.is_empty()).then(|| {
                disks
                    .iter()
                    .map(|disk| disk.free.unwrap_or(f64::MAX))
                    .fold(f64::MAX, f64::min)
            }),
        }
    }

    /// Two samples in one point: the busier processor and GPU, the least memory and disk.
    #[must_use]
    pub fn worst(self, newer: Self) -> Self {
        fn pick(a: Option<f64>, b: Option<f64>, f: fn(f64, f64) -> f64) -> Option<f64> {
            match (a, b) {
                (Some(a), Some(b)) => Some(f(a, b)),
                (a, b) => a.or(b),
            }
        }
        Self {
            bucket: newer.bucket,
            cpu: pick(self.cpu, newer.cpu, f64::max),
            load: pick(self.load, newer.load, f64::max),
            mem_avail: pick(self.mem_avail, newer.mem_avail, f64::min),
            gpu: pick(self.gpu, newer.gpu, f64::max),
            vram: pick(self.vram, newer.vram, f64::max),
            disk_free: pick(self.disk_free, newer.disk_free, f64::min),
        }
    }
}

/// One line about a machine for the assistant: `Build box (runner): ok · load 0.6 on 32 cores
/// · 53.0 GB of 61.0 GB memory available · ...`.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "one part per figure, in a fixed order"
)]
pub fn summary(
    name: &str,
    machine: &str,
    state: MachineState,
    age: Option<f64>,
    sample: Option<&Sample>,
    limits: &Thresholds,
) -> String {
    let Some(sample) = sample else {
        return format!("{name} ({machine}): {}", status(state, age));
    };
    let mut parts = vec![format!("{name} ({machine}): {}", state.as_str())];
    if matches!(state, MachineState::Stale | MachineState::Offline) {
        parts.push(status(state, age));
    }
    if let Some(cpu) = &sample.cpu {
        let busy = cpu
            .usage
            .map_or_default(|usage| format!(" ({}% busy)", num((usage * 100.0).round_ties_even())));
        parts.push(format!(
            "load {} on {} cores{busy}",
            cpu.load1.map_or_default(|load| format!("{load:.1}")),
            cpu.cores.map_or_default(num)
        ));
    }
    if let Some(memory) = &sample.memory
        && let (Some(available), Some(total)) = (memory.available, memory.total)
    {
        let pressure = memory
            .pressure
            .as_deref()
            .filter(|pressure| *pressure != "normal")
            .map_or_default(|pressure| format!(" (pressure {pressure})"));
        parts.push(format!(
            "{} of {} memory available{pressure}",
            gb(available),
            gb(total)
        ));
    }
    for gpu in &sample.gpus {
        let holders = gpu
            .procs
            .iter()
            .map(|(name, unit, mem)| {
                let unit = unit.as_deref().map_or_default(|unit| format!(" [{unit}]"));
                format!("{name}{unit} {}", gb(*mem))
            })
            .collect::<Vec<_>>()
            .join(", ");
        let used = gb(gpu.mem_used.unwrap_or(0.0));
        let total = gb(gpu.mem_total.unwrap_or(0.0));
        let memory = if gpu.shared {
            format!("{used} of {total} unified memory in use")
        } else {
            format!("VRAM {used} of {total}")
        };
        let label = format!(
            "{}{}{}",
            gpu.name.as_deref().unwrap_or(""),
            if gpu.shared { " GPU" } else { "" },
            gpu.cores
                .map_or_default(|cores| format!(" ({} cores)", num(cores)))
        );
        let held = if holders.is_empty() {
            String::new()
        } else {
            format!(" (held by {holders})")
        };
        parts.push(format!(
            "{label}: {}% busy, {memory}{held}",
            num((gpu.util.unwrap_or(0.0) * 100.0).round_ties_even())
        ));
    }
    for disk in &sample.disks {
        parts.push(format!(
            "{} {} free of {}",
            disk.path.as_deref().unwrap_or(""),
            gb(disk.free.unwrap_or(0.0)),
            gb(disk.total.unwrap_or(0.0))
        ));
    }
    if let Some(battery) = &sample.battery {
        parts.push(format!(
            "battery {}% {}{}",
            battery.percent.map_or_default(num),
            battery.state.as_deref().unwrap_or(""),
            if battery.plugged { " (plugged in)" } else { "" }
        ));
    }
    if let Some(runner) = &sample.runner {
        parts.push(format!(
            "{} session turns running, {} agent processes",
            num(runner.sessions.unwrap_or(0.0)),
            num(runner.claude.unwrap_or(0.0))
        ));
        if let Some(cap) = &runner.cap
            && let Some(max) = cap.max
        {
            parts.push(format!(
                "runner cap {} of {} ({})",
                gb(cap.used().unwrap_or(0.0)),
                gb(max),
                cap.max_by.as_deref().unwrap_or("")
            ));
        }
    }
    let found = warnings(sample, limits);
    if !found.is_empty() {
        parts.push(format!(
            "warnings: {}",
            found
                .iter()
                .map(|warning| warning.text.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    if let Err(why) = can_place(state, age, Some(sample), limits) {
        parts.push(format!("NOT taking new sessions: {why}"));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_disk() -> Sample {
        Sample {
            disks: vec![DiskSample {
                path: Some("/".to_owned()),
                total: Some(500.0 * GB),
                free: Some(5.0 * GB),
            }],
            ..Sample::default()
        }
    }

    #[test]
    fn gigabytes_and_spans_are_written_short() {
        assert_eq!(gb(1.5 * GB), "1.5 GB");
        assert_eq!(gb(512.0 * GB), "512 GB");
        assert_eq!(num(4.0), "4");
        assert_eq!(num(1.5), "1.5");
        assert_eq!(span(10_000.0), "now");
        assert_eq!(span(5.0 * 60_000.0), "5m");
        assert_eq!(span(130.0 * 60_000.0), "2h 10m");
        assert_eq!(span(180.0 * 60_000.0), "3h");
        assert_eq!(span(3.0 * 1440.0 * 60_000.0), "3d");
    }

    #[test]
    fn warnings_come_worst_first() {
        let mut sample = full_disk();
        sample.memory = Some(MemorySample {
            available: Some(2.0 * GB),
            pressure: Some("warn".to_owned()),
            ..MemorySample::default()
        });
        sample.cpu = Some(CpuSample {
            cores: Some(4.0),
            load1: Some(1.0),
            load5: Some(8.0),
            usage: Some(0.9),
        });
        let found = warnings(&sample, &Thresholds::default());
        let texts: Vec<_> = found.iter().map(|w| (w.level, w.text.as_str())).collect();
        assert_eq!(
            texts,
            [
                (Level::Critical, "/ has only 5.0 GB free (99% full)"),
                (
                    Level::Low,
                    "only 2.0 GB memory available (new sessions need 4 GB)"
                ),
                (
                    Level::Low,
                    "5-minute load 8.0 on 4 cores (over 1.5× its cores)"
                ),
                (Level::Warn, "memory pressure is raised"),
            ]
        );
    }

    #[test]
    fn a_machine_is_offline_stale_low_or_ok_by_its_last_sample() {
        let limits = Thresholds::default();
        let start: Timestamp = "2026-11-25T10:00:00Z".parse().unwrap();
        let sample = Sample::default();
        let seen = Seen {
            sample: &sample,
            at: start,
        };
        let at = |s: i64| start + SignedDuration::from_secs(s);
        assert_eq!(
            state_of(None, start, at(5), &limits).0,
            MachineState::Unknown
        );
        assert_eq!(
            state_of(Some(&seen), start, at(5), &limits),
            (MachineState::Ok, Some(5.0))
        );
        assert_eq!(
            state_of(Some(&seen), start, at(90), &limits).0,
            MachineState::Stale
        );
        assert_eq!(
            state_of(Some(&seen), start, at(200), &limits).0,
            MachineState::Offline
        );
        // Just restarted: not offline before it had the time to send again.
        assert_eq!(
            state_of(Some(&seen), at(150), at(200), &limits).0,
            MachineState::Stale
        );
        let full = full_disk();
        let seen = Seen {
            sample: &full,
            at: start,
        };
        let (state, age) = state_of(Some(&seen), start, at(5), &limits);
        assert_eq!(state, MachineState::Low);
        assert_eq!(
            can_place(state, age, Some(&full), &limits),
            Err("/ has only 5.0 GB free (99% full)".to_owned())
        );
        assert_eq!(
            can_place(MachineState::Offline, Some(300.0), None, &limits),
            Err("it's offline (no sample from its runner for 5m)".to_owned())
        );
        assert_eq!(can_place(MachineState::Stale, None, None, &limits), Ok(()));
        assert_eq!(status(MachineState::Ok, Some(4.4)), "Online · 4 s ago");
    }

    #[test]
    fn a_point_keeps_the_worst_of_its_samples() {
        let a = Point {
            bucket: 0,
            cpu: Some(0.2),
            mem_avail: Some(8.0),
            disk_free: None,
            ..Point::default()
        };
        let b = Point {
            bucket: 0,
            cpu: Some(0.9),
            mem_avail: Some(16.0),
            disk_free: Some(3.0),
            ..Point::default()
        };
        let worst = a.worst(b);
        assert_eq!(worst.cpu, Some(0.9));
        assert_eq!(worst.mem_avail, Some(8.0));
        assert_eq!(worst.disk_free, Some(3.0));
        assert_eq!(bucket_of(BUCKET_MS + 5), BUCKET_MS);
        let point = Point::of(&full_disk(), 7);
        assert_eq!(point.disk_free, Some(5.0 * GB));
        assert_eq!(point.gpu, None);
    }
}
