//! Desktop wrapper: Linux (Wayland layer-shell, X11, KDE first) and Windows 10/11 (ordinary window). Shared widgets/tray do not import this crate.
// A Windows program without a console window; `attach_console` gives it back when a terminal started it.
#![cfg_attr(windows, windows_subsystem = "windows")]
mod host;
mod install;
mod native;
mod wallpaper;
use eframe::egui;
use fs2::FileExt;
use sq_collector::{now, state_root, Client, Service};
use sq_contract::*;
use sq_core::*;
use sq_store::private::PrivateOpen;
use sq_tray::{Intent, Tray};
use std::{
    fs::{self, OpenOptions},
    path::PathBuf,
    process::{Command as Process, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

/// Windows only: print to the terminal that started the program, if there is one.
#[cfg(windows)]
fn attach_console() {
    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(process: u32) -> i32;
    }
    // ATTACH_PARENT_PROCESS
    unsafe {
        AttachConsole(u32::MAX);
    }
}
fn main() {
    #[cfg(windows)]
    attach_console();
    if let Err(error) = run() {
        eprintln!("Status Quo: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("install") => return install::install(),
        Some("uninstall") => return install::uninstall(),
        Some("launch") => return install::launch(false),
        Some("login") => return install::launch(true),
        Some("collect") => {
            return match Service::new(state_root()) {
                Ok(service) => {
                    let stop = service.stop_handle();
                    ctrlc::set_handler(move || stop.store(true, Ordering::Release))
                        .map_err(|e| e.to_string())?;
                    service.serve(true)
                }
                Err(error) if error.contains("Another collector") => Ok(()),
                Err(error) => Err(error),
            }
        }
        Some("hook") => {
            use std::io::Read;
            let provider = args.get(1).ok_or("Hook provider missing")?;
            let event = args.get(2).ok_or("Hook event missing")?;
            let mut bytes = Vec::new();
            std::io::stdin()
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() > 1024 * 1024 {
                return Err("Hook payload exceeds size limit; not retained".into());
            }
            let input =
                serde_json::from_slice(&bytes).map_err(|_| "Invalid lifecycle hook JSON")?;
            let sequence = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .subsec_nanos() as u64;
            let record = sq_sources::lifecycle_event(provider, event, &input, now(), sequence)?;
            sq_collector::record_lifecycle(&state_root(), &record)?;
            return Ok(());
        }
        Some("inspect") => {
            ensure_collector()?;
            let client = Client::default_path();
            let reply = client.request(Command::Snapshot { after: None })?;
            if args.iter().any(|a| a == "--widgets") {
                let Reply::Snapshot(snapshot) = reply else {
                    return Err("Snapshot unavailable".into());
                };
                let Reply::Configuration { config, .. } = client.request(Command::Configuration)?
                else {
                    return Err("Configuration unavailable".into());
                };
                let registry = sq_widgets::registry();
                for widget in &config.widgets {
                    println!(
                        "{}",
                        serde_json::to_string(&registry.view(widget, &snapshot, now()))
                            .map_err(|e| e.to_string())?
                    );
                }
                return Ok(());
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&reply).map_err(|e| e.to_string())?
            );
            return Ok(());
        }
        Some("--help") | Some("help") => {
            println!("Status Quo\n  status-quo                  Open the desktop panel\n  status-quo collect          Run the shared background collector\n  status-quo inspect          Read collector status as JSON\n  status-quo inspect --widgets Render widget models in a second Rust host\n  status-quo --screenshot PATH Capture this panel's framebuffer and exit\n  status-quo --customize      Open customization");
            return Ok(());
        }
        _ => {}
    }
    ensure_collector()?;
    let root = state_root();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .private(0o600)
        .open(root.join("panel.lock"))
        .map_err(|e| e.to_string())?;
    if lock.try_lock_exclusive().is_err() {
        return Ok(());
    }
    let Reply::Configuration { revision, config } =
        Client::default_path().request(Command::Configuration)?
    else {
        return Err("Collector configuration unavailable".into());
    };
    let screenshot = args
        .iter()
        .position(|a| a == "--screenshot")
        .map(|i| {
            args.get(i + 1)
                .map(PathBuf::from)
                .ok_or("--screenshot needs a file path")
        })
        .transpose()?;
    let customize = args.iter().any(|a| a == "--customize");
    // Screenshot and customization need a framed window (capture and keyboard input).
    let selection = if screenshot.is_some() || customize {
        host::Selection {
            backend: host::Backend::Window,
            note: "window: screenshot and customization use an ordinary window.".into(),
        }
    } else {
        host::select(host::forced_backend(), &host::probe_session())
    };
    eprintln!("Status Quo host: {}", selection.note);
    #[cfg(target_os = "linux")]
    if selection.backend == host::Backend::LayerShell {
        let saved = config.geometry.get("tray").map(|g| (g.width, g.height));
        let (width, height) = host::surface_size(saved);
        let anchor = std::env::var("STATUS_QUO_ANCHOR")
            .ok()
            .and_then(|v| host::parse_anchor(&v))
            .unwrap_or(host::Anchor {
                top: true,
                right: true,
                ..Default::default()
            });
        let ctx = egui::Context::default();
        let mut app = App::new(ctx.clone(), *config, revision, None, false);
        app.tray.capabilities = format!("Layer-shell surface: {}", selection.note);
        app.tray.bare = true;
        let request = host::layer::Request {
            width,
            height,
            anchor,
            margin: 16,
            layer: app.tray.config.layer,
        };
        let exit = Arc::new(AtomicBool::new(false));
        let stop = exit.clone();
        ctrlc::set_handler(move || stop.store(true, Ordering::Release))
            .map_err(|e| e.to_string())?;
        host::layer::run(request, Box::new(LayerApp(app)), ctx, exit)?;
        let _ = FileExt::unlock(&lock);
        return Ok(());
    }
    let framed = selection.backend == host::Backend::Window;
    let layer = native::level(config.layer);
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Status Quo")
        .with_app_id("status-quo")
        .with_inner_size([760.0, 900.0])
        .with_min_inner_size([320.0, 240.0])
        .with_transparent(true)
        .with_decorations(framed)
        .with_window_level(layer);
    if let Some(g) = config.geometry.get("tray") {
        viewport = viewport
            .with_position([g.x, g.y])
            .with_inner_size([g.width, g.height]);
    }
    let desktop_hints = selection.backend == host::Backend::X11Desktop;
    // OpenGL is tried first. Some machines have no usable OpenGL 2 (virtual machines, remote
    // sessions, old drivers); on Windows the same window then opens through DirectX (wgpu).
    #[cfg(windows)]
    let renderers = [eframe::Renderer::Glow, eframe::Renderer::Wgpu];
    #[cfg(not(windows))]
    let renderers = [eframe::Renderer::Glow];
    let mut failure = String::new();
    for renderer in renderers {
        #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
        let mut options = eframe::NativeOptions {
            viewport: viewport.clone(),
            renderer,
            vsync: true,
            ..Default::default()
        };
        #[cfg(windows)]
        if matches!(renderer, eframe::Renderer::Wgpu) {
            accept_software_adapter(&mut options);
        }
        // Framed window on a Wayland session: prefer process-local XWayland, which honors
        // requested window layers. The X11 desktop backend needs X11 for its hints.
        #[cfg(target_os = "linux")]
        if std::env::var_os("DISPLAY").is_some() {
            options.event_loop_builder = Some(Box::new(|builder| {
                use winit::platform::x11::EventLoopBuilderExtX11;
                builder.with_x11();
            }));
        }
        let (config, screenshot, note) =
            (config.clone(), screenshot.clone(), selection.note.clone());
        let result = eframe::run_native(
            "Status Quo",
            options,
            Box::new(move |cc| {
                let mut app = App::new(
                    cc.egui_ctx.clone(),
                    *config,
                    revision,
                    screenshot,
                    customize,
                );
                app.tray.capabilities = format!("{note} {}", native::capabilities());
                app.x11_hints_pending = desktop_hints;
                app.tray.bare = desktop_hints;
                Ok(Box::new(app))
            }),
        );
        match result {
            Ok(()) => {
                failure.clear();
                break;
            }
            Err(error) => {
                failure = error.to_string();
                eprintln!("Status Quo: {failure}");
            }
        }
    }
    let _ = FileExt::unlock(&lock);
    if failure.is_empty() {
        Ok(())
    } else {
        Err(failure)
    }
}
/// wgpu normally ignores software adapters. This machine has no OpenGL 2, so a real GPU is
/// preferred and, failing that, any adapter is accepted, including Windows' built-in software
/// renderer (WARP): a slow window is better than none.
#[cfg(windows)]
fn accept_software_adapter(options: &mut eframe::NativeOptions) {
    use eframe::{
        egui_wgpu::{WgpuSetup, WgpuSetupCreateNew},
        wgpu::DeviceType,
    };
    let mut setup = WgpuSetupCreateNew::default();
    setup.native_adapter_selector = Some(std::sync::Arc::new(|adapters, surface| {
        for a in adapters {
            let info = a.get_info();
            eprintln!(
                "Status Quo: graphics adapter {} ({:?}, {:?}), can show window: {}",
                info.name,
                info.backend,
                info.device_type,
                surface.is_none_or(|s| a.is_surface_supported(s))
            );
        }
        eprintln!("Status Quo: {} graphics adapters found", adapters.len());
        adapters
            .iter()
            .filter(|a| surface.is_none_or(|s| a.is_surface_supported(s)))
            .min_by_key(|a| match a.get_info().device_type {
                DeviceType::DiscreteGpu => 0,
                DeviceType::IntegratedGpu => 1,
                DeviceType::VirtualGpu => 2,
                DeviceType::Other => 3,
                DeviceType::Cpu => 4,
            })
            .cloned()
            .ok_or_else(|| "no graphics adapter can show a window".to_string())
    }));
    options.wgpu_options.wgpu_setup = WgpuSetup::CreateNew(setup);
}
#[cfg(target_os = "linux")]
struct LayerApp(App);
#[cfg(target_os = "linux")]
impl host::layer::Frame for LayerApp {
    fn update(&mut self, ctx: &egui::Context) -> bool {
        self.0.tray.fullscreen_edit = true;
        self.0.tick(ctx);
        true
    }
    fn surface(&self) -> host::layer::Surface {
        let tray = &self.0.tray;
        if tray.editing {
            return host::layer::Surface::Editing;
        }
        match tray.desired_size() {
            Some(size) => {
                let (x, y) = tray
                    .config
                    .geometry
                    .get("tray")
                    .map(|g| (g.x, g.y))
                    .unwrap_or((16.0, 16.0));
                host::layer::Surface::Free {
                    x: x as i32,
                    y: y as i32,
                    width: size.x.ceil().clamp(330.0, 16_384.0) as u32,
                    height: size.y.ceil().clamp(100.0, 16_384.0) as u32,
                }
            }
            None => host::layer::Surface::Legacy,
        }
    }
    fn docked_origin(&mut self, origin: (f32, f32)) {
        self.0.tray.set_origin(egui::vec2(origin.0, origin.1));
    }
    fn layer(&self) -> Layer {
        // While editing, the tray covers the desktop above windows so every widget can be reached.
        if self.0.tray.editing {
            Layer::Above
        } else {
            self.0.tray.config.layer
        }
    }
}
fn ensure_collector() -> Result<(), String> {
    if Client::default_path().request(Command::Ping).is_ok() {
        return Ok(());
    }
    let root = state_root();
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let log = OpenOptions::new()
        .append(true)
        .create(true)
        .private(0o600)
        .open(root.join("collector.log"))
        .map_err(|e| e.to_string())?;
    {
        let mut command = Process::new(std::env::current_exe().map_err(|e| e.to_string())?);
        // The collector must outlive this panel and must not show a console window.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(
            &mut command,
            0x0000_0008 | 0x0000_0200 | 0x0800_0000, // detached, new process group, no window
        );
        command
    }
    .arg("collect")
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(log)
    .spawn()
    .map_err(|e| format!("Start collector: {e}"))?;
    for _ in 0..100 {
        if Client::default_path().request(Command::Ping).is_ok() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(50));
    }
    Err("Collector did not become ready; inspect the private collector.log".into())
}
struct Change {
    action: ConfigAction,
    close_draft: bool,
    revision: u64,
}
enum Update {
    Snapshot(Box<Snapshot>),
    Config {
        config: Box<Config>,
        close_draft: bool,
        revision: u64,
    },
    Error(String),
    ConfigurationError(String),
    Connected,
    Wallpaper(WallpaperColors),
}
struct App {
    tray: Tray,
    send: mpsc::Sender<Change>,
    receive: mpsc::Receiver<Update>,
    stop: Arc<AtomicBool>,
    /// Some theme follows the wallpaper, so the watcher thread looks at it.
    wallpaper_wanted: Arc<AtomicBool>,
    layer: Layer,
    screenshot: Option<PathBuf>,
    capture_sent: bool,
    started: Instant,
    last_geometry: Instant,
    revision: u64,
    draft_revision: Option<u64>,
    last_rect: Option<egui::Rect>,
    geometry_changed: Instant,
    x11_hints_pending: bool,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    x11_attempts: u8,
    last_size: Option<egui::Vec2>,
}
impl App {
    fn new(
        ctx: egui::Context,
        config: Config,
        revision: u64,
        screenshot: Option<PathBuf>,
        customize: bool,
    ) -> Self {
        let (send, commands) = mpsc::channel::<Change>();
        let (updates, receive) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker_ctx = ctx.clone();
        let worker_updates = updates.clone();
        thread::spawn(move || {
            client_worker(commands, worker_updates, worker_stop, worker_ctx, revision)
        });
        let wallpaper_wanted = Arc::new(AtomicBool::new(false));
        {
            let (updates, ctx) = (updates.clone(), ctx.clone());
            wallpaper::spawn(wallpaper_wanted.clone(), stop.clone(), move |colors| {
                let _ = updates.send(Update::Wallpaper(colors));
                ctx.request_repaint();
            });
        }
        let layer = config.layer;
        let mut tray = Tray::new(config);
        tray.capabilities = native::capabilities();
        tray.browser = Some(std::sync::Arc::new(native::Folders));
        if customize {
            tray.begin_customization();
        }
        Self {
            tray,
            send,
            receive,
            stop,
            wallpaper_wanted,
            layer,
            screenshot,
            capture_sent: false,
            started: Instant::now(),
            last_geometry: Instant::now(),
            revision,
            draft_revision: if customize { Some(revision) } else { None },
            last_rect: None,
            geometry_changed: Instant::now(),
            x11_hints_pending: false,
            last_size: None,
            x11_attempts: 0,
        }
    }
    fn intent(&mut self, intent: Intent) {
        match intent {
            Intent::Configure(action) => {
                let close_draft = matches!(action, ConfigAction::Replace(_));
                if self
                    .send
                    .send(Change {
                        action,
                        close_draft,
                        revision: if close_draft {
                            self.draft_revision.unwrap_or(self.revision)
                        } else {
                            self.revision
                        },
                    })
                    .is_err()
                {
                    self.tray.error = Some("Collector client stopped".into());
                }
            }
            Intent::Navigate(action) => {
                if self.tray.config.allows(&action) {
                    if let Err(error) = native::navigate(action) {
                        self.tray.error = Some(error);
                    }
                }
            }
        }
    }
}
impl Drop for App {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        #[cfg(target_os = "linux")]
        {
            if self.x11_hints_pending && self.started.elapsed() >= Duration::from_secs(1) {
                use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                let window = frame.window_handle().ok().and_then(|h| match h.as_raw() {
                    RawWindowHandle::Xlib(x) => Some(x.window as u32),
                    RawWindowHandle::Xcb(x) => Some(x.window.get()),
                    _ => None,
                });
                match window {
                    Some(window) if host::x11::hints_active(window) => {
                        self.x11_hints_pending = false;
                        eprintln!("Status Quo: X11 desktop hints active");
                    }
                    Some(window) if self.x11_attempts < 8 => {
                        self.x11_attempts += 1;
                        if let Err(error) = host::x11::apply_desktop_hints(window) {
                            self.tray.error = Some(error);
                        }
                        ctx.request_repaint_after(Duration::from_millis(700));
                    }
                    _ => {
                        self.x11_hints_pending = false;
                        self.tray.error =
                            Some("The window manager did not accept the desktop hints".into());
                    }
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = frame;
        self.tick(ctx);
    }
}
impl App {
    fn tick(&mut self, ctx: &egui::Context) {
        while let Ok(update) = self.receive.try_recv() {
            match update {
                Update::Snapshot(snapshot) => {
                    self.tray.snapshot = *snapshot;
                    if !self.tray.connected {
                        self.tray.error = None;
                    }
                    self.tray.connected = true;
                }
                Update::Config {
                    config,
                    close_draft,
                    revision,
                } => {
                    self.revision = revision;
                    if close_draft {
                        self.tray.accept(*config);
                    } else {
                        self.tray.config = *config;
                    }
                }
                Update::Error(error) => {
                    self.tray.connected = false;
                    self.tray.error = Some(error);
                }
                Update::ConfigurationError(error) => self.tray.error = Some(error),
                Update::Wallpaper(colors) => self.tray.wallpaper = colors,
                Update::Connected => {
                    self.tray.connected = true;
                    self.tray.error = None;
                }
            }
        }
        self.wallpaper_wanted.store(
            [Some(&self.tray.config), self.tray.draft.as_ref()]
                .into_iter()
                .flatten()
                .any(follows_wallpaper),
            Ordering::Release,
        );
        if self.layer != self.tray.config.layer {
            self.layer = self.tray.config.layer;
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(native::level(
                self.layer,
            )));
        }
        if !self.tray.fullscreen_edit {
            if let Some(size) = self.tray.desired_size() {
                if self.last_size != Some(size) {
                    self.last_size = Some(size);
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                }
            }
        }
        let was_customizing = self.tray.draft.is_some();
        let mut intents = self.tray.show(ctx, now());
        intents.extend(self.tray.detached_windows(ctx, now()));
        if !was_customizing && self.tray.draft.is_some() {
            self.draft_revision = Some(self.revision);
        } else if self.tray.draft.is_none() {
            self.draft_revision = None;
        }
        for intent in intents {
            self.intent(intent);
        }
        if self.last_geometry.elapsed() >= Duration::from_secs(2) {
            native::recover_geometry(ctx);
            self.last_geometry = Instant::now();
            let rect = ctx.input(|i| i.viewport().inner_rect);
            if rect != self.last_rect {
                self.last_rect = rect;
                self.geometry_changed = Instant::now();
            } else if self.geometry_changed.elapsed() >= Duration::from_secs(2)
                && self.tray.draft.is_none()
            {
                if let Some(rect) = rect {
                    let monitors = native::monitors(ctx.pixels_per_point());
                    let monitor = native::monitor_for(rect, &monitors)
                        .map(|m| m.name.clone())
                        .unwrap_or_default();
                    let value = Geometry {
                        x: rect.min.x,
                        y: rect.min.y,
                        width: rect.width(),
                        height: rect.height(),
                        monitor,
                    };
                    if self.tray.config.geometry.get("tray") != Some(&value) {
                        self.intent(Intent::Configure(ConfigAction::Geometry {
                            key: "tray".into(),
                            value,
                        }));
                    }
                }
            }
        }
        ctx.request_repaint_after(Duration::from_secs(2));
        if self.screenshot.is_some()
            && !self.capture_sent
            && self.started.elapsed() >= Duration::from_secs(8)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.capture_sent = true;
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                if let Some(path) = self.screenshot.take() {
                    let bytes: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                    match image::save_buffer(
                        &path,
                        &bytes,
                        image.size[0] as u32,
                        image.size[1] as u32,
                        image::ColorType::Rgba8,
                    ) {
                        Ok(()) => println!("Panel framebuffer captured: {}", path.display()),
                        Err(error) => {
                            eprintln!("Panel capture failed: {error}");
                        }
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        if self.screenshot.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}
fn follows_wallpaper(config: &Config) -> bool {
    config.theme.follow_wallpaper
        || config
            .widgets
            .iter()
            .any(|w| w.theme.as_ref().is_some_and(|t| t.follow_wallpaper))
}
fn client_worker(
    commands: mpsc::Receiver<Change>,
    updates: mpsc::Sender<Update>,
    stop: Arc<AtomicBool>,
    ctx: egui::Context,
    mut revision: u64,
) {
    let client = Client::default_path();
    let mut after = None;
    let mut last_error = String::new();
    while !stop.load(Ordering::Acquire) {
        let command = commands.recv_timeout(Duration::from_millis(500));
        if let Ok(change) = command {
            match client.request(Command::Configure {
                revision: change.revision,
                action: change.action,
            }) {
                Ok(Reply::Configuration {
                    revision: next,
                    config,
                }) => {
                    revision = next;
                    let _ = updates.send(Update::Config {
                        config,
                        close_draft: change.close_draft,
                        revision: next,
                    });
                    ctx.request_repaint();
                }
                Ok(Reply::Error(error)) => {
                    let _ = updates.send(Update::ConfigurationError(error));
                    ctx.request_repaint();
                }
                Err(error) => {
                    let _ = updates.send(Update::Error(error));
                    ctx.request_repaint();
                }
                _ => {}
            }
        }
        match client.request(Command::Snapshot { after }) {
            Ok(Reply::Snapshot(snapshot)) => {
                after = Some(snapshot.revision);
                let _ = updates.send(Update::Snapshot(snapshot));
                ctx.request_repaint();
                last_error.clear();
            }
            Ok(Reply::Unchanged { .. }) => {
                if !last_error.is_empty() {
                    let _ = updates.send(Update::Connected);
                    ctx.request_repaint();
                    last_error.clear();
                }
            }
            Err(error) => {
                if error != last_error {
                    let _ = updates.send(Update::Error(error.clone()));
                    ctx.request_repaint();
                    last_error = error;
                }
            }
            _ => {}
        }
        if let Ok(Reply::Configuration {
            revision: next,
            config,
        }) = client.request(Command::Configuration)
        {
            if next != revision {
                revision = next;
                let _ = updates.send(Update::Config {
                    config,
                    close_draft: false,
                    revision: next,
                });
                ctx.request_repaint();
            }
        }
    }
}
