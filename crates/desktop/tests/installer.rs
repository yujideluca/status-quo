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
/// The prebuilt package layout: the script next to a `status-quo` file. The file is a stub, so
/// a dry run proves what would be executed without installing anything.
fn run_prebuilt(name: &str, with_binary: bool, args: &[&str]) -> (bool, String) {
    run_prebuilt_on(name, with_binary, args, "ID=ubuntu\nID_LIKE=debian\n", "0")
}
fn run_prebuilt_on(
    name: &str,
    with_binary: bool,
    args: &[&str],
    os_release: &str,
    libs_missing: &str,
) -> (bool, String) {
    run_prebuilt_with(name, with_binary, args, os_release, libs_missing, None)
}
fn run_prebuilt_with(
    name: &str,
    with_binary: bool,
    args: &[&str],
    os_release: &str,
    libs_missing: &str,
    ldconfig_list: Option<&str>,
) -> (bool, String) {
    let dir = scratch(name);
    fs::copy(prebuilt_script(), dir.join("install.sh")).unwrap();
    if with_binary {
        fs::write(dir.join("status-quo"), "#!/bin/sh\nexit 0\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir.join("status-quo"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    let osr = dir.join("os-release");
    fs::write(&osr, os_release).unwrap();
    let mut cmd = Command::new("bash");
    if let Some(list) = ldconfig_list {
        let file = dir.join("ldconfig.txt");
        fs::write(&file, list).unwrap();
        cmd.env("SQ_LDCONFIG_LIST", file);
    }
    let out = cmd
        .arg(dir.join("install.sh"))
        .args(args)
        .env("HOME", dir.join("home"))
        .env("SQ_OS_RELEASE", &osr)
        .env_remove("SQ_ASSUME_MISSING_LIBS")
        .envs((!libs_missing.is_empty()).then_some(("SQ_ASSUME_MISSING_LIBS", libs_missing)))
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
    fs::write(pkg.join("status-quo"), "#!/bin/sh\nexit 0\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(pkg.join("status-quo"), fs::Permissions::from_mode(0o755)).unwrap();
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
    let out = Command::new("bash")
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
    assert!(text.contains("no prebuilt package"), "{text}");
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
        ("lib-fedora", "ID=fedora\n", "dnf install -y libX11"),
        (
            "lib-arch",
            "ID=arch\n",
            "pacman -S --needed --noconfirm libx11",
        ),
        (
            "lib-suse",
            "ID=opensuse-tumbleweed\nID_LIKE=\"opensuse suse\"\n",
            "zypper --non-interactive install libX11-6",
        ),
    ] {
        let (ok, text) = run_prebuilt_on(name, true, &["--dry-run"], os, "1");
        assert!(ok, "{name}: {text}");
        assert!(
            text.contains(expect),
            "{name} should print `{expect}`:\n{text}"
        );
        for forbidden in ["cargo", "rustup", "git clone"] {
            assert!(!text.contains(forbidden), "{name}: {forbidden} in:\n{text}");
        }
    }
    let (ok, text) = run_prebuilt_on(
        "lib-skip",
        true,
        &["--dry-run", "--no-deps"],
        "ID=ubuntu\n",
        "1",
    );
    assert!(
        ok && text.contains("skipped (--no-deps)") && !text.contains("apt-get"),
        "{text}"
    );
    let (ok, text) = run_prebuilt_on("lib-unknown", true, &["--dry-run"], "ID=plan9\n", "1");
    assert!(!ok && text.contains("unknown distribution"), "{text}");
}

// REQ SQ-45
#[test]
fn the_prebuilt_installer_reads_the_library_list_correctly_even_when_it_is_long() {
    // Regression: `printf | grep -q` under pipefail called every library missing, so the real
    // installer asked for sudo on computers that had everything.
    let mut all = String::new();
    for i in 0..20000 {
        all.push_str(&format!(
            "\tlibfiller{i}.so.1 (libc6,x86-64) => /usr/lib/x/libfiller{i}.so.1\n"
        ));
    }
    for lib in [
        "libX11.so.6",
        "libxcb.so.1",
        "libxkbcommon.so.0",
        "libxkbcommon-x11.so.0",
        "libwayland-client.so.0",
        "libGL.so.1",
        "libEGL.so.1",
        "libXcursor.so.1",
        "libXi.so.6",
        "libXrandr.so.2",
    ] {
        all.push_str(&format!("\t{lib} (libc6,x86-64) => /usr/lib/x/{lib}\n"));
    }
    let (ok, text) = run_prebuilt_with(
        "lib-present",
        true,
        &["--dry-run"],
        "ID=ubuntu\n",
        "",
        Some(&all),
    );
    assert!(
        ok && text.contains("already installed") && !text.contains("apt-get"),
        "{text}"
    );
    let without = all.replace("libEGL.so.1", "libgone.so.1");
    let (ok, text) = run_prebuilt_with(
        "lib-one-missing",
        true,
        &["--dry-run"],
        "ID=ubuntu\n",
        "",
        Some(&without),
    );
    assert!(ok && text.contains("apt-get install -y"), "{text}");
}
