//! The text the machine's files and programs print, read into figures. Every function here is
//! pure and compiled on every system, so each one is tested on captured samples wherever the
//! tests run; only the reading of the files and the running of the programs depends on the
//! system.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

use super::{Battery, Cgroup, Gpu, GpuProc, Memory, Pressure, Psi};

/// One mebibyte, the unit `nvidia-smi` reports memory in.
const MIB: u64 = 1024 * 1024;

/// A pattern known to compile; `None` only if it didn't, which the tests rule out.
fn re(pattern: &str) -> Option<Regex> {
    Regex::new(pattern).ok()
}

/// The first capture of `pattern` in `text`.
fn capture<'t>(pattern: &LazyLock<Option<Regex>>, text: &'t str) -> Option<&'t str> {
    pattern.as_ref()?.captures(text)?.get(1).map(|m| m.as_str())
}

/// A number, or `None` for an empty or unreadable one (`[N/A]`, `max`).
fn number(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|n| n.is_finite())
}

/// A whole non-negative number.
fn whole(text: &str) -> Option<u64> {
    text.trim().parse::<u64>().ok()
}

/// A byte count from a figure in some unit, rounded down; negative or unreadable counts are 0.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the figure is checked to be finite and non-negative, and byte counts fit in u64"
)]
fn bytes(figure: f64) -> u64 {
    if figure.is_finite() && figure > 0.0 {
        figure as u64
    } else {
        0
    }
}

/// `/proc/meminfo`: each key's value in bytes (the file counts in kibibytes).
#[must_use]
pub fn meminfo(text: &str) -> BTreeMap<String, u64> {
    text.lines()
        .filter_map(|line| {
            let (key, rest) = line.split_once(':')?;
            let kib = whole(rest.split_whitespace().next()?)?;
            Some((key.trim().to_owned(), kib.saturating_mul(1024)))
        })
        .collect()
}

/// `/proc/pressure/memory`: the share of the last minute (percent) in which some tasks, and in
/// which all of them at once, waited on memory.
#[must_use]
pub fn psi(text: &str) -> Psi {
    let avg60 = |kind: &str| {
        text.lines()
            .find(|line| line.split_whitespace().next() == Some(kind))
            .and_then(|line| {
                line.split_whitespace()
                    .find_map(|field| field.strip_prefix("avg60="))
            })
            .and_then(number)
            .unwrap_or(0.0)
    };
    Psi {
        some60: avg60("some"),
        full60: avg60("full"),
    }
}

/// How hard memory is pressed. Critical when every task stalled on memory for 10% of the last
/// minute (thrashing) or under 3% of it is left; warn at 20% of the minute with some task
/// stalled or under 8% left.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "a share of memory; a byte count's last digits don't change it"
)]
pub fn pressure(total: u64, available: u64, psi: Option<Psi>) -> Pressure {
    let (total, available) = (total as f64, available as f64);
    let psi = psi.unwrap_or_default();
    if psi.full60 >= 10.0 || available < total * 0.03 {
        Pressure::Critical
    } else if psi.some60 >= 20.0 || available < total * 0.08 {
        Pressure::Warn
    } else {
        Pressure::Normal
    }
}

/// Linux's memory from `/proc/meminfo` (and the pressure stall, when the kernel keeps it).
#[must_use]
pub fn linux_memory(meminfo_text: &str, psi_text: Option<&str>) -> Option<Memory> {
    let m = meminfo(meminfo_text);
    let total = *m.get("MemTotal")?;
    let available = m
        .get("MemAvailable")
        .or_else(|| m.get("MemFree"))
        .copied()?
        .min(total);
    let swap_total = m.get("SwapTotal").copied().unwrap_or(0);
    let swap_free = m.get("SwapFree").copied().unwrap_or(0);
    let psi = psi_text.map(psi);
    Some(Memory {
        total,
        available,
        used: total - available,
        swap_total,
        swap_used: swap_total.saturating_sub(swap_free),
        pressure: pressure(total, available, psi),
        psi,
    })
}

/// `/proc/stat`'s total CPU times: the busy part (user, nice, system, irq) and the whole (busy and
/// idle), in clock ticks, and the number of cores it lists.
#[must_use]
pub fn proc_stat(text: &str) -> Option<(CpuTimes, u32)> {
    let mut times = None;
    let mut cores = 0u32;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        match fields.next() {
            Some("cpu") => {
                let v: Vec<u64> = fields.filter_map(whole).collect();
                let at = |i: usize| v.get(i).copied().unwrap_or(0);
                let busy = at(0) + at(1) + at(2) + at(5);
                times = Some(CpuTimes {
                    busy,
                    total: busy + at(3),
                });
            }
            Some(name) if name.starts_with("cpu") => cores += 1,
            _ => {}
        }
    }
    times.map(|t| (t, cores))
}

/// The CPU's busy and total times at one moment, in any one unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuTimes {
    /// Time spent running (user, nice, system, interrupts).
    pub busy: u64,
    /// Busy time and idle time together.
    pub total: u64,
}

/// The busy share (0..1, three decimals) between two readings of the CPU's times; `None` when
/// no time passed.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "a share of time; the ticks' last digits don't change it"
)]
pub fn busy_share(was: CpuTimes, now: CpuTimes) -> Option<f64> {
    let total = now.total.checked_sub(was.total).filter(|t| *t > 0)?;
    let busy = now.busy.saturating_sub(was.busy).min(total);
    Some((busy as f64 / total as f64 * 1000.0).round() / 1000.0)
}

/// The three load averages (1, 5 and 15 minutes) from `/proc/loadavg` or macOS's `vm.loadavg`
/// (`{ 1.20 1.45 1.67 }`).
#[must_use]
pub fn loadavg(text: &str) -> Option<[f64; 3]> {
    let mut it = text
        .split_whitespace()
        .filter(|s| *s != "{" && *s != "}")
        .map(number);
    Some([it.next()??, it.next()??, it.next()??])
}

/// The cgroup v2 path in `/proc/<pid>/cgroup` (the `0::` line).
#[must_use]
pub fn cgroup_path(text: &str) -> Option<&str> {
    text.lines()
        .find_map(|line| line.strip_prefix("0::"))
        .map(str::trim)
        .filter(|p| p.starts_with('/'))
}

/// The systemd unit (a `.service` or `.scope`) a process lives in, from its `/proc/<pid>/cgroup`.
#[must_use]
pub fn unit(text: &str) -> Option<&str> {
    static UNIT: LazyLock<Option<Regex>> =
        LazyLock::new(|| re(r"(?m)/([^/\n]+\.(?:service|scope))\s*$"));
    capture(&UNIT, text)
}

/// A cgroup's `inactive_file` from its `memory.stat`, in bytes.
#[must_use]
pub fn inactive_file(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix("inactive_file "))
        .and_then(whole)
}

/// The memory cap over the cgroup at `path`: the tightest `memory.high` and `memory.max` on the
/// way up to (not including) the root, and what the cgroup holding the cap uses of it.
///
/// `read(cgroup, file)` reads one of a cgroup's files (`memory.current`, `memory.high`,
/// `memory.max`, `memory.stat`). What the cap is measured against is the working set of the
/// cgroup that has it: `memory.current` less the inactive file cache the kernel takes back
/// first, as container runtimes count it; a slice full of page cache is not a full cap.
#[must_use]
pub fn cgroup(path: &str, read: impl Fn(&str, &str) -> Option<String>) -> Cgroup {
    let limit = |p: &str, file: &str| read(p, file).as_deref().and_then(whole);
    let mut high: Option<(u64, &str)> = None;
    let mut max: Option<(u64, &str)> = None;
    let mut at = path;
    while !at.is_empty() && at != "/" {
        if let Some(h) = limit(at, "memory.high")
            && high.is_none_or(|(was, _)| h < was)
        {
            high = Some((h, at));
        }
        if let Some(m) = limit(at, "memory.max")
            && max.is_none_or(|(was, _)| m < was)
        {
            max = Some((m, at));
        }
        at = match at.rfind('/') {
            Some(0) | None => "/",
            Some(i) => &at[..i],
        };
    }
    let name = |p: &str| p.rsplit('/').next().unwrap_or(p).to_owned();
    let cap_used = max.or(high).and_then(|(_, by)| {
        let current = limit(by, "memory.current")?;
        let inactive = read(by, "memory.stat")
            .as_deref()
            .and_then(inactive_file)
            .unwrap_or(0);
        Some(current.saturating_sub(inactive))
    });
    Cgroup {
        name: Some(name(path)),
        current: limit(path, "memory.current"),
        high: high.map(|(h, _)| h),
        max: max.map(|(m, _)| m),
        high_by: high.map(|(_, by)| name(by)),
        max_by: max.map(|(_, by)| name(by)),
        cap_used,
    }
}

/// A battery from Linux's `power_supply` class: its `capacity`, `status`, `power_now` and
/// `energy_now`, and whether any mains supply is online. The minutes left are counted only while
/// it discharges.
#[must_use]
pub fn linux_battery(
    capacity: Option<&str>,
    status: Option<&str>,
    power_now: Option<&str>,
    energy_now: Option<&str>,
    plugged: bool,
) -> Battery {
    let status = status.map_or("Unknown", str::trim);
    let minutes_left = match (power_now.and_then(number), energy_now.and_then(number)) {
        (Some(power), Some(energy)) if status == "Discharging" && power > 0.0 && energy > 0.0 => {
            Some(bytes((energy / power * 60.0).round()))
        }
        _ => None,
    };
    Battery {
        percent: capacity.and_then(whole),
        plugged,
        state: status.to_lowercase(),
        minutes_left,
    }
}

/// `pmset -g batt` on macOS; `None` without an internal battery.
#[must_use]
pub fn pmset(text: &str) -> Option<Battery> {
    static PERCENT: LazyLock<Option<Regex>> = LazyLock::new(|| re(r"(\d+)%"));
    static STATE: LazyLock<Option<Regex>> = LazyLock::new(|| re(r"\d+%;\s*([\w ]+?);"));
    static LEFT: LazyLock<Option<Regex>> = LazyLock::new(|| re(r"(\d+):(\d+) remaining"));
    if !text.contains("InternalBattery") {
        return None;
    }
    let state =
        capture(&STATE, text).map_or_else(|| "unknown".to_owned(), |s| s.trim().to_lowercase());
    let minutes_left = LEFT
        .as_ref()
        .and_then(|re| re.captures(text))
        .filter(|_| state == "discharging")
        .and_then(|c| Some(whole(c.get(1)?.as_str())? * 60 + whole(c.get(2)?.as_str())?));
    Some(Battery {
        percent: capture(&PERCENT, text).and_then(whole),
        plugged: text.contains("AC Power"),
        state,
        minutes_left,
    })
}

/// `nvidia-smi --query-gpu=index,uuid,name,utilization.gpu,memory.used,memory.total,
/// temperature.gpu,power.draw --format=csv,noheader,nounits`: each GPU with its UUID.
#[must_use]
pub fn nvidia_gpus(text: &str) -> Vec<(String, Gpu)> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let f: Vec<&str> = line.split(',').map(str::trim).collect();
            let at = |i: usize| f.get(i).copied().unwrap_or("");
            let gpu = Gpu {
                index: at(0).parse().ok()?,
                name: Some(at(2))
                    .filter(|n| !n.is_empty())
                    .unwrap_or("GPU")
                    .to_owned(),
                util: number(at(3)).map(|u| u / 100.0),
                mem_used: number(at(4)).map_or(0, |m| bytes(m * MIB_F)),
                mem_total: number(at(5)).map_or(0, |m| bytes(m * MIB_F)),
                temp: number(at(6)),
                power: number(at(7)),
                procs: Vec::new(),
                cores: None,
                shared: false,
            };
            Some((at(1).to_owned(), gpu))
        })
        .collect()
}

/// [`MIB`] as a float, for the figures `nvidia-smi` prints with decimals.
#[expect(clippy::cast_precision_loss, reason = "2^20 is exact in an f64")]
const MIB_F: f64 = MIB as f64;

/// One compute process on a GPU, as `nvidia-smi --query-compute-apps` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputeApp {
    /// The UUID of the GPU it runs on.
    pub gpu: String,
    /// Its process id.
    pub pid: u32,
    /// Its program's name (the last part of its path).
    pub name: String,
    /// The GPU memory it holds, in bytes.
    pub mem: u64,
}

/// `nvidia-smi --query-compute-apps=gpu_uuid,pid,process_name,used_memory
/// --format=csv,noheader,nounits`.
#[must_use]
pub fn nvidia_apps(text: &str) -> Vec<ComputeApp> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(',').map(str::trim).collect();
            let at = |i: usize| f.get(i).copied().unwrap_or("");
            Some(ComputeApp {
                gpu: at(0).to_owned(),
                pid: at(1).parse().ok()?,
                name: at(2).rsplit('/').next().unwrap_or("").to_owned(),
                mem: number(at(3)).map_or(0, |m| bytes(m * MIB_F)),
            })
        })
        .collect()
}

/// Puts each compute process on its GPU (the first one when its UUID matches none).
pub fn attach(gpus: &mut [(String, Gpu)], apps: Vec<(ComputeApp, GpuProc)>) {
    for (app, proc) in apps {
        let at = gpus
            .iter()
            .position(|(uuid, _)| *uuid == app.gpu)
            .unwrap_or(0);
        if let Some((_, gpu)) = gpus.get_mut(at) {
            gpu.procs.push(proc);
        }
    }
}

/// Apple silicon's GPU from `ioreg -r -d 1 -w 0 -c IOAccelerator` (no root needed): its
/// `model`, `gpu-core-count`, and in its `PerformanceStatistics` the `Device Utilization %` and
/// `In use system memory` (its share of the unified memory; `total` is the machine's memory).
#[must_use]
pub fn ioreg(text: &str, total: u64) -> Vec<Gpu> {
    static MODEL: LazyLock<Option<Regex>> = LazyLock::new(|| re(r#"(?m)^\s*"model" = "([^"]+)""#));
    static FIRST: LazyLock<Option<Regex>> = LazyLock::new(|| re(r"^(\w+)"));
    static CORES: LazyLock<Option<Regex>> =
        LazyLock::new(|| re(r#"(?m)^\s*"gpu-core-count" = (\d+)"#));
    static UTIL: LazyLock<Option<Regex>> =
        LazyLock::new(|| re(r#""Device Utilization %"\s*=\s*(\d+)"#));
    static MEM: LazyLock<Option<Regex>> =
        LazyLock::new(|| re(r#""In use system memory"\s*=\s*(\d+)"#));
    let blocks = text
        .split("\n+-o ")
        .map(|b| b.strip_prefix("+-o ").unwrap_or(b))
        .filter(|b| b.contains("\"PerformanceStatistics\""));
    blocks
        .zip(0u32..)
        .map(|(b, index)| Gpu {
            index,
            name: capture(&MODEL, b)
                .or_else(|| capture(&FIRST, b))
                .unwrap_or("Apple GPU")
                .to_owned(),
            cores: capture(&CORES, b).and_then(|c| c.parse().ok()),
            util: capture(&UTIL, b)
                .and_then(number)
                .map(|u| u.min(100.0) / 100.0),
            mem_used: capture(&MEM, b).and_then(whole).unwrap_or(0),
            mem_total: total,
            temp: None,
            power: None,
            shared: true,
            procs: Vec::new(),
        })
        .collect()
}

/// `vm_stat` on macOS: the bytes it can hand out (free, inactive, purgeable and speculative
/// pages, at the page size it prints).
#[must_use]
pub fn vm_stat(text: &str) -> Option<u64> {
    static PAGE: LazyLock<Option<Regex>> = LazyLock::new(|| re(r"page size of (\d+)"));
    let page = capture(&PAGE, text).and_then(whole).unwrap_or(16_384);
    let pages = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix(':'))
            .and_then(|v| whole(v.trim().trim_end_matches('.')))
    };
    let keys = [
        "Pages free",
        "Pages inactive",
        "Pages purgeable",
        "Pages speculative",
    ];
    let found: Vec<u64> = keys.iter().filter_map(|k| pages(k)).collect();
    (!found.is_empty()).then(|| found.iter().sum::<u64>().saturating_mul(page))
}

/// `sysctl <name>…` (without `-n`): each name's value.
#[must_use]
pub fn sysctl(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(": ")?;
            Some((key.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
}

/// macOS's `vm.swapusage` (`total = 2048.00M  used = 1024.50M  free = 1023.50M`): the swap's
/// size and what is used of it, in bytes.
#[must_use]
pub fn swapusage(text: &str) -> (u64, u64) {
    let mb = |key: &str| {
        let rest = text.split(key).nth(1)?.trim_start().strip_prefix('=')?;
        let figure = rest.split_whitespace().next()?;
        let (n, unit) = figure.split_at(figure.find(|c: char| c.is_ascii_alphabetic())?);
        let scale = match unit {
            "K" => 1024.0,
            "M" => MIB_F,
            "G" => MIB_F * 1024.0,
            _ => return None,
        };
        Some(bytes(number(n)? * scale))
    };
    (mb("total").unwrap_or(0), mb("used").unwrap_or(0))
}

/// macOS's memory from `vm_stat` and `sysctl hw.memsize kern.memorystatus_vm_pressure_level
/// vm.swapusage`. The pressure is the kernel's own level (1 normal, 2 warn, 4 critical: what
/// Activity Monitor's graph shows).
#[must_use]
pub fn mac_memory(vm_stat_text: &str, sysctl: &BTreeMap<String, String>) -> Option<Memory> {
    let total = sysctl.get("hw.memsize").and_then(|v| whole(v))?;
    let available = vm_stat(vm_stat_text)?.min(total);
    let (swap_total, swap_used) = sysctl.get("vm.swapusage").map_or((0, 0), |v| swapusage(v));
    let pressure = match sysctl
        .get("kern.memorystatus_vm_pressure_level")
        .and_then(|v| whole(v))
    {
        Some(4) => Pressure::Critical,
        Some(2) => Pressure::Warn,
        _ => Pressure::Normal,
    };
    Some(Memory {
        total,
        available,
        used: total - available,
        swap_total,
        swap_used,
        pressure,
        psi: None,
    })
}

/// The agent's processes in `ps -o pid=,rss=,comm=`: how many are named `claude`, the memory
/// they hold (bytes), and the resident memory of the process `own` (this runner), if listed.
#[must_use]
pub fn agents(text: &str, own: u32) -> Agents {
    let mut out = Agents::default();
    for line in text.lines() {
        let mut f = line.split_whitespace();
        let (Some(pid), Some(rss)) = (
            f.next().and_then(|p| p.parse::<u32>().ok()),
            f.next().and_then(whole),
        ) else {
            continue;
        };
        let rss = rss.saturating_mul(1024);
        let command = f.collect::<Vec<_>>().join(" ");
        if command == "claude" || command.ends_with("/claude") {
            out.count += 1;
            out.rss += rss;
        }
        if pid == own {
            out.own_rss = Some(rss);
        }
    }
    out
}

/// What [`agents`] finds in a process list.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Agents {
    /// The agent's processes.
    pub count: u32,
    /// The memory they hold, in bytes.
    pub rss: u64,
    /// This runner's resident memory, in bytes.
    pub own_rss: Option<u64>,
}

/// `VmRSS` in `/proc/<pid>/status`, in bytes.
#[must_use]
pub fn status_rss(text: &str) -> Option<u64> {
    meminfo(text).get("VmRSS").copied()
}

/// The CPU time (user and system, in clock ticks) in `/proc/<pid>/stat`.
#[must_use]
pub fn stat_ticks(text: &str) -> Option<u64> {
    // The command name, in parentheses, may itself hold spaces and parentheses.
    let rest = &text[text.rfind(')')? + 1..];
    let mut fields = rest.split_whitespace().skip(11);
    Some(whole(fields.next()?)? + whole(fields.next()?)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEMINFO: &str = "MemTotal:       65536000 kB\nMemFree:         2048000 kB\nMemAvailable:   32768000 kB\nBuffers:          512000 kB\nCached:         20480000 kB\nSwapCached:            0 kB\nSwapTotal:       8388604 kB\nSwapFree:        8000000 kB\nHugePages_Total:       0\n";
    const PSI: &str = "some avg10=0.00 avg60=1.25 avg300=0.80 total=123456\nfull avg10=0.00 avg60=0.40 avg300=0.10 total=65432\n";

    #[test]
    fn meminfo_counts_bytes() {
        let m = meminfo(MEMINFO);
        assert_eq!(m["MemTotal"], 65_536_000 * 1024);
        assert_eq!(m["HugePages_Total"], 0);
    }

    #[test]
    fn linux_memory_reads_available_and_swap() {
        let m = linux_memory(MEMINFO, Some(PSI)).unwrap();
        assert_eq!(m.total, 65_536_000 * 1024);
        assert_eq!(m.available, 32_768_000 * 1024);
        assert_eq!(m.used, m.total - m.available);
        assert_eq!(m.swap_total, 8_388_604 * 1024);
        assert_eq!(m.swap_used, 388_604 * 1024);
        assert_eq!(m.pressure, Pressure::Normal);
        assert_eq!(
            m.psi,
            Some(Psi {
                some60: 1.25,
                full60: 0.4
            })
        );
        assert!(linux_memory("garbage", None).is_none());
    }

    #[test]
    fn pressure_thresholds() {
        let calm = Psi {
            some60: 0.0,
            full60: 0.0,
        };
        assert_eq!(pressure(100, 50, Some(calm)), Pressure::Normal);
        assert_eq!(pressure(100, 7, Some(calm)), Pressure::Warn);
        assert_eq!(pressure(100, 2, Some(calm)), Pressure::Critical);
        assert_eq!(
            pressure(
                100,
                50,
                Some(Psi {
                    some60: 20.0,
                    full60: 0.0
                })
            ),
            Pressure::Warn
        );
        assert_eq!(
            pressure(
                100,
                50,
                Some(Psi {
                    some60: 30.0,
                    full60: 10.0
                })
            ),
            Pressure::Critical
        );
        assert_eq!(pressure(100, 50, None), Pressure::Normal);
    }

    #[test]
    fn cpu_times_and_share() {
        let stat = "cpu  100 20 30 800 50 5 3 0 0 0\ncpu0 50 10 15 400 25 3 2 0 0 0\ncpu1 50 10 15 400 25 2 1 0 0 0\nintr 12345\nctxt 999\n";
        let (t, cores) = proc_stat(stat).unwrap();
        assert_eq!(cores, 2);
        assert_eq!(
            t,
            CpuTimes {
                busy: 155,
                total: 955
            }
        );
        let later = CpuTimes {
            busy: 255,
            total: 1355,
        };
        assert_eq!(busy_share(t, later), Some(0.25));
        assert_eq!(busy_share(t, t), None);
        assert!(proc_stat("").is_none());
    }

    #[test]
    fn load_averages() {
        assert_eq!(
            loadavg("0.52 0.61 0.70 2/1234 98765\n"),
            Some([0.52, 0.61, 0.7])
        );
        assert_eq!(loadavg("{ 1.20 1.45 1.67 }"), Some([1.2, 1.45, 1.67]));
        assert_eq!(loadavg("1.0"), None);
    }

    #[test]
    fn cgroup_lines() {
        let text = "0::/system.slice/runners.slice/emberhut-runner.service\n";
        assert_eq!(
            cgroup_path(text),
            Some("/system.slice/runners.slice/emberhut-runner.service")
        );
        assert_eq!(unit(text), Some("emberhut-runner.service"));
        assert_eq!(
            unit("0::/system.slice/llama-example.service\n"),
            Some("llama-example.service")
        );
        assert_eq!(
            unit("0::/user.slice/user-1000.slice/session-3.scope"),
            Some("session-3.scope")
        );
        assert_eq!(unit("0::/\n"), None);
        assert_eq!(cgroup_path("12:cpu:/x\n"), None);
    }

    #[test]
    fn cgroup_takes_the_tightest_cap_and_its_working_set() {
        let files: BTreeMap<(&str, &str), &str> = [
            (
                ("/a.slice/app.slice/runner.service", "memory.current"),
                "1000",
            ),
            (("/a.slice/app.slice/runner.service", "memory.high"), "max"),
            (("/a.slice/app.slice/runner.service", "memory.max"), "max"),
            (("/a.slice/app.slice", "memory.current"), "9000"),
            (("/a.slice/app.slice", "memory.high"), "8000"),
            (("/a.slice/app.slice", "memory.max"), "10000"),
            (
                ("/a.slice/app.slice", "memory.stat"),
                "anon 4000\nfile 5000\ninactive_file 3000\nactive_file 2000\n",
            ),
            (("/a.slice", "memory.high"), "9000"),
            (("/a.slice", "memory.max"), "20000"),
        ]
        .into_iter()
        .collect();
        let read = |p: &str, f: &str| files.get(&(p, f)).map(|v| (*v).to_owned());
        let c = cgroup("/a.slice/app.slice/runner.service", read);
        assert_eq!(c.name.as_deref(), Some("runner.service"));
        assert_eq!(c.current, Some(1000));
        assert_eq!(c.high, Some(8000));
        assert_eq!(c.high_by.as_deref(), Some("app.slice"));
        assert_eq!(c.max, Some(10000));
        assert_eq!(c.max_by.as_deref(), Some("app.slice"));
        assert_eq!(c.cap_used, Some(6000));

        let none = cgroup("/a.slice/b.service", |_: &str, _: &str| None);
        assert_eq!((none.high, none.max, none.cap_used), (None, None, None));
        assert_eq!(none.name.as_deref(), Some("b.service"));
    }

    #[test]
    fn linux_battery_minutes_only_while_discharging() {
        let b = linux_battery(
            Some("76\n"),
            Some("Discharging\n"),
            Some("10000000"),
            Some("40000000"),
            false,
        );
        assert_eq!(b.percent, Some(76));
        assert_eq!(b.state, "discharging");
        assert_eq!(b.minutes_left, Some(240));
        assert!(!b.plugged);
        let c = linux_battery(
            Some("90"),
            Some("Charging"),
            Some("10000000"),
            Some("40000000"),
            true,
        );
        assert_eq!(
            (c.state.as_str(), c.minutes_left, c.plugged),
            ("charging", None, true)
        );
        assert_eq!(
            linux_battery(None, None, None, None, false).state,
            "unknown"
        );
    }

    #[test]
    fn pmset_battery() {
        let on_battery = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=12345678)\t81%; discharging; 4:37 remaining present: true\n";
        let b = pmset(on_battery).unwrap();
        assert_eq!(b.percent, Some(81));
        assert_eq!(b.state, "discharging");
        assert_eq!(b.minutes_left, Some(277));
        assert!(!b.plugged);
        let charging = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=12345678)\t95%; charging; 0:20 remaining present: true\n";
        let c = pmset(charging).unwrap();
        assert_eq!(
            (c.percent, c.state.as_str(), c.minutes_left, c.plugged),
            (Some(95), "charging", None, true)
        );
        let full = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=12345678)\t100%; charged; 0:00 remaining present: true\n";
        assert_eq!(pmset(full).unwrap().state, "charged");
        assert!(pmset("Now drawing from 'AC Power'\n").is_none());
    }

    #[test]
    fn nvidia_csv() {
        let gpus = "0, GPU-11111111-2222-3333-4444-555555555555, NVIDIA GeForce RTX 4090, 37, 20480, 24564, 52, 120.35\n1, GPU-66666666-7777-8888-9999-000000000000, NVIDIA RTX A2000, 0, 3, 6138, 35, [N/A]\n";
        let mut list = nvidia_gpus(gpus);
        assert_eq!(list.len(), 2);
        let (uuid, g) = &list[0];
        assert_eq!(uuid, "GPU-11111111-2222-3333-4444-555555555555");
        assert_eq!(g.index, 0);
        assert_eq!(g.name, "NVIDIA GeForce RTX 4090");
        assert_eq!(g.util, Some(0.37));
        assert_eq!(g.mem_used, 20480 * MIB);
        assert_eq!(g.mem_total, 24564 * MIB);
        assert_eq!(g.temp, Some(52.0));
        assert_eq!(g.power, Some(120.35));
        assert_eq!(list[1].1.power, None);

        let apps = nvidia_apps(
            "GPU-11111111-2222-3333-4444-555555555555, 4242, /opt/llama/bin/llama-server, 18000\nGPU-unknown, 77, python3, 512\nbad line\n",
        );
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].pid, 4242);
        assert_eq!(apps[0].name, "llama-server");
        assert_eq!(apps[0].mem, 18000 * MIB);
        let with = apps
            .into_iter()
            .map(|a| {
                let p = GpuProc {
                    pid: a.pid,
                    name: a.name.clone(),
                    mem: a.mem,
                    unit: None,
                    user: None,
                };
                (a, p)
            })
            .collect();
        attach(&mut list, with);
        assert_eq!(
            list[0].1.procs.len(),
            2,
            "an unknown GPU's process goes on the first one"
        );
        assert_eq!(list[1].1.procs.len(), 0);
        assert_eq!(nvidia_gpus("").len(), 0);
    }

    const IOREG: &str = r#"+-o AGXAcceleratorG14X  <class AGXAcceleratorG14X, id 0x100000abc, registered, matched, active, busy 0 (0 ms), retain 42>
    {
      "IOClass" = "AGXAcceleratorG14X"
      "model" = "Apple M2 Pro"
      "gpu-core-count" = 19
      "PerformanceStatistics" = {"In use system memory (driver)"=0,"Alloc system memory"=1234567890,"Tiler Utilization %"=3,"Renderer Utilization %"=5,"Device Utilization %"=7,"In use system memory"=987654321}
      "AGXParameterBufferMaxSize" = 1073741824
    }
"#;

    #[test]
    fn ioreg_apple_gpu() {
        let g = ioreg(IOREG, 32 * 1024 * MIB);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].index, 0);
        assert_eq!(g[0].name, "Apple M2 Pro");
        assert_eq!(g[0].cores, Some(19));
        assert_eq!(g[0].util, Some(0.07));
        assert_eq!(g[0].mem_used, 987_654_321);
        assert_eq!(g[0].mem_total, 32 * 1024 * MIB);
        assert!(g[0].shared);
        let nameless = IOREG.replace("      \"model\" = \"Apple M2 Pro\"\n", "");
        assert_eq!(ioreg(&nameless, 1)[0].name, "AGXAcceleratorG14X");
        assert_eq!(ioreg("+-o Other <class Other>\n{\n}\n", 1).len(), 0);
    }

    const VM_STAT: &str = "Mach Virtual Memory Statistics: (page size of 16384 bytes)\nPages free:                               12000.\nPages active:                            400000.\nPages inactive:                          380000.\nPages speculative:                        8000.\nPages throttled:                              0.\nPages wired down:                        150000.\nPages purgeable:                          20000.\n\"Translation faults\":                 123456789.\n";

    #[test]
    fn mac_memory_from_vm_stat_and_sysctl() {
        assert_eq!(vm_stat(VM_STAT), Some(420_000 * 16_384));
        let s = sysctl(
            "hw.memsize: 17179869184\nkern.memorystatus_vm_pressure_level: 2\nvm.swapusage: total = 2048.00M  used = 1024.50M  free = 1023.50M  (encrypted)\nvm.loadavg: { 2.10 1.90 1.75 }\nhw.logicalcpu: 10\n",
        );
        assert_eq!(s["hw.logicalcpu"], "10");
        assert_eq!(loadavg(&s["vm.loadavg"]), Some([2.1, 1.9, 1.75]));
        let m = mac_memory(VM_STAT, &s).unwrap();
        assert_eq!(m.total, 17_179_869_184);
        assert_eq!(m.available, 420_000 * 16_384);
        assert_eq!(m.pressure, Pressure::Warn);
        assert_eq!(m.swap_total, 2048 * MIB);
        assert_eq!(m.swap_used, 1_074_266_112);
        assert!(m.psi.is_none());
        assert!(mac_memory("", &s).is_none());
        assert_eq!(
            swapusage("total = 0.00M  used = 0.00M  free = 0.00M"),
            (0, 0)
        );
    }

    #[test]
    fn agents_in_ps() {
        let ps = "    1  12000 systemd\n  900 450000 claude\n  901 300000 /Users/example/.local/bin/claude\n  902  80000 claude-helper\n 4242  65000 emberhut-runner\n";
        let a = agents(ps, 4242);
        assert_eq!(a.count, 2);
        assert_eq!(a.rss, 750_000 * 1024);
        assert_eq!(a.own_rss, Some(65_000 * 1024));
        assert_eq!(agents("", 1), Agents::default());
    }

    #[test]
    fn own_process_figures() {
        assert_eq!(
            status_rss("Name:\temberhut-runne\nVmPeak:\t  200000 kB\nVmRSS:\t   51200 kB\n"),
            Some(51_200 * 1024)
        );
        let stat = "4242 (emberhut (x) runner) S 1 4242 4242 0 -1 4194560 1234 0 0 0 150 30 0 0 20 0 8 0 1000 300000000 12000\n";
        assert_eq!(stat_ticks(stat), Some(180));
        assert_eq!(stat_ticks("garbage"), None);
    }
}
