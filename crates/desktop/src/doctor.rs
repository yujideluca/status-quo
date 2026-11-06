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
/// The major version in the output of `cmd /c ver` ("Microsoft Windows [Version 10.0.19044.1]").
pub fn windows_major(text: &str) -> Option<u32> {
    let after = text.split("Version").nth(1)?;
    after
        .trim_start()
        .split(['.', ' ', ']'])
        .next()?
        .parse()
        .ok()
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
            .and_then(|o| windows_major(&String::from_utf8_lossy(&o.stdout)));
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

    // REQ SQ-41
    #[test]
    fn windows_version_is_read_from_the_ver_output() {
        assert_eq!(
            windows_major("\r\nMicrosoft Windows [Version 10.0.19044.1288]\r\n"),
            Some(10)
        );
        assert_eq!(
            windows_major("Microsoft Windows [Version 6.1.7601]"),
            Some(6)
        );
        assert_eq!(windows_major("garbage"), None);
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
        let root = std::env::temp_dir().join(format!("sq-doctor-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
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
