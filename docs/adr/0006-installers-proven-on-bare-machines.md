# ADR-0006: Prove the installers by real runs on bare machines, and ship programs that need nothing preinstalled

- **Status:** accepted
- **Date:** 2026-11-18
- **Type (ISO/IEC/IEEE 14764):** corrective
- **Decided by:** Giovanni (owner) / proposed by Claude Code

## Context

Releases v0.1.0 to v0.1.2 could not be installed by the people who tried, although every one of them had passed its installer tests and had been reported as verified. Run for the first time on a clean Windows 10 (a virtual machine with nothing added) and inspected file by file, the published release showed:

1. The Windows one-paste line failed on every machine. `get.ps1` compared the checksum of the download with text it read straight from the web; in Windows PowerShell that text arrives as raw bytes, so the comparison never matched.
2. `status-quo-setup.exe` did not start on a clean Windows. It needed `VCRUNTIME140.dll`, part of the Visual C++ runtime, which Windows does not include.
3. A failure closed the user's PowerShell window (`exit` in a pasted script), so nobody could read why.
4. The Linux program needed glibc 2.35 (it was built on Ubuntu 22.04) and refused to start on older distributions.
5. The Linux line needed `curl` and `bash`; a fresh Ubuntu or Debian desktop has `wget`, not `curl`.
6. The Linux installer needed systemd, stopped on distributions it did not know, and printed "Done" without checking that the panel had started.

The cause behind all six: every installer test was a simulation. The tests ran the scripts with `--dry-run` and with environment variables that faked which libraries were missing; CI did the same. The command a person pastes was never run on a machine like theirs, so each "fix" passed while the real installation kept failing. Giovanni's judgement of this: a simulated check reported as proof is a false report, and it is forbidden.

## Decision

1. **Only a real run is proof.** A claim that installation works rests on running the install line of the README, as written, on a bare system and seeing the real panel run and draw. `.github/workflows/packages.yml` does this on every push: bare base images of the supported Linux distributions (in containers, as root), a full Ubuntu with an ordinary user (sudo, systemd), ARM64, and Windows in Windows PowerShell 5.1 and PowerShell 7. `release.yml` publishes only after those jobs pass, then repeats the line against the published release. Unit tests with stand-in programs remain for the scripts' branching only and are never cited as proof of installation.
2. **The downloads need nothing preinstalled.** Windows programs link the C runtime statically (`.cargo/config.toml`, `crt-static`). The Linux program is built inside the manylinux2014 image (glibc 2.17) for x86-64 and ARM64. `tools/check_binary.rs` reads each built file and fails the build when it imports a library a clean system lacks or needs a newer glibc.
3. **The installer finds out, installs and confirms.** The program reports which drawing libraries it cannot load (`status-quo doctor --libs`); the installer installs their packages with the system's package manager and never stops on a library it could not install. It starts the panel with `status-quo launch --verify` and fails loudly, with the panel's last lines, when the panel did not stay up. Without systemd the panel runs as an ordinary background program.
4. **The scripts run in what every system has.** `get.sh` and the packaged `install.sh` are POSIX `sh` and use `curl` or `wget`; `get.ps1` runs in Windows PowerShell 5.1, sets TLS 1.2, retries downloads and never calls `exit`.

## Alternatives considered

- **Bundle the drawing libraries (AppImage, Flatpak)** — OpenGL drivers must match the machine's graphics hardware and cannot be bundled reliably; Flatpak itself is not installed everywhere. Installing the distribution's own packages is what works on every family tested.
- **Ship the Visual C++ redistributable with the installer** — larger, needs administrator rights; static linking removes the need.
- **Keep dry-run tests and add a manual checklist** — a manual step is skipped under pressure; this is how the failure happened.
- **Build on the oldest Ubuntu runner** — still glibc 2.35 today, and the baseline moves whenever GitHub retires a runner.

## Consequences

- A push that breaks installation on any tested system fails CI (`packages / install-proof` and its jobs are required checks on main); a release cannot be published from it.
- CI takes longer (about fifteen extra jobs, a few minutes each).
- Not covered, and said so in the README: musl systems such as Alpine and processors other than x86-64 and ARM64 fall back to building from source; a setup file downloaded in a browser shows the SmartScreen notice until the project buys a code-signing certificate.
- Kept true by: `packages.yml`, `release.yml`, `tools/check_binary.rs`, the test `installation_is_proven_by_real_runs_never_by_dry_runs` (it fails when a workflow uses a dry run or when publishing stops depending on the real installs), and requirement SQ-47.
