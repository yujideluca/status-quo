# Status Quo for contributors

The user tutorial is the [README](../README.md). This page keeps the development, quality-gate and structure notes.

A modular Rust desktop widget tray for Linux, with KDE as its first desktop wrapper. Every function is a removable, replaceable widget: Claude and Codex account limits, token history, project sessions with subagents, recent GitHub activity and workflow-memory history. The initial style resembles a Linux terminal; themes and host wrappers register separately from data collection.

## Development

Requires Rust 1.87 or newer, Linux graphics libraries (X11/Wayland, xkbcommon, OpenGL), and existing Claude/Codex CLI authentication for live account status. GitHub reads use the installed `gh` CLI and its existing authorization. Status Quo does not create or refresh credentials.

```sh
cargo build --locked -p status-quo
cargo run --locked -p status-quo
cargo run --locked -p status-quo -- inspect --widgets
```

The desktop process starts one per-user collector if absent. `inspect --widgets` renders the same registered widget models in a second Rust host. The browser sample uses simulated data and no local credentials; see [browser demo](browser-demo.md).

```sh
rustup component add rustfmt clippy
rustup target add wasm32-unknown-unknown
tools/quality_gate.sh
tools/pipeline-check.sh --mode local
```

The full runner uses the shared `pipeline-check` tag pinned in `pipeline-check.version`. `tools/quality_gate.sh` runs formatting, warning-free compiler diagnostics, workspace tests, Rust dependency boundaries, traceability and fitness measurements. `docs/TRACEABILITY.md` is generated, not a substitute for tests or real desktop acceptance. GUI stacking, login startup, monitor changes and resource targets also need actual host evidence.

## Structure and privacy

| Crate | Responsibility |
|---|---|
| `sq-core` | Validated configuration, normalized data and token ledger; no GUI or IO |
| `sq-contract` | Widget registry and bounded versioned transport messages |
| `sq-widgets` | Portable widget display models |
| `sq-store` | Atomic private persistence and collector ownership lock |
| `sq-sources` | Read-only account, transcript, GitHub and memory adapters |
| `sq-collector` | Background source workers and per-user Unix socket |
| `sq-tray` | Reusable egui layout/customization and style registration |
| `status-quo` | Linux desktop wrapper and CLI |
| `sq-web` | Browser sample using the same tray with simulated data |

State is under `$XDG_STATE_HOME/status-quo` (normally `~/.local/state/status-quo`); `STATUS_QUO_STATE_DIR` overrides this for isolated tests. Configuration names the declared Claude, Codex, workflow-memory and progress locations. Token/session summaries are bounded to 30 days. Source errors and incomplete reads remain visible. No analytics, generated summaries or complete conversations are stored by the application. Navigation can be disabled independently of local layout controls.

[Requirements](REQUIREMENTS.md), [design](../DESIGN.md), [accepted scope](2026-11-01-status-quo-design.md), [test evidence](TESTES-AUTOMATIZADOS.md), [maintenance](MAINTENANCE.md).

This repository is under active implementation. A successful build alone does not establish installed-package, desktop or performance acceptance.
