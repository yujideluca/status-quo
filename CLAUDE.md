# Status Quo: working notes

Status Quo is Giovanni's all-Rust Linux desktop widget tray. KDE is the first wrapper, not a dependency of portable widgets. The approved scope is `docs/2026-11-01-status-quo-design.md`; `docs/REQUIREMENTS.md` records its acceptance requirements. All functionality is an independently removable/interchangeable registered widget, never a permanent provider slot.

## Enforced principles

1. Requirements precede meaningful failing tests and implementation. Tests carry standalone `// REQ SQ-*` comments; the pinned traceability check scans Rust files under `crates` and `tools`.
   An ID is two capital letters, a dash and the next free number (`XX-12`), never a sub-scheme such as `XX-V2-1`; tasks run in parallel each get a block of free numbers reserved by whoever starts them. A tag is `REQ <ID>` as the first word of a comment, docstring or test name. pipeline-check v0.7.0 fails on any other shape (PC-17, PC-18).
2. `sq-core`, `sq-contract` and `sq-widgets` have no GUI, filesystem, process or network dependencies. `tools/rust_architecture.rs` checks their manifests and source IO boundaries, and keeps IO out of the reusable tray, with planted-defect detector tests. `sq-tray` depends only on portable models and egui; platform work belongs to wrappers.
3. `tools/quality_gate.sh` defines passing. CI calls the pinned shared runner, reads `pipeline-check.toml` and fails on `falha`. Do not weaken limits, disable tests or relabel unverified behavior.
4. Shared tools stay in their own repository and are consumed at a pinned version. Do not copy the shared pipeline implementation.
5. Structural decisions get ADRs and a dated `DESIGN.md` row. Never write credentials, conversations or private memory feeds into fixtures, diagnostics, browser demos or wiki examples.

## Working rules

Continue approved scopes without milestone permission pauses. Ask Giovanni only for an unresolved crucial scope decision, credentials/login, or root access. Divide work by file ownership; never reset another session's changes. Configuration is data-only and transactional. Preserve existing assistant hooks when adding lifecycle observations. A Stop event means waiting, not proven task completion.

Run `tools/quality_gate.sh` and the full `tools/pipeline-check.sh --mode local`; after a major tray change also run `tools/ui_e2e.sh` and update `crates/desktop/tests/e2e/manual-baseline.json` and the test (docs/MAINTENANCE.md section 7); inspect the report, then verify the real changed flow. Desktop claims require actual rendering/actions and compositor observations. Performance targets require the production binary over the stated interval. Read `docs/MAINTENANCE.md` and apply the maintenance/test-review skills at task closure.

Record changes and reasons in commits. CI must pass the exact commit before main promotion; never bypass required protection or force-push. Finish real changes with source-matched wiki English/German/Portuguese pages and publication in an isolated wiki worktree, then `mem use`. Report evidence and remaining limitations honestly.
