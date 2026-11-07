use eframe::egui;
use sq_core::{HostAction, Layer};
use std::{
    path::Path,
    process::{Command, Stdio},
};
#[cfg(target_os = "linux")]
use x11rb::{
    connection::Connection,
    protocol::{randr::ConnectionExt as _, xproto::ConnectionExt as _},
};

pub fn level(layer: Layer) -> egui::WindowLevel {
    match layer {
        Layer::Behind => egui::WindowLevel::AlwaysOnBottom,
        Layer::Normal => egui::WindowLevel::Normal,
        Layer::Above => egui::WindowLevel::AlwaysOnTop,
    }
}
#[cfg(not(target_os = "linux"))]
pub fn capabilities() -> String {
    "Windows: ordinary window; it can stay on top or behind other windows (Window layer setting)."
        .into()
}
#[cfg(target_os = "linux")]
pub fn capabilities() -> String {
    if x11rb::connect(None).is_ok() {
        "X11 / XWayland: window layers available; compositor controls background transparency."
            .into()
    } else {
        "Native Wayland: the compositor may restrict stacking and position. Separate windows remain available.".into()
    }
}
#[derive(Clone, Debug)]
pub struct Monitor {
    pub name: String,
    pub rect: egui::Rect,
}
/// Coordinates use the same logical scale as egui's viewport rectangles.
#[cfg(not(target_os = "linux"))]
pub fn monitors(_pixels_per_point: f32) -> Vec<Monitor> {
    // No global monitor list here: `recover_geometry` falls back to the window's own monitor size.
    vec![]
}
#[cfg(target_os = "linux")]
pub fn monitors(pixels_per_point: f32) -> Vec<Monitor> {
    let Ok((connection, screen)) = x11rb::connect(None) else {
        return vec![];
    };
    let root = connection.setup().roots[screen].root;
    let scale = pixels_per_point.max(0.1);
    if let Ok(cookie) = connection.randr_get_monitors(root, true) {
        if let Ok(reply) = cookie.reply() {
            let monitors: Vec<_> = reply
                .monitors
                .into_iter()
                .filter(|m| m.width > 0 && m.height > 0)
                .map(|m| {
                    let name = connection
                        .get_atom_name(m.name)
                        .ok()
                        .and_then(|c| c.reply().ok())
                        .map(|r| String::from_utf8_lossy(&r.name).into_owned())
                        .unwrap_or_else(|| format!("monitor-{}", m.name));
                    Monitor {
                        name,
                        rect: egui::Rect::from_min_size(
                            egui::pos2(m.x as f32 / scale, m.y as f32 / scale),
                            egui::vec2(m.width as f32 / scale, m.height as f32 / scale),
                        ),
                    }
                })
                .collect();
            if !monitors.is_empty() {
                return monitors;
            }
        }
    }
    connection
        .get_geometry(root)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| {
            vec![Monitor {
                name: "X11 screen".into(),
                rect: egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(r.width as f32 / scale, r.height as f32 / scale),
                ),
            }]
        })
        .unwrap_or_default()
}
pub fn monitor_for(rect: egui::Rect, monitors: &[Monitor]) -> Option<&Monitor> {
    monitors.iter().min_by(|a, b| {
        a.rect
            .distance_sq_to_pos(rect.center())
            .total_cmp(&b.rect.distance_sq_to_pos(rect.center()))
    })
}
/// Keep intentional cross-monitor positions; recover only an unreachable title bar or oversized window.
pub fn recovered_rect(rect: egui::Rect, monitors: &[Monitor]) -> Option<egui::Rect> {
    let monitor = monitor_for(rect, monitors)?;
    let title = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), 32.0));
    let reachable = monitors.iter().any(|m| {
        let visible = title.intersect(m.rect);
        visible.width() >= 64.0 && visible.height() >= 16.0
    });
    let size = rect.size().min(monitor.rect.size());
    if reachable && size == rect.size() {
        return None;
    }
    let max = monitor.rect.max - size;
    let position = egui::pos2(
        rect.min.x.clamp(monitor.rect.min.x, max.x),
        rect.min.y.clamp(monitor.rect.min.y, max.y),
    );
    Some(egui::Rect::from_min_size(position, size))
}
pub fn recover_geometry(ctx: &egui::Context) {
    let info = ctx.input(|i| i.viewport().clone());
    if let Some(rect) = info.outer_rect {
        let available = monitors(ctx.pixels_per_point());
        // Wayland does not expose global monitor positions. Avoid inventing a global origin.
        if available.is_empty() {
            if let Some(size) = info.monitor_size {
                if rect.width() > size.x || rect.height() > size.y {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(rect.size().min(size)));
                }
            }
            return;
        }
        if let Some(recovered) = recovered_rect(rect, &available) {
            if recovered.size() != rect.size() {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(recovered.size()));
            }
            if recovered.min != rect.min {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(recovered.min));
            }
        }
    }
}
/// The program that opens a URL or a folder with the desktop's default handler. Never a shell:
/// the target is one argument, so characters like `&` in it cannot start another command.
fn opener(target: impl AsRef<std::ffi::OsStr>) -> Command {
    if cfg!(windows) {
        let mut c = Command::new("rundll32.exe");
        c.arg("url.dll,FileProtocolHandler").arg(target);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(target);
        c
    }
}
/// Find a program on `PATH` the way a Windows shell would: npm installs tools as `.cmd` files.
fn find_on_path(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        ["exe", "cmd", "bat"]
            .iter()
            .map(|ext| dir.join(format!("{name}.{ext}")))
            .find(|candidate| candidate.is_file())
    })
}
/// Resume a session in a new console window (Windows): the tool runs directly in the project
/// folder; there is no shell command line to quote.
pub fn windows_session(tool: &str, id: &str, project: &Path) -> Command {
    let program = find_on_path(tool).unwrap_or_else(|| std::path::PathBuf::from(tool));
    let mut c = Command::new(program);
    if tool == "claude" {
        c.arg("--resume").arg(id);
    } else {
        c.arg("resume").arg(id);
    }
    c.current_dir(project);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut c, 0x0000_0010); // CREATE_NEW_CONSOLE
    c
}
fn navigation_command(action: HostAction) -> Result<Option<Command>, String> {
    let command = match action {
        HostAction::OpenUrl(url) => {
            if ![
                "https://github.com/",
                "https://chatgpt.com/",
                "https://claude.ai/",
            ]
            .iter()
            .any(|prefix| url.starts_with(prefix))
                || url.chars().any(char::is_control)
            {
                return Err("Navigation URL scheme or host is not allowed".into());
            }
            opener(url)
        }
        HostAction::OpenProject(path) | HostAction::OpenMemory(path) => {
            if !Path::new(&path).is_absolute() || !Path::new(&path).exists() {
                return Err("Source path is unavailable".into());
            }
            opener(path)
        }
        HostAction::OpenSession {
            assistant,
            id,
            project,
        } => {
            let tool = sq_core::account_kind(&assistant).to_string();
            if !["claude", "codex"].contains(&tool.as_str())
                || id.is_empty()
                || id.len() > 200
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                || !Path::new(&project).is_absolute()
                || !Path::new(&project).is_dir()
            {
                return Err("Session navigation requires a supported provider, identity and project directory".into());
            }
            if cfg!(windows) {
                return Ok(Some(windows_session(&tool, &id, Path::new(&project))));
            }
            let mut c = Command::new("konsole");
            c.arg("--workdir").arg(project).arg("-e").arg(&tool);
            if tool == "claude" {
                c.arg("--resume").arg(id);
            } else {
                c.arg("resume").arg(id);
            }
            c
        }
        HostAction::LocalControl => return Ok(None),
    };
    Ok(Some(command))
}
pub fn navigate(action: HostAction) -> Result<(), String> {
    if let Some(mut command) = navigation_command(action)? {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Open source: {e}"))?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
    Ok(())
}
/// File access for the folder navigator of Customize (SQ-39): read-only folder listings.
pub struct Folders;
impl sq_tray::FolderBrowser for Folders {
    fn home(&self) -> String {
        std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| {
                if cfg!(windows) {
                    "C:\\".into()
                } else {
                    "/".into()
                }
            })
    }
    fn roots(&self) -> Vec<String> {
        if cfg!(windows) {
            ('A'..='Z')
                .map(|c| format!("{c}:\\"))
                .filter(|d| std::path::Path::new(d).exists())
                .collect()
        } else {
            vec!["/".into()]
        }
    }
    fn folders(&self, folder: &str) -> Result<Vec<String>, String> {
        let mut names: Vec<String> = std::fs::read_dir(folder)
            .map_err(|e| e.to_string())?
            .flatten()
            .take(20_000)
            .filter(|e| std::fs::metadata(e.path()).is_ok_and(|m| m.is_dir()))
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        // Visible folders first, then hidden ones (the assistants keep theirs in `.claude`, `.codex`).
        names.sort_by_key(|n| (n.starts_with('.'), n.to_lowercase()));
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn monitor(x: f32, width: f32) -> Monitor {
        Monitor {
            name: format!("display-{x}"),
            rect: egui::Rect::from_min_size(egui::pos2(x, 0.0), egui::vec2(width, 1080.0)),
        }
    }
    // REQ SQ-15 SQ-22
    #[test]
    fn disconnected_monitor_recovers_and_negative_monitor_stays_reachable() {
        let rect = egui::Rect::from_min_size(egui::pos2(2100.0, 100.0), egui::vec2(760.0, 900.0));
        let monitors = vec![monitor(0.0, 1920.0), monitor(1920.0, 1920.0)];
        assert!(recovered_rect(rect, &monitors).is_none());
        let recovered = recovered_rect(rect, &monitors[..1]).unwrap();
        assert_eq!(recovered.min.x, 1160.0);
        let negative = egui::Rect::from_min_size(egui::pos2(-1400.0, 100.0), rect.size());
        assert!(recovered_rect(negative, &[monitor(-1920.0, 1920.0)]).is_none());
        let huge = egui::Rect::from_min_size(egui::pos2(0.0, -100.0), egui::vec2(4000.0, 2000.0));
        assert_eq!(
            recovered_rect(huge, &monitors[..1]).unwrap(),
            monitors[0].rect
        );
    }
    // REQ SQ-21
    #[test]
    fn navigation_rejects_empty_sessions_and_keeps_paths_as_single_arguments() {
        assert!(navigation_command(HostAction::OpenSession {
            assistant: "claude".into(),
            id: "".into(),
            project: "/tmp".into()
        })
        .is_err());
        assert!(navigation_command(HostAction::OpenUrl(
            "https://github.com.evil/example".into()
        ))
        .is_err());
        assert!(
            navigation_command(HostAction::OpenUrl("https://github.com/example\n".into())).is_err()
        );
        let project = std::env::temp_dir(); // scratch-ok: an existing folder, nothing is created
        let command = navigation_command(HostAction::OpenSession {
            assistant: "codex".into(),
            id: "safe-id".into(),
            project: project.to_string_lossy().into(),
        })
        .unwrap()
        .unwrap();
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_str().unwrap().to_string())
            .collect();
        if cfg!(windows) {
            // No terminal wrapper: the tool itself runs in a new console in the project folder.
            assert_eq!(args, ["resume", "safe-id"]);
            assert_eq!(command.get_current_dir(), Some(project.as_path()));
        } else {
            assert_eq!(
                args,
                [
                    "--workdir",
                    project.to_str().unwrap(),
                    "-e",
                    "codex",
                    "resume",
                    "safe-id"
                ]
            );
        }
    }
    // REQ SQ-36: on Windows a session is resumed in the project folder without a shell line,
    // and a link is opened by one argument to the system handler.
    #[test]
    fn windows_navigation_uses_no_shell_and_keeps_targets_as_one_argument() {
        let command = windows_session("claude", "abc-123", Path::new("/work/my app"));
        let args: Vec<_> = command.get_args().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(args, ["--resume", "abc-123"]);
        assert_eq!(command.get_current_dir(), Some(Path::new("/work/my app")));
        let command = windows_session("codex", "xyz", Path::new("/p"));
        let args: Vec<_> = command.get_args().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(args, ["resume", "xyz"]);
        let tricky = "https://github.com/o/r/pull/1?a=1&b=calc.exe";
        let command = opener(tricky);
        let last = command.get_args().last().unwrap().to_str().unwrap();
        assert_eq!(last, tricky, "one argument, never split or interpreted");
        let program = command.get_program().to_str().unwrap();
        assert!(["rundll32.exe", "xdg-open"].contains(&program));
        assert!(!program.contains("cmd") && !program.contains("sh"));
    }
}
