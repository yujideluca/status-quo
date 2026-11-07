//! status-quo-setup.exe: the one-file Windows installer (SQ-41).
//!
//! It carries `status-quo.exe` inside itself (or finds one beside itself), checks that this is
//! Windows 10 or 11, installs the program for the current user (no administrator rights), starts
//! it and prints the compatibility report. Everything is explained in plain words, and the
//! window stays open at the end so a double click can be read.
use std::{
    env, fs,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

static PAYLOAD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload.bin"));

const HELP: &str = "Status Quo setup

  status-quo-setup.exe              install for this user, start it and check compatibility
  status-quo-setup.exe --check      only run the compatibility check
  status-quo-setup.exe --uninstall  remove Status Quo (layouts and history stay)
  status-quo-setup.exe --dry-run    show the steps without doing them
Other options: --yes (do not wait for a key at the end), --no-launch (do not start it now).
";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub check: bool,
    pub uninstall: bool,
    pub dry_run: bool,
    pub yes: bool,
    pub launch: bool,
    pub help: bool,
}
pub fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        launch: true,
        ..Options::default()
    };
    for arg in args {
        match arg.as_str() {
            "--check" => o.check = true,
            "--uninstall" => o.uninstall = true,
            "--dry-run" => o.dry_run = true,
            "--yes" | "-y" => o.yes = true,
            "--no-launch" => o.launch = false,
            "--help" | "-h" | "/?" => o.help = true,
            other => return Err(format!("unknown option: {other} (try --help)")),
        }
    }
    if o.check && o.uninstall {
        return Err("--check and --uninstall cannot be combined".into());
    }
    Ok(o)
}
/// The major version in the output of `cmd /c ver` ("Microsoft Windows [Version 10.0.19044.1]").
pub fn windows_major(text: &str) -> Option<u32> {
    text.split("Version")
        .nth(1)?
        .trim_start()
        .split(['.', ' ', ']'])
        .next()?
        .parse()
        .ok()
}
/// Where the installed program lives for this user (what `status-quo install` creates).
pub fn installed_path(local_app_data: &Path) -> PathBuf {
    local_app_data
        .join("status-quo")
        .join("bin")
        .join("status-quo.exe")
}
/// The steps, in order, as shown by `--dry-run` and printed while running.
pub fn plan(o: &Options) -> Vec<&'static str> {
    if o.uninstall {
        return vec!["Remove Status Quo (your layouts and history stay)"];
    }
    if o.check {
        return vec!["Run the compatibility check"];
    }
    let mut steps = vec![
        "Check that this is Windows 10 or 11 on a 64-bit PC",
        "Unpack the program",
        "Install it for this user (no administrator rights)",
    ];
    if o.launch {
        steps.push("Start it");
    }
    steps.push("Run the compatibility check");
    steps
}

fn say(text: &str) {
    println!("\n==> {text}");
}
fn windows_version() -> Option<u32> {
    let out = Command::new("cmd").args(["/C", "ver"]).output().ok()?;
    windows_major(&String::from_utf8_lossy(&out.stdout))
}
/// The program to run: the embedded copy, or a `status-quo.exe` next to this installer.
fn unpack() -> Result<PathBuf, String> {
    let dir = env::temp_dir().join(format!("status-quo-setup-{}", std::process::id())); // scratch-ok: production, the unpacked program must outlive the installer
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let target = dir.join("status-quo.exe");
    if !PAYLOAD.is_empty() {
        fs::write(&target, PAYLOAD).map_err(|e| format!("cannot unpack the program: {e}"))?;
        return Ok(target);
    }
    let beside = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("status-quo.exe")))
        .filter(|p| p.is_file());
    match beside {
        Some(path) => Ok(path),
        None => Err("this installer carries no program and no status-quo.exe is beside it. \
            Download the complete status-quo-setup.exe, or build from source (see the install guide)."
            .into()),
    }
}
fn run_program(program: &Path, args: &[&str]) -> Result<bool, String> {
    Command::new(program)
        .args(args)
        .status()
        .map(|s| s.success())
        .map_err(|e| format!("cannot run {}: {e}", program.display()))
}
fn ask_to_close(yes: bool) {
    if yes {
        return;
    }
    print!("\nPress Enter to close this window. ");
    let _ = io::stdout().flush();
    let mut line = String::new();
    let _ = io::stdin().lock().read_line(&mut line);
}

fn real_main(o: &Options) -> Result<(), String> {
    let steps = plan(o);
    println!("Status Quo setup: {} step(s)", steps.len());
    for (n, step) in steps.iter().enumerate() {
        println!("  {}. {step}", n + 1);
    }
    if o.dry_run {
        return Ok(());
    }
    if !cfg!(windows) {
        return Err(
            "this installer is for Windows 10 and 11. On Linux run: bash tools/install.sh".into(),
        );
    }
    let local = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or("LOCALAPPDATA is not set")?;
    let installed = installed_path(&local);
    if o.uninstall {
        say("Removing Status Quo");
        let program = if installed.is_file() {
            installed
        } else {
            unpack()?
        };
        return run_program(&program, &["uninstall"]).and_then(|ok| {
            ok.then_some(())
                .ok_or_else(|| "uninstall reported a problem".into())
        });
    }
    if o.check {
        say("Compatibility check");
        let program = if installed.is_file() {
            installed
        } else {
            unpack()?
        };
        run_program(&program, &["doctor"])?;
        return Ok(());
    }
    say("1/4 Checking this PC");
    match windows_version() {
        Some(v) if v >= 10 => println!("  [ok]  Windows {v}"),
        Some(v) => {
            return Err(format!(
                "Windows {v} is too old. Status Quo needs Windows 10 or 11."
            ))
        }
        None => println!("  [!]   could not read the Windows version; continuing"),
    }
    if cfg!(target_arch = "x86_64") {
        println!("  [ok]  64-bit PC");
    } else {
        println!("  [!]   this PC is not x86-64; the installer continues but this is untested");
    }
    say("2/4 Unpacking");
    let program = unpack()?;
    println!("  [ok]  program ready ({})", program.display());
    say("3/4 Installing for your user (no administrator rights)");
    if !run_program(&program, &["install"])? {
        return Err("the installation stopped with an error; the lines above say why".into());
    }
    if o.launch {
        say("Starting Status Quo");
        let target = if installed.is_file() {
            installed.clone()
        } else {
            program.clone()
        };
        if !run_program(&target, &["launch"]).unwrap_or(false) {
            println!("  [!]   could not start it now; it starts at your next sign-in");
        }
    }
    say("4/4 Compatibility check");
    let target = if installed.is_file() {
        installed
    } else {
        program
    };
    run_program(&target, &["doctor"])?;
    say("Done. To remove Status Quo later, run this file again with --uninstall.");
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let options = match parse_args(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    if options.help {
        print!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let result = real_main(&options);
    let code = match &result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("\n[x]   {e}");
            ExitCode::FAILURE
        }
    };
    if !options.dry_run {
        ask_to_close(options.yes);
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    // REQ SQ-41
    #[test]
    fn options_parse_and_bad_ones_are_refused() {
        let o = parse_args(&args(&[])).unwrap();
        assert!(o.launch && !o.check && !o.uninstall && !o.yes);
        let o = parse_args(&args(&["--yes", "--no-launch", "--dry-run"])).unwrap();
        assert!(o.yes && !o.launch && o.dry_run);
        assert!(parse_args(&args(&["--frobnicate"])).is_err());
        assert!(parse_args(&args(&["--check", "--uninstall"])).is_err());
        assert!(parse_args(&args(&["/?"])).unwrap().help);
    }

    // REQ SQ-41
    #[test]
    fn the_plan_lists_every_step_in_order() {
        let full = plan(&parse_args(&args(&[])).unwrap());
        assert_eq!(full.len(), 5);
        assert!(full[0].contains("Windows 10 or 11") && full[2].contains("Install it"));
        assert_eq!(full.last().unwrap(), &"Run the compatibility check");
        assert_eq!(plan(&parse_args(&args(&["--no-launch"])).unwrap()).len(), 4);
        assert_eq!(plan(&parse_args(&args(&["--check"])).unwrap()).len(), 1);
        assert!(plan(&parse_args(&args(&["--uninstall"])).unwrap())[0].contains("Remove"));
    }

    // REQ SQ-41
    #[test]
    fn windows_version_and_install_path_are_read_correctly() {
        assert_eq!(
            windows_major("\r\nMicrosoft Windows [Version 10.0.26100.1]\r\n"),
            Some(10)
        );
        assert_eq!(
            windows_major("Microsoft Windows [Version 6.1.7601]"),
            Some(6)
        );
        assert_eq!(windows_major("nothing"), None);
        let path = installed_path(Path::new("C:/Users/a/AppData/Local"));
        assert!(path.ends_with("status-quo/bin/status-quo.exe"));
    }
}
