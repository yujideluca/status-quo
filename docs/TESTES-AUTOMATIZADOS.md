# Automated bug-finding tests: Status Quo

Last reviewed: initial implementation (no commit yet), 2026-11-02

## Strategy and matrix

The inherited template record described another repository and is replaced here. Tests use Rust's built-in runner with synthetic fixtures. Real account reads, compositor checks and installed-package acceptance are separate from deterministic unit tests.

| Risk / requirements | Test location | Independent expected behavior |
|---|---|---|
| Configuration and counters, SQ-2..5, SQ-21 | `crates/core/tests/behavior.rs` | Invalid changes preserve the original; exact duplicate counters do not increase totals; unavailable caches remain unavailable; overflow/regression is rejected |
| Extensible contract, SQ-1, SQ-13..14 | `crates/contract/tests/registry.rs` | An unrelated registered widget works; incompatible and oversized transport frames fail |
| View models, SQ-5, SQ-8, SQ-18 | `crates/widgets/tests/models.rs` | Missing quota is not zero; stale known values survive; children group by project and explicit state |
| Persistence, SQ-12, SQ-17 | `crates/store/tests/durable.rs` | Concurrent readers observe complete JSON; stale writers cannot overwrite valid settings; malformed originals survive |
| Source records, SQ-4, SQ-7..11, SQ-24 | `crates/sources/tests/adapters.rs` | Partial writes retry, rotated files deduplicate, cumulative totals use correct cache semantics, event schemas reject malformed input without transmitting prompts |
| Transport, SQ-13 | `crates/collector/tests/transport.rs` | Independent clients see one configuration, conflicting revisions fail, oversized frames are closed |
| Tray, SQ-1..3, SQ-16 | `crates/tray/tests/layout.rs` | Narrow packing preserves identity; hidden/detached instances do not consume tray slots; cancel/invalid import preserves configuration |
| Architecture, SQ-14, SQ-24 | `tools/rust_architecture.rs` | Planted GUI dependency, grouped IO import and unsafe/FFI defects fail the detector |
| Browser reuse, SQ-20 | `crates/web` | Same portable registry/tray compiles for WASM with deterministic simulated data |

## Commands and current evidence

`tools/quality_gate.sh` is the production validation command; `tools/pipeline-check.sh --mode local` adds the report and GitHub observations. `cargo test --workspace --all-targets --locked` runs the Rust suite. `build/rust-architecture --self-test` plants isolated detector inputs rather than mutating a concurrent agent's source files.

The implementation and test suite are still changing. This record does not claim a final passing run, coverage percentage, mutation score or complete acceptance. Record final exact-commit results at release verification. No thresholds have been lowered. Coverage tools available on the laptop (`cargo-llvm-cov`, `cargo-mutants`) must be invoked with the final toolchain/source revision before making coverage or mutation claims.

## Gaps requiring actual evidence

Account timeout/backoff behavior, lifecycle hook preservation, concurrent assistant/subagent state changes, true window stacking, monitor recovery, login startup, installation/uninstall, browser rendering/actions and resource/latency limits require dedicated integration or real-host checks. Traceability scans requirement comments; it cannot prove those outcomes. No private conversation, memory feed or credentials may be committed as fixtures.

### Boundary checker evidence, 2026-11-02

`rustc --edition 2021 -D warnings tools/rust_architecture.rs` compiled the standalone detector; its `--self-test` and actual repository scan exited 0. A separate temporary detector copy was changed to accept grouped `process::Command` imports: `--self-test` then exited 101 at the expected assertion. The original source was not changed for this controlled mutation. The detector also tests dependency aliases, target-specific dependencies and aliased standard-library IO. This is evidence about the architecture detector, not a mutation score for application behavior.

### Actual resource acceptance harness

`rustc --edition 2021 -D warnings tools/resource_check.rs -o build/resource-check` builds a Linux `/proc` observer. Run `build/resource-check 300 COLLECTOR_PID PANEL_PID` with the two actual production PIDs. It measures summed CPU time as a percentage of one core and the highest sampled combined RSS, detects PID reuse/counter regressions and fails if either process exits. Missing/malformed observations fail explicitly. An interval shorter than 300 seconds always fails acceptance, even when sampled values are low. Its parser self-test verifies spaced process names, CPU tick fields, KiB conversion and malformed data. This checks the collector and panel themselves; it does not measure compositor/GPU memory or unrelated CLI subprocesses. No five-minute observation has yet been recorded in this file.

### First five-minute resource observations, 2026-11-02 (SQ-23 not met)

Release build, panel plus collector started by hand (not installed), `build/resource-check 300` with the two exact PIDs, on the Acer Nitro during a normal working session (assistant sessions were writing transcripts).

| Run | State | CPU, % of one core | Peak combined RSS | Result |
|---|---|---|---|---|
| 1 | Isolated empty state directory | 7.73 | 114 MiB | Failed |
| 2 | Real state directory, after the fix below | 2.05 | 115 MiB | Failed (target 1%) |

Memory meets the 200 MiB target. CPU does not. Run 1 exposed a real defect: the collector watched its own `events` directory, which did not exist until a lifecycle hook fired, and a failed watch forces the 2-second polling fallback. The collector now creates that directory (mode 0700) before watching it; test `owned_events_directory_is_private_and_avoids_polling_fallback`. Run 2 still sits above 1%: the panel uses about 0.4% and the collector about 1.2% with a steady rate plus a burst every ~25 seconds. The cause is not yet identified (ptrace and perf are blocked on this laptop; transcript activity from live assistant sessions is a suspect). Interaction latency (100 ms), window stacking, installation, uninstall and login startup are not yet observed. The installer was not run in this session because it edits the assistant hook files and the session's permission check refused that edit; it needs Giovanni's own run.

### Host compatibility layer observations, 2026-11-02 (SQ-26, SQ-27)

Plasma 6 on Wayland, Acer Nitro, release build.

| Check | Observation |
|---|---|
| Backend choice | Auto selected layer-shell (compositor offers `zwlr_layer_shell_v1`); the note is printed at start and shown in the capabilities line |
| Stacking | With Chrome maximized the surface was hidden behind it; on an empty virtual desktop it was visible, frameless, anchored top-right, no taskbar entry (screenshots by spectacle) |
| Input | A virtual pointer (`/dev/uinput`) clicked "7d" on the empty desktop: the button highlighted and the chart redrew; "24h" restored the original setting |
| X11 backend (forced) | After the window mapped, `_NET_WM_STATE` showed BELOW, SKIP_TASKBAR, SKIP_PAGER read back with `xprop`. STICKY was not accepted by KWin on XWayland (`_NET_WM_DESKTOP` stayed 0), so it is best effort. A real X11 session was not available and is untested |
| Plain window (forced) | Opens as the earlier framed window |
| 60-second idle sample | layer host 1.08% of one core, peak combined RSS 195 MB (panel about 156 MB, mostly shared GPU driver pages). This is not the five-minute acceptance run |
| Not tested | keyboard input in the layer host (customization uses the window backend), GNOME or other compositors, fractional scaling, monitor hot-plug in layer mode, the 100 ms interaction limit |
