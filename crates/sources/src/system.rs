//! CPU and temperature readers: tiny kernel files, no processes, no allocation beyond one string.
use sq_core::RamSample;
use std::fs;
use std::path::{Path, PathBuf};

/// Cumulative (busy, total) jiffies from the aggregate `cpu` line of `/proc/stat`.
pub fn parse_cpu_stat(text: &str) -> Option<(u64, u64)> {
    let line = text.lines().next()?;
    let mut fields = line.split_ascii_whitespace();
    if fields.next()? != "cpu" {
        return None;
    }
    let values: Vec<u64> = fields
        .take(8)
        .map(|f| f.parse().ok())
        .collect::<Option<_>>()?;
    if values.len() < 4 {
        return None;
    }
    // user nice system idle iowait irq softirq steal
    let idle = values[3].checked_add(*values.get(4).unwrap_or(&0))?;
    let total = values.iter().try_fold(0u64, |a, v| a.checked_add(*v))?;
    Some((total.checked_sub(idle)?, total))
}
/// Busy share between two readings; a counter that went backwards or did not move is unknown.
pub fn cpu_percent(previous: (u64, u64), current: (u64, u64)) -> Option<f32> {
    let total = current.1.checked_sub(previous.1).filter(|t| *t > 0)?;
    let busy = current.0.checked_sub(previous.0)?;
    Some((busy as f64 / total as f64 * 100.0).clamp(0.0, 100.0) as f32)
}
/// Millidegrees from a hwmon/thermal file to Celsius; implausible values are unknown.
pub fn parse_millidegrees(text: &str) -> Option<f32> {
    let celsius = text.trim().parse::<i64>().ok()? as f32 / 1000.0;
    (-30.0..=150.0).contains(&celsius).then_some(celsius)
}
/// Finds the CPU package sensor once: coretemp/k10temp/zenpower hwmon, else the x86 thermal zone.
pub fn find_cpu_sensor(sys: &Path) -> Option<PathBuf> {
    let hwmon = fs::read_dir(sys.join("class/hwmon")).ok()?;
    for entry in hwmon.flatten() {
        let name = fs::read_to_string(entry.path().join("name")).unwrap_or_default();
        if matches!(name.trim(), "coretemp" | "k10temp" | "zenpower") {
            let input = entry.path().join("temp1_input");
            if input.exists() {
                return Some(input);
            }
        }
    }
    for entry in fs::read_dir(sys.join("class/thermal")).ok()?.flatten() {
        let kind = fs::read_to_string(entry.path().join("type")).unwrap_or_default();
        if kind.trim() == "x86_pkg_temp" {
            return Some(entry.path().join("temp"));
        }
    }
    None
}
pub fn read_cpu_stat(proc: &Path) -> Option<(u64, u64)> {
    parse_cpu_stat(&fs::read_to_string(proc.join("stat")).ok()?)
}
pub fn read_temperature(sensor: &Path) -> Option<f32> {
    parse_millidegrees(&fs::read_to_string(sensor).ok()?)
}
/// Used/total memory and swap in MiB from `/proc/meminfo` (kB lines); used = total - available.
pub fn parse_meminfo(text: &str) -> Option<RamSample> {
    let (mut total, mut available, mut swap_total, mut swap_free) = (None, None, None, None);
    for line in text.lines() {
        let mut parts = line.split_ascii_whitespace();
        let (Some(key), Some(value)) = (parts.next(), parts.next()) else {
            continue;
        };
        let value = value.parse::<u64>().ok();
        match key {
            "MemTotal:" => total = value,
            "MemAvailable:" => available = value,
            "SwapTotal:" => swap_total = value,
            "SwapFree:" => swap_free = value,
            _ => {}
        }
    }
    let (total, available) = (total?, available?.min(total?));
    let swap_total = swap_total.unwrap_or(0);
    Some(RamSample {
        used: (total - available) / 1024,
        total: total / 1024,
        swap_used: swap_total.saturating_sub(swap_free.unwrap_or(swap_total)) / 1024,
        swap_total: swap_total / 1024,
    })
}
pub fn read_meminfo(proc: &Path) -> Option<RamSample> {
    parse_meminfo(&fs::read_to_string(proc.join("meminfo")).ok()?)
}
/// Cumulative (busy, total) CPU time of this machine, whatever the platform. Linux reads
/// `/proc/stat`; Windows asks `GetSystemTimes` (100-nanosecond units, kernel time includes idle).
pub fn cpu_counters() -> Option<(u64, u64)> {
    #[cfg(windows)]
    {
        windows::cpu_counters()
    }
    #[cfg(not(windows))]
    {
        read_cpu_stat(Path::new("/proc"))
    }
}
/// Memory and swap of this machine, whatever the platform.
pub fn memory_sample() -> Option<RamSample> {
    #[cfg(windows)]
    {
        windows::memory_sample()
    }
    #[cfg(not(windows))]
    {
        read_meminfo(Path::new("/proc"))
    }
}
/// Package temperature where the platform exposes one without extra drivers or administrator
/// rights: Linux hwmon. Windows offers none, so it is unknown there (never invented).
pub fn find_temperature_sensor() -> Option<PathBuf> {
    if cfg!(windows) {
        None
    } else {
        find_cpu_sensor(Path::new("/sys"))
    }
}
/// Values of a Windows `MEMORYSTATUSEX` turned into MiB. Pure, so it is tested everywhere.
/// `page_file_*` includes physical memory, so swap is what lies beyond it.
pub fn windows_memory(
    total_phys: u64,
    avail_phys: u64,
    total_page_file: u64,
    avail_page_file: u64,
) -> Option<RamSample> {
    if total_phys == 0 {
        return None;
    }
    let mib = 1024 * 1024;
    let avail_phys = avail_phys.min(total_phys);
    let swap_total = total_page_file.saturating_sub(total_phys);
    let swap_free = avail_page_file.saturating_sub(avail_phys).min(swap_total);
    Some(RamSample {
        used: (total_phys - avail_phys) / mib,
        total: total_phys / mib,
        swap_used: (swap_total - swap_free) / mib,
        swap_total: swap_total / mib,
    })
}
#[cfg(windows)]
mod windows {
    use super::*;
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    impl FileTime {
        fn value(self) -> u64 {
            (u64::from(self.high) << 32) | u64::from(self.low)
        }
    }
    #[repr(C)]
    struct MemoryStatus {
        length: u32,
        load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetSystemTimes(idle: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;
        fn GlobalMemoryStatusEx(status: *mut MemoryStatus) -> i32;
    }
    pub fn cpu_counters() -> Option<(u64, u64)> {
        let (mut idle, mut kernel, mut user) = (
            FileTime::default(),
            FileTime::default(),
            FileTime::default(),
        );
        // SAFETY: three valid, writable FILETIME structures.
        if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
            return None;
        }
        let total = kernel.value().checked_add(user.value())?;
        Some((total.checked_sub(idle.value())?, total))
    }
    pub fn memory_sample() -> Option<RamSample> {
        let mut status = MemoryStatus {
            length: std::mem::size_of::<MemoryStatus>() as u32,
            load: 0,
            total_phys: 0,
            avail_phys: 0,
            total_page_file: 0,
            avail_page_file: 0,
            total_virtual: 0,
            avail_virtual: 0,
            avail_extended_virtual: 0,
        };
        // SAFETY: `status` is a valid MEMORYSTATUSEX with its length field set.
        if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
            return None;
        }
        windows_memory(
            status.total_phys,
            status.avail_phys,
            status.total_page_file,
            status.avail_page_file,
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    // REQ SQ-36: Windows memory counters become MiB with swap counted beyond physical memory.
    #[test]
    fn windows_memory_counters_become_used_and_swap() {
        let gib = 1024u64 * 1024 * 1024;
        let sample = windows_memory(16 * gib, 4 * gib, 20 * gib, 8 * gib).unwrap();
        assert_eq!((sample.used, sample.total), (12 * 1024, 16 * 1024));
        // Page file 20 GiB = 16 physical + 4 swap; 8 free in total, 4 of it physical -> all 4 swap free.
        assert_eq!((sample.swap_total, sample.swap_used), (4 * 1024, 0));
        let idle = windows_memory(8 * gib, 8 * gib, 8 * gib, 8 * gib).unwrap();
        assert_eq!((idle.used, idle.swap_total, idle.swap_used), (0, 0, 0));
        assert!(windows_memory(0, 0, 0, 0).is_none());
        // Nonsense (available above total) cannot make "used" wrap around.
        assert_eq!(windows_memory(gib, 2 * gib, gib, 2 * gib).unwrap().used, 0);
    }
    // REQ SQ-36: the portable readers answer on this machine without panicking.
    #[test]
    fn portable_readers_answer_on_this_machine() {
        let cpu = cpu_counters();
        assert!(cpu.is_some_and(|(busy, total)| busy <= total && total > 0));
        let ram = memory_sample().unwrap();
        assert!(ram.total > 0 && ram.used <= ram.total);
    }
    #[test] // SQ-30: counters become a share, regressions and stalls are unknown.
    fn cpu_share_from_counters() {
        let a = parse_cpu_stat("cpu  100 0 100 700 100 0 0 0 0 0\ncpu0 1 2 3 4").unwrap();
        let b = parse_cpu_stat("cpu  150 0 150 750 150 0 0 0 0 0").unwrap();
        assert_eq!(cpu_percent(a, b), Some(50.0));
        assert_eq!(cpu_percent(b, a), None);
        assert_eq!(cpu_percent(a, a), None);
        assert_eq!(parse_cpu_stat("intr 1 2 3"), None);
        assert_eq!(parse_cpu_stat("cpu  1 x 3 4"), None);
    }
    #[test] // SQ-30: implausible sensor values are unavailable, never invented.
    fn temperature_range_is_checked() {
        assert_eq!(parse_millidegrees("45500\n"), Some(45.5));
        assert_eq!(parse_millidegrees("-274000"), None);
        assert_eq!(parse_millidegrees("junk"), None);
        assert_eq!(parse_millidegrees("999999999"), None);
    }
    #[test] // SQ-32: memory is total minus available, in MiB; swap likewise.
    fn meminfo_used_is_total_minus_available() {
        let text = "MemTotal:       24300484 kB\nMemFree: 1 kB\nMemAvailable:   10597884 kB\nSwapTotal:      31457276 kB\nSwapFree:       24389764 kB\n";
        let ram = parse_meminfo(text).unwrap();
        assert_eq!((ram.used, ram.total), (13_381, 23_730));
        assert_eq!((ram.swap_used, ram.swap_total), (6_901, 30_719));
        assert_eq!(parse_meminfo("MemTotal: 5 kB"), None);
    }
}
