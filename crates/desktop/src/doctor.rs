//! `status-quo doctor`: checks, in plain words, whether this computer can run Status Quo and
//! what is still missing (SQ-41). The installers run it at the end. It only reads.
use sq_core::SourceConfig;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// Works.
    Ok,
    /// Works, but something optional is missing.
    Warn,
    /// Status Quo cannot run until this is fixed.
    Fail,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub level: Level,
    pub name: &'static str,
    pub detail: String,
    /// What to do about it, when there is something to do.
    pub fix: Option<String>,
}
fn check(level: Level, name: &'static str, detail: impl Into<String>, fix: Option<&str>) -> Check {
    Check {
        level,
        name,
        detail: detail.into(),
        fix: fix.map(String::from),
    }
}
/// The report as the user reads it: one line per check, a hint under each problem, a summary.
pub fn render(checks: &[Check]) -> String {
    let mut out = String::from("Status Quo compatibility check\n\n");
    for c in checks {
        let mark = match c.level {
            Level::Ok => "[ok]  ",
            Level::Warn => "[!]   ",
            Level::Fail => "[x]   ",
        };
        out.push_str(&format!("{mark}{}: {}\n", c.name, c.detail));
        if let (Some(fix), true) = (&c.fix, c.level != Level::Ok) {
            out.push_str(&format!("      -> {fix}\n"));
        }
    }
    let fails = checks.iter().filter(|c| c.level == Level::Fail).count();
    let warns = checks.iter().filter(|c| c.level == Level::Warn).count();
    out.push('\n');
    out.push_str(&match (fails, warns) {
        (0, 0) => "Everything is ready.\n".to_string(),
        (0, w) => format!("Ready to run. {w} optional item(s) above would add more data.\n"),
        (f, _) => format!("{f} problem(s) above must be fixed before Status Quo can run.\n"),
    });
    out
}
/// 0 when Status Quo can run, 1 when a blocking problem was found.
pub fn exit_code(checks: &[Check]) -> i32 {
    i32::from(checks.iter().any(|c| c.level == Level::Fail))
}
/// The version numbers in the output of `cmd /c ver` ("Microsoft Windows [Version 10.0.19044.1]").
fn windows_numbers(text: &str) -> Option<Vec<u32>> {
    let after = text.split("Version").nth(1)?;
    let numbers: Vec<u32> = after
        .trim_start()
        .split(['.', ' ', ']'])
        .map_while(|part| part.parse().ok())
        .collect();
    (!numbers.is_empty()).then_some(numbers)
}
/// The name people know: Windows 11 still calls itself version 10.0, with a build of 22000 or more.
pub fn windows_name(text: &str) -> Option<u32> {
    let numbers = windows_numbers(text)?;
    Some(match numbers[..] {
        [10, _, build, ..] if build >= 22000 => 11,
        [major, ..] => major,
        [] => return None,
    })
}
/// Session facts the display check needs; filled from the environment, or by tests.
pub struct Session<'a> {
    pub windows: bool,
    pub wayland_display: bool,
    pub x_display: bool,
    pub chosen: &'a str,
}
pub fn check_display(s: &Session) -> Check {
    if s.windows {
        return check(
            Level::Ok,
            "Display",
            "Windows: the tray is an ordinary window",
            None,
        );
    }
    if !s.wayland_display && !s.x_display {
        return check(
            Level::Fail,
            "Display",
            "no graphical session found (neither WAYLAND_DISPLAY nor DISPLAY is set)",
            Some("Run it from your desktop session, not from a plain terminal or an SSH login."),
        );
    }
    check(Level::Ok, "Display", s.chosen, None)
}
/// The system libraries the panel loads when it starts. They are not inside the program: the
/// graphics driver decides which OpenGL is right, so they must come from the system.
pub const X11_LIBRARIES: &[&str] = &[
    "libX11.so.6",
    "libX11-xcb.so.1",
    "libxcb.so.1",
    "libXcursor.so.1",
    "libXi.so.6",
    "libXrender.so.1",
    "libxkbcommon-x11.so.0",
];
pub const WAYLAND_LIBRARIES: &[&str] = &["libwayland-client.so.0", "libwayland-egl.so.1"];
pub const SHARED_LIBRARIES: &[&str] = &["libxkbcommon.so.0", "libGL.so.1", "libEGL.so.1"];
/// Whether the system can load a library by name, exactly as the panel will.
#[cfg(target_os = "linux")]
pub fn loads(name: &str) -> bool {
    // SAFETY: these are plain C libraries of the system; loading one runs no code of ours and
    // the handle is dropped at once.
    unsafe { libloading::Library::new(name) }.is_ok()
}
#[cfg(not(target_os = "linux"))]
pub fn loads(_name: &str) -> bool {
    true
}
/// Every drawing library that cannot be loaded, in a fixed order.
pub fn missing_libraries(loads: impl Fn(&str) -> bool) -> Vec<&'static str> {
    SHARED_LIBRARIES
        .iter()
        .chain(X11_LIBRARIES)
        .chain(WAYLAND_LIBRARIES)
        .copied()
        .filter(|name| !loads(name))
        .collect()
}
/// Blocking only when the session in use lacks one of its own libraries: an X11 library missing
/// on a pure Wayland session, or one of the two OpenGL entry points, does not stop the panel.
pub fn check_libraries(missing: &[&str], s: &Session) -> Check {
    if s.windows {
        return check(
            Level::Ok,
            "Drawing libraries",
            "part of Windows (DirectX and OpenGL)",
            None,
        );
    }
    if missing.is_empty() {
        return check(Level::Ok, "Drawing libraries", "all present", None);
    }
    let lacks = |set: &[&str]| set.iter().any(|name| missing.contains(name));
    let no_opengl = missing.contains(&"libGL.so.1") && missing.contains(&"libEGL.so.1");
    let session_broken = if s.x_display {
        lacks(X11_LIBRARIES)
    } else {
        s.wayland_display && lacks(WAYLAND_LIBRARIES)
    };
    let blocking = no_opengl || missing.contains(&"libxkbcommon.so.0") || session_broken;
    check(
        if blocking { Level::Fail } else { Level::Warn },
        "Drawing libraries",
        format!("not installed: {}", missing.join(", ")),
        Some("Run the installer again: it installs them with your system's package manager (see the install guide for the package names)."),
    )
}
pub fn check_folder(
    name: &'static str,
    what: &str,
    path: &Path,
    missing: Level,
    fix: &str,
) -> Check {
    if path.is_dir() {
        check(
            Level::Ok,
            name,
            format!("{what} found at {}", path.display()),
            None,
        )
    } else {
        check(
            missing,
            name,
            format!("{what} not found at {}", path.display()),
            Some(fix),
        )
    }
}
fn command_output(program: &str, args: &[&str]) -> Option<(bool, String)> {
    let out = std::process::Command::new(sq_sources::program(program))
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(if out.stdout.is_empty() {
        &out.stderr
    } else {
        &out.stdout
    })
    .trim()
    .to_string();
    Some((out.status.success(), text))
}
fn check_github() -> Check {
    match command_output("gh", &["auth", "status"]) {
        None => check(
            Level::Warn,
            "GitHub",
            "the `gh` command is not installed",
            Some("Install the GitHub CLI (https://cli.github.com) to see GitHub activity."),
        ),
        Some((false, _)) => check(
            Level::Warn,
            "GitHub",
            "`gh` is installed but not signed in",
            Some("Run: gh auth login"),
        ),
        Some((true, _)) => check(Level::Ok, "GitHub", "`gh` is signed in", None),
    }
}
fn check_writable(dir: &Path) -> Check {
    let probe = dir.join(".doctor-write-test");
    let ok = std::fs::create_dir_all(dir).is_ok()
        && std::fs::write(&probe, b"x").is_ok()
        && std::fs::remove_file(&probe).is_ok();
    if ok {
        check(
            Level::Ok,
            "Settings folder",
            format!("{} is writable", dir.display()),
            None,
        )
    } else {
        check(
            Level::Fail,
            "Settings folder",
            format!("{} cannot be written", dir.display()),
            Some("Check the folder's permissions, or set STATUS_QUO_STATE_DIR to another folder."),
        )
    }
}
/// Gather the facts of this computer and build the report.
pub fn gather() -> Vec<Check> {
    let mut checks = Vec::new();
    let windows = cfg!(windows);
    let system = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);
    if windows {
        let version = std::process::Command::new("cmd")
            .args(["/C", "ver"])
            .output()
            .ok()
            .and_then(|o| windows_name(&String::from_utf8_lossy(&o.stdout)));
        checks.push(match version {
            Some(v) if v >= 10 => {
                check(Level::Ok, "System", format!("Windows {v} ({system})"), None)
            }
            Some(v) => check(
                Level::Fail,
                "System",
                format!("Windows {v} is too old"),
                Some("Status Quo needs Windows 10 or 11."),
            ),
            None => check(
                Level::Warn,
                "System",
                format!("{system}; Windows version unknown"),
                None,
            ),
        });
    } else {
        checks.push(check(Level::Ok, "System", system, None));
    }
    let selection =
        crate::host::select(crate::host::forced_backend(), &crate::host::probe_session());
    checks.push(check_display(&Session {
        windows,
        wayland_display: std::env::var_os("WAYLAND_DISPLAY").is_some(),
        x_display: std::env::var_os("DISPLAY").is_some(),
        chosen: &selection.note,
    }));
    checks.push(check_libraries(
        &missing_libraries(loads),
        &Session {
            windows,
            wayland_display: std::env::var_os("WAYLAND_DISPLAY").is_some(),
            x_display: std::env::var_os("DISPLAY").is_some(),
            chosen: "",
        },
    ));
    let state = sq_collector::state_root();
    checks.push(check_writable(&state));
    let sources = std::fs::read_to_string(state.join("config.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| serde_json::from_value::<SourceConfig>(v["config"]["sources"].clone()).ok())
        .unwrap_or_default();
    checks.push(check_folder(
        "Claude",
        "its folder",
        &sq_sources::expand(&sources.claude),
        Level::Warn,
        "Sign in with the `claude` command, or pick the folder in Customize > Sources.",
    ));
    checks.push(check_folder(
        "Codex",
        "its folder",
        &sq_sources::expand(&sources.codex),
        Level::Warn,
        "Sign in with `codex login`, or pick the folder in Customize > Sources.",
    ));
    checks.push(check_github());
    checks.push(check_folder(
        "Workflow memory",
        "the notes folder",
        &sq_sources::expand(&sources.memory),
        Level::Warn,
        "Optional. Pick your notes folder in Customize > Sources.",
    ));
    checks.push(
        match (sq_sources::cpu_counters(), sq_sources::memory_sample()) {
            (Some(_), Some(_)) => check(
                Level::Ok,
                "Machine sensors",
                "CPU and memory can be read",
                None,
            ),
            _ => check(
                Level::Warn,
                "Machine sensors",
                "CPU or memory cannot be read here",
                Some("The system and RAM widgets will show unavailable."),
            ),
        },
    );
    checks.push(match sq_sources::locate() {
        Ok(path) => check(
            Level::Ok,
            "Wallpaper",
            format!("found {}", path.display()),
            None,
        ),
        Err(why) => check(
            Level::Warn,
            "Wallpaper",
            why,
            Some("Optional: set STATUS_QUO_WALLPAPER to a picture to use wallpaper colours."),
        ),
    });
    checks.push(if crate::install::is_installed() {
        check(Level::Ok, "Installation", "installed for this user", None)
    } else {
        check(
            Level::Warn,
            "Installation",
            "not installed for this user",
            Some("Run: status-quo install"),
        )
    });
    checks.push(
        match sq_collector::Client::default_path().request(sq_contract::Command::Ping) {
            Ok(_) => check(Level::Ok, "Collector", "running", None),
            Err(_) => check(
                Level::Warn,
                "Collector",
                "not running",
                Some("It starts together with the panel: run status-quo."),
            ),
        },
    );
    checks
}
pub fn run() -> i32 {
    let checks = gather();
    print!("{}", render(&checks));
    exit_code(&checks)
}

#[cfg(test)]
mod tests {
    use super::*;

    // REQ SQ-41
    #[test]
    fn report_lists_each_check_with_a_hint_only_under_problems_and_sets_the_exit_code() {
        let ok = check(Level::Ok, "A", "fine", Some("never shown"));
        let warn = check(Level::Warn, "B", "optional missing", Some("do this"));
        let fail = check(Level::Fail, "C", "broken", Some("fix that"));
        let text = render(&[ok.clone(), warn.clone()]);
        assert!(text.contains("[ok]  A: fine\n"));
        assert!(text.contains("[!]   B: optional missing\n      -> do this\n"));
        assert!(!text.contains("never shown"));
        assert!(text.contains("Ready to run. 1 optional"));
        assert_eq!(exit_code(&[ok.clone(), warn.clone()]), 0);
        let text = render(&[ok.clone(), fail.clone()]);
        assert!(text.contains("[x]   C: broken\n      -> fix that\n"));
        assert!(text.contains("1 problem(s) above must be fixed"));
        assert_eq!(exit_code(&[ok.clone(), fail]), 1);
        assert!(render(&[ok]).contains("Everything is ready."));
    }

    // REQ SQ-47
    #[test]
    fn missing_drawing_libraries_block_only_the_session_that_needs_them() {
        let session = |wayland_display, x_display| Session {
            windows: false,
            wayland_display,
            x_display,
            chosen: "",
        };
        // The list is exactly what cannot be loaded, in a fixed order.
        let missing = missing_libraries(|name| name != "libXi.so.6" && name != "libGL.so.1");
        assert_eq!(missing, ["libGL.so.1", "libXi.so.6"]);
        assert!(missing_libraries(|_| true).is_empty());
        assert_eq!(missing_libraries(|_| false).len(), 12);
        assert_eq!(check_libraries(&[], &session(false, true)).level, Level::Ok);
        // An X11 library missing: blocking on X11, a warning on a pure Wayland session.
        let x = check_libraries(&["libXi.so.6"], &session(false, true));
        assert_eq!(x.level, Level::Fail);
        assert!(x.detail.contains("libXi.so.6") && x.fix.is_some());
        assert_eq!(
            check_libraries(&["libXi.so.6"], &session(true, false)).level,
            Level::Warn
        );
        assert_eq!(
            check_libraries(&["libwayland-egl.so.1"], &session(true, false)).level,
            Level::Fail
        );
        // One OpenGL entry point is enough; none is blocking everywhere.
        assert_eq!(
            check_libraries(&["libEGL.so.1"], &session(false, true)).level,
            Level::Warn
        );
        assert_eq!(
            check_libraries(&["libGL.so.1", "libEGL.so.1"], &session(false, false)).level,
            Level::Fail
        );
        let windows = Session {
            windows: true,
            ..session(false, false)
        };
        assert_eq!(check_libraries(&["libGL.so.1"], &windows).level, Level::Ok);
    }

    // REQ SQ-41
    #[test]
    fn windows_version_is_read_from_the_ver_output() {
        assert_eq!(windows_name("garbage"), None);
        assert_eq!(
            windows_name("Microsoft Windows [Version 10.0.26100.1]"),
            Some(11)
        );
        assert_eq!(
            windows_name("Microsoft Windows [Version 10.0.19044.1288]"),
            Some(10)
        );
        assert_eq!(
            windows_name("Microsoft Windows [Version 6.1.7601]"),
            Some(6)
        );
    }

    // REQ SQ-41
    #[test]
    fn a_session_without_a_display_is_a_blocking_problem_on_linux_only() {
        let none = Session {
            windows: false,
            wayland_display: false,
            x_display: false,
            chosen: "x",
        };
        assert_eq!(check_display(&none).level, Level::Fail);
        let wayland = Session {
            wayland_display: true,
            chosen: "layer-shell",
            ..none
        };
        let c = check_display(&wayland);
        assert_eq!((c.level, c.detail.as_str()), (Level::Ok, "layer-shell"));
        let windows = Session {
            windows: true,
            wayland_display: false,
            x_display: false,
            chosen: "",
        };
        assert_eq!(check_display(&windows).level, Level::Ok);
    }

    // REQ SQ-41
    #[test]
    fn missing_folders_use_the_level_the_caller_chose_and_name_the_fix() {
        let scratch = crate::scratch::Scratch::new("sq-doctor");
        let root = scratch.to_path_buf();
        let found = check_folder("N", "its folder", &root, Level::Warn, "fix");
        assert_eq!(found.level, Level::Ok);
        let gone = check_folder(
            "N",
            "its folder",
            &root.join("nope"),
            Level::Fail,
            "make it",
        );
        assert_eq!(
            (gone.level, gone.fix.as_deref()),
            (Level::Fail, Some("make it"))
        );
        assert!(check_writable(&root).level == Level::Ok);
        assert!(check_writable(&root.join("file-not-dir").join("x")).level != Level::Warn);
    }
}
