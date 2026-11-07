use crate::{now, update};
use notify::{EventKind, RecursiveMode, Watcher};
use sq_core::*;
use sq_sources::*;
use sq_store::{atomic_json, read_json, Store};
use std::{
    collections::BTreeMap,
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
    thread,
    time::{Duration, Instant},
};

pub fn start(
    store: Arc<Store>,
    snapshot: Arc<RwLock<Snapshot>>,
    stop: Arc<AtomicBool>,
) -> Vec<thread::JoinHandle<()>> {
    if let Ok(saved) = store.configuration() {
        if saved.revision == 0 && saved.config.repositories.is_empty() {
            let repositories = seed_repositories(&saved.config.sources.project_roots);
            if !repositories.is_empty() {
                let mut config = saved.config;
                config.repositories = repositories;
                let _ = store.configure(saved.revision, ConfigAction::Replace(Box::new(config)));
            }
        }
    }
    let mut workers = Vec::new();
    {
        let store = store.clone();
        let snapshot = snapshot.clone();
        let stop = stop.clone();
        workers.push(thread::spawn(move || accounts(store, snapshot, stop)));
    }
    {
        let store = store.clone();
        let snapshot = snapshot.clone();
        let stop = stop.clone();
        workers.push(thread::spawn(move || github(store, snapshot, stop)));
    }
    {
        let store = store.clone();
        let snapshot = snapshot.clone();
        let stop = stop.clone();
        workers.push(thread::spawn(move || memory(store, snapshot, stop)));
    }
    {
        let store = store.clone();
        let snapshot = snapshot.clone();
        let stop = stop.clone();
        workers.push(thread::spawn(move || system(store, snapshot, stop)));
    }
    {
        let store = store.clone();
        let snapshot = snapshot.clone();
        let stop = stop.clone();
        workers.push(thread::spawn(move || gpu(store, snapshot, stop)));
    }
    workers.push(thread::spawn(move || local(store, snapshot, stop)));
    workers
}
/// Keeps one `nvidia-smi -l 5` child that streams readings; killing it lets the GPU sleep again.
struct GpuReader {
    child: std::process::Child,
    latest: Arc<Mutex<Option<(Instant, String)>>>,
}
impl GpuReader {
    fn start(slot: &str) -> Option<Self> {
        use std::io::BufRead;
        let mut child = std::process::Command::new("nvidia-smi")
            .args([
                "-i",
                slot,
                "--query-gpu=utilization.gpu,memory.used,memory.total,temperature.gpu,power.draw",
                "--format=csv,noheader,nounits",
                "-l",
                "5",
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let out = child.stdout.take()?;
        let latest = Arc::new(Mutex::new(None));
        let sink = latest.clone();
        thread::spawn(move || {
            for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                if let Ok(mut slot) = sink.lock() {
                    *slot = Some((Instant::now(), line));
                }
            }
        });
        Some(Self { child, latest })
    }
    fn latest(&self) -> Option<GpuReading> {
        let (at, line) = self.latest.lock().ok()?.clone()?;
        (at.elapsed() < Duration::from_secs(15))
            .then(|| parse_gpu_line(&line))
            .flatten()
    }
}
impl Drop for GpuReader {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
/// How long after we last touched a sleeping-capable GPU before an awake GPU counts as used by
/// something else (a device stays awake for some seconds after any reading).
const GPU_OWN_WAKE: Duration = Duration::from_secs(30);
const GPU_IDLE_SAMPLES: u32 = 3;
/// Per-device sampling state.
struct GpuTrack {
    device: GpuDevice,
    nvidia: Option<GpuReader>,
    /// A discrete device is being read right now (it was awake for another reason).
    reading: bool,
    last_touch: Option<Instant>,
    idle_samples: u32,
    rc6: Option<(u64, Instant)>,
}
impl GpuTrack {
    fn new(device: GpuDevice) -> Self {
        Self {
            device,
            nvidia: None,
            reading: false,
            last_touch: None,
            idle_samples: 0,
            rc6: None,
        }
    }
    fn sample(
        &mut self,
        windows: Option<&std::collections::BTreeMap<String, GpuReading>>,
    ) -> GpuSample {
        let blank = |state, reading: GpuReading| GpuSample {
            id: self.device.id.clone(),
            name: self.device.name.clone(),
            at: now(),
            state,
            util: reading.util,
            vram_used: reading.vram_used,
            vram_total: reading.vram_total,
            temp: reading.temp,
            power: reading.power,
            clock: reading.clock,
            awake_share: reading.awake_share,
        };
        if !self.device.discrete {
            if self.device.device.as_os_str().is_empty() && self.device.slot.is_empty() {
                // Windows, any vendor: the shared performance-counter reading of this adapter.
                let reading = self
                    .device
                    .luid
                    .as_ref()
                    .and_then(|luid| windows?.get(luid))
                    .copied();
                return match reading {
                    Some(mut reading) => {
                        reading.vram_total = self
                            .device
                            .vram_bytes
                            .map(|bytes| (bytes / (1024 * 1024)) as f32);
                        blank(GpuState::Reading, reading)
                    }
                    None => blank(GpuState::Waiting, GpuReading::default()),
                };
            }
            if self.device.device.as_os_str().is_empty() {
                // Windows, NVIDIA with `nvidia-smi`: one child streams the readings.
                if self.nvidia.is_none() {
                    self.nvidia = GpuReader::start(&self.device.slot);
                }
                return match self.nvidia.as_ref().and_then(GpuReader::latest) {
                    Some(reading) => blank(GpuState::Reading, reading),
                    None => blank(GpuState::Waiting, GpuReading::default()),
                };
            }
            // The display adapter is always awake: reading it costs nothing extra.
            let reading = read_gpu(&self.device, &mut self.rc6);
            return blank(GpuState::Reading, reading);
        }
        if !self.reading {
            if runtime_status(&self.device) != "active" {
                self.rc6 = None;
                return blank(GpuState::Asleep, GpuReading::default());
            }
            if self.last_touch.is_some_and(|t| t.elapsed() < GPU_OWN_WAKE) {
                return blank(GpuState::Waiting, GpuReading::default());
            }
            // Awake and not by our doing: something else is using it, so start reading.
            self.reading = true;
            self.idle_samples = 0;
            if self.device.vendor == Vendor::Nvidia {
                self.nvidia = GpuReader::start(&self.device.slot);
            }
        }
        self.last_touch = Some(Instant::now());
        let reading = match (&self.nvidia, self.device.vendor) {
            (Some(reader), _) => reader.latest(),
            (None, Vendor::Nvidia) => None,
            (None, _) => Some(read_gpu(&self.device, &mut self.rc6)),
        };
        let Some(reading) = reading else {
            return blank(GpuState::Waiting, GpuReading::default());
        };
        self.idle_samples = if gpu_is_idle(&reading) {
            self.idle_samples + 1
        } else {
            0
        };
        if self.idle_samples >= GPU_IDLE_SAMPLES {
            // Idle again: stop touching it so it can sleep.
            self.reading = false;
            self.nvidia = None;
        }
        blank(GpuState::Reading, reading)
    }
}
/// The GPU widget's sampler, for every GPU of any vendor. Display adapters are read directly.
/// A discrete GPU can power itself down and many queries wake it, so its power state is read from
/// sysfs (free) and it is measured only while something else has it awake, until it is idle again.
fn gpu(store: Arc<Store>, snapshot: Arc<RwLock<Snapshot>>, stop: Arc<AtomicBool>) {
    let mut tracks: Vec<GpuTrack> = Vec::new();
    let mut scanned: Option<Instant> = None;
    // Windows performance counters, opened when an adapter needs them.
    let mut counters: Option<WindowsGpuCounters> = None;
    let mut counters_tried = false;
    while !stop.load(Ordering::Acquire) {
        let wanted = store
            .configuration()
            .map(|c| c.config.widgets.iter().any(|w| w.kind == "gpu"))
            .unwrap_or(false);
        if !wanted {
            tracks.clear();
            scanned = None;
            counters = None;
            counters_tried = false;
            if let Ok(mut current) = snapshot.write() {
                if !current.gpu.is_empty() {
                    current.gpu.clear();
                    current.revision = current.revision.saturating_add(1);
                }
            }
            sleep(&stop, Duration::from_secs(SYSTEM_INTERVAL));
            continue;
        }
        if scanned.is_none_or(|at| at.elapsed() >= Duration::from_secs(60)) {
            // Rescan once a minute for added or removed adapters; keep state of known ones.
            let mut next = Vec::new();
            for device in discover_gpus() {
                match tracks.iter().position(|t| t.device.id == device.id) {
                    Some(index) => next.push(tracks.swap_remove(index)),
                    None => next.push(GpuTrack::new(device)),
                }
            }
            tracks = next;
            scanned = Some(Instant::now());
        }
        let samples: Vec<GpuSample> = if tracks.is_empty() {
            vec![GpuSample {
                id: "none".into(),
                name: "GPU".into(),
                at: now(),
                state: GpuState::Absent,
                util: None,
                vram_used: None,
                vram_total: None,
                temp: None,
                power: None,
                clock: None,
                awake_share: false,
            }]
        } else {
            if !counters_tried && tracks.iter().any(|t| t.device.luid.is_some()) {
                counters_tried = true;
                counters = WindowsGpuCounters::open();
            }
            let windows = counters.as_mut().and_then(WindowsGpuCounters::sample);
            // The registry also lists adapters that are gone (an old display driver, another
            // boot's software adapter): once the counters answer, those have no instance.
            tracks
                .iter_mut()
                .filter(|track| match (&track.device.luid, &windows) {
                    (Some(luid), Some(map)) if !map.is_empty() => map.contains_key(luid),
                    // Counters are open but still priming: show nothing rather than ghosts.
                    (Some(_), None) if counters.is_some() => false,
                    _ => true,
                })
                .map(|track| track.sample(windows.as_ref()))
                .collect()
        };
        if let Ok(mut current) = snapshot.write() {
            let keep = SYSTEM_KEEP * samples.len();
            current.gpu.extend(samples);
            let excess = current.gpu.len().saturating_sub(keep);
            current.gpu.drain(..excess);
            current.revision = current.revision.saturating_add(1);
        }
        sleep(&stop, Duration::from_secs(SYSTEM_INTERVAL));
    }
}
/// Seconds between samples and samples kept: ten minutes of history in memory only.
const SYSTEM_INTERVAL: u64 = 5;
const SYSTEM_KEEP: usize = 120;
/// Samples CPU and temperature only while a system widget exists. It reads two tiny kernel files,
/// pushes onto a bounded buffer under one short lock (no snapshot clone) and sleeps otherwise.
fn system(store: Arc<Store>, snapshot: Arc<RwLock<Snapshot>>, stop: Arc<AtomicBool>) {
    let mut previous = None;
    let mut sensor: Option<PathBuf> = None;
    while !stop.load(Ordering::Acquire) {
        let wanted = store
            .configuration()
            .map(|c| {
                c.config
                    .widgets
                    .iter()
                    .any(|w| w.kind == "system" || w.kind == "ram")
            })
            .unwrap_or(false);
        if !wanted {
            previous = None;
            if let Ok(mut current) = snapshot.write() {
                if !current.system.is_empty() {
                    current.system.clear();
                    current.revision = current.revision.saturating_add(1);
                }
            }
            sleep(&stop, Duration::from_secs(SYSTEM_INTERVAL));
            continue;
        }
        let reading = cpu_counters();
        let Some(before) = previous.replace(reading).flatten() else {
            // Baseline for the first share: a second reading comes one second later.
            sleep(&stop, Duration::from_secs(1));
            continue;
        };
        sensor = sensor.or_else(find_temperature_sensor);
        let sample = SystemSample {
            at: now(),
            cpu: reading.and_then(|r| cpu_percent(before, r)),
            temp: sensor.as_deref().and_then(read_temperature),
            ram: memory_sample(),
        };
        if let Ok(mut current) = snapshot.write() {
            current.system.push(sample);
            let excess = current.system.len().saturating_sub(SYSTEM_KEEP);
            current.system.drain(..excess);
            current.revision = current.revision.saturating_add(1);
        }
        sleep(&stop, Duration::from_secs(SYSTEM_INTERVAL));
    }
}
fn sleep(stop: &AtomicBool, duration: Duration) {
    let until = Instant::now() + duration;
    while !stop.load(Ordering::Acquire) && Instant::now() < until {
        thread::sleep(Duration::from_millis(100));
    }
}
/// Reads the usage of every Claude and Codex account in turn: the two default ones and any extra
/// account of the configuration. Each account keeps its own failure back-off and health entry.
fn accounts(store: Arc<Store>, snapshot: Arc<RwLock<Snapshot>>, stop: Arc<AtomicBool>) {
    let mut due: BTreeMap<String, (Instant, u32)> = BTreeMap::new();
    while !stop.load(Ordering::Acquire) {
        let Ok(saved) = store.configuration() else {
            break;
        };
        let sources = saved.config.sources.clone();
        let list = sources.assistant_accounts();
        due.retain(|key, _| list.iter().any(|(k, ..)| k == key));
        for (key, kind, home) in list {
            let (next, failures) = due.get(&key).copied().unwrap_or((Instant::now(), 0));
            if Instant::now() < next || stop.load(Ordering::Acquire) {
                continue;
            }
            let time = now();
            let result = if kind == "claude" {
                fetch_claude(&expand(&home), time)
            } else if key == "codex" {
                fetch_codex(None, time)
            } else {
                fetch_codex(Some(&expand(&home)), time)
            };
            let failures = if result.is_ok() { 0 } else { failures + 1 };
            let interval = if result.is_ok() {
                sources.poll_seconds
            } else {
                (sources.poll_seconds * 2u64.saturating_pow(failures.min(3))).min(3600)
            };
            due.insert(
                key.clone(),
                (Instant::now() + Duration::from_secs(interval), failures),
            );
            let provider = key.as_str();
            update(&snapshot, |s| match result {
                Ok(usage) => {
                    s.usage.insert(provider.into(), usage);
                    s.health.insert(
                        provider.into(),
                        Health {
                            state: SourceState::Fresh,
                            last_success: Some(time),
                            retry_at: Some(time + interval as i64),
                            message: "Account status; no generation request".into(),
                            ..Health::default()
                        },
                    );
                }
                Err(error) => {
                    let last = s.health.get(provider).and_then(|h| h.last_success);
                    s.health.insert(
                        provider.into(),
                        Health {
                            state: if s.usage.contains_key(provider) {
                                SourceState::Stale
                            } else {
                                SourceState::Unavailable
                            },
                            last_success: last,
                            retry_at: Some(time + interval as i64),
                            message: error,
                            ..Health::default()
                        },
                    );
                }
            });
        }
        // Wake often enough to notice a new account or a due retry without busy waiting.
        sleep(&stop, Duration::from_secs(15));
    }
}
/// Reads recent GitHub activity for the default `gh` login and for every extra GitHub account.
fn github(store: Arc<Store>, snapshot: Arc<RwLock<Snapshot>>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Acquire) {
        let Ok(saved) = store.configuration() else {
            break;
        };
        let time = now();
        // (key, gh configuration folder, repositories)
        let mut logins: Vec<(String, Option<PathBuf>, Vec<String>)> =
            vec![("github".into(), None, saved.config.repositories.clone())];
        for a in saved
            .config
            .sources
            .accounts
            .iter()
            .filter(|a| a.kind == "github")
        {
            let folder = (!a.path.is_empty()).then(|| expand(&a.path));
            logins.push((a.key(), folder, a.repositories.clone()));
        }
        for (key, folder, repositories) in logins {
            if stop.load(Ordering::Acquire) {
                return;
            }
            let result = fetch_github_as(&key, folder.as_deref(), &repositories);
            update(&snapshot, |s| {
                // Keep previous observations when one endpoint fails, then merge stable identities.
                let mut items: std::collections::BTreeMap<_, _> = s
                    .activity
                    .iter()
                    .filter(|a| a.account != key || repositories.contains(&a.repository))
                    .map(|a| ((a.account.clone(), a.id.clone()), a.clone()))
                    .collect();
                for item in result.items {
                    items.insert((item.account.clone(), item.id.clone()), item);
                }
                s.activity = items
                    .into_values()
                    .filter(|a| a.at >= time - 30 * 86400)
                    .collect();
                s.activity.sort_by_key(|a| std::cmp::Reverse(a.at));
                s.activity
                    .truncate(500 * (1 + saved.config.sources.accounts.len()));
                let mine = s.activity.iter().filter(|a| a.account == key).count();
                let state = if result.errors.is_empty() {
                    if mine == 0 {
                        SourceState::Empty
                    } else {
                        SourceState::Fresh
                    }
                } else if mine == 0 {
                    SourceState::Unavailable
                } else {
                    SourceState::Stale
                };
                let last = if result.errors.is_empty() {
                    Some(time)
                } else {
                    s.health.get(&key).and_then(|h| h.last_success)
                };
                s.health.insert(key.clone(),Health{state,last_success:last,retry_at:Some(time+300),malformed:0,deferred:result.deferred as u64,
                    message:if result.errors.is_empty(){format!("{} selected repositories · most recent 40/category · no repositories selected means no network reads",repositories.len())}else{result.errors.join("; ")},});
            });
        }
        sleep(&stop, Duration::from_secs(300));
    }
}
/// Watches the Codex, progress and event folders recursively, the Claude projects folder only for
/// new project folders, and each project folder that holds transcripts (never the ~100,000 empty
/// ones that throwaway working directories leave behind).
fn transcript_changes(
    sources: &SourceConfig,
    root: &std::path::Path,
    projects: &BTreeSet<PathBuf>,
) -> Changes {
    let mut recursive = vec![root.join("events")];
    if !sources.progress.is_empty() {
        recursive.push(expand(&sources.progress));
    }
    let mut roots = vec![];
    for (_, kind, home) in sources.assistant_accounts() {
        if kind == "codex" {
            recursive.push(expand(&home).join("sessions"));
        } else {
            roots.push(expand(&home).join("projects"));
        }
    }
    recursive.extend(projects.iter().cloned());
    Changes::with(recursive, roots)
}
struct Changes {
    paths: Arc<Mutex<BTreeSet<String>>>,
    discovery: Arc<AtomicBool>,
    failures: Arc<Mutex<BTreeSet<String>>>,
    _watcher: Option<notify::RecommendedWatcher>,
}
impl Changes {
    fn new(paths: Vec<PathBuf>) -> Self {
        Self::with(paths, vec![])
    }
    /// `shallow` folders are watched without descending (only their direct entries).
    fn with(paths: Vec<PathBuf>, shallow: Vec<PathBuf>) -> Self {
        let changed = Arc::new(Mutex::new(BTreeSet::new()));
        let discovery = Arc::new(AtomicBool::new(true));
        let failures = Arc::new(Mutex::new(BTreeSet::new()));
        let callback_paths = changed.clone();
        let callback_discovery = discovery.clone();
        let callback_failures = failures.clone();
        let result =
            notify::recommended_watcher(move |event: Result<notify::Event, notify::Error>| {
                match event {
                    Ok(event) if !matches!(event.kind, EventKind::Access(_)) => {
                        let discovers = matches!(
                            event.kind,
                            EventKind::Create(_)
                                | EventKind::Remove(_)
                                | EventKind::Modify(notify::event::ModifyKind::Name(_))
                        );
                        if discovers
                            && event
                                .paths
                                .iter()
                                .any(|p| p.extension().is_some_and(|e| e == "jsonl"))
                        {
                            callback_discovery.store(true, Ordering::Release);
                        }
                        if let Ok(mut paths) = callback_paths.lock() {
                            for path in event.paths {
                                if paths.len() < 10000 {
                                    paths.insert(path.to_string_lossy().into());
                                } else {
                                    callback_discovery.store(true, Ordering::Release);
                                    if let Ok(mut errors) = callback_failures.lock() {
                                        errors.insert(
                                            "Watch event queue exceeded 10000 paths".into(),
                                        );
                                    }
                                }
                            }
                        }
                    }
                    Err(error) => {
                        callback_discovery.store(true, Ordering::Release);
                        if let Ok(mut errors) = callback_failures.lock() {
                            if errors.len() < 32 {
                                errors.insert(error.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            });
        let mut watcher = match result {
            Ok(watcher) => Some(watcher),
            Err(error) => {
                if let Ok(mut errors) = failures.lock() {
                    errors.insert(error.to_string());
                }
                None
            }
        };
        if let Some(watcher) = watcher.as_mut() {
            let all = paths
                .into_iter()
                .map(|p| (p, RecursiveMode::Recursive))
                .chain(
                    shallow
                        .into_iter()
                        .map(|p| (p, RecursiveMode::NonRecursive)),
                );
            for (path, mode) in all {
                // A tool that is not installed has no folder to watch: that is not a failure. Later
                // discovery (every 30 seconds) notices the folder once it exists.
                if !path.exists() {
                    continue;
                }
                if let Err(error) = watcher.watch(&path, mode) {
                    if let Ok(mut errors) = failures.lock() {
                        if errors.len() < 32 {
                            errors.insert(format!("{}: {error}", path.display()));
                        }
                    }
                }
            }
        }
        Self {
            paths: changed,
            discovery,
            failures,
            _watcher: watcher,
        }
    }
    fn absorb(&self, pending: BTreeSet<String>) {
        if let Ok(mut paths) = self.paths.lock() {
            paths.extend(pending);
        }
    }
    fn take(&self) -> BTreeSet<String> {
        self.paths
            .lock()
            .map(|mut s| std::mem::take(&mut *s))
            .unwrap_or_default()
    }
    fn fallback(&self) -> bool {
        self.failures.lock().map(|e| !e.is_empty()).unwrap_or(true)
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(if self.fallback() { 2 } else { 30 })
    }
    fn health(&self) -> Health {
        let errors = self
            .failures
            .lock()
            .map(|e| e.iter().cloned().collect::<Vec<_>>().join("; "))
            .unwrap_or_else(|_| "Watch state unavailable".into());
        Health {
            state: if errors.is_empty() {
                SourceState::Fresh
            } else {
                SourceState::Stale
            },
            message: if errors.is_empty() {
                "Filesystem notifications active; 30-second safety scan".into()
            } else {
                format!("Filesystem notifications incomplete; 2-second polling fallback: {errors}")
            },
            ..Health::default()
        }
    }
}
fn private_events_dir(root: &std::path::Path) -> std::io::Result<()> {
    let directory = root.join("events");
    std::fs::create_dir_all(&directory)?;
    sq_store::private::private_dir(&directory)
}
fn merge_memory(snapshot: &mut Snapshot, items: Vec<MemoryItem>, mut health: Health) {
    if matches!(health.state, SourceState::Unavailable | SourceState::Error) && items.is_empty() {
        health.last_success = snapshot.health.get("memory").and_then(|h| h.last_success);
        if !snapshot.memory.is_empty() {
            health.state = SourceState::Stale;
            health.message = format!("{}; previous valid memory events retained", health.message);
        }
    } else {
        snapshot.memory = items;
    }
    snapshot.health.insert("memory".into(), health);
}
fn memory(store: Arc<Store>, snapshot: Arc<RwLock<Snapshot>>, stop: Arc<AtomicBool>) {
    let mut root = PathBuf::new();
    let mut changes = Changes::new(vec![]);
    let mut last: Option<Instant> = None;
    while !stop.load(Ordering::Acquire) {
        let Ok(saved) = store.configuration() else {
            break;
        };
        let next = expand(&saved.config.sources.memory);
        if root != next {
            root = next;
            changes = Changes::new(vec![root.join("events")]);
            last = None;
        }
        if !changes.take().is_empty() || last.is_none_or(|t| t.elapsed() >= changes.interval()) {
            let (items, health) = memory_feed(&root, now());
            update(&snapshot, |s| {
                merge_memory(s, items, health);
                let watch = changes.health();
                if changes.fallback() {
                    if let Some(health) = s.health.get_mut("memory") {
                        if matches!(health.state, SourceState::Fresh | SourceState::Empty) {
                            health.state = SourceState::Stale;
                        }
                        health.message.push_str(&format!("; {}", watch.message));
                    }
                }
                s.health.insert("memory-watch".into(), watch);
            });
            last = Some(Instant::now());
        }
        sleep(&stop, Duration::from_millis(500));
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Stamp {
    modified: Option<std::time::SystemTime>,
    length: u64,
    inode: u64,
}
impl Stamp {
    fn read(path: &std::path::Path) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let metadata = file.metadata()?;
        Ok(Self {
            modified: metadata.modified().ok(),
            length: metadata.len(),
            inode: file_identity(&file, &metadata),
        })
    }
}
struct Cached<T> {
    stamp: Stamp,
    value: Option<T>,
    malformed: bool,
}
struct Evidence<T> {
    root: PathBuf,
    entries: std::collections::BTreeMap<PathBuf, Cached<T>>,
    unavailable: Option<String>,
    deferred: u64,
    last_success: Option<i64>,
    reads: u64,
}
impl<T> Default for Evidence<T> {
    fn default() -> Self {
        Self {
            root: PathBuf::new(),
            entries: Default::default(),
            unavailable: None,
            deferred: 0,
            last_success: None,
            reads: 0,
        }
    }
}
trait Observation: Clone {
    fn at(&self) -> i64;
    fn sequence(&self) -> u64 {
        0
    }
}
impl Observation for LifecycleEvent {
    fn at(&self) -> i64 {
        self.at
    }
    fn sequence(&self) -> u64 {
        self.sequence
    }
}
#[derive(Clone)]
struct Progress {
    at: i64,
    id: String,
    label: String,
    done: u32,
    total: u32,
}
impl Observation for Progress {
    fn at(&self) -> i64 {
        self.at
    }
}
impl<T: Observation> Evidence<T> {
    fn refresh(
        &mut self,
        root: &std::path::Path,
        paths: &BTreeSet<String>,
        full: bool,
        time: i64,
        parse: fn(&std::path::Path) -> Result<T, String>,
        owned: bool,
    ) {
        let root_changed = self.root != root;
        if root_changed {
            *self = Self::default();
            self.root = root.to_path_buf();
        }
        let full = full || root_changed;
        let mut selected = BTreeSet::new();
        if full {
            match std::fs::read_dir(root) {
                Ok(entries) => {
                    self.unavailable = None;
                    self.deferred = 0;
                    for (index, entry) in entries.enumerate() {
                        if index >= 20000 {
                            self.deferred += 1;
                            break;
                        }
                        match entry {
                            Ok(entry) => {
                                if entry.path().extension().is_some_and(|e| e == "json") {
                                    selected.insert(entry.path());
                                }
                            }
                            Err(error) => {
                                self.unavailable =
                                    Some(format!("Evidence directory read failed: {error}"));
                            }
                        }
                    }
                    if self.deferred == 0 && self.unavailable.is_none() {
                        self.entries.retain(|p, _| selected.contains(p));
                    }
                }
                // No folder means the tool that writes there is not installed: no evidence, not a fault.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    self.unavailable = None;
                    self.deferred = 0;
                    self.entries.clear();
                }
                Err(error) => {
                    self.unavailable = Some(format!("Evidence directory unavailable: {error}"));
                }
            }
        } else {
            selected.extend(paths.iter().map(PathBuf::from).filter(|p| {
                p.parent() == Some(root) && p.extension().is_some_and(|e| e == "json")
            }));
        }
        let mut successful = false;
        for path in selected {
            if self.entries.len() >= 20000 && !self.entries.contains_key(&path) {
                self.deferred = self.deferred.saturating_add(1);
                continue;
            }
            let stamp = match Stamp::read(&path) {
                Ok(stamp) => stamp,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    self.entries.remove(&path);
                    continue;
                }
                Err(error) => {
                    self.unavailable = Some(format!("Evidence metadata unavailable: {error}"));
                    continue;
                }
            };
            if self
                .entries
                .get(&path)
                .is_some_and(|old| old.stamp == stamp)
            {
                continue;
            }
            self.reads += 1;
            let result = if stamp.length > 1024 * 1024 {
                Err("Evidence exceeds 1MiB bound".into())
            } else {
                parse(&path)
            };
            match result {
                Ok(value) if value.at() <= time.saturating_add(300) => {
                    let prior = self.entries.get(&path).and_then(|e| e.value.as_ref());
                    if prior.is_some_and(|old| {
                        (old.at(), old.sequence()) > (value.at(), value.sequence())
                    }) {
                        let value = prior.cloned();
                        self.entries.insert(
                            path,
                            Cached {
                                stamp,
                                value,
                                malformed: true,
                            },
                        );
                    } else {
                        self.entries.insert(
                            path,
                            Cached {
                                stamp,
                                value: Some(value),
                                malformed: false,
                            },
                        );
                        successful = true;
                    }
                }
                _ => {
                    let value = self.entries.get(&path).and_then(|e| e.value.clone());
                    self.entries.insert(
                        path,
                        Cached {
                            stamp,
                            value,
                            malformed: true,
                        },
                    );
                }
            }
        }
        self.entries.retain(|path, cached| {
            if cached
                .value
                .as_ref()
                .is_some_and(|v| v.at() < time - 30 * 86400)
            {
                if owned {
                    if let Err(error) = std::fs::remove_file(path) {
                        if error.kind() != std::io::ErrorKind::NotFound {
                            self.unavailable = Some(format!(
                                "Expired lifecycle record could not be removed: {error}"
                            ));
                            return true;
                        }
                    }
                }
                return false;
            }
            true
        });
        if (successful || self.last_success.is_none() && full)
            && self.unavailable.is_none()
            && self.entries.values().all(|v| !v.malformed)
        {
            self.last_success = Some(time);
        }
    }
    fn values(&self) -> impl Iterator<Item = &T> {
        self.entries.values().filter_map(|e| e.value.as_ref())
    }
    fn health(&self, name: &str) -> Health {
        let malformed = self.entries.values().filter(|e| e.malformed).count() as u64;
        let count = self.values().count();
        Health{state:if self.unavailable.is_some()||malformed>0{if count>0{SourceState::Stale}else{SourceState::Unavailable}}else if count==0{SourceState::Empty}else{SourceState::Fresh},last_success:self.last_success,malformed,deferred:self.deferred,
            message:format!("{count} cached {name} observations; {malformed} malformed or regressing files retained and excluded{}",self.unavailable.as_ref().map(|e|format!("; {e}")).unwrap_or_default()),retry_at:None}
    }
}
fn parse_progress(path: &std::path::Path) -> Result<Progress, String> {
    let value: serde_json::Value = read_json(path)?;
    let at = value
        .get("updated_at")
        .and_then(serde_json::Value::as_f64)
        .filter(|v| v.is_finite() && *v > 0.0 && *v < i64::MAX as f64)
        .ok_or("Invalid progress timestamp")? as i64;
    let done = value
        .get("done")
        .and_then(serde_json::Value::as_u64)
        .ok_or("Invalid progress done")?;
    let total = value
        .get("total")
        .and_then(serde_json::Value::as_u64)
        .filter(|n| *n > 0 && *n <= u32::MAX as u64 && done <= *n)
        .ok_or("Invalid progress total")?;
    let label = value
        .get("label")
        .and_then(serde_json::Value::as_str)
        .ok_or("Invalid progress label")?
        .chars()
        .filter(|c| !c.is_control())
        .take(160)
        .collect();
    let id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|id| valid_identity(id))
        .ok_or("Invalid progress identity")?
        .to_string();
    Ok(Progress {
        at,
        id,
        label,
        done: done as u32,
        total: total as u32,
    })
}
fn valid_identity(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 200
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn parse_lifecycle(path: &std::path::Path) -> Result<LifecycleEvent, String> {
    let event: LifecycleEvent = read_json(path)?;
    if event.version != 1
        || !["claude", "codex"].contains(&event.assistant.as_str())
        || !valid_identity(&event.id)
        || event.parent.as_ref().is_some_and(|p| !valid_identity(p))
        || event.label.chars().count() > 160
        || event.label.chars().any(char::is_control)
        || event.project_path.len() > 4096
        || event.at <= 0
    {
        return Err("Invalid lifecycle observation".into());
    }
    Ok(event)
}
fn state_times(snapshot: &Snapshot) -> std::collections::BTreeMap<(String, String), i64> {
    snapshot
        .sessions
        .iter()
        .map(|s| {
            (
                (s.assistant.clone(), s.id.clone()),
                if s.state == SessionState::Unknown {
                    i64::MIN
                } else {
                    s.updated_at
                },
            )
        })
        .collect()
}
fn progress(
    cache: &Evidence<Progress>,
    snapshot: &mut Snapshot,
    proofs: &mut std::collections::BTreeMap<(String, String), i64>,
) {
    for value in cache.values() {
        if let Some(session) = snapshot
            .sessions
            .iter_mut()
            .find(|s| s.assistant == "codex" && s.id == value.id)
        {
            if session.progress.is_none() {
                session.progress = Some((value.done, value.total));
            }
            let proof = proofs
                .entry((session.assistant.clone(), session.id.clone()))
                .or_insert(i64::MIN);
            if value.at >= *proof {
                if value.done == value.total {
                    session.state = SessionState::Completed;
                    *proof = value.at;
                } else if value.label.to_lowercase().starts_with("blocked:") {
                    session.state = SessionState::Blocked;
                    *proof = value.at;
                }
            }
            if !value.label.is_empty() {
                session.task = value.label.clone();
            }
            session.updated_at = session.updated_at.max(value.at);
        }
    }
    snapshot
        .health
        .insert("progress".into(), cache.health("progress"));
}
fn lifecycle(
    cache: &Evidence<LifecycleEvent>,
    snapshot: &mut Snapshot,
    proofs: &mut std::collections::BTreeMap<(String, String), i64>,
) {
    let mut latest = std::collections::BTreeMap::new();
    for event in cache.values() {
        let key = (&event.assistant, &event.id);
        if latest
            .get(&key)
            .is_none_or(|old: &&LifecycleEvent| (old.at, old.sequence) < (event.at, event.sequence))
        {
            latest.insert(key, event);
        }
    }
    for event in latest.into_values() {
        let index = snapshot
            .sessions
            .iter()
            .position(|s| s.id == event.id && s.assistant == event.assistant)
            .unwrap_or_else(|| {
                snapshot.sessions.push(Session {
                    id: event.id.clone(),
                    assistant: event.assistant.clone(),
                    project: std::path::Path::new(&event.project_path)
                        .file_name()
                        .map(|s| s.to_string_lossy().into())
                        .unwrap_or_else(|| "Unknown project".into()),
                    project_path: event.project_path.clone(),
                    children_supported: true,
                    ..Session::default()
                });
                snapshot.sessions.len() - 1
            });
        let session = &mut snapshot.sessions[index];
        let proof = proofs
            .entry((session.assistant.clone(), session.id.clone()))
            .or_insert(i64::MIN);
        if event.at >= *proof {
            *proof = event.at;
            session.state = event.state.clone();
            session.updated_at = session.updated_at.max(event.at);
            session.task = event.label.clone();
            if event.parent.is_some() {
                session.parent = event.parent.clone();
            }
            session.children_supported = true;
            if session.project_path.is_empty() {
                session.project_path = event.project_path.clone();
                session.project = std::path::Path::new(&event.project_path)
                    .file_name()
                    .map(|s| s.to_string_lossy().into())
                    .unwrap_or_else(|| "Unknown project".into());
            }
        }
    }
    snapshot
        .health
        .insert("lifecycle".into(), cache.health("lifecycle"));
}
fn merge_local(snapshot: &mut Snapshot, local: Snapshot) {
    snapshot.tokens = local.tokens;
    snapshot.sessions = local.sessions;
    if !local.health.contains_key("progress") {
        snapshot.health.remove("progress");
    }
    for (key, health) in local.health {
        if key.ends_with("-logs")
            || ["sessions", "lifecycle", "progress", "local-watch"].contains(&key.as_str())
        {
            snapshot.health.insert(key, health);
        }
    }
}
fn persist(
    root: &std::path::Path,
    state: &LocalState,
    snapshot: &RwLock<Snapshot>,
    dirty: &mut bool,
    saved_revision: &mut u64,
) {
    if *dirty {
        match atomic_json(&root.join("local.json"), state) {
            Ok(()) => *dirty = false,
            Err(error) => update(snapshot, |s| {
                s.health.insert(
                    "storage".into(),
                    Health {
                        state: SourceState::Error,
                        message: format!("Save local checkpoint: {error}"),
                        ..Health::default()
                    },
                );
            }),
        }
    }
    let current = snapshot.read().map(|s| s.clone());
    if let Ok(mut current) = current {
        if current.revision != *saved_revision {
            // Memory event bodies are reconstructed from the declared source, never persisted here.
            current.memory.clear();
            match atomic_json(&root.join("snapshot.json"), &current) {
                Ok(()) => *saved_revision = current.revision,
                Err(error) => update(snapshot, |s| {
                    s.health.insert(
                        "storage".into(),
                        Health {
                            state: SourceState::Error,
                            message: format!("Save snapshot: {error}"),
                            ..Health::default()
                        },
                    );
                }),
            }
        }
    }
}
fn local(store: Arc<Store>, snapshot: Arc<RwLock<Snapshot>>, stop: Arc<AtomicBool>) {
    let path = store.root().join("local.json");
    let mut state: LocalState = if path.exists() {
        match read_json(&path) {
            Ok(state) => state,
            Err(error) => {
                update(&snapshot, |s| {
                    s.health.insert(
                        "sessions".into(),
                        Health {
                            state: SourceState::Error,
                            message: error,
                            ..Health::default()
                        },
                    );
                });
                return;
            }
        }
    } else {
        LocalState::default()
    };
    if state.version > 2 {
        update(&snapshot, |s| {
            s.health.insert(
                "sessions".into(),
                Health {
                    state: SourceState::Error,
                    message: "Unsupported local checkpoint; original retained".into(),
                    ..Health::default()
                },
            );
        });
        return;
    }
    let upgraded = state.version < 2;
    let rebuilt = state.upgrade();
    if upgraded && rebuilt > 0 {
        update(&snapshot, |s| {
            s.health.insert("migration".into(),Health{state:SourceState::Loading,message:format!("Rebuilding {rebuilt} derived Codex token observations from original transcripts for checkpoint version2; Claude observations retained"),..Health::default()});
        });
    }
    let mut sources = SourceConfig::default();
    let mut changes = Changes::new(vec![]);
    let mut first = true;
    let mut watched: BTreeSet<PathBuf> = BTreeSet::new();
    let mut last_discovery: Option<Instant> = None;
    let mut last_save = Instant::now();
    let mut last_full: Option<Instant> = None;
    let mut saved_revision = 0;
    let mut dirty = upgraded;
    let mut progress_cache = Evidence::<Progress>::default();
    let mut lifecycle_cache = Evidence::<LifecycleEvent>::default();
    while !stop.load(Ordering::Acquire) {
        let Ok(saved) = store.configuration() else {
            break;
        };
        let time = now();
        let config_changed = first || saved.config.sources != sources;
        if config_changed {
            sources = saved.config.sources;
            // A missing watched directory forces 2-second polling; the collector owns this one.
            let _ = private_events_dir(store.root());
            watched = BTreeSet::new();
            changes = transcript_changes(&sources, store.root(), &watched);
            first = false;
            last_discovery = None;
            last_full = None;
        }
        let full = last_full.is_none_or(|t| t.elapsed() >= changes.interval());
        if full {
            last_full = Some(Instant::now());
        }
        let discovery = changes.discovery.swap(false, Ordering::AcqRel)
            || last_discovery.is_none_or(|t| t.elapsed() >= changes.interval());
        let error = if discovery {
            last_discovery = Some(Instant::now());
            match state.discover(&sources, time) {
                Ok(count) => {
                    dirty |= count > 0;
                    if state.watch_dirs != watched {
                        // Watch only project folders that hold transcripts; keep queued events.
                        watched = state.watch_dirs.clone();
                        let pending = changes.take();
                        changes = transcript_changes(&sources, store.root(), &watched);
                        changes.absorb(pending);
                    }
                    None
                }
                Err(error) => Some(error),
            }
        } else {
            None
        };
        let changed_paths = changes.take();
        let relevant = !changed_paths.is_empty()
            || full
            || discovery
            || state.cursors.values().any(|c| c.deferred > 0);
        if relevant {
            // The progress folder is an optional extra: sessions find their own progress in
            // their transcripts (SQ-38). A missing folder is not a fault.
            let extra_progress = !sources.progress.is_empty() && expand(&sources.progress).is_dir();
            if extra_progress {
                progress_cache.refresh(
                    &expand(&sources.progress),
                    &changed_paths,
                    full,
                    time,
                    parse_progress,
                    false,
                );
            } else {
                progress_cache = Evidence::<Progress>::default();
            }
            lifecycle_cache.refresh(
                &store.root().join("events"),
                &changed_paths,
                full,
                time,
                parse_lifecycle,
                true,
            );
            let mut local = snapshot.read().map(|s| s.clone()).unwrap_or_default();
            dirty |= state.poll_selected(
                &mut local,
                time,
                if full || discovery {
                    None
                } else {
                    Some(&changed_paths)
                },
            );
            // Reapply cached observations after transcript reconstruction, including hook-only children.
            let mut proofs = state_times(&local);
            lifecycle(&lifecycle_cache, &mut local, &mut proofs);
            progress(&progress_cache, &mut local, &mut proofs);
            if !extra_progress {
                local.health.remove("progress");
            }
            local.health.insert("sessions".into(),Health{state:if error.is_some(){SourceState::Error}else if local.sessions.is_empty(){SourceState::Empty}else{SourceState::Fresh},message:error.unwrap_or_else(||"Explicit lifecycle/progress evidence; task labels may be unavailable for older sessions".into()),..Health::default()});
            let watch = changes.health();
            let mut evidence_health = vec![lifecycle_cache.health("lifecycle"), watch.clone()];
            if extra_progress {
                evidence_health.push(progress_cache.health("progress"));
            }
            if let Some(health) = local.health.get_mut("sessions") {
                for evidence in &evidence_health {
                    if matches!(
                        evidence.state,
                        SourceState::Stale | SourceState::Unavailable | SourceState::Error
                    ) {
                        health.state = SourceState::Stale;
                        health.message.push_str(&format!("; {}", evidence.message));
                    }
                    health.malformed += evidence.malformed;
                    health.deferred += evidence.deferred;
                }
            }
            local.health.insert("local-watch".into(), watch);
            update(&snapshot, |s| merge_local(s, local));
        }
        if last_save.elapsed() >= Duration::from_secs(10) {
            persist(
                store.root(),
                &state,
                &snapshot,
                &mut dirty,
                &mut saved_revision,
            );
            last_save = Instant::now();
        }
        sleep(&stop, Duration::from_millis(500));
    }
    persist(
        store.root(),
        &state,
        &snapshot,
        &mut dirty,
        &mut saved_revision,
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    // REQ SQ-23 SQ-19
    #[test]
    fn owned_events_directory_is_private_and_avoids_polling_fallback() {
        let scratch = root("events-dir");
        let root = scratch.to_path_buf();
        private_events_dir(&root).unwrap();
        // Windows has no mode bits: its per-user profile folders are the protection there.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(root.join("events"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        assert!(!Changes::new(vec![root.join("events")]).fallback());
        std::fs::remove_dir_all(root).unwrap();
    }
    fn root(name: &str) -> crate::scratch::Scratch {
        let root = crate::scratch::Scratch::new(&format!("sq-workers-{name}"));
        std::fs::create_dir_all(root.join("events")).unwrap();
        root
    }
    // REQ SQ-8 SQ-9
    #[test]
    fn explicit_hook_state_beats_newer_unknown_transcript() {
        let scratch = root("unknown");
        let root = scratch.to_path_buf();
        let event = lifecycle_event(
            "claude",
            "PermissionRequest",
            &serde_json::json!({"session_id":"session","cwd":"/tmp/project"}),
            100,
            1,
        )
        .unwrap();
        atomic_json(&root.join("events/session.json"), &event).unwrap();
        let mut snapshot = Snapshot::default();
        snapshot.sessions.push(Session {
            id: "session".into(),
            assistant: "claude".into(),
            state: SessionState::Unknown,
            updated_at: 200,
            ..Session::default()
        });
        let mut cache = Evidence::default();
        cache.refresh(
            &root.join("events"),
            &BTreeSet::new(),
            true,
            200,
            parse_lifecycle,
            true,
        );
        let mut proofs = state_times(&snapshot);
        lifecycle(&cache, &mut snapshot, &mut proofs);
        assert_eq!(snapshot.sessions[0].state, SessionState::Waiting);
        std::fs::remove_dir_all(root).unwrap();
    }
    // REQ SQ-9 SQ-18
    #[test]
    fn malformed_progress_is_visible() {
        let scratch = root("progress");
        let root = scratch.to_path_buf();
        std::fs::write(root.join("session.json"), "{broken").unwrap();
        let sources = SourceConfig {
            progress: root.to_string_lossy().into(),
            ..SourceConfig::default()
        };
        let mut snapshot = Snapshot::default();
        snapshot.sessions.push(Session {
            id: "session".into(),
            assistant: "codex".into(),
            ..Session::default()
        });
        let mut cache = Evidence::default();
        cache.refresh(
            &expand(&sources.progress),
            &BTreeSet::new(),
            true,
            200,
            parse_progress,
            false,
        );
        let mut proofs = state_times(&snapshot);
        progress(&cache, &mut snapshot, &mut proofs);
        assert_eq!(
            snapshot.health.get("progress").map(|h| h.malformed),
            Some(1)
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    // REQ SQ-9 SQ-19
    #[test]
    fn cached_evidence_avoids_idle_reparse_and_retains_good_value_during_torn_write() {
        let scratch = root("cached");
        let root = scratch.to_path_buf();
        let events = root.join("events");
        let path = events.join("claude-session.json");
        let mut event = lifecycle_event(
            "claude",
            "UserPromptSubmit",
            &serde_json::json!({"session_id":"session","cwd":"/tmp/project"}),
            100,
            2,
        )
        .unwrap();
        atomic_json(&path, &event).unwrap();
        let mut cache = Evidence::default();
        cache.refresh(&events, &BTreeSet::new(), true, 200, parse_lifecycle, true);
        assert_eq!(cache.reads, 1);
        for _ in 0..20 {
            cache.refresh(&events, &BTreeSet::new(), false, 200, parse_lifecycle, true);
        }
        cache.refresh(&events, &BTreeSet::new(), true, 200, parse_lifecycle, true);
        assert_eq!(cache.reads, 1);
        std::fs::write(&path, "{torn").unwrap();
        let paths = [path.to_string_lossy().into()].into_iter().collect();
        cache.refresh(&events, &paths, false, 200, parse_lifecycle, true);
        assert_eq!(cache.health("lifecycle").malformed, 1);
        assert_eq!(cache.values().next().unwrap().state, SessionState::Running);
        event.state = SessionState::Waiting;
        event.sequence = 3;
        atomic_json(&path, &event).unwrap();
        cache.refresh(&events, &paths, false, 200, parse_lifecycle, true);
        assert_eq!(cache.health("lifecycle").malformed, 0);
        event.state = SessionState::Completed;
        event.sequence = 1;
        atomic_json(&path, &event).unwrap();
        cache.refresh(&events, &paths, false, 200, parse_lifecycle, true);
        assert_eq!(cache.health("lifecycle").malformed, 1);
        assert_eq!(cache.values().next().unwrap().state, SessionState::Waiting);
        std::fs::remove_dir_all(root).unwrap();
    }
    // REQ SQ-30: no sampling without a system widget; real samples once one exists.
    #[test]
    fn system_worker_samples_only_while_a_system_widget_exists() {
        let scratch = root("system");
        let root = scratch.to_path_buf();
        let store = Arc::new(Store::new(root.clone()).unwrap());
        let snapshot = Arc::new(RwLock::new(Snapshot::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let (store, snapshot, stop) = (store.clone(), snapshot.clone(), stop.clone());
            thread::spawn(move || system(store, snapshot, stop))
        };
        thread::sleep(Duration::from_millis(1500));
        assert!(snapshot.read().unwrap().system.is_empty());
        assert_eq!(
            snapshot.read().unwrap().revision,
            0,
            "idle costs no revision"
        );
        let saved = store.configuration().unwrap();
        let mut config = saved.config;
        config.add_widget("system", "local");
        store
            .configure(saved.revision, ConfigAction::Replace(Box::new(config)))
            .unwrap();
        let waited = Instant::now();
        while snapshot.read().unwrap().system.is_empty()
            && waited.elapsed() < Duration::from_secs(8)
        {
            thread::sleep(Duration::from_millis(100));
        }
        stop.store(true, Ordering::Release);
        handle.join().unwrap();
        let sample = snapshot.read().unwrap().system[0].clone();
        assert!(sample.cpu.is_some_and(|c| (0.0..=100.0).contains(&c)));
        let _ = std::fs::remove_dir_all(root);
    }
    // REQ SQ-32: real RAM sample, and a GPU sample that never invents numbers while asleep.
    #[test]
    fn ram_and_gpu_workers_sample_real_hardware_without_inventing_gpu_values() {
        let scratch = root("ramgpu");
        let root = scratch.to_path_buf();
        let store = Arc::new(Store::new(root.clone()).unwrap());
        let saved = store.configuration().unwrap();
        let mut config = saved.config;
        config.add_widget("ram", "local");
        config.add_widget("gpu", "local");
        store
            .configure(saved.revision, ConfigAction::Replace(Box::new(config)))
            .unwrap();
        let snapshot = Arc::new(RwLock::new(Snapshot::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let handles: Vec<_> = {
            let (a, b) = (
                (store.clone(), snapshot.clone(), stop.clone()),
                (store.clone(), snapshot.clone(), stop.clone()),
            );
            vec![
                thread::spawn(move || system(a.0, a.1, a.2)),
                thread::spawn(move || gpu(b.0, b.1, b.2)),
            ]
        };
        let waited = Instant::now();
        while waited.elapsed() < Duration::from_secs(8) {
            let current = snapshot.read().unwrap();
            if !current.system.is_empty() && !current.gpu.is_empty() {
                break;
            }
            drop(current);
            thread::sleep(Duration::from_millis(100));
        }
        stop.store(true, Ordering::Release);
        for handle in handles {
            handle.join().unwrap();
        }
        let current = snapshot.read().unwrap();
        let ram = current.system[0].ram.expect("meminfo readable");
        assert!(ram.total > 0 && ram.used <= ram.total);
        for sample in current.gpu.iter().filter(|g| g.state != GpuState::Reading) {
            assert!(sample.util.is_none() && sample.temp.is_none() && sample.clock.is_none());
        }
        let _ = std::fs::remove_dir_all(root);
    }
    // REQ SQ-8 SQ-9
    #[test]
    fn cached_child_survives_reconstruction_and_state_evidence_orders_independently_of_unknown_metadata(
    ) {
        let scratch = root("ordering");
        let root = scratch.to_path_buf();
        let events = root.join("events");
        let progress_root = root.join("progress");
        std::fs::create_dir(&progress_root).unwrap();
        for (name, input, kind, at) in [
            (
                "main",
                serde_json::json!({"session_id":"session","cwd":"/tmp/project"}),
                "PermissionRequest",
                100,
            ),
            (
                "child",
                serde_json::json!({"session_id":"session","agent_id":"child","cwd":"/tmp/project"}),
                "SubagentStart",
                120,
            ),
        ] {
            let event = lifecycle_event("codex", kind, &input, at, 1).unwrap();
            atomic_json(&events.join(format!("{name}.json")), &event).unwrap();
        }
        atomic_json(&progress_root.join("session.json"),&serde_json::json!({"done":1,"total":3,"updated_at":150.0,"label":"Blocked: waiting on build"})).unwrap();
        let mut lifecycle_cache = Evidence::default();
        lifecycle_cache.refresh(&events, &BTreeSet::new(), true, 200, parse_lifecycle, true);
        let mut progress_cache = Evidence::default();
        progress_cache.refresh(
            &progress_root,
            &BTreeSet::new(),
            true,
            200,
            parse_progress,
            false,
        );
        for _ in 0..2 {
            let mut snapshot = Snapshot::default();
            snapshot.sessions.push(Session {
                id: "session".into(),
                assistant: "codex".into(),
                state: SessionState::Unknown,
                updated_at: 200,
                ..Session::default()
            });
            let mut proofs = state_times(&snapshot);
            lifecycle(&lifecycle_cache, &mut snapshot, &mut proofs);
            progress(&progress_cache, &mut snapshot, &mut proofs);
            assert_eq!(
                snapshot
                    .sessions
                    .iter()
                    .find(|s| s.id == "session")
                    .unwrap()
                    .state,
                SessionState::Blocked
            );
            let child = snapshot.sessions.iter().find(|s| s.id == "child").unwrap();
            assert_eq!(child.parent.as_deref(), Some("session"));
            assert_eq!(child.state, SessionState::Running);
            let mut shared = Snapshot::default();
            merge_local(&mut shared, snapshot);
            assert!(shared.health.contains_key("lifecycle"));
            assert!(shared.health.contains_key("progress"));
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    // REQ SQ-11 SQ-18
    #[test]
    fn memory_failure_retains_previous_events_and_success_timestamp() {
        let mut snapshot = Snapshot::default();
        snapshot.memory.push(MemoryItem {
            id: "previous".into(),
            body: "Keep this event".into(),
            ..MemoryItem::default()
        });
        snapshot.health.insert(
            "memory".into(),
            Health {
                state: SourceState::Fresh,
                last_success: Some(100),
                ..Health::default()
            },
        );
        merge_memory(
            &mut snapshot,
            vec![],
            Health {
                state: SourceState::Unavailable,
                message: "Directory unavailable".into(),
                ..Health::default()
            },
        );
        assert_eq!(snapshot.memory[0].body, "Keep this event");
        assert_eq!(snapshot.health["memory"].last_success, Some(100));
        assert_eq!(snapshot.health["memory"].state, SourceState::Stale);
    }
    // REQ SQ-19
    #[test]
    fn watch_failure_exposes_two_second_fallback() {
        let scratch = root("watch");
        let root = scratch.to_path_buf();
        // A folder that does not exist is a tool that is not installed, not a failure.
        let changes = Changes::new(vec![root.join("absent")]);
        assert!(!changes.fallback());
        assert_eq!(changes.interval(), Duration::from_secs(30));
        // A folder that exists but cannot be watched is a failure and is shown as one.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let locked = root.join("locked");
            std::fs::create_dir_all(&locked).unwrap();
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
            let unreadable = std::fs::read_dir(&locked).is_err();
            let changes = Changes::new(vec![locked.clone()]);
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
            if unreadable {
                assert!(changes.fallback());
                assert_eq!(changes.interval(), Duration::from_secs(2));
                assert_eq!(changes.health().state, SourceState::Stale);
                assert!(changes
                    .health()
                    .message
                    .contains("2-second polling fallback"));
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    // REQ SQ-12 SQ-24
    #[test]
    fn persisted_snapshot_omits_memory_bodies_and_save_failure_is_visible() {
        let scratch = root("persist");
        let root = scratch.to_path_buf();
        let mut current = Snapshot {
            revision: 10,
            ..Default::default()
        };
        current.memory.push(MemoryItem {
            id: "private-memory".into(),
            body: "sensitive complete memory body".into(),
            ..MemoryItem::default()
        });
        let snapshot = RwLock::new(current);
        let mut dirty = true;
        let mut revision = 0;
        persist(
            &root,
            &LocalState::default(),
            &snapshot,
            &mut dirty,
            &mut revision,
        );
        let saved: Snapshot = read_json(&root.join("snapshot.json")).unwrap();
        assert!(saved.memory.is_empty());
        assert_eq!(snapshot.read().unwrap().memory.len(), 1);
        assert!(!dirty);
        assert!(!std::fs::read_to_string(root.join("snapshot.json"))
            .unwrap()
            .contains("sensitive complete memory body"));
        let impossible = root.join("not-a-directory");
        std::fs::write(&impossible, "keep").unwrap();
        dirty = true;
        persist(
            &impossible,
            &LocalState::default(),
            &snapshot,
            &mut dirty,
            &mut revision,
        );
        assert!(dirty);
        assert_eq!(
            snapshot.read().unwrap().health["storage"].state,
            SourceState::Error
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    // REQ SQ-9 SQ-19 SQ-24
    #[test]
    fn real_local_worker_observes_new_hook_within_two_seconds_and_flushes_on_stop() {
        let scratch = root("live-local");
        let root = scratch.to_path_buf();
        let store = Arc::new(Store::new(root.clone()).unwrap());
        let mut config = Config::default();
        for name in ["claude/projects", "codex/sessions", "progress"] {
            std::fs::create_dir_all(root.join(name)).unwrap();
        }
        config.sources.claude = root.join("claude").to_string_lossy().into();
        config.sources.codex = root.join("codex").to_string_lossy().into();
        config.sources.progress = root.join("progress").to_string_lossy().into();
        store
            .configure(0, ConfigAction::Replace(Box::new(config)))
            .unwrap();
        let snapshot = Arc::new(RwLock::new(Snapshot::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let store = store.clone();
            let snapshot = snapshot.clone();
            let stop = stop.clone();
            thread::spawn(move || local(store, snapshot, stop))
        };
        let ready = Instant::now();
        while !snapshot.read().unwrap().health.contains_key("sessions")
            && ready.elapsed() < Duration::from_secs(2)
        {
            thread::sleep(Duration::from_millis(10));
        }
        let event=lifecycle_event("claude","PermissionRequest",&serde_json::json!({"session_id":"live-session","cwd":"/tmp/project","prompt":"never retain this prompt"}),now(),1).unwrap();
        let start = Instant::now();
        atomic_json(&root.join("events/live.json"), &event).unwrap();
        let mut observed = false;
        while start.elapsed() < Duration::from_secs(2) {
            observed = snapshot
                .read()
                .unwrap()
                .sessions
                .iter()
                .any(|s| s.id == "live-session" && s.state == SessionState::Waiting);
            if observed {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        stop.store(true, Ordering::Release);
        worker.join().unwrap();
        assert!(observed, "Hook missed2seconddeadline");
        assert!(root.join("local.json").exists());
        let saved = std::fs::read_to_string(root.join("snapshot.json")).unwrap();
        assert!(saved.contains("live-session"));
        assert!(!saved.contains("never retain this prompt"));
        std::fs::remove_dir_all(root).unwrap();
    }

    // REQ SQ-36: a tool that is not installed has no evidence folder, and that is not a fault.
    #[test]
    fn a_missing_evidence_folder_is_empty_not_unavailable() {
        let scratch = root("missing-evidence");
        let root = scratch.to_path_buf();
        let mut evidence: Evidence<Progress> = Evidence::default();
        evidence.refresh(
            &root.join("not-installed"),
            &BTreeSet::new(),
            true,
            100,
            |_| Err("never read".into()),
            false,
        );
        assert!(evidence.unavailable.is_none());
        assert_eq!(evidence.health("progress").state, SourceState::Empty);
        std::fs::remove_dir_all(root).unwrap();
    }
}
