//! GPU readers for any vendor: sysfs files (AMD, Intel, others) and one nvidia-smi line (NVIDIA).
//! Pure parsers are separate from the file readers so they are tested without hardware.
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
    Other,
}
#[derive(Clone, Debug)]
pub struct GpuDevice {
    pub id: String,
    pub name: String,
    pub vendor: Vendor,
    /// `/sys/class/drm/cardN/device`
    pub device: PathBuf,
    /// `/sys/class/drm/cardN`
    pub card: PathBuf,
    /// PCI address such as `0000:01:00.0`.
    pub slot: String,
    /// Not the boot display adapter, so it may power itself down and must not be woken to measure.
    pub discrete: bool,
    /// Windows: the adapter's LUID as the performance counters write it (`0x00000000_0x0000a1b2`).
    pub luid: Option<String>,
    /// Windows: dedicated video memory in bytes, from the DirectX registry entry.
    pub vram_bytes: Option<u64>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GpuReading {
    pub util: Option<f32>,
    pub vram_used: Option<f32>,
    pub vram_total: Option<f32>,
    pub temp: Option<f32>,
    pub power: Option<f32>,
    pub clock: Option<f32>,
    pub awake_share: bool,
}
pub fn vendor_of(id: &str) -> Vendor {
    match id.trim() {
        "0x10de" => Vendor::Nvidia,
        "0x1002" => Vendor::Amd,
        "0x8086" => Vendor::Intel,
        _ => Vendor::Other,
    }
}
/// Every display adapter the kernel exposes (`cardN`, not the connector entries `cardN-DP-1`).
pub fn find_gpus(sys: &Path) -> Vec<GpuDevice> {
    let Ok(entries) = fs::read_dir(sys.join("class/drm")) else {
        return vec![];
    };
    let mut found: Vec<GpuDevice> = entries
        .flatten()
        .filter_map(|entry| {
            let id = entry.file_name().to_string_lossy().to_string();
            let number = id.strip_prefix("card")?;
            if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let card = entry.path();
            let device = card.join("device");
            let vendor = vendor_of(&fs::read_to_string(device.join("vendor")).ok()?);
            let driver = fs::read_link(device.join("driver"))
                .ok()
                .and_then(|d| d.file_name().map(|n| n.to_string_lossy().to_string()))
                .unwrap_or_else(|| "unknown".into());
            let slot = fs::canonicalize(&device)
                .ok()
                .and_then(|d| d.file_name().map(|n| n.to_string_lossy().to_string()))
                .unwrap_or_default();
            let brand = match vendor {
                Vendor::Nvidia => "NVIDIA",
                Vendor::Amd => "AMD",
                Vendor::Intel => "Intel",
                Vendor::Other => "GPU",
            };
            let boot = fs::read_to_string(device.join("boot_vga")).is_ok_and(|b| b.trim() == "1");
            Some(GpuDevice {
                name: format!("{brand} {driver} ({id})"),
                id,
                vendor,
                device,
                card,
                slot,
                discrete: !boot,
                luid: None,
                vram_bytes: None,
            })
        })
        .collect();
    found.sort_by(|a, b| a.id.cmp(&b.id));
    found
}
/// `nvidia-smi --query-gpu=index,name,pci.bus_id --format=csv,noheader` lines as devices. This is
/// how GPUs are found where there is no `/sys/class/drm` (Windows). Such devices have no sysfs
/// folders (empty paths) and are read through `nvidia-smi` only.
pub fn parse_nvidia_list(text: &str) -> Vec<GpuDevice> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split(',').map(str::trim);
            let index = fields.next()?;
            let name = fields.next()?;
            let bus = fields.next()?;
            if index.is_empty() || !index.bytes().all(|b| b.is_ascii_digit()) || name.is_empty() {
                return None;
            }
            // Same form as the sysfs PCI address: 00000000:01:00.0 -> 0000:01:00.0.
            let slot = bus
                .get(bus.len().saturating_sub(12)..)
                .unwrap_or(bus)
                .to_string();
            Some(GpuDevice {
                id: format!("nvidia{index}"),
                name: name.to_string(),
                vendor: Vendor::Nvidia,
                device: PathBuf::new(),
                card: PathBuf::new(),
                slot,
                discrete: false,
                luid: None,
                vram_bytes: None,
            })
        })
        .collect()
}
/// A display adapter as DirectX lists it in the registry (Windows).
#[derive(Clone, Debug, PartialEq)]
pub struct WindowsAdapter {
    pub name: String,
    pub vendor_id: u32,
    pub vram_bytes: u64,
    pub luid: u64,
}
fn hex_value(text: &str) -> Option<u64> {
    u64::from_str_radix(
        text.trim()
            .trim_start_matches("0x")
            .trim_start_matches("0X"),
        16,
    )
    .ok()
}
/// `reg query HKLM\SOFTWARE\Microsoft\DirectX /s` output as adapters. Every GPU the system has
/// ever shown a DirectX program is there with its name, vendor, video memory and LUID.
pub fn parse_directx_registry(text: &str) -> Vec<WindowsAdapter> {
    let mut found = Vec::new();
    let mut current: Option<(Option<String>, u32, u64, Option<u64>)> = None;
    let finish = |entry: Option<(Option<String>, u32, u64, Option<u64>)>,
                  found: &mut Vec<WindowsAdapter>| {
        if let Some((Some(name), vendor_id, vram_bytes, Some(luid))) = entry {
            if !found.iter().any(|a: &WindowsAdapter| a.luid == luid) {
                found.push(WindowsAdapter {
                    name,
                    vendor_id,
                    vram_bytes,
                    luid,
                });
            }
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("HKEY_") {
            finish(current.take(), &mut found);
            current = Some((None, 0, 0, None));
            continue;
        }
        let Some(entry) = current.as_mut() else {
            continue;
        };
        let mut words = line.split_whitespace();
        let (Some(key), Some(kind)) = (words.next(), words.next()) else {
            continue;
        };
        let value = line[key.len()..].trim_start()[kind.len()..].trim();
        match (key, kind) {
            ("Description", "REG_SZ") if !value.is_empty() => entry.0 = Some(value.to_string()),
            ("VendorId", "REG_DWORD") => entry.1 = hex_value(value).unwrap_or(0) as u32,
            ("DedicatedVideoMemory", "REG_QWORD") => entry.2 = hex_value(value).unwrap_or(0),
            ("AdapterLuid", "REG_QWORD") => entry.3 = hex_value(value),
            _ => {}
        }
    }
    finish(current.take(), &mut found);
    found
}
/// The registry keeps adapters of earlier boots (their LUIDs change on every start). When the
/// counters report which adapters exist now, only those stay; with no report, all stay.
pub fn keep_live_adapters(
    adapters: Vec<WindowsAdapter>,
    live: &std::collections::BTreeSet<String>,
) -> Vec<WindowsAdapter> {
    if live.is_empty() {
        return adapters;
    }
    let kept: Vec<_> = adapters
        .iter()
        .filter(|a| live.contains(&luid_key(a.luid)))
        .cloned()
        .collect();
    if kept.is_empty() {
        adapters
    } else {
        kept
    }
}

/// The LUID as the GPU performance counters spell it: high and low halves, lower case.
pub fn luid_key(luid: u64) -> String {
    format!("0x{:08x}_0x{:08x}", luid >> 32, luid & 0xffff_ffff)
}
/// Devices for Windows adapters. An NVIDIA adapter whose name matches an `nvidia-smi` entry is read
/// through it (temperature and power too); every other adapter, of any vendor, through the counters.
pub fn windows_devices(adapters: &[WindowsAdapter], nvidia: &[GpuDevice]) -> Vec<GpuDevice> {
    adapters
        .iter()
        .map(|a| {
            let vendor = match a.vendor_id {
                0x10de => Vendor::Nvidia,
                0x1002 => Vendor::Amd,
                0x8086 => Vendor::Intel,
                _ => Vendor::Other,
            };
            let slot = nvidia
                .iter()
                .find(|n| vendor == Vendor::Nvidia && n.name == a.name)
                .map(|n| n.slot.clone())
                .unwrap_or_default();
            GpuDevice {
                id: format!("gpu-{}", luid_key(a.luid)),
                name: a.name.clone(),
                vendor,
                device: PathBuf::new(),
                card: PathBuf::new(),
                slot,
                discrete: false,
                luid: Some(luid_key(a.luid)),
                vram_bytes: (a.vram_bytes > 0).then_some(a.vram_bytes),
            }
        })
        .collect()
}
/// Utilization and memory per adapter from GPU performance counter instances, as Task Manager
/// does it: an adapter's load is its busiest engine type (3D, copy, video decode ...), where the
/// load of one engine type is the sum over all processes. Instances look like
/// `pid_4242_luid_0x00000000_0x0000a1b2_phys_0_eng_0_engtype_3D` and
/// `luid_0x00000000_0x0000a1b2_phys_0`; `engines` are percentages, `memory` bytes.
pub fn aggregate_gpu_counters(
    engines: &[(String, f64)],
    memory: &[(String, f64)],
) -> std::collections::BTreeMap<String, GpuReading> {
    use std::collections::BTreeMap;
    fn luid_of(instance: &str) -> Option<String> {
        let rest = &instance[instance.find("luid_0x")? + 5..];
        let mut parts = rest.splitn(3, '_');
        let (high, low) = (parts.next()?, parts.next()?);
        (high.starts_with("0x") && low.starts_with("0x")).then(|| format!("{high}_{low}"))
    }
    let mut load: BTreeMap<(String, String), f64> = BTreeMap::new();
    for (name, value) in engines {
        let name = name.to_ascii_lowercase();
        let (Some(luid), Some(kind)) = (
            luid_of(&name),
            name.rsplit_once("engtype_").map(|(_, k)| k.to_string()),
        ) else {
            continue;
        };
        if value.is_finite() {
            *load.entry((luid, kind)).or_default() += value.max(0.0);
        }
    }
    let mut result: BTreeMap<String, GpuReading> = BTreeMap::new();
    for ((luid, _), value) in load {
        let entry = result.entry(luid).or_default();
        let value = value.min(100.0) as f32;
        entry.util = Some(entry.util.map_or(value, |u| u.max(value)));
    }
    for (name, bytes) in memory {
        let name = name.to_ascii_lowercase();
        if let (Some(luid), true) = (luid_of(&name), bytes.is_finite()) {
            let entry = result.entry(luid).or_default();
            entry.vram_used = Some((bytes.max(0.0) / (1024.0 * 1024.0)) as f32);
        }
    }
    result
}
/// Windows GPU performance counters through the Performance Data Helper, which re-reads the list
/// of processes on every collection. One reader serves every adapter, of any vendor.
#[cfg(windows)]
pub struct WindowsGpuCounters {
    query: isize,
    engine: isize,
    memory: isize,
    primed: bool,
}
#[cfg(windows)]
mod pdh {
    #[link(name = "pdh")]
    extern "system" {
        pub fn PdhOpenQueryW(source: *const u16, user: usize, query: *mut isize) -> u32;
        pub fn PdhAddEnglishCounterW(
            query: isize,
            path: *const u16,
            user: usize,
            counter: *mut isize,
        ) -> u32;
        pub fn PdhCollectQueryData(query: isize) -> u32;
        pub fn PdhGetFormattedCounterArrayW(
            counter: isize,
            format: u32,
            size: *mut u32,
            count: *mut u32,
            items: *mut u8,
        ) -> u32;
        pub fn PdhCloseQuery(query: isize) -> u32;
    }
    pub const FORMAT_DOUBLE: u32 = 0x0000_0200;
    pub const MORE_DATA: u32 = 0x8000_07D2;
}
#[cfg(windows)]
impl WindowsGpuCounters {
    pub fn open() -> Option<Self> {
        let wide = |text: &str| -> Vec<u16> { text.encode_utf16().chain([0]).collect() };
        let mut query = 0isize;
        // SAFETY: valid out-pointers; the null data source means the local machine.
        if unsafe { pdh::PdhOpenQueryW(std::ptr::null(), 0, &mut query) } != 0 {
            return None;
        }
        let (mut engine, mut memory) = (0isize, 0isize);
        // A machine without GPU counters (before Windows 10 1709) simply has no loads to show.
        let ok = |path: &str, handle: &mut isize| {
            let path = wide(path);
            // SAFETY: `path` is NUL-terminated and outlives the call.
            unsafe { pdh::PdhAddEnglishCounterW(query, path.as_ptr(), 0, handle) == 0 }
        };
        let has_engine = ok("\\GPU Engine(*)\\Utilization Percentage", &mut engine);
        let has_memory = ok("\\GPU Adapter Memory(*)\\Dedicated Usage", &mut memory);
        if !has_engine && !has_memory {
            // SAFETY: `query` was opened above.
            unsafe { pdh::PdhCloseQuery(query) };
            return None;
        }
        Some(Self {
            query,
            engine: if has_engine { engine } else { 0 },
            memory: if has_memory { memory } else { 0 },
            primed: false,
        })
    }
    fn array(counter: isize) -> Vec<(String, f64)> {
        if counter == 0 {
            return vec![];
        }
        let (mut size, mut count) = (0u32, 0u32);
        // SAFETY: a null buffer with size 0 asks for the size that is needed.
        let status = unsafe {
            pdh::PdhGetFormattedCounterArrayW(
                counter,
                pdh::FORMAT_DOUBLE,
                &mut size,
                &mut count,
                std::ptr::null_mut(),
            )
        };
        if status != pdh::MORE_DATA || size == 0 {
            return vec![];
        }
        // 8-byte units keep the items aligned for their pointer and double fields.
        let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
        // SAFETY: the buffer holds at least `size` bytes.
        let status = unsafe {
            pdh::PdhGetFormattedCounterArrayW(
                counter,
                pdh::FORMAT_DOUBLE,
                &mut size,
                &mut count,
                buffer.as_mut_ptr().cast(),
            )
        };
        if status != 0 {
            return vec![];
        }
        #[repr(C)]
        struct Item {
            name: *const u16,
            status: u32,
            value: f64,
        }
        let items = buffer.as_ptr().cast::<Item>();
        (0..count as usize)
            .filter_map(|i| {
                // SAFETY: PDH wrote `count` items at the start of the buffer, each pointing at a
                // NUL-terminated name inside the same buffer.
                let item = unsafe { &*items.add(i) };
                if item.status > 1 || item.name.is_null() {
                    return None;
                }
                let mut length = 0;
                while unsafe { *item.name.add(length) } != 0 {
                    length += 1;
                }
                let name = String::from_utf16_lossy(unsafe {
                    std::slice::from_raw_parts(item.name, length)
                });
                Some((name, item.value))
            })
            .collect()
    }
    /// LUIDs of the adapters that exist now, from the adapter memory counter instances.
    pub fn live_luids() -> std::collections::BTreeSet<String> {
        let Some(counters) = Self::open() else {
            return Default::default();
        };
        // SAFETY: the query is open for the life of `counters`.
        if unsafe { pdh::PdhCollectQueryData(counters.query) } != 0 {
            return Default::default();
        }
        aggregate_gpu_counters(&[], &Self::array(counters.memory))
            .into_keys()
            .collect()
    }
    /// Loads and memory per adapter since the previous call; the first call only primes the
    /// counters (a percentage needs two readings).
    pub fn sample(&mut self) -> Option<std::collections::BTreeMap<String, GpuReading>> {
        // SAFETY: the query is open for the life of `self`.
        if unsafe { pdh::PdhCollectQueryData(self.query) } != 0 {
            return None;
        }
        if !std::mem::replace(&mut self.primed, true) {
            return None;
        }
        Some(aggregate_gpu_counters(
            &Self::array(self.engine),
            &Self::array(self.memory),
        ))
    }
}
#[cfg(windows)]
impl Drop for WindowsGpuCounters {
    fn drop(&mut self) {
        // SAFETY: the query was opened by `open` and is closed once.
        unsafe { pdh::PdhCloseQuery(self.query) };
    }
}
/// Stand-in elsewhere: no Windows counters, so nothing is read through them.
#[cfg(not(windows))]
pub struct WindowsGpuCounters;
#[cfg(not(windows))]
impl WindowsGpuCounters {
    pub fn open() -> Option<Self> {
        None
    }
    pub fn sample(&mut self) -> Option<std::collections::BTreeMap<String, GpuReading>> {
        None
    }
    pub fn live_luids() -> std::collections::BTreeSet<String> {
        Default::default()
    }
}
/// Every GPU this machine reports, of any vendor. Linux: display adapters from sysfs. Windows:
/// the adapters DirectX lists in the registry; NVIDIA ones are matched to `nvidia-smi` for
/// temperature and power. Found once (adapters rarely change while the program runs).
pub fn discover_gpus() -> Vec<GpuDevice> {
    if cfg!(target_os = "linux") {
        return find_gpus(Path::new("/sys"));
    }
    static WINDOWS: std::sync::OnceLock<Vec<GpuDevice>> = std::sync::OnceLock::new();
    WINDOWS
        .get_or_init(|| {
            let run = |program: &str, args: &[&str]| -> Option<String> {
                let out = std::process::Command::new(crate::program(program))
                    .args(args)
                    .stdin(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .output()
                    .ok()?;
                out.status
                    .success()
                    .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
            };
            let nvidia = run(
                "nvidia-smi",
                &["--query-gpu=index,name,pci.bus_id", "--format=csv,noheader"],
            )
            .map(|t| parse_nvidia_list(&t))
            .unwrap_or_default();
            let adapters = run(
                "reg",
                &["query", "HKLM\\SOFTWARE\\Microsoft\\DirectX", "/s"],
            )
            .map(|t| parse_directx_registry(&t))
            .unwrap_or_default();
            let adapters = keep_live_adapters(adapters, &WindowsGpuCounters::live_luids());
            if adapters.is_empty() {
                nvidia
            } else {
                windows_devices(&adapters, &nvidia)
            }
        })
        .clone()
}
/// Runtime power state (`active`, `suspending`, `suspended`); a device without runtime power
/// management is always `active`. Reading this file never wakes the device.
pub fn runtime_status(device: &GpuDevice) -> String {
    fs::read_to_string(device.device.join("power/runtime_status"))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "active".into())
}
fn number(path: &Path) -> Option<f64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}
fn first_hwmon(device: &Path, file: &str) -> Option<f64> {
    fs::read_dir(device.join("hwmon"))
        .ok()?
        .flatten()
        .find_map(|h| number(&h.path().join(file)))
}
/// Share of wall time the GPU was not in its idle (RC6) state, from two residency readings.
pub fn active_percent(previous_ms: u64, current_ms: u64, wall_ms: u64) -> Option<f32> {
    let idle = current_ms.checked_sub(previous_ms)?;
    if wall_ms == 0 {
        return None;
    }
    Some((100.0 - idle as f64 / wall_ms as f64 * 100.0).clamp(0.0, 100.0) as f32)
}
/// Reads a device that is already awake. NVIDIA is read through `nvidia-smi` instead.
/// `rc6` keeps the previous idle-residency reading (Intel).
pub fn read_gpu(device: &GpuDevice, rc6: &mut Option<(u64, Instant)>) -> GpuReading {
    let mut reading = GpuReading {
        temp: first_hwmon(&device.device, "temp1_input").map(|m| m as f32 / 1000.0),
        power: first_hwmon(&device.device, "power1_average")
            .or_else(|| first_hwmon(&device.device, "power1_input"))
            .map(|uw| uw as f32 / 1_000_000.0),
        ..GpuReading::default()
    };
    match device.vendor {
        Vendor::Amd | Vendor::Other => {
            reading.util = number(&device.device.join("gpu_busy_percent")).map(|v| v as f32);
            reading.vram_used =
                number(&device.device.join("mem_info_vram_used")).map(|b| (b / 1_048_576.0) as f32);
            reading.vram_total = number(&device.device.join("mem_info_vram_total"))
                .map(|b| (b / 1_048_576.0) as f32);
        }
        Vendor::Intel => {
            // i915: gt_act_freq_mhz and gt/gt0/rc6_residency_ms; xe: tile0/gt0/freq0 and gtidle.
            reading.clock = number(&device.card.join("gt_act_freq_mhz"))
                .or_else(|| number(&device.device.join("tile0/gt0/freq0/act_freq")))
                .map(|v| v as f32);
            let residency = number(&device.card.join("gt/gt0/rc6_residency_ms"))
                .or_else(|| number(&device.device.join("tile0/gt0/gtidle/idle_residency_ms")))
                .map(|v| v as u64);
            if let Some(current) = residency {
                let now = Instant::now();
                if let Some((before, at)) = rc6.replace((current, now)) {
                    reading.util =
                        active_percent(before, current, now.duration_since(at).as_millis() as u64);
                    reading.awake_share = true;
                }
            }
        }
        Vendor::Nvidia => {}
    }
    reading
}
/// One `nvidia-smi --format=csv,noheader,nounits` line: utilization, VRAM used/total (MiB),
/// temperature (C), power (W). `[N/A]` or junk fields are unknown.
pub fn parse_gpu_line(line: &str) -> Option<GpuReading> {
    let fields: Vec<_> = line.split(',').map(str::trim).collect();
    if fields.len() != 5 {
        return None;
    }
    let value = |i: usize| {
        fields[i]
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite() && *v >= 0.0)
    };
    Some(GpuReading {
        util: value(0),
        vram_used: value(1),
        vram_total: value(2),
        temp: value(3),
        power: value(4),
        clock: None,
        awake_share: false,
    })
}
/// Nothing is asking for the GPU: no known load and (almost) no memory in use. A device that
/// reports no load at all cannot prove it is busy, so it counts as idle.
pub fn gpu_is_idle(reading: &GpuReading) -> bool {
    reading.util.is_none_or(|u| u == 0.0) && reading.vram_used.is_none_or(|m| m <= 16.0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] // SQ-32: vendors are told apart by PCI id.
    fn vendors() {
        assert_eq!(vendor_of("0x10de\n"), Vendor::Nvidia);
        assert_eq!(vendor_of("0x1002"), Vendor::Amd);
        assert_eq!(vendor_of("0x8086"), Vendor::Intel);
        assert_eq!(vendor_of("0x1af4"), Vendor::Other);
    }
    #[test] // SQ-32: GPU lines keep unknown fields unknown and idle needs known zeros or no data.
    fn gpu_line_and_idle() {
        let v = parse_gpu_line("4, 2, 6144, 54, 312.23").unwrap();
        assert_eq!(
            (v.util, v.vram_total, v.power),
            (Some(4.0), Some(6144.0), Some(312.23))
        );
        assert!(!gpu_is_idle(&v));
        let v = parse_gpu_line("0, 2, 6144, [N/A], [N/A]").unwrap();
        assert_eq!(v.temp, None);
        assert!(gpu_is_idle(&v));
        assert!(!gpu_is_idle(&parse_gpu_line("0, 900, 6144, 5, 5").unwrap()));
        assert_eq!(parse_gpu_line("1,2,3"), None);
    }
    #[test] // SQ-32: Intel activity is the time not spent in the idle state.
    fn idle_residency_becomes_activity() {
        assert_eq!(active_percent(1000, 1900, 1000), Some(10.0));
        assert_eq!(active_percent(1000, 2000, 1000), Some(0.0));
        assert_eq!(active_percent(2000, 1000, 1000), None);
        assert_eq!(active_percent(0, 0, 0), None);
    }
    #[test] // SQ-32: AMD files and an Intel device are read from a fake sysfs tree.
    fn reads_amd_and_intel_trees() {
        let root = std::env::temp_dir().join(format!("sq-gpu-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let amd = root.join("class/drm/card0");
        fs::create_dir_all(amd.join("device/hwmon/hwmon3")).unwrap();
        fs::write(amd.join("device/vendor"), "0x1002\n").unwrap();
        fs::write(amd.join("device/boot_vga"), "0\n").unwrap();
        fs::write(amd.join("device/gpu_busy_percent"), "37\n").unwrap();
        fs::write(amd.join("device/mem_info_vram_used"), "1073741824\n").unwrap();
        fs::write(amd.join("device/mem_info_vram_total"), "8589934592\n").unwrap();
        fs::write(amd.join("device/hwmon/hwmon3/temp1_input"), "61000\n").unwrap();
        fs::write(amd.join("device/hwmon/hwmon3/power1_average"), "45000000\n").unwrap();
        fs::create_dir_all(root.join("class/drm/card0-DP-1")).unwrap();
        let intel = root.join("class/drm/card1");
        fs::create_dir_all(intel.join("device")).unwrap();
        fs::create_dir_all(intel.join("gt/gt0")).unwrap();
        fs::write(intel.join("device/vendor"), "0x8086\n").unwrap();
        fs::write(intel.join("device/boot_vga"), "1\n").unwrap();
        fs::write(intel.join("gt_act_freq_mhz"), "300\n").unwrap();
        fs::write(intel.join("gt/gt0/rc6_residency_ms"), "5000\n").unwrap();
        let found = find_gpus(&root);
        assert_eq!(
            found.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(),
            ["card0", "card1"]
        );
        assert!(found[0].discrete && !found[1].discrete);
        let mut none = None;
        let r = read_gpu(&found[0], &mut none);
        assert_eq!(
            (r.util, r.vram_used, r.vram_total, r.temp, r.power),
            (
                Some(37.0),
                Some(1024.0),
                Some(8192.0),
                Some(61.0),
                Some(45.0)
            )
        );
        let mut rc6 = None;
        let first = read_gpu(&found[1], &mut rc6);
        assert_eq!((first.clock, first.util), (Some(300.0), None));
        assert!(rc6.is_some());
        let _ = fs::remove_dir_all(&root);
    }

    // REQ SQ-36: GPUs are found from nvidia-smi output where there is no sysfs (Windows).
    #[test]
    fn nvidia_list_becomes_devices_with_sysfs_style_slots() {
        let text = "0, NVIDIA GeForce RTX 3050 Laptop GPU, 00000000:01:00.0\n1, NVIDIA T400, 00000000:81:00.0\ngarbage line\n, no index, 00000000:02:00.0\n";
        let devices = parse_nvidia_list(text);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].id, "nvidia0");
        assert_eq!(devices[0].name, "NVIDIA GeForce RTX 3050 Laptop GPU");
        assert_eq!(devices[0].slot, "0000:01:00.0");
        assert_eq!(devices[1].slot, "0000:81:00.0");
        assert!(devices
            .iter()
            .all(|d| d.vendor == Vendor::Nvidia && !d.discrete && d.device.as_os_str().is_empty()));
        assert!(parse_nvidia_list("").is_empty());
        assert!(parse_nvidia_list("No devices were found").is_empty());
    }

    // REQ SQ-36 SQ-32: Windows adapters of any vendor come from the DirectX registry entries.
    #[test]
    fn directx_registry_lists_adapters_of_every_vendor() {
        let text = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\DirectX\r\n    Version    REG_SZ    4.09.00.0904\r\n\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\DirectX\\{aaa}\r\n    Description    REG_SZ    Intel(R) UHD Graphics\r\n    VendorId    REG_DWORD    0x8086\r\n    DedicatedVideoMemory    REG_QWORD    0x8000000\r\n    AdapterLuid    REG_QWORD    0xa1b2\r\n\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\DirectX\\{bbb}\r\n    Description    REG_SZ    AMD Radeon RX 6600\r\n    VendorId    REG_DWORD    0x1002\r\n    DedicatedVideoMemory    REG_QWORD    0x200000000\r\n    AdapterLuid    REG_QWORD    0xzz\r\n\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\DirectX\\{ccc}\r\n    Description    REG_SZ    NVIDIA GeForce RTX 3050 Laptop GPU\r\n    VendorId    REG_DWORD    0x10de\r\n    DedicatedVideoMemory    REG_QWORD    0x180000000\r\n    AdapterLuid    REG_QWORD    0x1_0000_c3d4\r\n";
        // The AMD and NVIDIA luids above are not valid hex on purpose: such entries are skipped, not guessed.
        let adapters = parse_directx_registry(text);
        let names: Vec<_> = adapters.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Intel(R) UHD Graphics"]);
        let good = "HKEY_LOCAL_MACHINE\\X\\{a}\n    Description    REG_SZ    AMD Radeon RX 6600\n    VendorId    REG_DWORD    0x1002\n    DedicatedVideoMemory    REG_QWORD    0x200000000\n    AdapterLuid    REG_QWORD    0xb3f1\nHKEY_LOCAL_MACHINE\\X\\{b}\n    Description    REG_SZ    NVIDIA GeForce RTX 3050 Laptop GPU\n    VendorId    REG_DWORD    0x10de\n    DedicatedVideoMemory    REG_QWORD    0x180000000\n    AdapterLuid    REG_QWORD    0x1000c3d4\nHKEY_LOCAL_MACHINE\\X\\{c}\n    Description    REG_SZ    Orphan without a luid\n";
        let adapters = parse_directx_registry(good);
        assert_eq!(adapters.len(), 2, "{adapters:?}");
        assert_eq!(adapters[0].vram_bytes, 8 * 1024 * 1024 * 1024);
        assert_eq!(luid_key(adapters[0].luid), "0x00000000_0x0000b3f1");
        assert_eq!(luid_key(adapters[1].luid), "0x00000000_0x1000c3d4");
        assert_eq!(luid_key(0x1_0000_00ff), "0x00000001_0x000000ff");
        // NVIDIA is matched to nvidia-smi by name; the AMD adapter is read through the counters.
        let nvidia = parse_nvidia_list("0, NVIDIA GeForce RTX 3050 Laptop GPU, 00000000:01:00.0\n");
        let devices = windows_devices(&adapters, &nvidia);
        assert_eq!(devices[0].vendor, Vendor::Amd);
        assert!(devices[0].slot.is_empty() && devices[0].luid.is_some());
        assert_eq!(devices[1].vendor, Vendor::Nvidia);
        assert_eq!(devices[1].slot, "0000:01:00.0");
        assert!(devices
            .iter()
            .all(|d| d.device.as_os_str().is_empty() && !d.discrete));
    }
    // REQ SQ-36 SQ-32: load is the busiest engine type summed over processes, memory is per adapter.
    #[test]
    fn counters_become_one_reading_per_adapter() {
        let engines = vec![
            (
                "pid_100_luid_0x00000000_0x0000B3F1_phys_0_eng_0_engtype_3D".to_string(),
                30.0,
            ),
            (
                "pid_200_luid_0x00000000_0x0000B3F1_phys_0_eng_0_engtype_3D".to_string(),
                25.5,
            ),
            (
                "pid_200_luid_0x00000000_0x0000B3F1_phys_0_eng_1_engtype_Copy".to_string(),
                10.0,
            ),
            (
                "pid_300_luid_0x00000000_0x0000B3F1_phys_0_eng_0_engtype_VideoDecode".to_string(),
                70.0,
            ),
            (
                "pid_100_luid_0x00000000_0x1000C3D4_phys_0_eng_0_engtype_3D".to_string(),
                140.0,
            ),
            ("garbage".to_string(), 99.0),
            (
                "pid_1_luid_0x00000000_0x00000001_phys_0_eng_0_engtype_3D".to_string(),
                f64::NAN,
            ),
        ];
        let memory = vec![
            (
                "luid_0x00000000_0x0000B3F1_phys_0".to_string(),
                512.0 * 1024.0 * 1024.0,
            ),
            ("luid_0x00000000_0x1000C3D4_phys_0".to_string(), 0.0),
            ("nonsense".to_string(), 5.0),
        ];
        let readings = aggregate_gpu_counters(&engines, &memory);
        let amd = readings["0x00000000_0x0000b3f1"];
        // 3D = 55.5, Copy = 10, VideoDecode = 70: the busiest wins, as in Task Manager.
        assert_eq!(amd.util, Some(70.0));
        assert_eq!(amd.vram_used, Some(512.0));
        let nvidia = readings["0x00000000_0x1000c3d4"];
        assert_eq!(nvidia.util, Some(100.0), "capped at 100");
        assert_eq!(nvidia.vram_used, Some(0.0));
        assert!(
            !readings.contains_key("0x00000000_0x00000001"),
            "unreadable value is unknown"
        );
        assert_eq!(readings.len(), 2);
        assert!(aggregate_gpu_counters(&[], &[]).is_empty());
    }

    // REQ SQ-36 SQ-32: instance names from other Windows languages, odd casing, empty lists and
    // out-of-range values never become invented readings.
    #[test]
    fn counter_edge_cases_stay_unknown_or_bounded() {
        let engines = vec![
            (
                "PID_7_LUID_0X00000000_0X0000ABCD_PHYS_0_ENG_0_ENGTYPE_3D".to_string(),
                12.0,
            ),
            (
                "pid_7_luid_0x00000000_0x0000abcd_phys_0_eng_1_engtype_3D".to_string(),
                -5.0,
            ),
            (
                "pid_7_luid_0x00000000_0x0000abcd_phys_0_eng_2".to_string(),
                50.0,
            ),
            ("pid_7_luid_0x00000000_phys_0_engtype_3D".to_string(), 50.0),
            (
                "pid_7_luid_0x00000000_0x0000dead_phys_0_engtype_3D".to_string(),
                f64::INFINITY,
            ),
        ];
        let memory = vec![("luid_0x00000000_0x0000dead_phys_0".to_string(), f64::NAN)];
        let readings = aggregate_gpu_counters(&engines, &memory);
        assert_eq!(
            readings.len(),
            1,
            "only the adapter with a finite value appears"
        );
        assert_eq!(
            readings["0x00000000_0x0000abcd"].util,
            Some(12.0),
            "negative adds nothing"
        );
        assert_eq!(
            readings["0x00000000_0x0000abcd"].vram_used, None,
            "no memory counter: unknown"
        );
    }

    // REQ SQ-36 SQ-32: on a real Windows machine (also one with only the basic software adapter)
    // the counters open, the first sample only primes, and every listed adapter reads 0..=100.
    #[cfg(windows)]
    #[test]
    fn live_windows_counters_cover_listed_adapters() {
        let devices = discover_gpus();
        assert!(
            !devices.is_empty(),
            "DirectX lists at least the basic adapter"
        );
        let mut counters =
            WindowsGpuCounters::open().expect("GPU counters exist on Windows 10 1709+");
        assert!(counters.sample().is_none(), "first sample only primes");
        std::thread::sleep(std::time::Duration::from_millis(1200));
        let readings = counters.sample().expect("second sample reads");
        for device in &devices {
            let luid = device.luid.as_ref().expect("Windows adapter has a LUID");
            let reading = readings
                .get(luid)
                .unwrap_or_else(|| panic!("{luid} missing in {readings:?}"));
            let util = reading.util.expect("load is read");
            assert!((0.0..=100.0).contains(&util), "{util}");
        }
    }

    // REQ SQ-36 SQ-32: adapters left in the registry by earlier boots are dropped once the counters
    // say which adapters exist now; with no counter report nothing is dropped.
    #[test]
    fn stale_registry_adapters_are_dropped() {
        let adapter = |luid| WindowsAdapter {
            name: "GPU".into(),
            vendor_id: 0,
            vram_bytes: 0,
            luid,
        };
        let all = vec![adapter(0x57aa), adapter(0x583c)];
        let live: std::collections::BTreeSet<String> = [luid_key(0x583c)].into();
        let kept = keep_live_adapters(all.clone(), &live);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].luid, 0x583c);
        assert_eq!(
            keep_live_adapters(all.clone(), &Default::default()).len(),
            2
        );
        let none: std::collections::BTreeSet<String> = [luid_key(0x9999)].into();
        assert_eq!(
            keep_live_adapters(all, &none).len(),
            2,
            "never end with no adapter"
        );
    }
}
