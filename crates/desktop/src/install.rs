//! Reversible per-user integration with checked writes and explicit file ownership.
use fs2::FileExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sq_store::private::PrivateOpen;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};
const EVENTS: &[&str] = &[
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "SubagentStart",
    "SubagentStop",
    "Stop",
];
const MANAGED: &[&str] = &[
    ".local/bin/status-quo",
    ".config/systemd/user/status-quo-collector.service",
    ".config/systemd/user/status-quo-panel.service",
    ".local/share/applications/status-quo.desktop",
    ".config/autostart/status-quo.desktop",
];
const MANIFEST: &str = ".local/share/status-quo/install.json";
/// Windows keeps the program with the user's local application data, and starts it at login from
/// the Startup folder (no service manager, no administrator rights).
const WINDOWS_MANAGED: &[&str] = &[
    "AppData/Local/status-quo/bin/status-quo.exe",
    "AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Startup/status-quo.vbs",
];
const WINDOWS_MANIFEST: &str = "AppData/Local/status-quo/install.json";
fn managed() -> &'static [&'static str] {
    if cfg!(windows) {
        WINDOWS_MANAGED
    } else {
        MANAGED
    }
}
/// A managed path below the home folder with the platform's own separators, so a Windows path
/// reads `C:\Users\me\AppData\Local\...` instead of mixing `\` and `/`.
fn under(home: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(home.to_path_buf(), |path, part| path.join(part))
}
fn manifest() -> &'static str {
    if cfg!(windows) {
        WINDOWS_MANIFEST
    } else {
        MANIFEST
    }
}
const MAX_FILE: u64 = 256 * 1024 * 1024;
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
fn shell_word(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
/// The program path as the assistants' hook shells accept it. Unix shells take single quotes.
/// On Windows the path is double-quoted with forward slashes, which Git Bash and cmd both run.
fn hook_program(binary: &Path) -> String {
    if cfg!(windows) {
        format!("\"{}\"", binary.to_string_lossy().replace('\\', "/"))
    } else {
        shell_word(&binary.to_string_lossy())
    }
}
fn hook_command(binary: &Path, provider: &str, event: &str) -> String {
    format!("{} hook {} {}", hook_program(binary), provider, event)
}
pub fn hooks(
    mut value: Value,
    binary: &Path,
    provider: &str,
    enable: bool,
) -> Result<Value, String> {
    if !matches!(provider, "claude" | "codex") {
        return Err("Unsupported assistant hook provider".into());
    }
    let object = value
        .as_object_mut()
        .ok_or("Assistant settings must be an object; original retained")?;
    if !enable && !object.contains_key("hooks") {
        return Ok(value);
    }
    let hooks = object
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("Hooks must be an object; original retained")?;
    for event in EVENTS {
        if !enable && !hooks.contains_key(*event) {
            continue;
        }
        let groups = hooks
            .entry(*event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or("Hook event must be an array; original retained")?;
        for group in groups.iter() {
            let group = group
                .as_object()
                .ok_or("Hook group must be an object; original retained")?;
            let commands = group
                .get("hooks")
                .and_then(Value::as_array)
                .ok_or("Hook group commands must be an array; original retained")?;
            if commands.iter().any(|entry| !entry.is_object()) {
                return Err("Hook commands must be objects; original retained".into());
            }
        }
        let command = hook_command(binary, provider, event);
        groups.retain_mut(|group| {
            let has_metadata = group.as_object().is_none_or(|object| object.len() != 1);
            let Some(commands) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                return true;
            };
            let previous = commands.len();
            commands.retain(|entry| {
                entry.get("command").and_then(Value::as_str) != Some(command.as_str())
            });
            has_metadata || !commands.is_empty() || previous == commands.len()
        });
        if enable {
            groups.push(json!({"hooks":[{"type":"command","command":command,"timeout":3}]}));
        }
    }
    Ok(value)
}
fn fingerprint(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Inspect {}: {e}", path.display())),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!(
            "{} is not a regular file; retained",
            path.display()
        ));
    }
    if metadata.len() > MAX_FILE {
        return Err(format!(
            "{} exceeds installation size limit; retained",
            path.display()
        ));
    }
    fs::read(path)
        .map(Some)
        .map_err(|e| format!("Read {}: {e}", path.display()))
}
fn unchanged(path: &Path, expected: &Option<Vec<u8>>) -> Result<(), String> {
    if &read_optional(path)? != expected {
        return Err(format!("{} changed during this operation; original retained, rerun after the other writer finishes",path.display()));
    }
    Ok(())
}
fn write_checked(
    path: &Path,
    expected: &Option<Vec<u8>>,
    bytes: &[u8],
    mode: u32,
) -> Result<(), String> {
    unchanged(path, expected)?;
    let parent = path.parent().ok_or("Installation file needs a parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = parent.join(format!(
        ".status-quo-{}-{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .private(mode)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        unchanged(path, expected)?;
        fs::rename(&temporary, path).map_err(|e| e.to_string())?;
        sq_store::private::sync_dir(parent).map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
fn remove_checked(path: &Path, expected: &Option<Vec<u8>>) -> Result<(), String> {
    unchanged(path, expected)?;
    if expected.is_some() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn encode(value: &Value) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    Ok(bytes)
}
#[derive(Clone)]
struct Ownership {
    hashes: BTreeMap<String, Vec<String>>,
}
impl Ownership {
    fn parse(bytes: &[u8]) -> Result<Self, String> {
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|e| format!("Invalid installation manifest: {e}; retained"))?;
        if value["application"] != "Status Quo"
            || value["schema"] != 1
            || !matches!(
                value["phase"].as_str(),
                Some("pending" | "installed" | "removing")
            )
        {
            return Err(
                "Unrecognized Status Quo installation manifest; no ownership assumed".into(),
            );
        }
        let records = value["files"]
            .as_object()
            .ok_or("Installation manifest has no managed-file fingerprints")?;
        if records.len() != managed().len() {
            return Err("Installation manifest file list is incompatible; retained".into());
        }
        let mut hashes = BTreeMap::new();
        for relative in managed() {
            let list = records
                .get(*relative)
                .and_then(Value::as_array)
                .ok_or("Installation manifest has an unexpected file list")?;
            if list.is_empty() || list.len() > 4 {
                return Err("Invalid installation ownership fingerprints".into());
            }
            let mut accepted = Vec::new();
            for hash in list {
                let hash = hash.as_str().ok_or("Invalid installation fingerprint")?;
                if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("Invalid installation fingerprint".into());
                }
                accepted.push(hash.to_string());
            }
            hashes.insert(relative.to_string(), accepted);
        }
        Ok(Self { hashes })
    }
    fn encode(&self, phase: &str) -> Result<Vec<u8>, String> {
        encode(
            &json!({"application":"Status Quo","schema":1,"version":env!("CARGO_PKG_VERSION"),"phase":phase,"files":self.hashes}),
        )
    }
    fn permits(&self, relative: &str, bytes: &[u8]) -> bool {
        self.hashes
            .get(relative)
            .is_some_and(|hashes| hashes.contains(&fingerprint(bytes)))
    }
}
struct Edit {
    path: PathBuf,
    before: Option<Vec<u8>>,
    after: Vec<u8>,
    mode: u32,
}
fn hook_edits(home: &Path, binary: &Path, enable: bool) -> Result<Vec<Edit>, String> {
    let mut edits = Vec::new();
    for (relative, provider) in [
        (".claude/settings.json", "claude"),
        (".codex/hooks.json", "codex"),
    ] {
        let path = under(home, relative);
        let before = read_optional(&path)?;
        if before.is_none() && !enable {
            continue;
        }
        let value = match &before {
            Some(bytes) => serde_json::from_slice(bytes)
                .map_err(|e| format!("Invalid {}: {e}; original retained", path.display()))?,
            None => json!({}),
        };
        let updated = hooks(value.clone(), binary, provider, enable)?;
        if updated != value {
            edits.push(Edit {
                path,
                before,
                after: encode(&updated)?,
                mode: 0o600,
            });
        }
    }
    Ok(edits)
}
struct InstallLock {
    file: File,
}
impl InstallLock {
    fn acquire(home: &Path) -> Result<Self, String> {
        let directory = under(home, manifest())
            .parent()
            .map_or_else(|| home.to_path_buf(), Path::to_path_buf);
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let path = directory.join("install.lock");
        if fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file() || m.file_type().is_symlink()) {
            return Err("Installation lock is not a regular file; retained".into());
        }
        let file = OpenOptions::new()
            .write(true)
            .read(true)
            .create(true)
            .truncate(false)
            .private(0o600)
            .open(path)
            .map_err(|e| e.to_string())?;
        file.try_lock_exclusive()
            .map_err(|_| "Another Status Quo installation operation is running")?;
        Ok(Self { file })
    }
}
impl Drop for InstallLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}
fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|h| !h.is_empty()))
        .map(PathBuf::from)
        .ok_or_else(|| "HOME and USERPROFILE unavailable".into())
}
fn text_path(path: &Path) -> Result<&str, String> {
    let value = path
        .to_str()
        .ok_or("Installation path must be valid UTF-8")?;
    if !path.is_absolute() || value.chars().any(char::is_control) {
        return Err("Installation path must be absolute and contain no control characters".into());
    }
    Ok(value)
}
fn escaped(path: &Path, unit: bool) -> Result<String, String> {
    let value = text_path(path)?
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    Ok(if unit {
        value.replace('$', "$$")
    } else {
        value.replace('$', "\\$").replace('`', "\\`")
    })
}
/// The files Windows needs: the program and a hidden launcher in the Startup folder. The launcher
/// is a script because a shortcut cannot be written without COM, and a plain batch file would
/// flash a console window.
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_files(home: &Path, current: &Path) -> Result<Vec<(String, Vec<u8>, u32)>, String> {
    let binary = under(home, WINDOWS_MANAGED[0]);
    let program = text_path(&binary)?.replace('"', "\"\"");
    let bytes = read_optional(current)?.ok_or("Running executable is unavailable")?;
    let launcher = format!(
        "' Status Quo managed launcher\r\nCreateObject(\"WScript.Shell\").Run \"\"\"{program}\"\" login\", 0, False\r\n"
    );
    Ok(vec![
        (WINDOWS_MANAGED[0].into(), bytes, 0o755),
        (WINDOWS_MANAGED[1].into(), launcher.into_bytes(), 0o644),
    ])
}
fn desired_files(home: &Path, current: &Path) -> Result<Vec<(String, Vec<u8>, u32)>, String> {
    if cfg!(windows) {
        return windows_files(home, current);
    }
    let binary = home.join(MANAGED[0]);
    let executable = escaped(&binary, true)?;
    let desktop_executable = escaped(&binary, false)?;
    let bytes = read_optional(current)?.ok_or("Running executable is unavailable")?;
    let mut files = vec![(MANAGED[0].into(), bytes, 0o755)];
    for (relative, name, args) in [
        (MANAGED[1], "collector", " collect"),
        (MANAGED[2], "panel", ""),
    ] {
        let timeout = if name == "collector" {
            "TimeoutStopSec=150\n"
        } else {
            ""
        };
        files.push((relative.into(),format!("# Status Quo managed user service\n[Unit]\nDescription=Status Quo {name}\nAfter=graphical-session.target\n[Service]\nExecStart=\"{executable}\"{args}\nRestart=on-failure\nRestartSec=3\n{timeout}").into_bytes(),0o644));
    }
    for (relative, verb) in [(MANAGED[3], "launch"), (MANAGED[4], "login")] {
        files.push((relative.into(),format!("# Status Quo managed desktop entry\n[Desktop Entry]\nType=Application\nName=Status Quo\nComment=Modular AI development desktop widgets\nExec=\"{desktop_executable}\" {verb}\nTerminal=false\nCategories=Utility;Development;\n").into_bytes(),0o644));
    }
    Ok(files)
}
fn install_at(
    home: &Path,
    current: &Path,
    adopt: bool,
    control: &mut impl FnMut(&[&str]) -> Result<(), String>,
) -> Result<(), String> {
    text_path(home)?;
    let _lock = InstallLock::acquire(home)?;
    let manifest_path = under(home, manifest());
    let previous_manifest = read_optional(&manifest_path)?;
    let ownership = previous_manifest
        .as_deref()
        .map(Ownership::parse)
        .transpose()?;
    let files = desired_files(home, current)?;
    let mut edits = Vec::new();
    let mut pending = Ownership {
        hashes: BTreeMap::new(),
    };
    let mut finished = Ownership {
        hashes: BTreeMap::new(),
    };
    let mut backups = Vec::new();
    for (relative, after, mode) in files {
        let path = under(home, &relative);
        let before = read_optional(&path)?;
        if let Some(old) = &before {
            if !ownership
                .as_ref()
                .is_some_and(|owner| owner.permits(&relative, old))
            {
                if !adopt {
                    return Err(format!(
                        "{} is unmanaged or changed ownership; original retained. To keep a copy and take it over, run `status-quo install --adopt`",
                        path.display()
                    ));
                }
                backups.push((backup_path(&path), old.clone(), mode));
            }
        }
        let hash = fingerprint(&after);
        let mut hashes = before
            .as_ref()
            .map(|bytes| vec![fingerprint(bytes)])
            .unwrap_or_default();
        if !hashes.contains(&hash) {
            hashes.push(hash.clone());
        }
        pending.hashes.insert(relative.clone(), hashes);
        finished.hashes.insert(relative, vec![hash]);
        edits.push(Edit {
            path,
            before,
            after,
            mode,
        });
    }
    let hook_edits = hook_edits(home, &under(home, managed()[0]), true)?;
    // All managed paths and both assistant configurations have passed preflight. Adopted files
    // are kept first, so nothing is overwritten before its copy exists.
    for (backup, bytes, mode) in &backups {
        write_checked(backup, &None, bytes, *mode)?;
    }
    // A pending manifest makes an interrupted first install/upgrade recoverable.
    let pending_bytes = pending.encode("pending")?;
    write_checked(&manifest_path, &previous_manifest, &pending_bytes, 0o600)?;
    // Windows cannot replace a program that is running, so stop it first (no-op elsewhere).
    control(&["stop-all"])?;
    for edit in edits.iter().chain(hook_edits.iter()) {
        if edit.before.as_deref() != Some(edit.after.as_slice()) {
            if let Err(error) = write_checked(&edit.path, &edit.before, &edit.after, edit.mode) {
                return Err(format!(
                    "{error}. Pending installation ownership retained; rerun install to resume"
                ));
            }
        }
    }
    control(&["daemon-reload"])
        .map_err(|e| format!("{e}; pending installation retained, rerun install to resume"))?;
    write_checked(
        &manifest_path,
        &Some(pending_bytes),
        &finished.encode("installed")?,
        0o600,
    )?;
    Ok(())
}
/// A name next to `path` that does not exist yet: `<file>.before-status-quo`, then `.1`, `.2`.
fn backup_path(path: &Path) -> PathBuf {
    let base = format!("{}.before-status-quo", path.display());
    let mut candidate = PathBuf::from(&base);
    let mut n = 0;
    while candidate.symlink_metadata().is_ok() {
        n += 1;
        candidate = PathBuf::from(format!("{base}.{n}"));
    }
    candidate
}
pub fn install(adopt: bool) -> Result<(), String> {
    let home = home()?;
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    install_at(&home, &current, adopt, &mut systemctl)?;
    // An upgrade: a panel that is running the previous version restarts with the new one.
    let _ = systemctl(&["try-restart", UNITS[0], UNITS[1]]);
    println!("Installed Status Quo to {}. Existing hooks retained. Login startup follows the app configuration.",under(&home, managed()[0]).display());
    Ok(())
}
const UNITS: [&str; 2] = ["status-quo-collector.service", "status-quo-panel.service"];
/// How long `launch --verify` watches the freshly started panel before calling it running.
const VERIFY: std::time::Duration = std::time::Duration::from_secs(8);

/// Whether this user has a systemd manager to talk to. Many systems have none (Void, Alpine,
/// Devuan, MX Linux, containers, Windows): the panel then runs as an ordinary background process
/// and starts at login from the desktop's autostart entry.
fn systemd_user() -> bool {
    static ANSWER: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ANSWER.get_or_init(|| {
        cfg!(not(windows))
            && std::env::var_os("STATUS_QUO_NO_SYSTEMD").is_none()
            && Command::new("systemctl")
                .args(["--user", "show-environment"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
    })
}
/// What a service request means on a system without systemd: the panel and its collector are
/// plain processes of this user. Returns the started panel, when one was started.
#[derive(Debug, PartialEq, Eq)]
pub enum Plain {
    Nothing,
    Stop,
    Start,
    StopThenStart,
}
pub fn plain_action(args: &[&str], running: bool) -> Plain {
    match args.first().copied() {
        Some("start") => Plain::Start,
        Some("restart") => Plain::StopThenStart,
        Some("try-restart") if running => Plain::StopThenStart,
        Some("stop") | Some("stop-all") => Plain::Stop,
        _ => Plain::Nothing,
    }
}
/// Every other Status Quo process of this user (panel, collector), never this one.
#[cfg(not(windows))]
fn other_processes() -> Vec<u32> {
    let mut programs: Vec<PathBuf> = std::env::current_exe().ok().into_iter().collect();
    if let Ok(home) = home() {
        programs.push(under(&home, MANAGED[0]));
    }
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| *pid != std::process::id())
        .filter(|pid| {
            fs::read_link(format!("/proc/{pid}/exe")).is_ok_and(|exe| {
                // A replaced program reads "<path> (deleted)".
                let exe = exe.to_string_lossy().replace(" (deleted)", "");
                programs.iter().any(|p| p.to_string_lossy() == exe)
            })
        })
        .collect()
}
#[cfg(not(windows))]
fn stop_others() {
    // `kill` of the shell itself: minimal systems (Debian and Void base installs) have no
    // separate kill program, and then the old panel was never stopped.
    let signal = |how: &str| {
        let pids: Vec<String> = other_processes().iter().map(u32::to_string).collect();
        if pids.is_empty() {
            return true;
        }
        let _ = Command::new("sh")
            .arg("-c")
            .arg(format!("kill {how} {}", pids.join(" ")))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        for _ in 0..30 {
            if other_processes().is_empty() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        false
    };
    // Ask first; a panel that does not answer within three seconds is ended.
    if !signal("-TERM") {
        signal("-KILL");
    }
}
#[cfg(windows)]
fn other_processes() -> Vec<u32> {
    Vec::new()
}
#[cfg(windows)]
fn stop_others() {
    let _ = Command::new("taskkill")
        .args([
            "/F",
            "/FI",
            &format!("PID ne {}", std::process::id()),
            "/IM",
            "status-quo.exe",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}
/// Where the background panel writes what it would print (the reason, when it cannot start).
fn panel_log() -> PathBuf {
    sq_collector::state_root().join("panel.log")
}
/// Windows hands every inheritable handle of this process to a program it starts: not only
/// the three standard streams, also handles this process merely received from the shell that
/// ran the installer. When the installer's output goes to a pipe or a file (a script, a log,
/// CI), the panel would hold that pipe open for as long as it runs and whoever reads it would
/// wait forever. Before the panel is started, every handle of this short-lived process is
/// therefore marked "not inherited"; the panel gets its own streams (nothing in, nothing out,
/// errors to panel.log), which are created after this call.
#[cfg(windows)]
fn keep_own_handles() {
    #[link(name = "kernel32")]
    extern "system" {
        fn SetHandleInformation(handle: *mut std::ffi::c_void, mask: u32, flags: u32) -> i32;
    }
    const HANDLE_FLAG_INHERIT: u32 = 1;
    // Handle values are small multiples of four; a value that is no handle fails harmlessly.
    for value in (4..=0x1_0000usize).step_by(4) {
        // SAFETY: the call only clears the inherit flag of a handle of this process, or fails
        // when the value is not a handle; it never closes or changes what the handle refers to.
        unsafe { SetHandleInformation(value as *mut std::ffi::c_void, HANDLE_FLAG_INHERIT, 0) };
    }
}
/// Start the panel as a background process of this user, detached from this terminal and
/// without a console window. The installed program is used when there is one.
fn start_detached() -> Result<std::process::Child, String> {
    let installed = home().map(|h| under(&h, managed()[0]));
    let program = match installed {
        Ok(path) if path.is_file() => path,
        _ => std::env::current_exe().map_err(|e| e.to_string())?,
    };
    let log_path = panel_log();
    if let Some(dir) = log_path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let log = File::create(&log_path)
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null());
    let mut command = Command::new(program);
    #[cfg(windows)]
    keep_own_handles();
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(
        &mut command,
        0x0000_0008 | 0x0800_0000, // detached, no console window
    );
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .spawn()
        .map_err(|e| format!("Start Status Quo: {e}"))
}
fn plain_control(args: &[&str]) -> Result<Option<std::process::Child>, String> {
    match plain_action(args, !other_processes().is_empty()) {
        Plain::Nothing => Ok(None),
        Plain::Stop => {
            stop_others();
            Ok(None)
        }
        Plain::Start => start_detached().map(Some),
        Plain::StopThenStart => {
            stop_others();
            start_detached().map(Some)
        }
    }
}
/// Service control: `systemctl --user` where this user has systemd, plain processes elsewhere
/// (Windows, and Linux systems without systemd).
pub fn systemctl(args: &[&str]) -> Result<(), String> {
    if !systemd_user() {
        // Linux replaces a running program file safely; only Windows must stop it first.
        if cfg!(not(windows)) && args.first() == Some(&"stop-all") {
            return Ok(());
        }
        return plain_control(args).map(|_| ());
    }
    if args.first() == Some(&"stop-all") {
        return Ok(());
    }
    let status = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("User service operation failed: {}", args.join(" ")))
    }
}
/// The last lines of a text, for an error message.
pub fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}
/// Close the running panel and its collector. They start again at the next login.
pub fn stop() -> Result<(), String> {
    systemctl(&["stop", UNITS[0], UNITS[1]])?;
    println!("Status Quo is stopped. Start it again with: status-quo launch");
    Ok(())
}
/// Watch a freshly started panel process: it must still be running after `VERIFY`. A panel that
/// ends at once without an error found another panel already running, which is fine.
fn verify_child(mut child: std::process::Child) -> Result<(), String> {
    let deadline = std::time::Instant::now() + VERIFY;
    while std::time::Instant::now() < deadline {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) if status.success() => {
                println!("Status Quo is already running.");
                return Ok(());
            }
            Some(status) => {
                let why = tail(&fs::read_to_string(panel_log()).unwrap_or_default(), 12);
                return Err(format!(
                    "the panel stopped right after starting ({status}). What it reported:\n{why}"
                ));
            }
            None => std::thread::sleep(std::time::Duration::from_millis(250)),
        }
    }
    println!("Status Quo is running.");
    Ok(())
}
/// The same check for the systemd service: active, and not being restarted over and over.
fn verify_units() -> Result<(), String> {
    let ask = |args: &[&str]| {
        Command::new("systemctl")
            .arg("--user")
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };
    let restarts = || ask(&["show", "-p", "NRestarts", "--value", UNITS[1]]);
    let before = restarts();
    std::thread::sleep(VERIFY);
    if ask(&["is-active", UNITS[1]]) == "active" && restarts() == before {
        println!("Status Quo is running.");
        return Ok(());
    }
    let journal = Command::new("journalctl")
        .args([
            "--user",
            "-u",
            UNITS[1],
            "-n",
            "12",
            "--no-pager",
            "-o",
            "cat",
        ])
        .stdin(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    Err(format!(
        "the panel does not stay running. What it reported:\n{}",
        tail(&journal, 12)
    ))
}
/// Start the panel (`login`: only when the saved configuration wants it at login). With
/// `verify` the call returns only after the panel has stayed up, or says why it did not.
pub fn launch(login: bool, verify: bool) -> Result<(), String> {
    if login {
        let path = sq_collector::state_root().join("config.json");
        if path.exists() {
            let saved: sq_store::SavedConfig = sq_store::read_json(&path)?;
            saved.config.validate()?;
            if !saved.config.autostart {
                return Ok(());
            }
        }
    }
    if !systemd_user() {
        let child = start_detached()?;
        return if verify { verify_child(child) } else { Ok(()) };
    }
    let mut environment = vec!["import-environment"];
    for name in [
        "DISPLAY",
        "XAUTHORITY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
    ] {
        if std::env::var_os(name).is_some() {
            environment.push(name);
        }
    }
    if environment.len() > 1 {
        systemctl(&environment)?;
    }
    systemctl(&["start", UNITS[0], UNITS[1]])?;
    if verify {
        verify_units()
    } else {
        Ok(())
    }
}
fn uninstall_at(
    home: &Path,
    control: &mut impl FnMut(&[&str]) -> Result<(), String>,
) -> Result<(), String> {
    text_path(home)?;
    let _lock = InstallLock::acquire(home)?;
    let manifest_path = under(home, manifest());
    let Some(manifest) = read_optional(&manifest_path)? else {
        for relative in managed() {
            if read_optional(&under(home, relative))?.is_some() {
                return Err("No Status Quo installation manifest; existing files retained without assuming ownership".into());
            }
        }
        return Ok(());
    };
    let ownership = Ownership::parse(&manifest)?;
    let mut files = Vec::new();
    for relative in managed() {
        let path = under(home, relative);
        let bytes = read_optional(&path)?;
        if let Some(bytes) = &bytes {
            if !ownership.permits(relative, bytes) {
                return Err(format!(
                    "{} changed ownership; retained. No managed files removed",
                    path.display()
                ));
            }
        }
        files.push((path, bytes));
    }
    let edits = hook_edits(home, &under(home, managed()[0]), false)?;
    // Preflight every removal and both hook files before service or file mutations.
    let removing = ownership.encode("removing")?;
    write_checked(&manifest_path, &Some(manifest), &removing, 0o600)?;
    let units: Vec<_> = files
        .iter()
        .filter(|(path, bytes)| bytes.is_some() && path.extension().is_some_and(|e| e == "service"))
        .filter_map(|(path, _)| path.file_name().and_then(|s| s.to_str()))
        .collect();
    control(&["stop-all"])?;
    if !units.is_empty() {
        let mut args = vec!["stop"];
        args.extend(units);
        control(&args)?;
    }
    for edit in edits {
        write_checked(&edit.path, &edit.before, &edit.after, edit.mode)?;
    }
    for (path, bytes) in &files {
        remove_checked(path, bytes)?;
    }
    control(&["daemon-reload"])?;
    remove_checked(&manifest_path, &Some(removing))?;
    Ok(())
}
/// Whether the install manifest of this user exists.
pub fn is_installed() -> bool {
    home().is_ok_and(|h| under(&h, manifest()).is_file())
}
pub fn uninstall() -> Result<(), String> {
    let home = home()?;
    uninstall_at(&home, &mut systemctl)?;
    println!(
        "Uninstalled Status Quo. Saved layouts and history retained in {}",
        sq_collector::state_root().display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // REQ SQ-47
    #[test]
    fn without_systemd_service_requests_become_plain_process_actions() {
        assert_eq!(plain_action(&["start", "a.service"], false), Plain::Start);
        assert_eq!(plain_action(&["restart"], true), Plain::StopThenStart);
        // An upgrade restarts a running panel and leaves a stopped one alone.
        assert_eq!(plain_action(&["try-restart"], true), Plain::StopThenStart);
        assert_eq!(plain_action(&["try-restart"], false), Plain::Nothing);
        assert_eq!(plain_action(&["stop", "a.service"], true), Plain::Stop);
        assert_eq!(plain_action(&["stop-all"], true), Plain::Stop);
        // systemd housekeeping means nothing without systemd, and must not fail.
        for request in ["daemon-reload", "import-environment"] {
            assert_eq!(plain_action(&[request], true), Plain::Nothing);
        }
        assert_eq!(plain_action(&[], true), Plain::Nothing);
    }

    // REQ SQ-47
    #[test]
    fn a_failed_start_reports_the_last_lines_the_panel_wrote() {
        assert_eq!(tail("a\n\nb\nc\n", 2), "b\nc");
        assert_eq!(tail("only", 5), "only");
        assert_eq!(tail("", 3), "");
    }

    struct Fixture {
        root: crate::scratch::Scratch,
        executable: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = crate::scratch::Scratch::new("sq-install");
            let executable = root.join("source-binary");
            fs::write(&executable, b"synthetic Rust executable fixture").unwrap();
            Self { root, executable }
        }
        fn write(&self, relative: &str, bytes: &[u8]) {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        fn install(&self) -> Result<(), String> {
            install_at(&self.root, &self.executable, false, &mut |_| Ok(()))
        }
        fn install_adopting(&self) -> Result<(), String> {
            install_at(&self.root, &self.executable, true, &mut |_| Ok(()))
        }
        fn uninstall(&self) -> Result<(), String> {
            uninstall_at(&self.root, &mut |_| Ok(()))
        }
    }
    // REQ SQ-42
    #[test]
    fn adopting_keeps_a_backup_takes_ownership_and_never_touches_hook_files() {
        let fixture = Fixture::new();
        let program = fixture.root.join(managed()[0]);
        fixture.write(managed()[0], b"hand-copied build");
        let error = fixture.install().unwrap_err();
        assert!(error.contains("--adopt"), "{error}");
        assert_eq!(fs::read(&program).unwrap(), b"hand-copied build");
        // A bad assistant file stops adoption before any backup or overwrite.
        fixture.write(".codex/hooks.json", b"malformed");
        assert!(fixture.install_adopting().is_err());
        assert_eq!(fs::read(&program).unwrap(), b"hand-copied build");
        assert!(!backup_path(&program).to_string_lossy().ends_with(".1"));
        fixture.write(".codex/hooks.json", b"{}");
        fixture.install_adopting().unwrap();
        let backup = PathBuf::from(format!("{}.before-status-quo", program.display()));
        assert_eq!(fs::read(&backup).unwrap(), b"hand-copied build");
        assert_eq!(
            fs::read(&program).unwrap(),
            fs::read(&fixture.executable).unwrap()
        );
        fixture.install().unwrap();
        // A second adoption never overwrites the first backup.
        fixture.write(managed()[0], b"another hand copy");
        fixture.install_adopting().unwrap();
        assert_eq!(fs::read(&backup).unwrap(), b"hand-copied build");
        assert_eq!(
            fs::read(format!("{}.1", backup.display())).unwrap(),
            b"another hand copy"
        );
        fixture.uninstall().unwrap();
        assert!(!program.exists() && backup.exists());
    }
    // REQ SQ-9 SQ-12 SQ-25
    #[test]
    fn preserves_lookalike_commands_and_preexisting_empty_groups() {
        let original = json!({"hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo /other/status-quo hook unrelated"}]},{"matcher":"original","hooks":[]}]}});
        let removed = hooks(
            original.clone(),
            Path::new("/tmp/status-quo"),
            "codex",
            false,
        )
        .unwrap();
        assert_eq!(removed, original);
        let once = hooks(
            original.clone(),
            Path::new("/tmp/a path/status-quo"),
            "codex",
            true,
        )
        .unwrap();
        let twice = hooks(
            once.clone(),
            Path::new("/tmp/a path/status-quo"),
            "codex",
            true,
        )
        .unwrap();
        assert_eq!(once, twice);
        let removed = hooks(twice, Path::new("/tmp/a path/status-quo"), "codex", false).unwrap();
        assert_eq!(removed["hooks"]["Stop"], original["hooks"]["Stop"]);
        assert_eq!(
            hooks(
                json!({"unrelated":true}),
                Path::new("/tmp/status-quo"),
                "codex",
                false
            )
            .unwrap(),
            json!({"unrelated":true})
        );
    }
    // REQ SQ-9 SQ-12 SQ-25
    #[test]
    fn malformed_hook_groups_are_reported_without_creating_entries() {
        for original in [
            json!({"hooks":{"Stop":["bad"]}}),
            json!({"hooks":{"Stop":[{"hooks":"bad"}]}}),
            json!({"hooks":{"Stop":[{"hooks":["bad"]}]}}),
        ] {
            assert!(hooks(original, Path::new("/tmp/status-quo"), "codex", true).is_err());
        }
    }
    // REQ SQ-9 SQ-12 SQ-17 SQ-25
    #[test]
    fn malformed_second_assistant_and_unmanaged_late_file_preflight_before_mutation() {
        let fixture = Fixture::new();
        let claude = br#"{"model":"keep","hooks":{"Stop":[{"hooks":[{"command":"wiki hook"}]}]}}"#;
        fixture.write(".claude/settings.json", claude);
        fixture.write(".codex/hooks.json", b"malformed");
        assert!(fixture.install().is_err());
        assert!(!fixture.root.join(managed()[0]).exists());
        assert!(!under(&fixture.root, manifest()).exists());
        assert_eq!(
            fs::read(fixture.root.join(".claude/settings.json")).unwrap(),
            claude
        );
        fixture.write(".codex/hooks.json", b"{}");
        fixture.write(managed().last().unwrap(), b"unrelated desktop file");
        assert!(fixture.install().is_err());
        assert!(!fixture.root.join(managed()[0]).exists());
        assert!(!under(&fixture.root, manifest()).exists());
        assert_eq!(
            fs::read(fixture.root.join(".claude/settings.json")).unwrap(),
            claude
        );
    }
    // REQ SQ-9 SQ-12 SQ-17 SQ-25
    #[test]
    fn install_repeat_upgrade_uninstall_preserve_other_hooks_and_saved_state() {
        let fixture = Fixture::new();
        fixture.write(
            ".claude/settings.json",
            br#"{"model":"keep","hooks":{"Stop":[{"hooks":[{"command":"wiki hook"}]}]}}"#,
        );
        fixture.write(
            ".local/state/status-quo/config.json",
            b"retained saved state",
        );
        fixture.install().unwrap();
        let first = fs::read(under(&fixture.root, manifest())).unwrap();
        fixture.install().unwrap();
        assert_eq!(first, fs::read(under(&fixture.root, manifest())).unwrap());
        fs::write(&fixture.executable, b"updated executable").unwrap();
        fixture.install().unwrap();
        assert_eq!(
            fs::read(fixture.root.join(managed()[0])).unwrap(),
            b"updated executable"
        );
        fixture.uninstall().unwrap();
        for path in managed() {
            assert!(!fixture.root.join(path).exists());
        }
        let settings: Value =
            sq_store::read_json(&fixture.root.join(".claude/settings.json")).unwrap();
        assert_eq!(settings["model"], "keep");
        assert_eq!(
            settings["hooks"]["Stop"],
            json!([{"hooks":[{"command":"wiki hook"}]}])
        );
        assert_eq!(
            fs::read(fixture.root.join(".local/state/status-quo/config.json")).unwrap(),
            b"retained saved state"
        );
        assert!(!under(&fixture.root, manifest()).exists());
        fixture.uninstall().unwrap();
    }
    // REQ SQ-12 SQ-17 SQ-25
    #[test]
    fn replaced_binary_or_late_file_is_retained_without_partial_uninstall() {
        for relative in [managed()[0], *managed().last().unwrap()] {
            let fixture = Fixture::new();
            fixture.install().unwrap();
            let settings = fs::read(fixture.root.join(".claude/settings.json")).unwrap();
            fixture.write(relative, b"replacement not owned by Status Quo");
            let mut service_called = false;
            let result = uninstall_at(&fixture.root, &mut |_| {
                service_called = true;
                Ok(())
            });
            assert!(result.is_err());
            assert!(!service_called);
            for path in managed() {
                assert!(fixture.root.join(path).exists());
            }
            assert_eq!(
                fs::read(fixture.root.join(relative)).unwrap(),
                b"replacement not owned by Status Quo"
            );
            assert_eq!(
                fs::read(fixture.root.join(".claude/settings.json")).unwrap(),
                settings
            );
        }
    }
    // REQ SQ-12 SQ-17 SQ-25
    #[test]
    fn interrupted_install_and_uninstall_resume_using_verified_ownership() {
        let fixture = Fixture::new();
        assert!(
            install_at(&fixture.root, &fixture.executable, false, &mut |_| Err(
                "simulated user-bus failure".into()
            ))
            .is_err()
        );
        let record: Value = sq_store::read_json(&under(&fixture.root, manifest())).unwrap();
        assert_eq!(record["phase"], "pending");
        fixture.install().unwrap();
        let result = uninstall_at(&fixture.root, &mut |args| {
            if args == ["daemon-reload"] {
                Err("simulated reload failure".into())
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());
        assert!(under(&fixture.root, manifest()).exists());
        fixture.uninstall().unwrap();
        assert!(!under(&fixture.root, manifest()).exists());
    }
    // REQ SQ-12 SQ-17 SQ-25
    #[test]
    fn concurrent_writer_and_installer_lock_are_detected() {
        let fixture = Fixture::new();
        let path = fixture.root.join("setting.json");
        fs::write(&path, b"before").unwrap();
        let expected = read_optional(&path).unwrap();
        fs::write(&path, b"external update").unwrap();
        assert!(write_checked(&path, &expected, b"ours", 0o600).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"external update");
        let lock = InstallLock::acquire(&fixture.root).unwrap();
        assert!(fixture.install().is_err());
        drop(lock);
        fixture.install().unwrap();
    }
    // REQ SQ-12 SQ-21 SQ-25
    #[test]
    fn manifest_is_validated_and_symlinks_are_retained() {
        let fixture = Fixture::new();
        fixture.write(
            manifest(),
            br#"{"application":"Status Quo","version":"0.1.0"}"#,
        );
        assert!(fixture.install().is_err());
        assert!(!fixture.root.join(managed()[0]).exists());
        fs::remove_file(under(&fixture.root, manifest())).unwrap();
        // Symbolic links need a privilege on Windows, and `escaped` serves the Unix service files.
        #[cfg(unix)]
        {
            let target = fixture.root.join("unrelated");
            fs::write(&target, b"original").unwrap();
            let destination = fixture.root.join(managed()[0]);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(&target, &destination).unwrap();
            assert!(fixture.install().is_err());
            assert_eq!(fs::read(&target).unwrap(), b"original");
            assert!(fs::symlink_metadata(destination)
                .unwrap()
                .file_type()
                .is_symlink());
            assert!(escaped(Path::new("/tmp/line\nbreak/status-quo"), true).is_err());
            assert_eq!(
                escaped(Path::new("/tmp/a$b/status-quo"), true).unwrap(),
                "/tmp/a$$b/status-quo"
            );
        }
    }
}

#[cfg(test)]
mod windows_tests {
    use super::*;
    // REQ SQ-36: the Windows installer writes the program and one hidden Startup launcher.
    #[test]
    fn windows_install_files_are_the_program_and_a_hidden_startup_launcher() {
        let scratch = crate::scratch::Scratch::new("sq-win-install");
        let root = scratch.to_path_buf();
        let exe = root.join("source.exe");
        fs::write(&exe, b"MZ fixture").unwrap();
        let home = std::env::temp_dir().join("Users").join("me home"); // scratch-ok: only a path, nothing is created
        let files = windows_files(&home, &exe).unwrap();
        let names: Vec<_> = files.iter().map(|(n, ..)| n.as_str()).collect();
        assert_eq!(names, WINDOWS_MANAGED);
        assert!(names[0].ends_with(".exe") && names[1].ends_with(".vbs"));
        assert!(names
            .iter()
            .all(|n| !n.starts_with('/') && !n.contains('\\')));
        assert_eq!(files[0].1, b"MZ fixture");
        let launcher = String::from_utf8(files[1].1.clone()).unwrap();
        // Hidden window (0), no waiting (False), the program quoted because the path has a space.
        assert!(launcher.contains("WScript.Shell"));
        assert!(launcher.contains(", 0, False"));
        assert!(launcher.contains("me home"));
        assert!(launcher.contains(" login"));
        assert!(launcher.ends_with("\r\n") && !launcher.contains("\n\n"));
        let _ = fs::remove_dir_all(root);
    }
    // REQ SQ-36: hook commands stay one quoted program path plus fixed words, on every platform.
    #[test]
    fn hook_commands_quote_the_program_path() {
        let command = hook_command(Path::new("/opt/my apps/status-quo"), "claude", "Stop");
        if cfg!(windows) {
            assert_eq!(command, "\"/opt/my apps/status-quo\" hook claude Stop");
        } else {
            assert_eq!(command, "'/opt/my apps/status-quo' hook claude Stop");
        }
        let tricky = hook_command(Path::new("/a/it's here/sq"), "codex", "Stop");
        assert!(tricky.ends_with(" hook codex Stop"));
    }
    // REQ SQ-36: both platforms keep their managed paths relative to the user's home folder.
    #[test]
    fn managed_paths_are_relative_and_distinct() {
        for list in [MANAGED, WINDOWS_MANAGED] {
            let mut seen = std::collections::BTreeSet::new();
            for path in list {
                assert!(Path::new(path).is_relative() && seen.insert(*path));
                assert!(!path.contains(".."));
            }
        }
        assert!(Path::new(WINDOWS_MANIFEST).is_relative());
    }
}
