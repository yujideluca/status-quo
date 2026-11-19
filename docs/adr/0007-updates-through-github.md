# ADR-0007: Status Quo keeps itself current through GitHub

- **Status:** accepted
- **Date:** 2026-11-19
- **Type (ISO/IEC/IEEE 14764):** adaptive
- **Decided by:** Giovanni (owner: "all versions of the tray update their dependencies and versions through GitHub when something new is there; you decide how") / designed by Claude Code

## Context

Until 0.1.5 every update was done by hand: Rust, the libraries, the pinned GitHub actions and the build images were raised in a session, and an installed tray stayed on its version until its owner pasted the install line again. Giovanni wants both to happen by themselves.

Two limits of GitHub shape the design. A push, tag or release made with a workflow's own key (`GITHUB_TOKEN`) starts no other workflow, except by name (`workflow_dispatch`). And that key may not change files in `.github/workflows`.

## Decision

1. **The repository updates itself (SQ-53).** `.github/workflows/update-dependencies.yml` runs every Monday and on demand. `tools/update_dependencies.sh` takes the newest stable Rust (`rust-toolchain.toml`, which CI, the packages and source installs use), the newest library versions allowed by Cargo.toml that still build with `rust-version` (`cargo update`, resolver `incompatible-rust-versions = "fallback"`), the newest `pipeline-check` release and the newest manylinux2014 build images (`tools/build-images`, by digest). The commit goes to a short-lived branch; the whole quality-gate workflow, with the real install tests, is started by name on exactly that commit; only when it passes is that commit pushed to `main` (branch protection still demands the three required checks) and, when the shipped program can change, the patch version is raised, the tag pushed and `release.yml` started by name. Failures and library versions that need a change of Cargo.toml are reported as GitHub issues.
2. **The pinned actions update through Dependabot.** They live in workflow files, which the workflow's key may not change. `.github/dependabot.yml` proposes them weekly; `dependabot-merge.yml` turns on GitHub's auto-merge, which merges only after the required checks pass.
3. **Installed trays update themselves (SQ-54).** The installed program (never a copy started from a source folder) asks `releases/latest/download/version.txt` three minutes after the panel starts and then every six hours. A newer version is downloaded for the system, refused unless its SHA-256 matches the published `.sha256`, and installed by the release's own installer (`status-quo-setup.exe --yes` on Windows, the package's `install.sh --no-deps` on Linux) in a process of its own, because installing restarts the panel. Customize → "Update automatically" (on by default) turns it off; `status-quo update` and `status-quo update --check` do it by hand; `update.log` in the state folder records every attempt.
4. **Only a release that passed its published install tests is offered.** `release.yml` uploads `version.txt` last, after `published-linux` and `published-windows`; then `published-update-linux` and `published-update-windows` install the previous release on a bare machine and wait until its running panel has updated itself to the new one (from v0.1.7 on: v0.1.6 is the first release with an updater).

## Alternatives considered

- **Dependabot or Renovate for everything** — neither raises the Rust release, the build images by digest or the program version, and neither publishes a release; their pull requests also leave a branch per update.
- **A personal access token or GitHub App key for the workflow** — would let one workflow change workflow files too, but is a long-lived secret to create, store and renew.
- **Updating from the collector service** — the collector is stopped by the installation it would start.
- **Asking before each update** — the owner asked for updates "when something new is there"; the setting turns it off.

## Consequences

- An automatic release skips the manual pass on a real screen (`tools/ui_e2e.sh`, MAINTENANCE section 7); CI's install tests, screenshots and the update tests are its proof.
- The code is still checked against `rust-version` (1.88) only by clippy's `incompatible_msrv`; CI builds with the newest Rust.
- A new release that needs a system library the old one did not need installs without it (`--no-deps`); `status-quo doctor` names it.
- On Windows the downloaded installer carries no Mark of the Web, so SmartScreen does not ask; the installer is not signed (Giovanni decided against buying a certificate).
