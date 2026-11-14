# Install Status Quo

Back to the [tour](../../README.md). Two automatic installers do everything on this page for you; the manual steps follow so you can see (or change) each one. Either way the program installs itself **for your user only**.

## The automatic installers

### Plug and play: the release downloads

The [Releases page](https://github.com/yujideluca/status-quo/releases/latest) carries ready-made files, built by CI from the tagged commit. You need no Rust, compiler or source.

| File | System | What it does |
|---|---|---|
| `status-quo-linux-x86_64.tar.gz` | Linux on x86_64 | unpack it and run `install.sh` inside: installs for your user only (no `sudo`), starts it and runs `status-quo doctor` |
| `status-quo-setup.exe` | Windows 10 or 11 | described under Windows below |

```sh
tar -xzf status-quo-linux-x86_64.tar.gz
bash status-quo-linux-x86_64/install.sh
```

| Option | Meaning |
|---|---|
| `--dry-run` | print every step instead of doing it |
| `--no-launch` | install but do not start it now |
| `--adopt` | take over a program file that was copied there by hand (a backup is kept) |
| `--uninstall` | remove Status Quo (settings and history stay) |

The program loads X11 or Wayland and OpenGL from the system when it starts; `status-quo doctor` names any that is missing. The package was built on Ubuntu 22.04, so it runs on systems with glibc 2.35 or newer. Other architectures: use the source installer below.

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
```

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

Status Quo asks the session what it can do and picks the best of three hosts. It tells you which one and why (Customize shows the line).

| Your session | Host chosen | Result |
|---|---|---|
| The compositor offers the *layer-shell* protocol (KDE Plasma, Hyprland, sway) | layer-shell | frameless, on the desktop layer, below windows by default, anchored to a corner |
| An X11 session | X11 hints | a frameless window kept below others, off the taskbar |
| Anything else, for example GNOME on Wayland | window | an ordinary window |

Verified here: KDE Plasma on Wayland. The X11 and window hosts and the other layer-shell compositors follow the design and the protocol but were not run on real machines for this guide. Two variables override the choice when it can work:

```sh
STATUS_QUO_HOST=window status-quo        # layer-shell, x11 or window
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

What is different on Windows: the tray is an ordinary window (use the *Window layer* setting to keep it behind or above others); the collector and panel talk through a private port on your own machine protected by a random token; CPU and memory come from Windows, every graphics card of any maker from the Windows performance counters (the ones Task Manager uses); the CPU temperature is **unavailable** because Windows offers none without a driver. No administrator rights are used.

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
