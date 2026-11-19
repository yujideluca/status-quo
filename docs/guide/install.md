# Install Status Quo

Back to the [tour](../../README.md). Two automatic installers do everything on this page for you; the manual steps follow so you can see (or change) each one. Either way the program installs itself **for your user only**.

## The automatic installers

### One paste: `tools/get.sh` and `tools/get.ps1`

One line in a terminal, nothing to clone, no Rust. On Linux (`tools/get.sh`) it downloads the package for the machine's processor (`status-quo-linux-x86_64.tar.gz` or `status-quo-linux-aarch64.tar.gz`) from the latest release, checks its SHA-256 checksum and runs the `install.sh` inside it; when there is no prebuilt package for the machine (a processor other than x86-64 and ARM64, or a system built on musl such as Alpine) it runs the source installer below instead. It needs only `curl` or `wget` and any POSIX shell. On Windows (`tools/get.ps1`) it downloads `status-quo-setup.exe`, checks its checksum and runs it. Options pass through to the installer that runs: `... | sh -s -- --dry-run`.

```sh
{ curl -fsSL https://raw.githubusercontent.com/yujideluca/status-quo/main/tools/get.sh 2>/dev/null || wget -qO- https://raw.githubusercontent.com/yujideluca/status-quo/main/tools/get.sh; } | sh
```

```powershell
irm https://raw.githubusercontent.com/yujideluca/status-quo/main/tools/get.ps1 | iex
```

### Plug and play: the release downloads

The [Releases page](https://github.com/yujideluca/status-quo/releases/latest) carries ready-made files, built by CI from the tagged commit. You need no Rust, compiler or source.

| File | System | What it does |
|---|---|---|
| `status-quo-linux-x86_64.tar.gz`, `status-quo-linux-aarch64.tar.gz` | Linux on Intel/AMD or ARM 64-bit | unpack it and run `install.sh` inside: installs the system libraries that are missing, installs the program for your user only, starts it, confirms the panel stayed up and runs `status-quo doctor` |
| `status-quo-setup.exe` | Windows 10 or 11 | described under Windows below |

```sh
tar -xzf status-quo-linux-x86_64.tar.gz
sh status-quo-linux-x86_64/install.sh
```

| Option | Meaning |
|---|---|
| `--dry-run` | print every step instead of doing it |
| `--no-launch` | install but do not start it now |
| `--no-deps` | never ask for a password: you install the system libraries yourself |
| `--adopt` | take over a program file that was copied there by hand (a backup is kept) |
| `--uninstall` | remove Status Quo (settings and history stay) |

The program loads X11 or Wayland and OpenGL from the system when it starts. The installer asks the program which of them are missing (`status-quo doctor --libs`) and installs the packages that hold them with `apt-get`, `dnf` (or `yum`), `pacman`, `zypper` or `xbps-install`, as the administrator through `sudo`, `doas` or `su` (the password is asked once, only then). A library that cannot be installed never stops the installation; `status-quo doctor` then says in plain words what is left. After installing, the installer starts the panel with `status-quo launch --verify` and reports an error if it did not stay up. With systemd the panel runs as a login service of your user; without systemd it runs as an ordinary background program. The packages are built on an old base system (glibc 2.17) and checked by `tools/check_binary.rs`, so they start on old and new distributions alike. Other processors and musl systems: use the source installer below.

**How this is proven.** `.github/workflows/packages.yml` runs the install line of the README, as written, on bare base images of Ubuntu 18.04 to 24.04, Debian 12 and 13, Fedora, Rocky 8 and 9, AlmaLinux 10, Arch, openSUSE Leap and Tumbleweed and Void, on ARM64, on a full Ubuntu as an ordinary user with `sudo` and systemd, and on Windows in Windows PowerShell 5.1 and PowerShell 7. Each run must end with the real panel running and drawing. A release is published only after all of them pass, and `release.yml` then repeats the line against the published release. Dry runs and stand-in programs are not accepted as proof that the installation works.

### Linux: `tools/install.sh`

```sh
git clone https://github.com/yujideluca/status-quo.git
cd status-quo
bash tools/install.sh
```

It runs six steps and says what it is doing in each:

| Step | What happens | Needs |
|---|---|---|
| 1. Check the computer | reads your distribution, architecture, session type, desktop, and whether `gh`, `claude` and `codex` are installed | nothing |
| 2. Build libraries | installs a compiler, git, curl and pkg-config (and the drawing libraries' development files) with your package manager: `apt-get` (Debian, Ubuntu, Mint), `dnf` (Fedora, RHEL family), `pacman` (Arch, Manjaro) or `zypper` (openSUSE) | `sudo`, asked once |
| 3. Rust | installs Rust 1.87 or newer through the official `rustup` into your home folder if it is missing or older | nothing |
| 4. Source | uses the checkout it is run from, or clones the repository into `~/.local/share/status-quo/src` | network |
| 5. Build and install | `cargo build --release --locked -p status-quo`, then `status-quo install`, then starts it if you are in a graphical session | nothing |
| 6. Compatibility check | runs `status-quo doctor` (below) | nothing |

| Option | Meaning |
|---|---|
| `--check-only` | look and report; change nothing |
| `--dry-run` | print every command instead of running it |
| `--yes` | do not ask questions (needed when there is no terminal) |
| `--no-deps` | never use `sudo`; you install the libraries yourself |
| `--uninstall` | remove Status Quo (settings and history stay) |
| `--adopt` | take over a program file that was copied there by hand: the installer normally refuses to overwrite a file it did not install; with this option it first keeps a copy named `status-quo.before-status-quo` next to it (assistant settings are never adopted) |
| `--repo URL`, `--src DIR` | use another repository address or source folder |

An unknown distribution stops at step 2 with the list of what to install by hand. The Debian and Ubuntu package names come from the project's CI; the dnf, pacman and zypper names are the usual ones and were checked only as printed commands (`--dry-run`), not installed on real machines. Only the Linux used for development (Kubuntu on KDE Plasma) has run the whole script for real.

### Windows: `status-quo-setup.exe`

One file, no administrator rights. Double-click it. It checks that this is Windows 10 or 11 on a 64-bit PC, unpacks the program it carries, installs it for your user, starts it, runs the compatibility check and waits for a key press so you can read the result.

```text
status-quo-setup.exe              install, start and check
status-quo-setup.exe --check      only run the compatibility check
status-quo-setup.exe --uninstall  remove Status Quo (layouts and history stay)
status-quo-setup.exe --dry-run    list the steps without doing them
status-quo-setup.exe --yes        do not wait for a key at the end
status-quo-setup.exe --no-launch  install but do not start it now
status-quo-setup.exe --adopt      install over a program file that was put there by hand
```

If a file that Status Quo did not install lies where the program goes, the installer stops, changes nothing and says so. `--adopt` installs anyway and first keeps that file as a copy whose name ends in `.before-status-quo`. With the one-line command, run `$env:SQ_SETUP_ARGS = '--adopt'` first and paste the line again.

Where to get it: attached to each tagged release (**Releases** page) and, for every CI run, as the artifact `status-quo-setup-windows` on the run's page. The installer is built by CI with the program embedded; if you build it yourself without the program inside, it looks for a `status-quo.exe` next to itself.

### The compatibility check: `status-quo doctor`

Both installers end with it, and you can run it whenever something looks wrong. Each line is `[ok]`, `[!]` (works, but something optional is missing) or `[x]` (blocks Status Quo), with the fix written under every problem:

| Check | Blocks when |
|---|---|
| System | Windows older than 10 |
| Display | a Linux terminal with no graphical session (neither `WAYLAND_DISPLAY` nor `DISPLAY`) |
| Settings folder | the folder cannot be written |
| Claude, Codex, workflow memory | never (the matching widgets show *unavailable*) |
| GitHub | never (needs `gh` installed and signed in) |
| Machine sensors, wallpaper | never |
| Installation, collector | never (it tells you to run `status-quo install` or `status-quo`) |

The exit code is 0 unless something blocks, so scripts can use it.

## What you need

| Need | Why |
|---|---|
| Rust 1.87 or newer ([rustup.rs](https://rustup.rs)) | the whole program is Rust |
| Drawing libraries (Linux only, below) | X11, Wayland, keyboard maps and OpenGL |
| `claude` and/or `codex` signed in | the usage, token and session widgets read what these tools saved |
| `gh` signed in (`gh auth login`) | the GitHub activity widget reads through it |

Status Quo never creates or refreshes a login. Without a sign-in, the matching widget shows *unavailable*; everything else keeps working.

## Linux, step by step

**1. Libraries.** The project's own continuous-integration job installs these on Ubuntu, so the Debian/Ubuntu line is the tested one:

```sh
# Debian, Ubuntu and relatives (tested in CI)
sudo apt-get install -y pkg-config libx11-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev
```

Other families need the same four libraries under their own names. These are the usual ones and have **not** been tested here:

```sh
# Fedora and relatives
sudo dnf install pkgconf-pkg-config libX11-devel libxkbcommon-devel wayland-devel mesa-libGL-devel
# Arch and relatives
sudo pacman -S pkgconf libx11 libxkbcommon wayland mesa
```

**2. Build and install.**

```sh
git clone https://github.com/yujideluca/status-quo.git
cd status-quo
cargo build --release --locked -p status-quo
target/release/status-quo install
```

**3. Start it.** Log out and in (it starts at login unless you turn that off in Customize), or run `status-quo` now. The first start also starts the collector.

### How it sits on your desktop

Status Quo asks the session what it can do and picks the best host. By default the tray sits on the desktop background on every system; an ordinary window with a title bar is a setting (**Customize → Desktop behavior → Floating window**, or `status-quo floating on` in a terminal; `status-quo floating off` puts it back). It tells you which one and why (Customize shows the line).

| Your session | Host chosen | Result |
|---|---|---|
| The compositor offers the *layer-shell* protocol (KDE Plasma, Hyprland, sway) | layer-shell | frameless, on the desktop layer, below windows by default, anchored to a corner |
| An X11 session, or Wayland without layer-shell such as GNOME (through XWayland) | X11 hints | a frameless window kept below others, off the taskbar |
| Windows 10 and 11 | Windows desktop | a frameless panel that belongs to the desktop: below other windows, not in the taskbar or Alt+Tab, still there after Show desktop (Win+D) |
| The Floating window setting is on, or nothing above is possible | window | an ordinary window |

Verified here: KDE Plasma on Wayland. The X11 and window hosts and the other layer-shell compositors follow the design and the protocol but were not run on real machines for this guide. Two variables override the choice when it can work:

```sh
STATUS_QUO_HOST=window status-quo        # layer-shell, x11, windows-desktop or window
STATUS_QUO_ANCHOR=bottom-left status-quo # top, bottom, left, right, or a pair such as top-right (default)
```

## Windows, step by step

Windows 10 and 11 are supported; the test suite passes on both (Windows 10 LTSC 2021 and Windows 11 24H2 test machines), and the window was seen opening on Windows 11 with a software OpenGL driver.

1. Install Rust with the **Microsoft C++ build tools** (the default rustup choice).
2. In PowerShell:

```powershell
git clone https://github.com/yujideluca/status-quo.git
cd status-quo
cargo build --release --locked -p status-quo
target\release\status-quo.exe install
```

What is different on Windows: the tray is a frameless window that belongs to the desktop: kept below your other windows, with no taskbar button and not in the Alt+Tab list (it stays behind also when you click it, ends above the taskbar and stays in place when you press the *Show desktop* shortcut Win+D; drag the empty part of its top row to move it; the *Window layer* setting can put it above others; it keeps running when you close the terminal that started it); the collector and panel talk through a private port on your own machine protected by a random token; CPU and memory come from Windows, every graphics card of any maker from the Windows performance counters (the ones Task Manager uses); the CPU temperature is **unavailable** because Windows offers none without a driver. No administrator rights are used.

## What `install` does

Everything it touches is listed in a manifest and removed again by `status-quo uninstall`. Saved layouts and history stay.

| Linux | Windows |
|---|---|
| `~/.local/bin/status-quo` | `%LOCALAPPDATA%\status-quo\bin\status-quo.exe` |
| `~/.config/systemd/user/status-quo-collector.service` and `…-panel.service` | a hidden launcher `status-quo.vbs` in your Startup folder |
| `~/.local/share/applications/status-quo.desktop`, `~/.config/autostart/status-quo.desktop` | |
| small hooks in `~/.claude/settings.json` and `~/.codex/hooks.json` | the same two files in your user folder |

The hooks only report that a session started, waited or stopped, so the sessions widget can show live states. Your existing hooks are kept, and a file that was changed by someone else is never overwritten (the install stops and says so). State lives in `~/.local/state/status-quo` (Linux) or `%LOCALAPPDATA%\status-quo` (Windows); set `STATUS_QUO_STATE_DIR` to use another folder, for example to try things without touching your real setup.

## Updating

Pull, rebuild and run `install` again: it replaces only files it owns. Then restart: `systemctl --user restart status-quo-collector status-quo-panel` on Linux, or sign out and in on Windows.

## Try it without installing

```sh
cargo run --locked -p status-quo                       # the real panel, nothing installed
cargo run --locked -p status-quo -- inspect --widgets  # the widgets as JSON, no window
```

A browser version with invented data is described in the [browser demo](../browser-demo.md).
