# ADR-0002: Portable Rust widgets with independent host wrappers

- Status: accepted
- Date: 2026-11-01
- Type: initial
- Decision: Giovanni's approved Status Quo scope

## Context

Each function must be a removable, interchangeable widget. Both the widgets and their tray must be usable in other environments. Giovanni selected Rust throughout and KDE first, with a Linux terminal visual style initially.

## Decision

Keep normalized data/configuration in `sq-core`, the widget/transport contract in `sq-contract`, and registered widget display models in `sq-widgets`. These crates have no GUI or IO dependencies. `sq-tray` renders registered models using egui and applies registered styles. A Linux desktop executable supplies windowing/navigation and connects to a single private per-user background collector. The browser demo uses the same tray with simulated data, without native credentials or collectors.

Version transport/configuration explicitly, bound frames and retained history, and keep settings updates atomic and revision checked. Source failures retain last valid observations with visible health. Widgets contain stable instance identity, provider/options, geometry and optional theme; no fixed provider slot is built into the tray.

## Alternatives

A native Plasma applet would require a different interface stack and would couple the first tray to KDE. A browser-only frontend would violate the all-Rust application request. Direct source reads inside widgets would prevent safe reuse and couple rendering latency to providers.

## Consequences

The first deliverable is a standalone desktop panel, not a native Plasma panel applet. X11/XWayland and native Wayland capabilities need actual host checks and explicit fallback reporting. New environments implement wrapper behavior without changing normalized source collection or widget registration.
