//! Other systems: what the standard library can tell (the cores), and nothing it would have to
//! guess.

use super::parse::CpuTimes;
use super::{Cgroup, Disk, Fast, Slow};

/// The CPU's times: not read here.
pub(super) fn cpu_times() -> Option<CpuTimes> {
    None
}

/// The runner's CPU time: not read here.
pub(super) fn cpu_ticks() -> Option<u64> {
    None
}

/// The runner's resident memory: not read here.
pub(super) fn own_rss() -> Option<u64> {
    None
}

/// The disks: not read here.
pub(super) fn disks() -> Vec<Disk> {
    Vec::new()
}

/// No cgroups here.
pub(super) fn cgroup() -> Option<Cgroup> {
    None
}

/// Nothing beyond the cores, which the sampler counts itself.
#[expect(
    clippy::unused_async,
    reason = "Linux and macOS wait on programs for the same"
)]
pub(super) async fn fast() -> Fast {
    Fast::default()
}

/// Nothing slow to read here.
#[expect(
    clippy::unused_async,
    reason = "Linux and macOS wait on programs for the same"
)]
pub(super) async fn slow() -> Slow {
    Slow::default()
}
