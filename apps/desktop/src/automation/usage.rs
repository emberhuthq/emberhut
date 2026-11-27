//! The process's memory and processor time, as the benchmarks report them. On Linux from
//! `/proc`; on macOS from `proc_pid_rusage`, where "footprint" is what Activity Monitor calls
//! Memory (what the app costs the system, compressed pages included).

/// The memory now, as `key=value` pairs in megabytes.
#[must_use]
pub(crate) fn memory() -> String {
    platform::memory()
}

/// The main thread's processor time so far, in milliseconds (the whole process's on macOS,
/// which doesn't keep it per thread).
#[must_use]
pub(crate) fn main_thread_cpu_ms() -> f64 {
    platform::main_thread_cpu_ms()
}

/// Every thread's processor time so far, in milliseconds.
#[must_use]
pub(crate) fn process_cpu_ms() -> f64 {
    platform::process_cpu_ms()
}

/// `VmRSS` and `VmHWM` from `/proc/self/status`, in kilobytes.
#[cfg_attr(
    not(any(target_os = "linux", test)),
    expect(dead_code, reason = "Linux only")
)]
fn resident_kb(status: &str) -> (u64, u64) {
    let field = |key: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|kb| kb.parse().ok())
            .unwrap_or(0)
    };
    (field("VmRSS:"), field("VmHWM:"))
}

/// User plus system time from `/proc/self/stat`, in clock ticks.
#[cfg_attr(
    not(any(target_os = "linux", test)),
    expect(dead_code, reason = "Linux only")
)]
fn cpu_ticks(stat: &str) -> u64 {
    // The command name, in parentheses, may hold spaces: count the fields after it.
    let fields: Vec<&str> = stat
        .rsplit(')')
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect();
    let field = |at: usize| {
        fields
            .get(at)
            .and_then(|n| n.parse::<u64>().ok())
            .unwrap_or(0)
    };
    field(11) + field(12)
}

#[cfg(target_os = "linux")]
mod platform {
    /// The kernel's clock ticks per second (`USER_HZ`), 100 on every Linux the app runs on.
    const TICKS_PER_SECOND: f64 = 100.;

    pub(super) fn memory() -> String {
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        let (resident, peak) = super::resident_kb(&status);
        format!("rss_mb={} peak_rss_mb={}", resident / 1024, peak / 1024)
    }

    pub(super) fn main_thread_cpu_ms() -> f64 {
        // /proc/self/task/<pid>/schedstat: nanoseconds on the processor, first.
        let path = format!("/proc/self/task/{}/schedstat", std::process::id());
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| text.split_whitespace().next()?.parse::<f64>().ok())
            .map_or(0., |ns| ns / 1e6)
    }

    pub(super) fn process_cpu_ms() -> f64 {
        let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
        #[expect(
            clippy::cast_precision_loss,
            reason = "tick counts stay far below 2^52"
        )]
        let ticks = super::cpu_ticks(&stat) as f64;
        ticks * 1000. / TICKS_PER_SECOND
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::mem::MaybeUninit;

    fn rusage() -> Option<libc::rusage_info_v4> {
        let pid = libc::c_int::try_from(std::process::id()).ok()?;
        let mut info = MaybeUninit::<libc::rusage_info_v4>::zeroed();
        #[expect(
            unsafe_code,
            reason = "proc_pid_rusage is the only way to read the footprint"
        )]
        // SAFETY: for RUSAGE_INFO_V4, proc_pid_rusage writes one rusage_info_v4 into the buffer,
        // which is exactly that big; the struct is plain integers, so zeroed is a valid value too.
        let info = unsafe {
            let status = libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V4, info.as_mut_ptr().cast());
            (status == 0).then(|| info.assume_init())
        };
        info
    }

    pub(super) fn memory() -> String {
        let mb = |bytes: u64| bytes / (1 << 20);
        match rusage() {
            Some(info) => format!(
                "rss_mb={} footprint_mb={} peak_footprint_mb={}",
                mb(info.ri_resident_size),
                mb(info.ri_phys_footprint),
                mb(info.ri_lifetime_max_phys_footprint)
            ),
            None => "memory=unknown".to_owned(),
        }
    }

    pub(super) fn main_thread_cpu_ms() -> f64 {
        process_cpu_ms()
    }

    pub(super) fn process_cpu_ms() -> f64 {
        let mut usage = MaybeUninit::<libc::rusage>::zeroed();
        #[expect(
            unsafe_code,
            reason = "getrusage is how a process reads its own processor time"
        )]
        // SAFETY: getrusage fills the rusage it is given; zeroed is a valid rusage (integers).
        let usage = unsafe {
            (libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) == 0)
                .then(|| usage.assume_init())
        };
        let Some(usage) = usage else { return 0. };
        let ms = |time: libc::timeval| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "seconds of processor time stay far below 2^52"
            )]
            let seconds = time.tv_sec as f64;
            seconds * 1000. + f64::from(time.tv_usec) / 1000.
        };
        ms(usage.ru_utime) + ms(usage.ru_stime)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod platform {
    pub(super) fn memory() -> String {
        "memory=unknown".to_owned()
    }

    pub(super) fn main_thread_cpu_ms() -> f64 {
        0.
    }

    pub(super) fn process_cpu_ms() -> f64 {
        0.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resident_memory_is_read_from_proc_status() {
        let status = "Name:\temberhut\nVmHWM:\t  204800 kB\nVmRSS:\t  102400 kB\n";
        assert_eq!(resident_kb(status), (102_400, 204_800));
        assert_eq!(resident_kb(""), (0, 0));
    }

    #[test]
    fn processor_ticks_are_read_after_the_command_name() {
        let stat = "42 (emberhut (desktop)) S 1 2 3 4 5 6 7 8 9 10 250 50 0 0";
        assert_eq!(cpu_ticks(stat), 300);
    }

    #[test]
    fn the_numbers_are_read_on_this_system() {
        let memory = memory();
        if cfg!(target_os = "macos") {
            assert!(memory.contains("footprint_mb="), "{memory}");
        } else if cfg!(target_os = "linux") {
            assert!(
                memory.contains("rss_mb=") && !memory.contains("rss_mb=0 "),
                "{memory}"
            );
        }
        // Spend some processor time, so a reader that always says 0 fails.
        let mut x: u64 = 1;
        for i in 0..20_000_000_u64 {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(i);
        }
        assert_ne!(x, 0);
        if cfg!(any(target_os = "linux", target_os = "macos")) {
            assert!(process_cpu_ms() > 0., "no processor time read");
            assert!(main_thread_cpu_ms() > 0., "no main-thread time read");
        }
    }
}
