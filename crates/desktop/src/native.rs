use eframe::egui;
use sq_core::{HostAction, Layer};
use std::{
    path::Path,
    process::{Command, Stdio},
};
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
            let mut c = Command::new("xdg-open");
            c.arg(url);
            c
        }
        HostAction::OpenProject(path) | HostAction::OpenMemory(path) => {
            if !Path::new(&path).is_absolute() || !Path::new(&path).exists() {
                return Err("Source path is unavailable".into());
            }
            let mut c = Command::new("xdg-open");
            c.arg(path);
            c
        }
        HostAction::OpenSession {
            assistant,
            id,
            project,
        } => {
            if !["claude", "codex"].contains(&assistant.as_str())
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
            let mut c = Command::new("konsole");
            c.arg("--workdir").arg(project).arg("-e").arg(&assistant);
            if assistant == "claude" {
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
        let command = navigation_command(HostAction::OpenSession {
            assistant: "codex".into(),
            id: "safe-id".into(),
            project: "/tmp".into(),
        })
        .unwrap()
        .unwrap();
        assert_eq!(
            command
                .get_args()
                .map(|a| a.to_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["--workdir", "/tmp", "-e", "codex", "resume", "safe-id"]
        );
    }
}
