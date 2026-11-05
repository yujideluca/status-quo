//! Desktop end-to-end test of the real tray: starts the production binary against a private
//! state directory, drives it with a virtual mouse made through /dev/uinput, and checks what the
//! tray saved. It needs a Wayland layer-shell desktop and write access to /dev/uinput, so it is
//! ignored by default. Run it with `tools/ui_e2e.sh` before closing any major tray update; it
//! writes screenshots and `report.json` to `build/e2e/`. The numbers it expects come from the
//! manual test recorded in `tests/e2e/manual-baseline.json`.
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

extern "C" {
    fn ioctl(fd: i32, request: u64, ...) -> i32;
}

/// A virtual absolute-position mouse.
struct Mouse {
    file: fs::File,
}
impl Mouse {
    fn new(width: i32, height: i32) -> Result<Self, String> {
        use std::os::unix::io::AsRawFd;
        let file = fs::OpenOptions::new()
            .write(true)
            .open("/dev/uinput")
            .map_err(|e| format!("/dev/uinput: {e}"))?;
        let fd = file.as_raw_fd();
        unsafe {
            ioctl(fd, 0x4004_5564, 1); // UI_SET_EVBIT EV_KEY
            ioctl(fd, 0x4004_5564, 3); // EV_ABS
            ioctl(fd, 0x4004_5565, 0x110); // UI_SET_KEYBIT BTN_LEFT
            ioctl(fd, 0x4004_5567, 0); // UI_SET_ABSBIT ABS_X
            ioctl(fd, 0x4004_5567, 1); // ABS_Y
        }
        let mut device = vec![0u8; 80 + 8 + 4 + 4 * 64 * 4];
        let name = b"sq-e2e-pointer";
        device[..name.len()].copy_from_slice(name);
        device[80..82].copy_from_slice(&3u16.to_ne_bytes()); // BUS_USB
        device[82..84].copy_from_slice(&0x1u16.to_ne_bytes());
        device[84..86].copy_from_slice(&0x1u16.to_ne_bytes());
        let max_at = 80 + 8 + 4;
        device[max_at..max_at + 4].copy_from_slice(&(width - 1).to_ne_bytes());
        device[max_at + 4..max_at + 8].copy_from_slice(&(height - 1).to_ne_bytes());
        let mut mouse = Self { file };
        mouse.file.write_all(&device).map_err(|e| e.to_string())?;
        if unsafe { ioctl(fd, 0x5501) } != 0 {
            return Err("UI_DEV_CREATE failed".into());
        }
        sleep(Duration::from_millis(1200));
        Ok(mouse)
    }
    fn event(&mut self, kind: u16, code: u16, value: i32) {
        let mut bytes = Vec::with_capacity(24);
        bytes.extend_from_slice(&[0u8; 16]);
        bytes.extend_from_slice(&kind.to_ne_bytes());
        bytes.extend_from_slice(&code.to_ne_bytes());
        bytes.extend_from_slice(&value.to_ne_bytes());
        let _ = self.file.write_all(&bytes);
    }
    fn sync(&mut self) {
        self.event(0, 0, 0);
        sleep(Duration::from_millis(40));
    }
    fn move_to(&mut self, x: f32, y: f32) {
        self.event(3, 0, x as i32);
        self.event(3, 1, y as i32);
        self.sync();
    }
    fn button(&mut self, down: bool) {
        self.event(1, 0x110, down as i32);
        self.sync();
        sleep(Duration::from_millis(80));
    }
    fn click(&mut self, x: f32, y: f32) {
        // Hover first: the tray only learns where the pointer is after its next frame, and the
        // first pointer event after the surface changed size can be lost.
        self.move_to(x - 2.0, y);
        sleep(Duration::from_millis(250));
        self.move_to(x, y);
        sleep(Duration::from_millis(250));
        self.button(true);
        self.button(false);
        sleep(Duration::from_millis(900));
    }
    fn drag(&mut self, from: (f32, f32), to: (f32, f32)) {
        self.move_to(from.0, from.1);
        sleep(Duration::from_millis(100));
        self.button(true);
        for step in 1..=12 {
            let t = step as f32 / 12.0;
            self.move_to(from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
        }
        sleep(Duration::from_millis(200));
        self.button(false);
        sleep(Duration::from_millis(800));
    }
}
impl Drop for Mouse {
    fn drop(&mut self) {
        use std::os::unix::io::AsRawFd;
        unsafe { ioctl(self.file.as_raw_fd(), 0x5502) }; // UI_DEV_DESTROY
    }
}

/// Everything one run owns: private state, the processes, the report.
struct Run {
    state: PathBuf,
    out: PathBuf,
    panel: Option<Child>,
    screen: (i32, i32),
    live_panel_was_active: bool,
    checks: Vec<Value>,
    data: serde_json::Map<String, Value>,
}
impl Run {
    fn config(&self) -> Value {
        fs::read_to_string(self.state.join("config.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .map(|v| v.get("config").cloned().unwrap_or(v))
            .unwrap_or_else(|| serde_json::to_value(sq_core::Config::default()).unwrap())
    }
    fn revision(&self) -> i64 {
        fs::read_to_string(self.state.join("config.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v["revision"].as_i64())
            .unwrap_or(0)
    }
    fn widget(&self, id: &str) -> Value {
        self.config()["widgets"]
            .as_array()
            .and_then(|w| w.iter().find(|w| w["id"] == id).cloned())
            .unwrap_or(Value::Null)
    }
    fn check(&mut self, name: &str, ok: bool, detail: impl Into<String>) {
        let detail = detail.into();
        eprintln!("{} {name}: {detail}", if ok { "ok  " } else { "FAIL" });
        self.checks
            .push(json!({"check": name, "ok": ok, "detail": detail}));
    }
    fn near(&mut self, name: &str, got: f64, want: f64, tolerance: f64) {
        self.check(
            name,
            (got - want).abs() <= tolerance,
            format!("got {got:.1}, want {want:.1} ± {tolerance}"),
        );
    }
    fn shot(&self, name: &str) {
        let _ = Command::new("spectacle")
            .args(["-b", "-n", "-f", "-o"])
            .arg(self.out.join(format!("{name}.png")))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        sleep(Duration::from_millis(400));
    }
    fn start_panel(&mut self) {
        let mut command = Command::new(panel_binary());
        if std::env::var("STATUS_QUO_E2E_EMPTY_HOME").as_deref() == Ok("1") {
            // Layout checks without the cold read of every real transcript, which can keep a
            // private collector busy for minutes and starve the virtual mouse. The idle CPU
            // figure of such a run says nothing about the real load.
            let home = self.out.join("home");
            let _ = fs::create_dir_all(&home);
            command.env("HOME", home);
        }
        let child = command
            .env("STATUS_QUO_STATE_DIR", &self.state)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(fs::File::create(self.out.join("panel.log")).unwrap())
            .spawn()
            .expect("start the panel");
        self.panel = Some(child);
        sleep(Duration::from_secs(7));
    }
    fn panel_alive(&mut self) -> bool {
        self.panel
            .as_mut()
            .is_some_and(|c| matches!(c.try_wait(), Ok(None)))
    }
    fn stop_panel(&mut self) -> Duration {
        let started = Instant::now();
        if let Some(mut child) = self.panel.take() {
            let _ = Command::new("kill").arg(child.id().to_string()).status();
            for _ in 0..80 {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    break;
                }
                sleep(Duration::from_millis(100));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        started.elapsed()
    }
    /// Processes started with this run's state directory (the panel and its collector).
    fn pids(&self) -> Vec<(i32, bool)> {
        let wanted = format!("STATUS_QUO_STATE_DIR={}", self.state.display());
        let mut found = Vec::new();
        for entry in fs::read_dir("/proc").into_iter().flatten().flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
                continue;
            };
            let env = fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();
            if env.split(|b| *b == 0).any(|v| v == wanted.as_bytes()) {
                let cmd = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
                let collector = cmd.windows(7).any(|w| w == b"collect");
                found.push((pid, collector));
            }
        }
        found
    }
}
impl Drop for Run {
    fn drop(&mut self) {
        self.stop_panel();
        for (pid, _) in self.pids() {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
        if self.live_panel_was_active {
            let _ = Command::new("systemctl")
                .args(["--user", "start", "status-quo-panel.service"])
                .status();
        }
    }
}

/// The production binary: performance claims need the release build, so the runner passes it in.
fn panel_binary() -> PathBuf {
    std::env::var_os("STATUS_QUO_E2E_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_status-quo")))
}
fn cpu_ticks(pid: i32) -> u64 {
    let text = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    let after = text.rsplit(')').next().unwrap_or("");
    let fields: Vec<&str> = after.split_whitespace().collect();
    fields
        .get(11)
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0)
        + fields
            .get(12)
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
}
fn rss_mib(pid: i32) -> f64 {
    fs::read_to_string(format!("/proc/{pid}/status"))
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|v| v.split_whitespace().next()?.parse::<f64>().ok())
        .unwrap_or(0.0)
        / 1024.0
}
fn png_size(path: &Path) -> Option<(i32, i32)> {
    let bytes = fs::read(path).ok()?;
    if bytes.len() < 24 || &bytes[1..4] != b"PNG" {
        return None;
    }
    let w = i32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = i32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((w, h))
}
/// Where the docked tray is on the screen: its saved position, kept on the screen like the host
/// does, or the first-run corner when nothing is saved yet. Returns x, y, width, height.
fn tray_rect(config: &Value, screen: (i32, i32), baseline: &Value) -> (f32, f32, f32, f32) {
    let g = &config["geometry"]["tray"];
    if let (Some(x), Some(y), Some(w), Some(h)) = (
        g["x"].as_f64(),
        g["y"].as_f64(),
        g["width"].as_f64(),
        g["height"].as_f64(),
    ) {
        let (w, h) = (w.ceil() as i32, h.ceil() as i32);
        let x = (x as i32).clamp(0, (screen.0 - w).max(0));
        let y = (y as i32).clamp(0, (screen.1 - h).max(0));
        return (x as f32, y as f32, w as f32, h as f32);
    }
    let first = &baseline["first_run_tray"];
    let (w, h, m) = (
        first["width"].as_f64().unwrap() as f32,
        first["height"].as_f64().unwrap() as f32,
        first["margin"].as_f64().unwrap() as f32,
    );
    (screen.0 as f32 - m - w, m, w, h)
}
/// Header buttons of the docked tray: Edit tray, Customize and Add widget, plus their row.
fn header_buttons(tray: (f32, f32, f32, f32), baseline: &Value) -> (f32, f32, f32, f32) {
    let b = &baseline["header_buttons_from_tray_right_edge"];
    let right = tray.0 + tray.2;
    (
        right - b["edit_tray"].as_f64().unwrap() as f32,
        right - b["customize"].as_f64().unwrap() as f32,
        right - b["add_widget"].as_f64().unwrap() as f32,
        tray.1 + b["y_from_tray_top"].as_f64().unwrap() as f32,
    )
}
/// A widget whose whole rectangle is on the screen, other than the ones in `skip`.
fn visible_widget(config: &Value, screen: (i32, i32), baseline: &Value, skip: &[&str]) -> String {
    let tray = tray_rect(config, screen, baseline);
    for w in config["widgets"].as_array().unwrap() {
        let id = w["id"].as_str().unwrap();
        if skip.contains(&id) {
            continue;
        }
        let a = screen_point(config, id, 0.0, 0.0, tray);
        let b = screen_point(config, id, 1.0, 1.0, tray);
        if a.0 > 20.0
            && a.1 > 60.0
            && b.0 < screen.0 as f32 - 20.0
            && b.1 < screen.1 as f32 - 20.0
            && b.0 - a.0 >= 200.0
            && b.1 - a.1 >= 100.0
        {
            return id.to_string();
        }
    }
    panic!("no widget is fully visible on the screen");
}
/// Screen position of a point inside a widget, from the saved free layout.
fn screen_point(
    config: &Value,
    id: &str,
    fx: f32,
    fy: f32,
    tray: (f32, f32, f32, f32),
) -> (f32, f32) {
    let geometry = &config["geometry"]["tray"];
    let widgets = config["widgets"].as_array().unwrap();
    let canvas_bottom = widgets
        .iter()
        .map(|w| w["y"].as_f64().unwrap() + w["height"].as_f64().unwrap())
        .fold(0.0, f64::max);
    let header = geometry["height"].as_f64().unwrap() - canvas_bottom;
    let w = widgets.iter().find(|w| w["id"] == id).unwrap();
    (
        tray.0 + w["x"].as_f64().unwrap() as f32 + fx * w["width"].as_f64().unwrap() as f32,
        tray.1
            + header as f32
            + w["y"].as_f64().unwrap() as f32
            + fy * w["height"].as_f64().unwrap() as f32,
    )
}

// REQ SQ-15 SQ-16 SQ-21 SQ-23 SQ-27 SQ-28 SQ-29
#[test]
#[ignore = "needs a Wayland layer-shell desktop and /dev/uinput; run tools/ui_e2e.sh"]
fn desktop_end_to_end() {
    assert_eq!(
        std::env::var("STATUS_QUO_E2E_SCREEN_IS_FREE").as_deref(),
        Ok("1"),
        "refused: the test clicks on the screen; clear it first (see tools/ui_e2e.sh)"
    );
    let baseline: Value = serde_json::from_str(include_str!("e2e/manual-baseline.json")).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../build/e2e");
    fs::create_dir_all(&root).unwrap();
    let out = root.canonicalize().unwrap();
    for old in fs::read_dir(&out).into_iter().flatten().flatten() {
        if old.path().extension().is_some_and(|e| e == "png") {
            let _ = fs::remove_file(old.path());
        }
    }
    let state = out.join("state");
    let _ = fs::remove_dir_all(&state);
    fs::create_dir_all(&state).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    // A fresh configuration is locked; the editor needs an unlocked one (the lock is unit-tested).
    let unlocked = sq_core::Config {
        locked: false,
        ..sq_core::Config::default()
    };
    fs::write(
        state.join("config.json"),
        serde_json::to_string(&json!({"revision": 1, "config": unlocked})).unwrap(),
    )
    .unwrap();

    // The live tray would share the screen with the test tray, so pause it and always restore it.
    let live = Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", "status-quo-panel.service"])
        .status()
        .is_ok_and(|s| s.success());
    if live {
        let _ = Command::new("systemctl")
            .args(["--user", "stop", "status-quo-panel.service"])
            .status();
        sleep(Duration::from_secs(4));
    }
    let mut run = Run {
        state,
        out,
        panel: None,
        screen: (1920, 1080),
        live_panel_was_active: live,
        checks: Vec::new(),
        data: Default::default(),
    };
    run.shot("00-screen");
    let screen = png_size(&run.out.join("00-screen.png")).expect("spectacle screenshot");
    run.screen = screen;
    let want = (
        baseline["screen"][0].as_i64().unwrap() as i32,
        baseline["screen"][1].as_i64().unwrap() as i32,
    );
    assert_eq!(
        screen, want,
        "the recorded positions assume the baseline screen size"
    );
    let mut mouse = Mouse::new(screen.0, screen.1).expect("virtual mouse");
    let centre = screen.0 as f32 / 2.0;
    let toolbar = &baseline["edit_toolbar_from_screen_centre"];
    let done = centre + toolbar["done"].as_f64().unwrap() as f32;
    let cancel = centre + toolbar["cancel"].as_f64().unwrap() as f32;
    let toolbar_y = toolbar["y"].as_f64().unwrap() as f32;
    let buttons =
        |run: &Run| header_buttons(tray_rect(&run.config(), screen, &baseline), &baseline);

    // 1. Start: the panel stays up on the first-run layout.
    run.start_panel();
    run.shot("01-start");
    let alive = run.panel_alive();
    run.check(
        "panel starts and stays up",
        alive,
        "process alive after 7 s",
    );
    let has_collector = run.pids().iter().any(|(_, collector)| *collector);
    run.check(
        "collector started for the private state",
        has_collector,
        "collect process found",
    );

    // 2. Idle cost over 20 seconds (SQ-23), once the first read of every transcript is done.
    let warm_start = Instant::now();
    loop {
        let sample: Vec<(i32, u64)> = run
            .pids()
            .iter()
            .map(|(p, _)| (*p, cpu_ticks(*p)))
            .collect();
        sleep(Duration::from_secs(5));
        let busy: f64 = sample
            .iter()
            .map(|(p, t)| cpu_ticks(*p).saturating_sub(*t) as f64 / 100.0 / 5.0 * 100.0)
            .sum();
        if busy < 3.0 || warm_start.elapsed() > Duration::from_secs(150) {
            run.data.insert(
                "warm_up_seconds".into(),
                json!(warm_start.elapsed().as_secs()),
            );
            break;
        }
    }
    let before: Vec<(i32, u64)> = run
        .pids()
        .iter()
        .map(|(p, _)| (*p, cpu_ticks(*p)))
        .collect();
    let started = Instant::now();
    sleep(Duration::from_secs(20));
    let seconds = started.elapsed().as_secs_f64();
    let (mut cpu, mut rss) = (0.0, 0.0);
    for (pid, ticks) in &before {
        cpu += cpu_ticks(*pid).saturating_sub(*ticks) as f64 / 100.0 / seconds * 100.0;
        rss += rss_mib(*pid);
    }
    run.data
        .insert("idle_cpu_percent_panel_plus_collector".into(), json!(cpu));
    run.data
        .insert("rss_mib_panel_plus_collector".into(), json!(rss));
    let target = baseline["limits"]["idle_cpu_percent_of_one_core_max"]
        .as_f64()
        .unwrap();
    run.data
        .insert("meets_sq23_cpu_target_over_20s".into(), json!(cpu < target));
    // The 1% target is still open (DESIGN.md section 5); the gate here is the accepted ceiling.
    run.check(
        "idle CPU stays under the accepted ceiling",
        cpu < target * 2.0,
        format!(
            "{cpu:.2}% of one core, ceiling {}%, target {target}%",
            target * 2.0
        ),
    );
    let rss_limit = baseline["limits"]["panel_plus_collector_rss_mib_max"]
        .as_f64()
        .unwrap();
    run.check(
        "memory of panel and collector",
        rss < rss_limit,
        format!("{rss:.0} MiB, limit {rss_limit} MiB"),
    );

    // 3. Edit tray, then Cancel twice: nothing is saved.
    let initial = run.config();
    let initial_revision = run.revision();
    let (edit_tray, _, _, button_y) = buttons(&run);
    mouse.click(edit_tray, button_y);
    run.shot("02-edit-mode");
    mouse.click(cancel, toolbar_y);
    mouse.click(edit_tray, button_y);
    mouse.click(cancel, toolbar_y);
    let unchanged = run.config() == initial && run.revision() == initial_revision;
    run.check(
        "Cancel leaves the saved configuration unchanged",
        unchanged,
        format!("revision {initial_revision} -> {}", run.revision()),
    );

    // 4. Edit tray, move one widget with its body, press Done.
    mouse.click(edit_tray, button_y);
    let first = initial["widgets"][0]["id"].as_str().unwrap().to_string();
    let drag = &baseline["observed"]["body_drag"]["pointer_delta"];
    let (dx, dy) = (
        drag[0].as_f64().unwrap().abs() as f32,
        drag[1].as_f64().unwrap().abs() as f32,
    );
    let tray = tray_rect(&initial, screen, &baseline);
    // A quarter in from the left: the widget centre holds its Remove button.
    let from = (tray.0 + tray.2 * 0.25, tray.1 + 70.0);
    mouse.drag(from, (from.0 + dx, from.1 + dy));
    run.shot("03-edit-after-move");
    mouse.click(done, toolbar_y);
    sleep(Duration::from_secs(2));
    run.shot("04-after-done");
    let saved = run.config();
    let widgets = saved["widgets"].as_array().unwrap();
    let explicit = widgets
        .iter()
        .all(|w| w["width"].as_f64().unwrap_or(0.0) > 0.0);
    run.check(
        "Done turns every widget into an explicit rectangle",
        explicit,
        format!("{} widgets", widgets.len()),
    );
    let moved = run.widget(&first);
    run.check(
        "body drag moved the first widget right and down",
        moved["x"].as_f64().unwrap() > 50.0 && moved["y"].as_f64().unwrap() > 50.0,
        format!("x {:.0}, y {:.0}", moved["x"], moved["y"]),
    );
    run.check(
        "Done records the tray position on the desktop",
        saved["geometry"]["tray"]["width"].as_f64().unwrap_or(0.0) >= 330.0,
        format!("{}", saved["geometry"]["tray"]),
    );
    let min_x = widgets
        .iter()
        .map(|w| w["x"].as_f64().unwrap())
        .fold(f64::MAX, f64::min);
    let min_y = widgets
        .iter()
        .map(|w| w["y"].as_f64().unwrap())
        .fold(f64::MAX, f64::min);
    run.check(
        "the saved canvas starts at its corner",
        min_x == 0.0 && min_y == 0.0,
        format!("min x {min_x}, min y {min_y}"),
    );
    let tray = tray_rect(&saved, screen, &baseline);
    let first_run_height = baseline["first_run_tray"]["height"].as_f64().unwrap();
    let docked_height = saved["geometry"]["tray"]["height"].as_f64().unwrap();
    run.check(
        "a first-run tray taller than its surface is frozen inside it",
        docked_height <= first_run_height + 130.0,
        format!("docked height {docked_height}, first-run surface {first_run_height}"),
    );
    run.check(
        "the docked tray stays on the screen",
        tray.0 >= 0.0
            && tray.1 >= 0.0
            && tray.0 + tray.2 <= screen.0 as f32
            && tray.1 + tray.3 <= screen.1 as f32 + 1.0
            || tray.3 > screen.1 as f32,
        format!("tray {tray:?} on {screen:?}"),
    );

    // 5. Resize by a corner, then by the left edge of another widget, both fully visible.
    let id = visible_widget(&saved, screen, &baseline, &[]);
    let w0 = run.widget(&id);
    let corner_delta = &baseline["observed"]["corner_drag"]["pointer_delta"];
    let (cx, cy) = (
        corner_delta[0].as_f64().unwrap() as f32,
        corner_delta[1].as_f64().unwrap() as f32,
    );
    let (edit_tray, _, _, button_y) = buttons(&run);
    mouse.click(edit_tray, button_y);
    let corner = screen_point(&saved, &id, 1.0, 1.0, tray);
    mouse.drag(
        (corner.0 - 4.0, corner.1 - 4.0),
        (corner.0 - 4.0 + cx, corner.1 - 4.0 + cy),
    );
    run.shot("05-edit-after-corner");
    mouse.click(done, toolbar_y);
    sleep(Duration::from_secs(2));
    let w1 = run.widget(&id);
    let tolerance = baseline["observed"]["corner_drag"]["size_delta_within"]
        .as_f64()
        .unwrap();
    run.near(
        "corner drag changes the width",
        w1["width"].as_f64().unwrap() - w0["width"].as_f64().unwrap(),
        cx as f64,
        tolerance,
    );
    run.near(
        "corner drag changes the height",
        w1["height"].as_f64().unwrap() - w0["height"].as_f64().unwrap(),
        cy as f64,
        tolerance,
    );

    let saved = run.config();
    let tray = tray_rect(&saved, screen, &baseline);
    let id2 = visible_widget(&saved, screen, &baseline, &[id.as_str()]);
    let w0 = run.widget(&id2);
    let left_delta = baseline["observed"]["left_edge_drag"]["pointer_delta"][0]
        .as_f64()
        .unwrap() as f32;
    let (edit_tray, _, _, button_y) = buttons(&run);
    mouse.click(edit_tray, button_y);
    let edge = screen_point(&saved, &id2, 0.0, 0.5, tray);
    mouse.drag((edge.0 + 3.0, edge.1), (edge.0 + 3.0 + left_delta, edge.1));
    mouse.click(done, toolbar_y);
    sleep(Duration::from_secs(2));
    let w1 = run.widget(&id2);
    let right = |w: &Value| w["x"].as_f64().unwrap() + w["width"].as_f64().unwrap();
    run.near(
        "left edge drag moves the left border",
        w1["x"].as_f64().unwrap() - w0["x"].as_f64().unwrap(),
        left_delta as f64,
        12.0,
    );
    run.near(
        "left edge drag keeps the right border",
        right(&w1) - right(&w0),
        0.0,
        12.0,
    );
    run.shot("06-after-left-edge");

    // 6. Widget controls in the docked tray: token range and cumulative.
    let saved = run.config();
    let tray = tray_rect(&saved, screen, &baseline);
    let tokens = saved["widgets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["kind"] == "tokens")
        .unwrap()
        .clone();
    let id = tokens["id"].as_str().unwrap().to_string();
    let corner = screen_point(&saved, &id, 0.0, 0.0, tray);
    let visible = corner.1 + 80.0 < screen.1 as f32;
    run.check(
        "a token widget is visible to test its controls",
        visible,
        format!("top-left {corner:?}"),
    );
    if visible {
        let controls = &baseline["token_controls_from_widget_top_left"];
        let at = |name: &str| {
            (
                corner.0 + controls[name][0].as_f64().unwrap() as f32,
                corner.1 + controls[name][1].as_f64().unwrap() as f32,
            )
        };
        let seven = at("range_7d");
        mouse.click(seven.0, seven.1);
        run.check(
            "7d button saves the range",
            run.widget(&id)["options"]["range"] == "7d",
            format!("{}", run.widget(&id)["options"]),
        );
        let cumulative = at("cumulative");
        mouse.click(cumulative.0, cumulative.1);
        run.check(
            "Cumulative checkbox saves",
            run.widget(&id)["options"]["cumulative"] == "true",
            format!("{}", run.widget(&id)["options"]),
        );
    }
    run.shot("07-docked");

    // 7. + Add widget adds a widget of the chosen kind.
    let count = run.config()["widgets"].as_array().unwrap().len();
    let (_, _, add_widget, button_y) = buttons(&run);
    mouse.click(add_widget, button_y);
    run.shot("08-add-menu");
    let below = baseline["add_menu"]["first_item_below_button_row"]
        .as_f64()
        .unwrap() as f32;
    let item_height = baseline["add_menu"]["item_height"].as_f64().unwrap() as f32;
    // The menu opens under the button row: item 4 is claude / Account usage.
    mouse.click(add_widget + 15.0, button_y + below + item_height * 4.0);
    let after = run.config();
    let widgets = after["widgets"].as_array().unwrap();
    run.check(
        "Add widget adds one widget",
        widgets.len() == count + 1,
        format!("{count} -> {}", widgets.len()),
    );
    if let Some(last) = widgets.last() {
        let x = last["x"].as_f64().unwrap_or(-1.0);
        let y = last["y"].as_f64().unwrap_or(-1.0);
        run.check(
            "the added widget lands at the top-left corner with its own rectangle",
            last["width"].as_f64().unwrap_or(0.0) > 0.0 && x == 0.0 && y == 0.0,
            format!("x {x}, y {y}, width {}", last["width"]),
        );
        run.check(
            "the added widget is claude / quota",
            last["kind"] == "quota" && last["provider"] == "claude",
            format!("{} / {}", last["provider"], last["kind"]),
        );
    }

    // 8. Customize opens the settings page and Cancel leaves the configuration alone.
    let before = run.config();
    let (_, customize, _, button_y) = buttons(&run);
    mouse.click(customize, button_y);
    run.shot("09-customize");
    let tray = tray_rect(&before, screen, &baseline);
    mouse.click(tray.0 + 153.0, tray.1 + 63.0);
    let unchanged = run.config() == before;
    run.check(
        "Customize then Cancel leaves the configuration unchanged",
        unchanged,
        "configuration compared before and after",
    );

    // 8b. Sticky borders: a bottom border dragged near the widget below settles exactly one gap
    // from it, the same space the flow layout and Customize use.
    let saved = run.config();
    let gap = saved["gap"].as_f64().unwrap_or(10.0);
    let tray = tray_rect(&saved, screen, &baseline);
    let rect = |w: &Value| {
        (
            w["x"].as_f64().unwrap(),
            w["y"].as_f64().unwrap(),
            w["width"].as_f64().unwrap(),
            w["height"].as_f64().unwrap(),
        )
    };
    let mut pair = None;
    let widgets = saved["widgets"].as_array().unwrap();
    for a in widgets
        .iter()
        .filter(|w| w["width"].as_f64().unwrap_or(0.0) > 0.0)
    {
        let id = a["id"].as_str().unwrap();
        let (ax, ay, aw, ah) = rect(a);
        let below = widgets
            .iter()
            .filter(|b| b["id"] != a["id"] && b["width"].as_f64().unwrap_or(0.0) > 0.0)
            .filter(|b| {
                let (bx, by, bw, _) = rect(b);
                by >= ay + ah - 1.0 && bx < ax + aw && ax < bx + bw
            })
            .min_by(|b1, b2| rect(b1).1.total_cmp(&rect(b2).1));
        let Some(b) = below else { continue };
        let top = screen_point(&saved, id, 0.5, 0.0, tray);
        let bottom = screen_point(&saved, id, 0.5, 1.0, tray);
        if top.1 > 60.0 && bottom.1 < screen.1 as f32 - 20.0 && rect(b).1 + 60.0 < ay + ah + 800.0 {
            pair = Some((id.to_string(), b["id"].as_str().unwrap().to_string()));
            break;
        }
    }
    match pair {
        None => run.check(
            "two stacked widgets are visible for the sticky test",
            false,
            "none found",
        ),
        Some((upper, lower)) => {
            let (_, ay, _, ah) = rect(&run.widget(&upper));
            let by = rect(&run.widget(&lower)).1;
            let (edit_tray, _, _, button_y) = buttons(&run);
            mouse.click(edit_tray, button_y);
            let bottom = screen_point(&saved, &upper, 0.5, 1.0, tray);
            // End the drag 6 px short of the sticky position (the widget below, minus the gap).
            let wanted_bottom = by - gap - 6.0;
            let delta = (wanted_bottom - (ay + ah)) as f32;
            mouse.drag(
                (bottom.0, bottom.1 - 3.0),
                (bottom.0, bottom.1 - 3.0 + delta),
            );
            mouse.click(done, toolbar_y);
            sleep(Duration::from_secs(2));
            let (_, ay, _, ah) = rect(&run.widget(&upper));
            let by = rect(&run.widget(&lower)).1;
            run.near(
                "the dragged border settles exactly one gap above the next widget",
                by - (ay + ah),
                gap,
                0.6,
            );
        }
    }
    run.shot("09b-after-sticky");

    // 8c. Anywhere on the desktop, and the centred Remove button (SQ-29).
    {
        let saved = run.config();
        let tray = tray_rect(&saved, screen, &baseline);
        let target = saved["widgets"]
            .as_array()
            .unwrap()
            .iter()
            .rfind(|w| w["width"].as_f64().unwrap_or(0.0) > 0.0)
            .unwrap()
            .clone();
        // The last widget is drawn on top where widgets overlap (the one added in step 7 lies
        // over the others), so it gets the pointer.
        let id = target["id"].as_str().unwrap().to_string();
        let count = saved["widgets"].as_array().unwrap().len();
        let (edit_tray, _, _, button_y) = buttons(&run);

        // Move it far left of the docked tray, off its centre (which holds Remove).
        mouse.click(edit_tray, button_y);
        let from = screen_point(&saved, &id, 0.25, 0.2, tray);
        let widget_left = screen_point(&saved, &id, 0.0, 0.0, tray).0;
        let dx = -(widget_left - 20.0);
        mouse.drag(from, (from.0 + dx, from.1));
        run.shot("10-edit-moved-left-of-tray");
        mouse.click(done, toolbar_y);
        sleep(Duration::from_secs(2));
        run.shot("11-after-left-move");
        let after = run.config();
        let new_tray_x = after["geometry"]["tray"]["x"].as_f64().unwrap_or(f64::MAX);
        let w = after["widgets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["id"] == id.as_str())
            .unwrap()
            .clone();
        let desktop_x = new_tray_x + w["x"].as_f64().unwrap_or(0.0);
        run.check(
            "a widget can be placed far left of where the tray was docked",
            (desktop_x - 20.0).abs() <= 12.0 && (new_tray_x as f32) < tray.0 - 50.0,
            format!(
                "widget at desktop x {desktop_x:.0} (wanted 20), tray x {new_tray_x:.0} was {:.0}",
                tray.0
            ),
        );

        // Remove then Cancel: the widget stays.
        let saved = after;
        let tray = tray_rect(&saved, screen, &baseline);
        let (edit_tray, _, _, button_y) = buttons(&run);
        mouse.click(edit_tray, button_y);
        let centre = screen_point(&saved, &id, 0.5, 0.5, tray);
        mouse.click(centre.0, centre.1);
        run.shot("12-edit-after-remove-click");
        mouse.click(cancel, toolbar_y);
        sleep(Duration::from_secs(1));
        run.check(
            "Remove then Cancel keeps the widget",
            run.config()["widgets"].as_array().unwrap().len() == count,
            format!("{count} widgets before"),
        );

        // Remove then Done: the widget is gone.
        mouse.click(edit_tray, button_y);
        mouse.click(centre.0, centre.1);
        mouse.click(done, toolbar_y);
        sleep(Duration::from_secs(2));
        run.shot("13-after-remove-done");
        let after = run.config();
        let widgets = after["widgets"].as_array().unwrap();
        run.check(
            "Remove then Done saves the tray without that widget",
            widgets.len() == count - 1 && widgets.iter().all(|w| w["id"] != id.as_str()),
            format!("{count} -> {}", widgets.len()),
        );
    }

    // 9. Restart: the layout and options persist and the panel comes back.
    let before = run.config();
    let stopped = run.stop_panel();
    let stop_limit = baseline["limits"]["stop_seconds_max"].as_f64().unwrap();
    run.check(
        "the panel stops promptly on a stop signal",
        stopped.as_secs_f64() < stop_limit,
        format!("{:.2} s, limit {stop_limit} s", stopped.as_secs_f64()),
    );
    sleep(Duration::from_secs(1));
    run.start_panel();
    run.shot("10-after-restart");
    let alive = run.panel_alive();
    run.check("panel runs again after a restart", alive, "process alive");
    let same = run.config() == before;
    run.check(
        "restart keeps the saved layout and options",
        same,
        "configuration compared before and after",
    );

    let report = json!({
        "ran": "desktop_end_to_end",
        "baseline": "crates/desktop/tests/e2e/manual-baseline.json",
        "screen": [screen.0, screen.1],
        "data": run.data,
        "checks": run.checks,
        "open_findings": baseline["open_findings"],
    });
    fs::write(
        run.out.join("report.json"),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .unwrap();
    let failed: Vec<&Value> = run.checks.iter().filter(|c| c["ok"] == false).collect();
    assert!(failed.is_empty(), "failed checks: {failed:#?}");
}
