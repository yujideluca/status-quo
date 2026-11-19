//! Self-update from the GitHub releases (SQ-54).
//!
//! The installed program asks the release page which version is the newest (`version.txt`, a
//! file every release publishes), and when it is newer than itself it downloads the file for
//! its system, checks it against the published SHA-256 and runs the published installer, which
//! stops the panel, replaces the program, keeps settings and hooks and starts the new version.
//! Only the installed program does this: a build started from a source folder, a test copy or
//! a screenshot run never updates itself.
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

/// Where the newest release's files are. `STATUS_QUO_UPDATE_BASE` points elsewhere for tests.
pub const RELEASE_BASE: &str = "https://github.com/yujideluca/status-quo/releases/latest/download";
/// First check this long after the panel starts, then every `EVERY`.
pub const FIRST: Duration = Duration::from_secs(3 * 60);
pub const EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// The wait before the first check; `STATUS_QUO_UPDATE_DELAY` (seconds) shortens it for the
/// real update tests, which run the published panel and wait for it to update itself.
pub fn first_delay(setting: Option<&str>) -> Duration {
    setting
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(FIRST)
}

pub fn base() -> String {
    std::env::var("STATUS_QUO_UPDATE_BASE")
        .ok()
        .filter(|b| !b.trim().is_empty())
        .map(|b| b.trim_end_matches('/').to_string())
        .unwrap_or_else(|| RELEASE_BASE.to_string())
}

/// `v0.1.6`, `0.1.6` or `0.1.6\n` as three numbers.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let text = text.trim();
    let text = text.strip_prefix('v').unwrap_or(text);
    let mut parts = text.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((major, minor, patch))
}

/// True only when `latest` is a readable version above `current`.
pub fn is_newer(current: &str, latest: &str) -> bool {
    match (parse_version(current), parse_version(latest)) {
        (Some(c), Some(l)) => l > c,
        _ => false,
    }
}

/// The release file for a system, or `None` where no prebuilt program exists.
pub fn asset_for(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("windows", "x86_64") => Some("status-quo-setup.exe"),
        ("linux", "x86_64") => Some("status-quo-linux-x86_64.tar.gz"),
        ("linux", "aarch64") => Some("status-quo-linux-aarch64.tar.gz"),
        _ => None,
    }
}

/// The checksum file holds `<64 hex digits> *<name>` (or two spaces): the file matches only
/// when its SHA-256 is that value.
pub fn checksum_matches(bytes: &[u8], checksum_file: &str) -> bool {
    use sha2::{Digest, Sha256};
    let Some(expected) = checksum_file.split_whitespace().next() else {
        return false;
    };
    if expected.len() != 64 || !expected.chars().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }
    let actual: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    actual.eq_ignore_ascii_case(expected)
}

fn log_path() -> PathBuf {
    sq_collector::state_root().join("update.log")
}

fn log(line: &str) {
    let stamp = sq_collector::now();
    // Keep the log small: start over past one megabyte.
    if fs::metadata(log_path()).is_ok_and(|m| m.len() > 1024 * 1024) {
        let _ = fs::remove_file(log_path());
    }
    eprintln!("Status Quo update: {line}");
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
    {
        let _ = writeln!(file, "{stamp} {line}");
    }
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("status-quo/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| e.to_string())
}

fn fetch(client: &reqwest::blocking::Client, url: &str) -> Result<Vec<u8>, String> {
    let response = client.get(url).send().map_err(|e| format!("{url}: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("{url}: {}", response.status()));
    }
    response
        .bytes()
        .map(|b| b.to_vec())
        .map_err(|e| format!("{url}: {e}"))
}

/// `status-quo update` (install a newer release now) and `status-quo update --check` (only say
/// whether there is one). Returns what happened, in one line.
pub fn run(check_only: bool) -> Result<String, String> {
    let current = env!("CARGO_PKG_VERSION");
    let base = base();
    let client = client()?;
    let latest = fetch(&client, &format!("{base}/version.txt"))?;
    let latest = String::from_utf8_lossy(&latest).trim().to_string();
    if parse_version(&latest).is_none() {
        return Err(format!("the release page names no version ({latest:?})"));
    }
    if !is_newer(current, &latest) {
        let message = format!("{current} is the newest version ({latest} published)");
        log(&message);
        return Ok(message);
    }
    if check_only {
        return Ok(format!("{latest} is available; this is {current}"));
    }
    let Some(asset) = asset_for(std::env::consts::OS, std::env::consts::ARCH) else {
        let message = format!(
            "{latest} is available, but there is no ready-made program for this system: reinstall from source"
        );
        log(&message);
        return Ok(message);
    };
    log(&format!("{current} -> {latest}: downloading {asset}"));
    let bytes = fetch(&client, &format!("{base}/{asset}"))?;
    let checksum = fetch(&client, &format!("{base}/{asset}.sha256"))?;
    if !checksum_matches(&bytes, &String::from_utf8_lossy(&checksum)) {
        let message = format!("{asset} of {latest} does not match its checksum: not installed");
        log(&message);
        return Err(message);
    }
    let folder = sq_collector::state_root().join("update");
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let file = folder.join(asset);
    fs::write(&file, &bytes).map_err(|e| e.to_string())?;
    let result = install(&file, &folder);
    match &result {
        Ok(()) => log(&format!("{latest} installed")),
        Err(e) => log(&format!("installing {latest} failed: {e}")),
    }
    result.map(|()| format!("{latest} installed (was {current})"))
}

#[cfg(windows)]
fn install(setup: &Path, _folder: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    // The installer stops this program's panel and collector before it replaces the program,
    // so it must not be a child that dies with them: detached, without a console window.
    Command::new(setup)
        .args(["--yes"])
        .creation_flags(0x0000_0008 | 0x0800_0000)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("start the installer: {e}"))
}

#[cfg(not(windows))]
fn install(archive: &Path, folder: &Path) -> Result<(), String> {
    let unpacked = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(folder)
        .status()
        .map_err(|e| format!("tar: {e}"))?;
    if !unpacked.success() {
        return Err("the download could not be unpacked".into());
    }
    let script = fs::read_dir(folder)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|entry| entry.path().join("install.sh"))
        .find(|path| path.is_file())
        .ok_or("the download has no install.sh")?;
    // Installing restarts the panel and the collector, and with them everything they started.
    // The installer therefore runs on its own: as a separate user service under systemd, else
    // in its own process group; its output goes to update.log, not to a pipe of this process.
    // `--no-deps`: the libraries the program needs are there, this program is running.
    let log = log_path();
    let mut command = if crate::install::systemd_user() {
        let mut command = Command::new("systemd-run");
        command
            .args(["--user", "--quiet", "--collect"])
            .arg(format!(
                "--property=StandardOutput=append:{}",
                log.display()
            ))
            .arg(format!("--property=StandardError=append:{}", log.display()))
            .arg("sh")
            .arg(&script)
            .arg("--no-deps");
        command.stdout(Stdio::null()).stderr(Stdio::null());
        command
    } else {
        let out = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .map_err(|e| e.to_string())?;
        let err = out.try_clone().map_err(|e| e.to_string())?;
        let mut command = Command::new("sh");
        command
            .arg(&script)
            .arg("--no-deps")
            .stdout(out)
            .stderr(err);
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        command
    };
    let status = command
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| format!("start install.sh: {e}"))?
        .wait()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "the installer ended with {status}; its output is above in this log"
        ))
    }
}

/// The panel's background check: only for the installed program, only while the setting is
/// on (asked again before every check), and in a separate process, because installing stops
/// the panel itself.
pub fn schedule(enabled: impl Fn() -> bool + Send + 'static) {
    let Ok(me) = std::env::current_exe() else {
        return;
    };
    let installed = crate::install::installed_program();
    let same = |a: &Path, b: &Path| match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if !installed.as_deref().is_some_and(|path| same(&me, path)) {
        return;
    }
    let first = first_delay(std::env::var("STATUS_QUO_UPDATE_DELAY").ok().as_deref());
    std::thread::spawn(move || {
        std::thread::sleep(first);
        loop {
            if enabled() {
                let mut command = Command::new(&me);
                command
                    .arg("update")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                #[cfg(windows)]
                std::os::windows::process::CommandExt::creation_flags(
                    &mut command,
                    0x0000_0008 | 0x0800_0000,
                );
                #[cfg(unix)]
                std::os::unix::process::CommandExt::process_group(&mut command, 0);
                if let Ok(mut child) = command.spawn() {
                    // Reap it on Linux; on Windows the installer may end this process first.
                    let _ = child.wait();
                }
            }
            std::thread::sleep(EVERY);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // REQ SQ-54: only a higher, readable version counts as an update.
    #[test]
    fn only_a_higher_version_is_an_update() {
        assert_eq!(parse_version("v0.1.6\n"), Some((0, 1, 6)));
        assert_eq!(parse_version("0.10.0"), Some((0, 10, 0)));
        assert_eq!(parse_version("v0.1"), None);
        assert_eq!(parse_version("v0.1.6-rc1"), None);
        assert_eq!(parse_version("<html>"), None);
        assert!(is_newer("0.1.5", "v0.1.6"));
        assert!(is_newer("0.1.9", "v0.1.10"), "numbers, not text");
        assert!(is_newer("0.1.5", "v1.0.0"));
        assert!(!is_newer("0.1.5", "v0.1.5"));
        assert!(!is_newer("0.1.6", "v0.1.5"), "never downgrade");
        assert!(!is_newer("0.1.5", "Not Found"));
    }

    // REQ SQ-54: each system gets the file the release publishes for it, and none elsewhere.
    #[test]
    fn each_system_gets_its_own_release_file() {
        assert_eq!(asset_for("windows", "x86_64"), Some("status-quo-setup.exe"));
        assert_eq!(
            asset_for("linux", "x86_64"),
            Some("status-quo-linux-x86_64.tar.gz")
        );
        assert_eq!(
            asset_for("linux", "aarch64"),
            Some("status-quo-linux-aarch64.tar.gz")
        );
        assert_eq!(asset_for("linux", "riscv64"), None);
        assert_eq!(asset_for("windows", "aarch64"), None);
        assert_eq!(asset_for("macos", "aarch64"), None);
    }

    // REQ SQ-54: a download is installed only when it matches the published checksum.
    #[test]
    fn a_download_must_match_the_published_checksum() {
        // SHA-256 of "abc".
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(checksum_matches(
            b"abc",
            &format!("{abc} *status-quo-setup.exe\n")
        ));
        assert!(checksum_matches(
            b"abc",
            &format!("{}  file", abc.to_uppercase())
        ));
        assert!(!checksum_matches(b"abd", &format!("{abc} *file")));
        assert!(!checksum_matches(b"abc", ""));
        assert!(!checksum_matches(b"abc", "not-a-checksum *file"));
        assert!(!checksum_matches(b"abc", &abc[..63]));
    }

    // REQ SQ-54: the first check waits three minutes unless a test shortens the wait.
    #[test]
    fn the_first_check_waits_unless_a_test_shortens_it() {
        assert_eq!(first_delay(None), Duration::from_secs(180));
        assert_eq!(first_delay(Some("5")), Duration::from_secs(5));
        assert_eq!(first_delay(Some("soon")), Duration::from_secs(180));
        assert_eq!(EVERY, Duration::from_secs(6 * 3600));
    }

    // REQ SQ-54: tests point the updater at their own release folder.
    #[test]
    fn the_release_base_can_be_pointed_elsewhere_for_tests() {
        assert!(RELEASE_BASE.starts_with("https://github.com/"));
        assert!(RELEASE_BASE.ends_with("/releases/latest/download"));
    }
}
