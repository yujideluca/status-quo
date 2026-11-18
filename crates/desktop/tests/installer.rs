//! The Linux installer script, run in dry-run mode: it prints what it would do, so every
//! distribution family can be checked on any Linux machine without changing it.
#![cfg(unix)]
#[path = "../../scratch.rs"]
mod scratch;
use scratch::Scratch;
use std::{fs, path::PathBuf, process::Command};

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/install.sh")
}
fn scratch(name: &str) -> Scratch {
    Scratch::new(&format!("sq-installer-{name}"))
}
/// Run the script with a made-up os-release and an empty home, as if every dependency were missing.
fn dry_run(name: &str, os_release: &str, args: &[&str]) -> (bool, String) {
    let dir = scratch(name);
    let release = dir.join("os-release");
    fs::write(&release, os_release).unwrap();
    let out = Command::new("bash")
        .arg(script())
        .args(["--dry-run", "--yes"])
        .args(args)
        .env("SQ_OS_RELEASE", &release)
        .env("SQ_ASSUME_MISSING_DEPS", "1")
        .env("SQ_SRC", dir.join("src"))
        .env("HOME", &dir)
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", dir.join("bin").display()),
        )
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

// REQ SQ-41
#[test]
fn each_distribution_family_gets_its_own_package_command() {
    for (name, os, expect) in [
        (
            "ubuntu",
            "ID=ubuntu\nID_LIKE=debian\nPRETTY_NAME=\"Ubuntu\"\n",
            "apt-get install -y build-essential",
        ),
        (
            "mint",
            "ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n",
            "apt-get install -y build-essential",
        ),
        ("fedora", "ID=fedora\n", "dnf install -y gcc"),
        (
            "rocky",
            "ID=rocky\nID_LIKE=\"rhel centos fedora\"\n",
            "dnf install -y gcc",
        ),
        (
            "arch",
            "ID=arch\n",
            "pacman -S --needed --noconfirm base-devel",
        ),
        (
            "manjaro",
            "ID=manjaro\nID_LIKE=arch\n",
            "pacman -S --needed --noconfirm base-devel",
        ),
        (
            "suse",
            "ID=opensuse-tumbleweed\nID_LIKE=\"opensuse suse\"\n",
            "zypper --non-interactive install gcc",
        ),
    ] {
        let (ok, text) = dry_run(name, os, &[]);
        assert!(ok, "{name}: {text}");
        assert!(
            text.contains(expect),
            "{name} should print `{expect}`:\n{text}"
        );
        // The whole pipeline is printed, in order, and nothing is run.
        let order = ["1/6", "2/6", "3/6", "4/6", "5/6", "6/6"];
        let mut at = 0;
        for step in order {
            let found = text[at..]
                .find(step)
                .unwrap_or_else(|| panic!("{name}: step {step} missing:\n{text}"));
            at += found;
        }
        assert!(
            text.contains("cargo build --release --locked -p status-quo"),
            "{name}:\n{text}"
        );
        assert!(
            text.contains("install") && text.contains("doctor"),
            "{name}:\n{text}"
        );
    }
}

// REQ SQ-41
#[test]
fn an_unknown_distribution_stops_with_the_list_of_what_to_install() {
    let (ok, text) = dry_run("unknown", "ID=plan9\n", &[]);
    assert!(!ok);
    assert!(
        text.contains("unknown distribution") && text.contains("--no-deps"),
        "{text}"
    );
    // With the libraries installed by hand the pipeline continues.
    let (ok, text) = dry_run("manual", "ID=plan9\n", &["--no-deps"]);
    assert!(ok, "{text}");
    assert!(text.contains("skipped (--no-deps)"));
}

// REQ SQ-41
#[test]
fn help_bad_options_uninstall_and_missing_rust_behave() {
    // Rust missing: the official rustup installer is the step printed (no sudo, home folder).
    let dir = scratch("norust");
    let out = Command::new("bash")
        .arg(script())
        .args(["--dry-run", "--yes", "--no-deps"])
        .env("SQ_ASSUME_NO_RUST", "1")
        .env("SQ_OS_RELEASE", dir.join("none"))
        .env("SQ_SRC", dir.join("src"))
        .env("HOME", &dir)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && text.contains("https://sh.rustup.rs"),
        "{text}"
    );
    let (ok, text) = dry_run("help", "ID=ubuntu\n", &["--help"]);
    assert!(
        ok && text.contains("--check-only") && text.contains("--uninstall"),
        "{text}"
    );
    let (ok, text) = dry_run("bad", "ID=ubuntu\n", &["--frobnicate"]);
    assert!(!ok && text.contains("unknown option"), "{text}");
    // Uninstall without any program to ask is a clear refusal, not a guess.
    let (ok, text) = dry_run("uninstall", "ID=ubuntu\n", &["--uninstall"]);
    assert!(
        !ok && text.contains("no status-quo program found"),
        "{text}"
    );
}

fn prebuilt_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/install-prebuilt.sh")
}
/// A stand-in for the packaged program. Like the real one it answers `doctor --libs` with the
/// libraries that are missing (here: whatever STUB_MISSING holds), and it fails `launch` when
/// STUB_LAUNCH_FAILS is set. Everything else succeeds silently.
const STUB: &str = "#!/bin/sh\n\
    if [ \"$1 $2\" = \"doctor --libs\" ]; then printf '%s' \"$STUB_MISSING\"; fi\n\
    if [ \"$1\" = launch ] && [ -n \"$STUB_LAUNCH_FAILS\" ]; then echo 'no OpenGL here' >&2; exit 1; fi\n\
    exit 0\n";
fn write_program(path: &std::path::Path, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}
/// One run of the prebuilt installer in a scratch folder.
struct Prebuilt<'a> {
    with_binary: bool,
    args: &'a [&'a str],
    os_release: &'a str,
    /// What the stub program reports as missing libraries.
    missing: &'a str,
    /// Extra environment (DISPLAY, STUB_LAUNCH_FAILS).
    env: &'a [(&'a str, &'a str)],
    /// Fake commands placed first on PATH: (name, script).
    fakes: &'a [(&'a str, &'a str)],
    /// PATH holds only the fakes and the handful of basic tools the script itself uses, so no
    /// package manager of the machine running the test is visible.
    bare_path: bool,
}
impl Default for Prebuilt<'_> {
    fn default() -> Self {
        Self {
            with_binary: true,
            args: &[],
            os_release: "ID=ubuntu\nID_LIKE=debian\n",
            missing: "",
            env: &[],
            fakes: &[],
            bare_path: false,
        }
    }
}
impl Prebuilt<'_> {
    fn run(&self, name: &str) -> (bool, String) {
        let dir = scratch(name);
        fs::copy(prebuilt_script(), dir.join("install.sh")).unwrap();
        if self.with_binary {
            write_program(&dir.join("status-quo"), STUB);
        }
        // The "installed" program the script starts and checks after installing.
        write_program(&dir.join("home/.local/bin/status-quo"), STUB);
        let osr = dir.join("os-release");
        fs::write(&osr, self.os_release).unwrap();
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        for (command, text) in self.fakes {
            write_program(&bin.join(command), text);
        }
        let path = if self.bare_path {
            for tool in [
                "sh", "uname", "dirname", "basename", "sed", "head", "tr", "id", "chmod", "mkdir",
                "cp", "rm",
            ] {
                let real = ["/usr/bin", "/bin"]
                    .iter()
                    .map(|d| PathBuf::from(d).join(tool))
                    .find(|p| p.exists())
                    .unwrap_or_else(|| panic!("{tool} not found"));
                std::os::unix::fs::symlink(real, bin.join(tool)).unwrap();
            }
            bin.display().to_string()
        } else {
            format!("{}:/usr/bin:/bin", bin.display())
        };
        let out = Command::new("sh")
            .arg(dir.join("install.sh"))
            .args(self.args)
            .env("HOME", dir.join("home"))
            .env("PATH", path)
            .env("SQ_OS_RELEASE", &osr)
            .env("STUB_MISSING", self.missing)
            .env_remove("STUB_LAUNCH_FAILS")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .envs(self.env.iter().copied())
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.success(), text)
    }
}
fn run_prebuilt(name: &str, with_binary: bool, args: &[&str]) -> (bool, String) {
    Prebuilt {
        with_binary,
        args,
        ..Prebuilt::default()
    }
    .run(name)
}

// REQ SQ-45
#[test]
fn the_prebuilt_linux_package_installs_without_rust_or_a_compiler() {
    let (ok, text) = run_prebuilt("prebuilt-dry", true, &["--dry-run"]);
    assert!(ok, "{text}");
    assert!(
        text.contains("status-quo install") && text.contains("doctor"),
        "{text}"
    );
    for forbidden in ["cargo", "rustup", "sudo", "git clone"] {
        assert!(!text.contains(forbidden), "{forbidden} in:\n{text}");
    }
    let (ok, text) = run_prebuilt("prebuilt-uninstall", true, &["--dry-run", "--uninstall"]);
    assert!(ok && text.contains("status-quo uninstall"), "{text}");
}

// REQ SQ-45
#[test]
fn the_prebuilt_installer_refuses_a_package_without_the_program_and_unknown_options() {
    let (ok, text) = run_prebuilt("prebuilt-empty", false, &[]);
    assert!(!ok && text.contains("status-quo"), "{text}");
    let (ok, text) = run_prebuilt("prebuilt-bad", true, &["--nonsense"]);
    assert!(!ok && text.contains("unknown option"), "{text}");
}

fn get_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/get.sh")
}
fn sha256_of(file: &std::path::Path) -> String {
    let out = Command::new("sha256sum").arg(file).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}
/// A fake "latest release" folder with a package whose program is a stub, reached by file://.
fn fake_release(dir: &std::path::Path, good_checksum: bool) -> PathBuf {
    let rel = dir.join("release");
    let pkg = dir.join("pkg/status-quo-linux-x86_64");
    fs::create_dir_all(&rel).unwrap();
    fs::create_dir_all(&pkg).unwrap();
    fs::copy(prebuilt_script(), pkg.join("install.sh")).unwrap();
    write_program(&pkg.join("status-quo"), STUB);
    let tar = rel.join("status-quo-linux-x86_64.tar.gz");
    let ok = Command::new("tar")
        .args(["-czf"])
        .arg(&tar)
        .arg("-C")
        .arg(dir.join("pkg"))
        .arg("status-quo-linux-x86_64")
        .status()
        .unwrap()
        .success();
    assert!(ok);
    let sum = if good_checksum {
        sha256_of(&tar)
    } else {
        "0".repeat(64)
    };
    fs::write(
        rel.join("status-quo-linux-x86_64.tar.gz.sha256"),
        format!("{sum}  status-quo-linux-x86_64.tar.gz\n"),
    )
    .unwrap();
    rel
}
fn run_get(dir: &std::path::Path, base: &str, args: &[&str]) -> (bool, String) {
    let out = Command::new("sh")
        .arg(get_script())
        .args(args)
        .env("SQ_RELEASE_BASE", base)
        .env(
            "SQ_RAW_BASE",
            format!("file://{}/../../tools", env!("CARGO_MANIFEST_DIR")),
        )
        .env("SQ_ASSUME_MISSING_DEPS", "0")
        .env("HOME", dir.join("home"))
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

// REQ SQ-46
#[test]
fn one_paste_downloads_verifies_and_runs_the_release_package() {
    let dir = scratch("get-ok");
    let rel = fake_release(&dir, true);
    let (ok, text) = run_get(&dir, &format!("file://{}", rel.display()), &["--dry-run"]);
    assert!(ok, "{text}");
    assert!(text.contains("checksum ok"), "{text}");
    assert!(text.contains("status-quo install"), "{text}");
    assert!(!text.contains("cargo"), "{text}");
}

// REQ SQ-46
#[test]
fn one_paste_refuses_a_package_whose_checksum_does_not_match() {
    let dir = scratch("get-bad");
    let rel = fake_release(&dir, false);
    let (ok, text) = run_get(&dir, &format!("file://{}", rel.display()), &["--dry-run"]);
    assert!(!ok, "{text}");
    assert!(text.contains("checksum"), "{text}");
    assert!(!text.contains("status-quo install"), "{text}");
}

// REQ SQ-46
#[test]
fn one_paste_falls_back_to_the_source_installer_when_no_release_exists() {
    let dir = scratch("get-none");
    let empty = dir.join("nothing");
    fs::create_dir_all(&empty).unwrap();
    let (ok, text) = run_get(
        &dir,
        &format!("file://{}", empty.display()),
        &["--dry-run", "--no-deps"],
    );
    assert!(ok, "{text}");
    assert!(text.contains("no ready-made package"), "{text}");
    assert!(text.contains("1/6") && text.contains("6/6"), "{text}");
}

// REQ SQ-45
#[test]
fn the_prebuilt_installer_installs_the_missing_system_libraries_itself() {
    for (name, os, expect) in [
        (
            "lib-ubuntu",
            "ID=ubuntu\nID_LIKE=debian\n",
            "apt-get install -y libx11-6",
        ),
        (
            "lib-mint",
            "ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n",
            "apt-get install -y libx11-6",
        ),
        ("lib-fedora", "ID=fedora\n", "dnf install -y libX11"),
        (
            "lib-rocky",
            "ID=rocky\nID_LIKE=\"rhel centos fedora\"\n",
            "install -y libX11",
        ),
        (
            "lib-arch",
            "ID=arch\n",
            "pacman -S --needed --noconfirm libx11",
        ),
        (
            "lib-manjaro",
            "ID=manjaro\nID_LIKE=arch\n",
            "pacman -S --needed --noconfirm libx11",
        ),
        (
            "lib-suse",
            "ID=opensuse-tumbleweed\nID_LIKE=\"opensuse suse\"\n",
            "zypper --non-interactive install libX11-6",
        ),
        ("lib-void", "ID=void\n", "xbps-install -Sy libX11"),
    ] {
        let (ok, text) = Prebuilt {
            args: &["--dry-run"],
            os_release: os,
            missing: "libX11.so.6\nlibGL.so.1\n",
            ..Prebuilt::default()
        }
        .run(name);
        assert!(ok, "{name}: {text}");
        assert!(
            text.contains(expect) && text.contains("missing: libX11.so.6 libGL.so.1"),
            "{name} should print `{expect}`:\n{text}"
        );
        for forbidden in ["cargo", "rustup", "git clone"] {
            assert!(!text.contains(forbidden), "{name}: {forbidden} in:\n{text}");
        }
    }
    let (ok, text) = Prebuilt {
        args: &["--dry-run", "--no-deps"],
        missing: "libX11.so.6\n",
        ..Prebuilt::default()
    }
    .run("lib-skip");
    assert!(
        ok && text.contains("--no-deps") && !text.contains("apt-get"),
        "{text}"
    );
}

// REQ SQ-45
#[test]
fn the_prebuilt_installer_asks_for_nothing_when_the_program_finds_every_library() {
    // Regression (v0.1.1): the installer asked for the password on computers that had
    // everything. The program itself now says what it cannot load; nothing missing, no install.
    let (ok, text) = Prebuilt {
        args: &["--dry-run"],
        ..Prebuilt::default()
    }
    .run("lib-present");
    assert!(
        ok && text.contains("already installed")
            && !text.contains("apt-get")
            && !text.contains("password"),
        "{text}"
    );
}

/// A fake `sudo` that refuses, as when the user has no administrator rights or types no password.
const SUDO_REFUSES: (&str, &str) = ("sudo", "#!/bin/sh\necho 'sudo: refused' >&2\nexit 1\n");

// REQ SQ-47
#[test]
fn libraries_that_cannot_be_installed_never_stop_the_installation() {
    // A real run, not a dry run: the administrator command fails for every package.
    let (ok, text) = Prebuilt {
        missing: "libXi.so.6\n",
        fakes: &[SUDO_REFUSES],
        ..Prebuilt::default()
    }
    .run("lib-refused");
    assert!(ok, "{text}");
    for expected in [
        "trying one package at a time",
        "could not install libxi6",
        "still missing: libXi.so.6",
        "Installing for your user only",
        "Done.",
    ] {
        assert!(text.contains(expected), "`{expected}` not in:\n{text}");
    }
    // A distribution nobody has heard of, with no known package manager installed.
    let (ok, text) = Prebuilt {
        os_release: "ID=plan9\n",
        missing: "libXi.so.6\n",
        bare_path: true,
        ..Prebuilt::default()
    }
    .run("lib-unknown");
    assert!(
        ok && text.contains("not one this installer knows") && text.contains("Done."),
        "{text}"
    );
    // The same unknown distribution with zypper installed: the installed manager is used.
    let (ok, text) = Prebuilt {
        args: &["--dry-run"],
        os_release: "ID=plan9\n",
        missing: "libXi.so.6\n",
        fakes: &[("zypper", "#!/bin/sh\nexit 0\n")],
        bare_path: true,
        ..Prebuilt::default()
    }
    .run("lib-by-command");
    assert!(
        ok && text.contains("zypper --non-interactive install"),
        "{text}"
    );
}

// REQ SQ-47
#[test]
fn the_installer_confirms_the_panel_started_and_fails_loudly_when_it_did_not() {
    let display = [("DISPLAY", ":99")];
    let (ok, text) = Prebuilt {
        env: &display,
        ..Prebuilt::default()
    }
    .run("launch-ok");
    assert!(
        ok && text.contains("The panel is on your desktop now"),
        "{text}"
    );
    let failing = [("DISPLAY", ":99"), ("STUB_LAUNCH_FAILS", "1")];
    let (ok, text) = Prebuilt {
        env: &failing,
        ..Prebuilt::default()
    }
    .run("launch-fails");
    assert!(!ok, "a panel that did not start must not read as success");
    assert!(
        text.contains("the panel did not start")
            && text.contains("no OpenGL here")
            && !text.contains("The panel is on your desktop now"),
        "{text}"
    );
    // No desktop session in this terminal (SSH, a console): nothing to verify, and no failure.
    let (ok, text) = Prebuilt::default().run("launch-no-display");
    assert!(ok && text.contains("opens at your next login"), "{text}");
}

// REQ SQ-47
#[test]
fn the_one_paste_scripts_run_in_a_plain_posix_shell_and_never_close_the_users_window() {
    let tools = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools");
    for script in ["get.sh", "install-prebuilt.sh"] {
        let text = fs::read_to_string(tools.join(script)).unwrap();
        assert!(
            text.starts_with("#!/bin/sh\n"),
            "{script} must not need bash"
        );
        for bashism in [
            "[[",
            "<<<",
            "pipefail",
            "local ",
            "BASH_SOURCE",
            "function ",
        ] {
            assert!(!text.contains(bashism), "{script} uses `{bashism}`");
        }
    }
    // Windows: `exit` inside a pasted line closes the user's PowerShell window before the error
    // can be read, and a checksum read with `.Content` is raw bytes that never compare equal
    // (both shipped in v0.1.0 to v0.1.2 and made the Windows line fail on every computer).
    let ps = fs::read_to_string(tools.join("get.ps1")).unwrap();
    let code: String = ps
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!code.contains("exit "), "get.ps1 must not call exit");
    assert!(
        !code.contains(".Content"),
        "get.ps1 must read the checksum from the saved file"
    );
    assert!(code.contains("Tls12") && code.contains("'--yes'"), "{code}");
}
