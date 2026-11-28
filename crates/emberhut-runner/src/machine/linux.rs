//! Linux: `/proc` and `/sys` files, `statfs`, and `nvidia-smi` and `ps` once a minute.

use std::os::unix::fs::MetadataExt;
use std::path::Path;

use super::parse::{self, ComputeApp, CpuTimes};
use super::{Battery, Cgroup, Disk, Fast, Gpu, GpuProc, Slow};

/// Where `nvidia-smi` is installed, looked for in this order.
const NVIDIA_SMI: [&str; 2] = ["/usr/bin/nvidia-smi", "/usr/local/bin/nvidia-smi"];
/// Linux's power supplies.
const POWER_SUPPLY: &str = "/sys/class/power_supply";

/// A file's text, if it can be read.
fn read(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// The CPU's busy and total times now.
pub(super) fn cpu_times() -> Option<CpuTimes> {
    parse::proc_stat(&read("/proc/stat")?).map(|(t, _)| t)
}

/// The runner's CPU time so far, in clock ticks.
pub(super) fn cpu_ticks() -> Option<u64> {
    parse::stat_ticks(&read("/proc/self/stat")?)
}

/// The runner's resident memory now.
pub(super) fn own_rss() -> Option<u64> {
    parse::status_rss(&read("/proc/self/status")?)
}

/// `/` and `/home`, one disk counted once.
pub(super) fn disks() -> Vec<Disk> {
    super::disks_at(&["/", "/home"])
}

/// The memory cap over the runner's cgroup (cgroup v2).
pub(super) fn cgroup() -> Option<Cgroup> {
    let text = read("/proc/self/cgroup")?;
    let path = parse::cgroup_path(&text)?;
    Some(parse::cgroup(path, |at, file| {
        read(format!("/sys/fs/cgroup{at}/{file}")).map(|s| s.trim().to_owned())
    }))
}

/// Cores, load and memory.
#[expect(
    clippy::unused_async,
    reason = "macOS runs programs, which are waited on, for the same"
)]
pub(super) async fn fast() -> Fast {
    let memory = read("/proc/meminfo").and_then(|m| {
        let psi = read("/proc/pressure/memory");
        parse::linux_memory(&m, psi.as_deref())
    });
    Fast {
        cores: read("/proc/stat")
            .and_then(|s| parse::proc_stat(&s))
            .map(|(_, cores)| cores)
            .filter(|c| *c > 0),
        load: read("/proc/loadavg").and_then(|l| parse::loadavg(&l)),
        memory,
        gpus: None,
    }
}

/// NVIDIA GPUs, the agent's processes and the battery.
pub(super) async fn slow() -> Slow {
    let ps = super::text_of("ps", &["-eo", "pid=,rss=,comm="]);
    let (gpus, ps) = tokio::join!(nvidia(), ps);
    Slow {
        gpus,
        agents: ps.map(|out| parse::agents(&out, std::process::id())),
        battery: battery_in(Path::new(POWER_SUPPLY)),
    }
}

/// NVIDIA GPUs and the processes holding their memory, each with its systemd unit and user.
async fn nvidia() -> Vec<Gpu> {
    let Some(smi) = NVIDIA_SMI.into_iter().find(|p| Path::new(p).exists()) else {
        return Vec::new();
    };
    let (gpus, apps) = tokio::join!(
        super::text_of(
            smi,
            &[
                "--query-gpu=index,uuid,name,utilization.gpu,memory.used,memory.total,temperature.gpu,power.draw",
                "--format=csv,noheader,nounits",
            ],
        ),
        super::text_of(
            smi,
            &[
                "--query-compute-apps=gpu_uuid,pid,process_name,used_memory",
                "--format=csv,noheader,nounits",
            ],
        ),
    );
    let Some(gpus) = gpus else {
        return Vec::new();
    };
    let mut list = parse::nvidia_gpus(&gpus);
    let apps = parse::nvidia_apps(apps.as_deref().unwrap_or(""))
        .into_iter()
        .map(|app| {
            let proc = gpu_proc(&app);
            (app, proc)
        })
        .collect();
    parse::attach(&mut list, apps);
    list.into_iter().map(|(_, gpu)| gpu).collect()
}

/// A GPU's process with its unit and its user.
fn gpu_proc(app: &ComputeApp) -> GpuProc {
    let unit = read(format!("/proc/{}/cgroup", app.pid))
        .as_deref()
        .and_then(parse::unit)
        .map(str::to_owned);
    let own = std::fs::metadata("/proc/self").map(|m| m.uid()).ok();
    let user = std::fs::metadata(format!("/proc/{}", app.pid))
        .ok()
        .map(|m| m.uid())
        .map(|uid| match std::env::var("USER") {
            Ok(name) if Some(uid) == own && !name.is_empty() => name,
            _ => uid.to_string(),
        });
    GpuProc {
        pid: app.pid,
        name: app.name.clone(),
        mem: app.mem,
        unit,
        user,
    }
}

/// The first battery under `base` (Linux's `power_supply` class) and whether a charger is
/// online; `None` without a battery.
fn battery_in(base: &Path) -> Option<Battery> {
    let mut names: Vec<String> = std::fs::read_dir(base)
        .ok()?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .collect();
    names.sort();
    let file = |name: &str, f: &str| read(base.join(name).join(f)).map(|s| s.trim().to_owned());
    let kind = |name: &str| file(name, "type");
    // A mouse's or a keyboard's battery says its scope is a device; the machine's says none.
    let bat = names.iter().find(|n| {
        kind(n).as_deref() == Some("Battery") && file(n, "scope").as_deref() != Some("Device")
    })?;
    let plugged = names
        .iter()
        .any(|n| kind(n).as_deref() == Some("Mains") && file(n, "online").as_deref() == Some("1"));
    Some(parse::linux_battery(
        file(bat, "capacity").as_deref(),
        file(bat, "status").as_deref(),
        file(bat, "power_now").as_deref(),
        file(bat, "energy_now").as_deref(),
        plugged,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, files: &[(&str, &str)]) {
        let at = dir.join(name);
        std::fs::create_dir_all(&at).unwrap();
        for (f, text) in files {
            std::fs::write(at.join(f), text).unwrap();
        }
    }

    #[test]
    fn a_laptops_power_supplies() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "AC", &[("type", "Mains\n"), ("online", "1\n")]);
        write(
            dir.path(),
            "BAT0",
            &[
                ("type", "Battery\n"),
                ("capacity", "64\n"),
                ("status", "Charging\n"),
                ("power_now", "15000000\n"),
                ("energy_now", "30000000\n"),
            ],
        );
        write(
            dir.path(),
            "0-mouse-battery",
            &[
                ("type", "Battery\n"),
                ("scope", "Device\n"),
                ("capacity", "10\n"),
            ],
        );
        let b = battery_in(dir.path()).unwrap();
        assert_eq!(b.percent, Some(64));
        assert!(b.plugged);
        assert_eq!(b.state, "charging");
        assert_eq!(b.minutes_left, None);
    }

    #[test]
    fn a_desktop_has_no_battery() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "AC", &[("type", "Mains\n"), ("online", "1\n")]);
        assert!(battery_in(dir.path()).is_none());
        assert!(battery_in(&dir.path().join("missing")).is_none());
    }

    #[test]
    fn this_process_is_read() {
        assert!(cpu_times().is_some());
        assert!(cpu_ticks().is_some());
        assert!(own_rss().is_some_and(|r| r > 0));
    }
}
