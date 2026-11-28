//! macOS: `vm_stat`, `sysctl` and `ioreg` in every sample (each a few milliseconds), `ps` and
//! `pmset` once a minute, `statfs` for the disk. No memory cap: launchd keeps none.

use super::parse::{self, CpuTimes};
use super::{Cgroup, Disk, Fast, Slow};

/// What `sysctl` is asked for in every sample.
const SYSCTL: [&str; 5] = [
    "hw.memsize",
    "hw.logicalcpu",
    "kern.memorystatus_vm_pressure_level",
    "vm.swapusage",
    "vm.loadavg",
];

/// The CPU's times: not read on macOS (they need a Mach call), so the busy share is `None`.
pub(super) fn cpu_times() -> Option<CpuTimes> {
    None
}

/// The runner's CPU time: not read on macOS.
pub(super) fn cpu_ticks() -> Option<u64> {
    None
}

/// The runner's resident memory comes from the slow part's `ps`.
pub(super) fn own_rss() -> Option<u64> {
    None
}

/// `/`: its APFS container's size and free space.
pub(super) fn disks() -> Vec<Disk> {
    super::disks_at(&["/"])
}

/// No cgroups on macOS.
pub(super) fn cgroup() -> Option<Cgroup> {
    None
}

/// Cores, load, memory and the Apple GPU.
pub(super) async fn fast() -> Fast {
    let vm = super::text_of("/usr/bin/vm_stat", &[]);
    // sysctl prints every name it knows and fails for the rest: what it printed still counts.
    let sys = super::run("/usr/sbin/sysctl", &SYSCTL);
    let gpu = super::text_of(
        "/usr/sbin/ioreg",
        &["-r", "-d", "1", "-w", "0", "-c", "IOAccelerator"],
    );
    let (vm, sys, gpu) = tokio::join!(vm, sys, gpu);
    let sys = sys.map_or_default(|o| parse::sysctl(&String::from_utf8_lossy(&o.stdout)));
    let memory = vm.and_then(|vm| parse::mac_memory(&vm, &sys));
    let total = memory.as_ref().map_or(0, |m| m.total);
    Fast {
        cores: sys.get("hw.logicalcpu").and_then(|c| c.parse().ok()),
        load: sys.get("vm.loadavg").and_then(|l| parse::loadavg(l)),
        memory,
        gpus: Some(gpu.map_or_default(|g| parse::ioreg(&g, total))),
    }
}

/// The agent's processes (and the runner's own memory) and the battery.
pub(super) async fn slow() -> Slow {
    let ps = super::text_of("/bin/ps", &["-axo", "pid=,rss=,comm="]);
    let pmset = super::text_of("/usr/bin/pmset", &["-g", "batt"]);
    let (ps, pmset) = tokio::join!(ps, pmset);
    Slow {
        gpus: Vec::new(),
        agents: ps.map(|out| parse::agents(&out, std::process::id())),
        battery: pmset.as_deref().and_then(parse::pmset),
    }
}
