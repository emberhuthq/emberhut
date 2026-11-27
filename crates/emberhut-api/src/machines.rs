//! `GET /api/machines`: every machine that runs sessions, what it has left, its warnings, its
//! sessions and (unless `?history=0`) its last day in 5-minute points.
//!
//! The figures come from each runner's own sample of its machine; a runner that doesn't measure
//! something leaves it out, so nearly every figure is optional. Bytes are bytes; shares
//! (`usage`, `util`) are 0 to 1.

use serde::{Deserialize, Serialize};

use crate::ids::{MachineId, SessionId};
use crate::open_enum::open_enum;
use crate::time::Timestamp;

open_enum! {
    /// How a machine is, as its row's dot shows it.
    pub enum MachineState {
        /// Fine.
        Ok = "ok",
        /// Look at it (see its warnings).
        Warn = "warn",
        /// Too full: no new sessions start there.
        Low = "low",
        /// No sample for a minute.
        Stale = "stale",
        /// No sample for two minutes.
        Offline = "offline",
        /// Never reported (an older runner).
        Unknown = "unknown",
    }
}

/// The page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MachinesPage {
    /// When the server answered.
    pub at: Timestamp,
    /// The limits the states and warnings are judged by.
    #[serde(default)]
    pub thresholds: Option<Thresholds>,
    /// The machines.
    pub machines: Vec<Machine>,
}

/// The limits behind the machines' states and warnings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Thresholds {
    /// Less available memory than this (GB) is low.
    pub min_available_gb: f64,
    /// Less free disk than this (GB) is low.
    pub min_free_disk_gb: f64,
    /// More load per core than this is a warning.
    pub max_load_per_core: f64,
    /// Less free disk than this (GB) tells the assistant.
    pub alert_disk_gb: f64,
    /// A disk fuller than this (%) is a warning.
    pub warn_disk_pct: f64,
    /// Less room under the memory cap than this (GB) is low.
    pub min_cap_room_gb: f64,
    /// No sample for this long (s) is stale.
    pub stale_seconds: f64,
    /// No sample for this long (s) is offline.
    pub offline_seconds: f64,
}

/// One machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Machine {
    /// Its runner's id.
    pub id: MachineId,
    /// Its name, as shown.
    pub name: String,
    /// Its host name, as its runner reports it.
    pub host: Option<String>,
    /// Its platform (`linux`, `darwin`).
    pub platform: Option<String>,
    /// How it is.
    pub state: MachineState,
    /// Its state in words (`ok`, `no sample for 3 min`).
    pub status: String,
    /// When its last sample came.
    #[serde(default)]
    pub seen: Option<Timestamp>,
    /// How old that sample is, in seconds.
    #[serde(default)]
    pub age: Option<f64>,
    /// Whether a new session may start there.
    #[serde(default)]
    pub placeable: Option<bool>,
    /// Why not.
    #[serde(default)]
    pub skip_why: Option<String>,
    /// What to look at.
    #[serde(default)]
    pub warnings: Vec<MachineWarning>,
    /// Its processors.
    #[serde(default)]
    pub cpu: Option<Cpu>,
    /// Its memory.
    #[serde(default)]
    pub memory: Option<Memory>,
    /// Its GPUs.
    #[serde(default)]
    pub gpus: Vec<Gpu>,
    /// Its disks.
    #[serde(default)]
    pub disks: Vec<Disk>,
    /// Its battery, on a laptop.
    #[serde(default)]
    pub battery: Option<Battery>,
    /// Its runner's own figures.
    #[serde(default)]
    pub runner: Option<RunnerFacts>,
    /// The sessions on it now.
    #[serde(default)]
    pub sessions: Vec<MachineSession>,
    /// Its last day, in 5-minute points (each the worst of its samples), oldest first; `[]` when
    /// it has none. Left out with `?history=0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<Vec<Point>>,
}

open_enum! {
    /// How serious a machine's warning is.
    pub enum WarningLevel {
        /// Look at it.
        Warn = "warn",
        /// Too full for new sessions.
        Low = "low",
        /// Act now.
        Critical = "critical",
    }
}

/// A warning about a machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MachineWarning {
    /// How serious it is.
    pub level: WarningLevel,
    /// What it is, in words.
    pub text: String,
}

/// Processor figures.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Cpu {
    /// Its cores.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cores: Option<f64>,
    /// The 1-minute load average.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load1: Option<f64>,
    /// How busy, 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<f64>,
}

/// Memory figures, in bytes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Memory {
    /// All of it.
    pub total: Option<f64>,
    /// What new work can have.
    pub available: Option<f64>,
    /// In use.
    pub used: Option<f64>,
    /// Swap, all of it.
    pub swap_total: Option<f64>,
    /// Swap in use.
    pub swap_used: Option<f64>,
    /// The system's memory pressure, where it has one (`normal`, `warn`, `critical`).
    pub pressure: Option<String>,
}

/// One GPU.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Gpu {
    /// Its index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    /// Its name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// How busy, 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub util: Option<f64>,
    /// Its memory in use, in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem_used: Option<f64>,
    /// Its memory, in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem_total: Option<f64>,
    /// Its temperature, °C.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temp: Option<f64>,
    /// Its power draw, W.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power: Option<f64>,
    /// Its cores (Apple GPUs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cores: Option<u32>,
    /// Whether it shares the system's memory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared: Option<bool>,
    /// The processes holding its memory (NVIDIA GPUs; none for a shared one).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub procs: Vec<GpuProcess>,
}

/// A process holding a GPU's memory.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GpuProcess {
    /// Its process id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Its program's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The memory it holds, in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem: Option<f64>,
    /// The service it runs under (a systemd unit), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// The user it runs as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

impl GpuProcess {
    /// What holds the memory, as one line: `llama-server · example.service`, `pid 41` when it
    /// has no name.
    #[must_use]
    pub fn holder(&self) -> String {
        let name = self.name.clone().unwrap_or_else(|| {
            self.pid
                .map_or_else(|| "a process".to_owned(), |pid| format!("pid {pid}"))
        });
        match &self.unit {
            Some(unit) => format!("{name} · {unit}"),
            None => name,
        }
    }
}

/// One disk, in bytes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Disk {
    /// Where it is mounted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Its size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
    /// What is free.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free: Option<f64>,
}

/// A battery.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Battery {
    /// Its charge, 0 to 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<f64>,
    /// On the charger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugged: Option<bool>,
    /// `charging`, `discharging`, `charged`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Minutes left on battery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minutes_left: Option<f64>,
}

/// A runner's own figures.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RunnerFacts {
    /// Its sessions running.
    pub sessions: Option<u32>,
    /// Its turns queued.
    pub queued: Option<u32>,
    /// Its sessions open.
    pub open: Option<u32>,
    /// Its Claude processes.
    pub claude: Option<u32>,
    /// Their memory, in bytes.
    pub claude_rss: Option<f64>,
    /// The runner's own memory, in bytes.
    #[serde(default)]
    pub rss: Option<f64>,
    /// Its memory cap, where it runs under one.
    pub cap: Option<Cap>,
}

/// A memory cap (a cgroup's), in bytes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Cap {
    /// The cap's name.
    #[serde(default)]
    pub name: Option<String>,
    /// In use.
    pub used: Option<f64>,
    /// Where it starts to throttle.
    pub high: Option<f64>,
    /// The hard limit.
    pub max: Option<f64>,
    /// What sets it.
    pub by: Option<String>,
}

/// A session on a machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MachineSession {
    /// The session.
    pub id: SessionId,
    /// Its title.
    pub title: Option<String>,
    /// Its status.
    pub status: Option<String>,
    /// Whether a turn runs now.
    pub running: bool,
    /// What it is doing.
    pub doing: Option<String>,
    /// Its account.
    pub account: Option<String>,
    /// That account's name.
    pub account_name: Option<String>,
    /// That account's use of its 5-hour window, 0 to 1.
    pub used5h: Option<f64>,
    /// That account's use of its week, 0 to 1.
    pub used7d: Option<f64>,
    /// That account is out until then.
    pub limited_until: Option<Timestamp>,
}

/// A 5-minute point of a machine's day: the worst of its samples.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Point {
    /// When.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Timestamp>,
    /// Peak processor use, 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<f64>,
    /// Peak load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load: Option<f64>,
    /// Least available memory, in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem_avail: Option<f64>,
    /// Peak GPU use, 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu: Option<f64>,
    /// Peak GPU memory in use, in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram: Option<f64>,
    /// Least free disk, in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_free: Option<f64>,
}
