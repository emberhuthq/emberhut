//! What this machine has left, sampled every 20 s and posted to the server.
//!
//! A [`Sample`] is what the server takes at `POST /api/runner/machine`: CPU load and busy share,
//! memory and its pressure, each GPU and the processes holding its memory, free disk, the
//! battery, and this runner's sessions, agent processes and memory cap. The server keeps the
//! latest and a day of history, marks a machine stale or offline when samples stop, and skips a
//! full machine when it places new sessions.
//!
//! Cheap on purpose: the fast part is file reads and `statfs` (a few milliseconds); what needs a
//! program that is slow to run (`nvidia-smi`, `ps`, `pmset`) runs at most once a minute, as a
//! child process the runtime waits on without blocking a thread. On macOS the memory figures and
//! the GPU come from `vm_stat`, `sysctl` and `ioreg`, which are quick, in every sample.
//!
//! Units: bytes are bytes, `usage` and `util` are 0..1, battery percent is 0..100; GPU indexes,
//! core counts and process ids are whole numbers.

pub mod parse;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as os;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as os;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod other;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use other as os;

use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use parse::{Agents, CpuTimes};

/// How long the slow parts of a sample are reused before they are read again.
const SLOW_EVERY: Duration = Duration::from_secs(55);
/// How long one program may take before its answer is given up on.
#[cfg(unix)]
const PROGRAM_TIMEOUT: Duration = Duration::from_secs(8);

/// What the sampler knows about the machine before it looks.
#[derive(Debug, Clone)]
pub struct MachineInfo {
    /// The runner's name on the server, sent as the sample's `machine`.
    pub name: String,
}

/// The runner's sessions, as a sample carries them.
#[derive(Debug, Clone, Default)]
pub struct SessionsSummary {
    /// Sessions running a turn now.
    pub running: u32,
    /// Sessions open, running or not.
    pub open: u32,
    /// Turns waiting for a session to be free.
    pub queued: u32,
    /// Each session: `{ id, title, status, running, account }`.
    pub list: Vec<Value>,
}

/// One sample of the machine, in the shape the server reads.
#[derive(Debug, Clone, Serialize)]
pub struct Sample {
    /// The runner's name on the server.
    pub machine: String,
    /// When it was taken (RFC 3339, milliseconds, UTC).
    pub at: String,
    /// The machine's host name.
    pub host: Option<String>,
    /// The system: `linux`, `darwin`, `win32`, or Rust's name for another one.
    pub platform: &'static str,
    /// The processors.
    pub cpu: Cpu,
    /// The memory; `None` where the runner can't read it.
    pub memory: Option<Memory>,
    /// The GPUs.
    pub gpus: Vec<Gpu>,
    /// The volumes that matter, each disk once.
    pub disks: Vec<Disk>,
    /// The battery; `None` on a machine without one.
    pub battery: Option<Battery>,
    /// The runner itself.
    pub runner: Runner,
    /// The runner's sessions.
    pub sessions: Vec<Value>,
    /// What taking this sample cost.
    pub cost: Cost,
}

/// The processors' load.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Cpu {
    /// Logical cores.
    pub cores: u32,
    /// The load average over 1 minute.
    pub load1: Option<f64>,
    /// The load average over 5 minutes.
    pub load5: Option<f64>,
    /// The load average over 15 minutes.
    pub load15: Option<f64>,
    /// The busy share since the last sample, 0..1; `None` where it isn't measured.
    pub usage: Option<f64>,
}

/// How hard memory is pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Pressure {
    /// Room to spare.
    Normal,
    /// Getting tight.
    Warn,
    /// Out of room: tasks wait on memory, or almost none is left.
    Critical,
}

/// Linux's memory pressure stall over the last minute, in percent of the time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Psi {
    /// Some task waited on memory.
    pub some60: f64,
    /// Every task waited on memory at once.
    pub full60: f64,
}

/// The machine's memory.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
    /// All of it, in bytes.
    pub total: u64,
    /// What the system can hand out now without swapping, in bytes.
    pub available: u64,
    /// `total` less `available`.
    pub used: u64,
    /// The swap's size.
    pub swap_total: u64,
    /// What is used of the swap.
    pub swap_used: u64,
    /// How hard memory is pressed.
    pub pressure: Pressure,
    /// Linux's pressure stall, when the kernel keeps it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub psi: Option<Psi>,
}

/// A GPU.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gpu {
    /// Its index on the machine.
    pub index: u32,
    /// Its model.
    pub name: String,
    /// Its busy share, 0..1.
    pub util: Option<f64>,
    /// The memory in use, in bytes.
    pub mem_used: u64,
    /// Its memory; for a GPU that shares the machine's memory, all of the machine's.
    pub mem_total: u64,
    /// Its temperature, °C.
    pub temp: Option<f64>,
    /// What it draws, watts.
    pub power: Option<f64>,
    /// The processes holding its memory.
    pub procs: Vec<GpuProc>,
    /// Its cores, where the system says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cores: Option<u32>,
    /// Its memory is the machine's unified memory.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub shared: bool,
}

/// A process holding GPU memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GpuProc {
    /// Its process id.
    pub pid: u32,
    /// Its program's name.
    pub name: String,
    /// The GPU memory it holds, in bytes.
    pub mem: u64,
    /// The systemd unit it lives in, if any.
    pub unit: Option<String>,
    /// The user it runs as: this runner's user by name, any other by uid.
    pub user: Option<String>,
}

/// A volume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Disk {
    /// Where it is mounted (one of the paths looked at).
    pub path: String,
    /// Its size, in bytes.
    pub total: u64,
    /// What an unprivileged user may still write, in bytes.
    pub free: u64,
}

/// The battery and the charger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Battery {
    /// Its charge, 0..100.
    pub percent: Option<u64>,
    /// A charger is connected.
    pub plugged: bool,
    /// What the system says it does, lower case (`charging`, `discharging`, `charged`, …).
    pub state: String,
    /// Minutes left while it discharges.
    pub minutes_left: Option<u64>,
}

/// The memory cap over the runner: its cgroup and the tightest limit above it (Linux).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cgroup {
    /// The runner's own cgroup (its unit, or a scope).
    pub name: Option<String>,
    /// What the runner's cgroup holds now, in bytes (its sessions' agents are in it too).
    pub current: Option<u64>,
    /// The tightest `memory.high` on the way up.
    pub high: Option<u64>,
    /// The tightest `memory.max` on the way up.
    pub max: Option<u64>,
    /// The cgroup that sets `high`.
    pub high_by: Option<String>,
    /// The cgroup that sets `max`.
    pub max_by: Option<String>,
    /// What the cgroup holding the cap uses, less the file cache the kernel takes back first.
    pub cap_used: Option<u64>,
}

/// The runner itself.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Runner {
    /// Its process id.
    pub pid: u32,
    /// Its resident memory, in bytes.
    pub rss: Option<u64>,
    /// The memory cap over it.
    pub cgroup: Option<Cgroup>,
    /// Sessions running a turn.
    pub sessions: u32,
    /// Turns waiting.
    pub queued: u32,
    /// Sessions open.
    pub open: u32,
    /// The agent's processes on the whole machine (this runner's and any other's).
    pub claude: Option<u32>,
    /// The memory they hold, in bytes.
    pub claude_rss: Option<u64>,
}

/// What a sample cost the machine.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cost {
    /// Wall time, milliseconds.
    pub ms: f64,
    /// The runner's CPU time, milliseconds, where the system says.
    pub cpu_ms: Option<f64>,
}

/// The parts of a sample read at most once a minute.
#[derive(Debug, Clone, Default)]
struct Slow {
    /// NVIDIA GPUs (Linux).
    gpus: Vec<Gpu>,
    /// The agent's processes, from `ps`.
    agents: Option<Agents>,
    /// The battery.
    battery: Option<Battery>,
}

/// What each sample reads afresh.
#[derive(Debug, Clone, Default)]
struct Fast {
    /// Logical cores.
    cores: Option<u32>,
    /// The three load averages.
    load: Option<[f64; 3]>,
    /// The memory.
    memory: Option<Memory>,
    /// GPUs read in every sample (Apple silicon's); `None`: the slow part's.
    gpus: Option<Vec<Gpu>>,
}

/// Takes samples of the machine, keeping what one sample needs from the one before (the CPU's
/// times) and the slow parts between their readings.
#[derive(Debug)]
pub struct Sampler {
    /// The machine.
    info: MachineInfo,
    /// The CPU's times at the last sample.
    cpu: Option<CpuTimes>,
    /// The slow parts and when they were read.
    slow: Option<(Instant, Slow)>,
}

impl Sampler {
    /// A sampler for this machine; it notes the CPU's times now, so the first sample has a busy
    /// share.
    #[must_use]
    pub fn new(info: MachineInfo) -> Self {
        Self {
            info,
            cpu: os::cpu_times(),
            slow: None,
        }
    }

    /// One sample of the machine with the runner's sessions.
    pub async fn sample(&mut self, sessions: SessionsSummary) -> Sample {
        let started = Instant::now();
        let ticks = os::cpu_ticks();
        let fresh = self
            .slow
            .as_ref()
            .is_some_and(|(at, _)| at.elapsed() < SLOW_EVERY);
        let (fast, slow) = if fresh {
            (os::fast().await, None)
        } else {
            let (fast, slow) = tokio::join!(os::fast(), os::slow());
            (fast, Some(slow))
        };
        if let Some(slow) = slow {
            self.slow = Some((Instant::now(), slow));
        }
        let slow = self.slow.as_ref().map_or_default(|(_, s)| s.clone());

        let now = os::cpu_times();
        let usage = match (self.cpu, now) {
            (Some(was), Some(now)) => parse::busy_share(was, now),
            _ => None,
        };
        if now.is_some() {
            self.cpu = now;
        }
        let load = fast.load.map_or([None; 3], |l| l.map(Some));
        let disks = os::disks();
        let cgroup = os::cgroup();
        let rss = os::own_rss().or(slow.agents.and_then(|a| a.own_rss));
        let cost = Cost {
            ms: tenths(started.elapsed().as_secs_f64() * 1000.0),
            cpu_ms: ticks
                .zip(os::cpu_ticks())
                .map(|(was, now)| ticks_ms(now.saturating_sub(was))),
        };
        Sample {
            machine: self.info.name.clone(),
            at: now_text(),
            host: host(),
            platform: platform(),
            cpu: Cpu {
                cores: fast.cores.unwrap_or_else(cores),
                load1: load[0],
                load5: load[1],
                load15: load[2],
                usage,
            },
            memory: fast.memory,
            gpus: fast.gpus.unwrap_or(slow.gpus),
            disks,
            battery: slow.battery,
            runner: Runner {
                pid: std::process::id(),
                rss,
                cgroup,
                sessions: sessions.running,
                queued: sessions.queued,
                open: sessions.open,
                claude: slow.agents.map(|a| a.count),
                claude_rss: slow.agents.map(|a| a.rss),
            },
            sessions: sessions.list,
            cost,
        }
    }
}

/// A figure rounded to one decimal.
fn tenths(n: f64) -> f64 {
    (n * 10.0).round() / 10.0
}

/// Clock ticks (Linux's fixed 100 per second for process times) in milliseconds.
#[expect(
    clippy::cast_precision_loss,
    reason = "a sample's CPU time is a handful of ticks"
)]
fn ticks_ms(ticks: u64) -> f64 {
    ticks as f64 * 10.0
}

/// Now, as the server's timestamps are written: RFC 3339 in UTC with milliseconds.
pub(crate) fn now_text() -> String {
    format!("{:.3}", jiff::Timestamp::now())
}

/// The system's name as the server knows it (Node's names, which the server's data uses).
fn platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    }
}

/// Logical cores, as the standard library counts them.
fn cores() -> u32 {
    std::thread::available_parallelism()
        .ok()
        .and_then(|n| u32::try_from(n.get()).ok())
        .unwrap_or(1)
}

/// The machine's host name, or `runner` when it has none to give.
#[must_use]
pub fn hostname() -> String {
    host().unwrap_or_else(|| "runner".to_owned())
}

/// The machine's host name.
#[cfg(unix)]
fn host() -> Option<String> {
    let name = rustix::system::uname();
    let name = name.nodename().to_string_lossy();
    (!name.is_empty()).then(|| name.into_owned())
}

/// The machine's host name.
#[cfg(not(unix))]
fn host() -> Option<String> {
    std::env::var("COMPUTERNAME").ok().filter(|n| !n.is_empty())
}

/// Each path's volume once: its size and free space.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn disks_at(paths: &[&str]) -> Vec<Disk> {
    use std::os::unix::fs::MetadataExt;
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for path in paths {
        let Ok(meta) = std::fs::metadata(path) else {
            continue;
        };
        if seen.contains(&meta.dev()) {
            continue;
        }
        seen.push(meta.dev());
        let Ok(fs) = rustix::fs::statvfs(*path) else {
            continue;
        };
        out.push(Disk {
            path: (*path).to_owned(),
            total: fs.f_blocks.saturating_mul(fs.f_frsize),
            free: fs.f_bavail.saturating_mul(fs.f_frsize),
        });
    }
    out
}

/// Runs a program and gives its output, or `None` when it can't start or takes too long.
/// Whether a failed exit still counts is the caller's: some programs print what they could
/// read and fail for the rest.
#[cfg(unix)]
pub(crate) async fn run(program: &str, args: &[&str]) -> Option<std::process::Output> {
    let child = tokio::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    tokio::time::timeout(PROGRAM_TIMEOUT, child)
        .await
        .ok()?
        .ok()
}

/// A program's output as text when it exited well.
#[cfg(any(target_os = "linux", target_os = "macos"))]
async fn text_of(program: &str, args: &[&str]) -> Option<String> {
    let out = run(program, args).await?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sample_has_the_servers_shape() {
        let sample = Sample {
            machine: "build".to_owned(),
            at: "2026-11-24T21:57:53.410Z".to_owned(),
            host: Some("build.example.com".to_owned()),
            platform: "linux",
            cpu: Cpu {
                cores: 8,
                load1: Some(1.0),
                load5: Some(1.2),
                load15: Some(1.1),
                usage: Some(0.25),
            },
            memory: Some(Memory {
                total: 64,
                available: 32,
                used: 32,
                swap_total: 0,
                swap_used: 0,
                pressure: Pressure::Normal,
                psi: None,
            }),
            gpus: vec![Gpu {
                index: 0,
                name: "GPU 0".to_owned(),
                util: Some(0.5),
                mem_used: 2,
                mem_total: 24,
                temp: Some(41.0),
                power: None,
                procs: vec![GpuProc {
                    pid: 41,
                    name: "llama-server".to_owned(),
                    mem: 1,
                    unit: Some("x.service".to_owned()),
                    user: Some("example".to_owned()),
                }],
                cores: None,
                shared: false,
            }],
            disks: vec![Disk {
                path: "/".to_owned(),
                total: 500,
                free: 100,
            }],
            battery: Some(Battery {
                percent: Some(80),
                plugged: true,
                state: "charging".to_owned(),
                minutes_left: None,
            }),
            runner: Runner {
                pid: 7,
                rss: Some(100),
                cgroup: Some(Cgroup {
                    name: Some("runner.service".to_owned()),
                    max: Some(10),
                    cap_used: Some(6),
                    max_by: Some("app.slice".to_owned()),
                    ..Cgroup::default()
                }),
                sessions: 1,
                queued: 0,
                open: 2,
                claude: Some(1),
                claude_rss: Some(2),
            },
            sessions: vec![serde_json::json!({ "id": "r-1a2b", "running": true })],
            cost: Cost {
                ms: 3.2,
                cpu_ms: None,
            },
        };
        let v = serde_json::to_value(&sample).unwrap();
        assert_eq!(v["memory"]["swapTotal"], 0);
        assert_eq!(v["memory"]["pressure"], "normal");
        assert!(v["memory"].get("psi").is_none());
        assert_eq!(v["gpus"][0]["memUsed"], 2);
        assert_eq!(v["gpus"][0]["index"].as_u64(), Some(0));
        assert_eq!(v["gpus"][0]["procs"][0]["pid"].as_u64(), Some(41));
        assert!(v["gpus"][0].get("shared").is_none());
        assert!(v["gpus"][0].get("cores").is_none());
        assert_eq!(v["battery"]["minutesLeft"], Value::Null);
        assert_eq!(v["runner"]["claudeRss"], 2);
        assert_eq!(v["runner"]["cgroup"]["capUsed"], 6);
        assert_eq!(v["runner"]["cgroup"]["maxBy"], "app.slice");
        assert_eq!(v["cost"]["cpuMs"], Value::Null);
        assert_eq!(v["sessions"][0]["id"], "r-1a2b");
    }

    #[test]
    fn timestamps_have_milliseconds() {
        let at = now_text();
        assert_eq!(at.len(), "2026-11-24T21:57:53.410Z".len(), "{at}");
        assert!(at.ends_with('Z'));
    }

    #[tokio::test]
    async fn a_sample_of_this_machine() {
        let mut sampler = Sampler::new(MachineInfo {
            name: "here".to_owned(),
        });
        let first = sampler
            .sample(SessionsSummary {
                running: 1,
                open: 2,
                queued: 0,
                list: Vec::new(),
            })
            .await;
        assert_eq!(first.machine, "here");
        assert!(first.cpu.cores >= 1);
        assert_eq!(first.runner.pid, std::process::id());
        assert_eq!((first.runner.sessions, first.runner.open), (1, 2));
        if cfg!(target_os = "linux") {
            let memory = first.memory.as_ref().unwrap();
            assert!(memory.total > 0 && memory.available <= memory.total);
            assert!(first.cpu.load1.is_some());
            assert_ne!(first.disks.len(), 0);
            assert!(first.runner.rss.is_some_and(|r| r > 0));
        }
        let second = sampler.sample(SessionsSummary::default()).await;
        assert_eq!(
            second.runner.claude, first.runner.claude,
            "the slow part is reused"
        );
        let json = serde_json::to_value(&second).unwrap();
        assert!(json["cpu"].is_object() && json["runner"].is_object());
    }
}
