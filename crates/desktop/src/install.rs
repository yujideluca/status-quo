//! Reversible per-user integration with checked writes and explicit file ownership.
use fs2::FileExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
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
const MAX_FILE: u64 = 256 * 1024 * 1024;
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
fn shell_word(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
fn hook_command(binary: &Path, provider: &str, event: &str) -> String {
    format!(
        "{} hook {} {}",
        shell_word(&binary.to_string_lossy()),
        provider,
        event
    )
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
            .mode(mode)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        unchanged(path, expected)?;
        fs::rename(&temporary, path).map_err(|e| e.to_string())?;
        File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
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
        if records.len() != MANAGED.len() {
            return Err("Installation manifest file list is incompatible; retained".into());
        }
        let mut hashes = BTreeMap::new();
        for relative in MANAGED {
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
        let path = home.join(relative);
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
        let directory = home.join(".local/share/status-quo");
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
            .mode(0o600)
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
        .map(PathBuf::from)
        .ok_or_else(|| "HOME unavailable".into())
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
fn desired_files(home: &Path, current: &Path) -> Result<Vec<(String, Vec<u8>, u32)>, String> {
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
    control: &mut impl FnMut(&[&str]) -> Result<(), String>,
) -> Result<(), String> {
    text_path(home)?;
    let _lock = InstallLock::acquire(home)?;
    let manifest_path = home.join(MANIFEST);
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
    for (relative, after, mode) in files {
        let path = home.join(&relative);
        let before = read_optional(&path)?;
        if let Some(old) = &before {
            if !ownership
                .as_ref()
                .is_some_and(|owner| owner.permits(&relative, old))
            {
                return Err(format!(
                    "{} is unmanaged or changed ownership; original retained",
                    path.display()
                ));
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
    let hook_edits = hook_edits(home, &home.join(MANAGED[0]), true)?;
    // All managed paths and both assistant configurations have passed preflight.
    // A pending manifest makes an interrupted first install/upgrade recoverable.
    let pending_bytes = pending.encode("pending")?;
    write_checked(&manifest_path, &previous_manifest, &pending_bytes, 0o600)?;
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
pub fn install() -> Result<(), String> {
    let home = home()?;
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    install_at(&home, &current, &mut systemctl)?;
    println!("Installed Status Quo to {}. Existing hooks retained. Login startup follows the app configuration.",home.join(MANAGED[0]).display());
    Ok(())
}
pub fn systemctl(args: &[&str]) -> Result<(), String> {
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
pub fn launch(login: bool) -> Result<(), String> {
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
    systemctl(&[
        "start",
        "status-quo-collector.service",
        "status-quo-panel.service",
    ])
}
fn uninstall_at(
    home: &Path,
    control: &mut impl FnMut(&[&str]) -> Result<(), String>,
) -> Result<(), String> {
    text_path(home)?;
    let _lock = InstallLock::acquire(home)?;
    let manifest_path = home.join(MANIFEST);
    let Some(manifest) = read_optional(&manifest_path)? else {
        for relative in MANAGED {
            if read_optional(&home.join(relative))?.is_some() {
                return Err("No Status Quo installation manifest; existing files retained without assuming ownership".into());
            }
        }
        return Ok(());
    };
    let ownership = Ownership::parse(&manifest)?;
    let mut files = Vec::new();
    for relative in MANAGED {
        let path = home.join(relative);
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
    let edits = hook_edits(home, &home.join(MANAGED[0]), false)?;
    // Preflight every removal and both hook files before service or file mutations.
    let removing = ownership.encode("removing")?;
    write_checked(&manifest_path, &Some(manifest), &removing, 0o600)?;
    let units: Vec<_> = files
        .iter()
        .filter(|(path, bytes)| bytes.is_some() && path.extension().is_some_and(|e| e == "service"))
        .filter_map(|(path, _)| path.file_name().and_then(|s| s.to_str()))
        .collect();
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
    struct Fixture {
        root: PathBuf,
        executable: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "sq-install-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&root).unwrap();
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
            install_at(&self.root, &self.executable, &mut |_| Ok(()))
        }
        fn uninstall(&self) -> Result<(), String> {
            uninstall_at(&self.root, &mut |_| Ok(()))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
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
        assert!(!fixture.root.join(MANAGED[0]).exists());
        assert!(!fixture.root.join(MANIFEST).exists());
        assert_eq!(
            fs::read(fixture.root.join(".claude/settings.json")).unwrap(),
            claude
        );
        fixture.write(".codex/hooks.json", b"{}");
        fixture.write(MANAGED[4], b"unrelated desktop file");
        assert!(fixture.install().is_err());
        assert!(!fixture.root.join(MANAGED[0]).exists());
        assert!(!fixture.root.join(MANIFEST).exists());
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
        let first = fs::read(fixture.root.join(MANIFEST)).unwrap();
        fixture.install().unwrap();
        assert_eq!(first, fs::read(fixture.root.join(MANIFEST)).unwrap());
        fs::write(&fixture.executable, b"updated executable").unwrap();
        fixture.install().unwrap();
        assert_eq!(
            fs::read(fixture.root.join(MANAGED[0])).unwrap(),
            b"updated executable"
        );
        fixture.uninstall().unwrap();
        for path in MANAGED {
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
        assert!(!fixture.root.join(MANIFEST).exists());
        fixture.uninstall().unwrap();
    }
    // REQ SQ-12 SQ-17 SQ-25
    #[test]
    fn replaced_binary_or_late_file_is_retained_without_partial_uninstall() {
        for relative in [MANAGED[0], MANAGED[4]] {
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
            for path in MANAGED {
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
        assert!(install_at(&fixture.root, &fixture.executable, &mut |_| Err(
            "simulated user-bus failure".into()
        ))
        .is_err());
        let manifest: Value = sq_store::read_json(&fixture.root.join(MANIFEST)).unwrap();
        assert_eq!(manifest["phase"], "pending");
        fixture.install().unwrap();
        let result = uninstall_at(&fixture.root, &mut |args| {
            if args == ["daemon-reload"] {
                Err("simulated reload failure".into())
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());
        assert!(fixture.root.join(MANIFEST).exists());
        fixture.uninstall().unwrap();
        assert!(!fixture.root.join(MANIFEST).exists());
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
            MANIFEST,
            br#"{"application":"Status Quo","version":"0.1.0"}"#,
        );
        assert!(fixture.install().is_err());
        assert!(!fixture.root.join(MANAGED[0]).exists());
        fs::remove_file(fixture.root.join(MANIFEST)).unwrap();
        let target = fixture.root.join("unrelated");
        fs::write(&target, b"original").unwrap();
        let destination = fixture.root.join(MANAGED[0]);
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
