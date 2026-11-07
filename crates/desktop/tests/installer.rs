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
