# Status Quo requirements

Approved scope: `2026-11-01-status-quo-design.md` plus independent removable/interchangeable widgets. G: Giovanni; D: derived implementation promise. Tests carry `REQ SQ-*`; real-host acceptance supplements unit tests. No milestone approval pauses.

| ID | Requirement | Source |
|---|---|---|
| SQ-1 | Every data functionality is an independently removable, replaceable registered widget; an empty tray remains usable | G |
| SQ-2 | Independent instances retain separate settings, identity, order, size and detachment across restart | G |
| SQ-3 | Customize light/dark/system mode, colors, accent, font, background-only opacity and density; navigation-off preserves local controls; invalid imports preserve valid settings | G |
| SQ-4 | Deduplicate token history, preserve unavailable counters, reject regressions/overflow, retain 30 days and graph hourly/daily/cumulative values for 24h/7d/30d | G |
| SQ-5 | Show provider five-hour/weekly percentages used and resets selected by duration; missing limits are unavailable, never zero | G |
| SQ-6 | Read account status without model calls; timeouts/auth/rate-limit failures preserve last valid values and expose freshness | G |
| SQ-7 | Incremental transcript adapters handle partial/malformed files, rotation, cumulative counters and resets without silent loss or double counting | D |
| SQ-8 | Group main sessions by project with expandable subagents and explicit states; inactivity proves neither completion nor blockage | G |
| SQ-9 | Consume versioned lifecycle/progress events; hook installation preserves existing settings; unavailable evidence is explicit | G |
| SQ-10 | Selected GitHub repositories show commits, pull requests, issues, releases and checks with deduplication, bounded reads and visible access failures | G |
| SQ-11 | Read recent workflow-memory preferences, decisions, open/resolved items from both assistants with filters/timestamps and no writes; report malformed records | G |
| SQ-12 | Atomic versioned persistence rejects conflicting/invalid updates, preserves valid data and never stores credentials or conversations | D |
| SQ-13 | Per-user bounded local transport rejects incompatible versions and oversized requests; one collector serves independent hosts | D |
| SQ-14 | Core/contract/widget models have no GUI, filesystem, process or network dependencies; hosts/styles extend through registration | G |
| SQ-15 | KDE defaults behind windows, supports configurable stacking and retained geometry, recovers after monitor changes and exposes unsupported capabilities | G |
| SQ-16 | Actual rendering/actions honor save/cancel, reorder/resize, per-widget themes and add/remove/replace | G |
| SQ-17 | Configurable login startup and crash recovery cannot create duplicate collectors or erase another writer's state | D |
| SQ-18 | One failed source never freezes/hides unrelated widgets; loading/empty/unavailable/stale/error differ visibly | D |
| SQ-19 | Provider work is bounded and off the interface thread; backoff respects safe polling and local events display within two seconds | D |
| SQ-20 | Same widgets/tray render in another Rust host and browser sample without KDE imports or browser access to local credentials | G |
| SQ-21 | Presets/layout import/export are validated data-only; navigation uses safe schemes and argument-separated launches | G |
| SQ-22 | Linux integration checks actual X11/Wayland capabilities; portability claims identify evidence and fallbacks | G |
| SQ-23 | Laptop acceptance: under 1% of one core over five idle minutes, under 200 MiB collector+panel RSS and normal interaction within 100 ms | D |
| SQ-24 | No analytics, prompt transmission, generation or credential writes; diagnostics/history contain no complete conversations | G |
| SQ-25 | Verify installed package/startup/uninstall, template gate/CI and source-matched wiki translations | G |
| SQ-26 | A host compatibility layer chooses layer-shell, X11 desktop hints or a plain window at run time from the real session, honors a forced choice only when it can work, and reports what was chosen and why | G |
| SQ-27 | Layer-shell host shows the tray frameless on the desktop layer (below windows by default, anchored to an edge or corner, validated size), accepts pointer input, and costs no frames while idle | G |
| SQ-28 | Desktop-widget look: the tray itself has no background or title, only the widgets and the Add widget and Customize buttons are visible, and the widget rows fill the tray space exactly | G |
