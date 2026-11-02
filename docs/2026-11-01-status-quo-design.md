---
id: status-quo-design
title: Status Quo — proposed design and delivery scope
aliases: [Status Quo, widget host]
kind: reference
project: none
status: planned
since: 2026-11-01
location: wiki:content/status-quo-design.md
fed_by: []
feeds: []
sources: [project-template:CLAUDE.md, project-template:pipeline-check.toml]
memory: [#f1e117, #d975f2, #63efec, #c1f1f8]
reviewed: 2026-11-01T22:17Z
---

## What it is
Status Quo is a proposed desktop panel that lets Giovanni see assistant usage, ongoing work, recent repository activity, and shared-memory changes in one place. This is the design for review, not a description of installed software. The [[status-quo-layout]] page contains the accompanying layout mockup. Implementation starts only after this design is approved.

The first environment wrapper targets KDE Plasma, the desktop shell that manages panels and windows on Giovanni's laptop. A wrapper is the small environment-specific part around a reusable component. All authored application logic and interface code will be Rust. Status Quo remains an independent desktop panel, rather than requiring a native Plasma applet written in another interface language. Support for embedding in the native KDE panel is a future wrapper, not a claim that the first desktop wrapper provides it.

## What it does
Every display functionality is its own removable and interchangeable widget inside the widget tray. Initial widget types cover account usage meters, token graphs, sessions grouped by project, recent GitHub activity, and workflow-memory changes. Claude and Codex meters and graphs are independently configured instances, giving seven initial widget instances. A widget is a self-contained display component that can run in the Status Quo tray or in another compatible host. The tray is itself reusable: it arranges registered widgets without knowing how their data is collected.

### Accepted behavior
- Show the percentage **used**, with explicit labels, for each assistant's five-hour and weekly limits. Display the reset time and separate model limits when the source provides them. Never derive account percentages from local token totals.
- Give each assistant its own token graph. Switch between hourly/daily buckets and cumulative totals, and between 24-hour, seven-day, and 30-day ranges. Separate input, output, cache-read, and cache-write counts where available; an unsupported category is unavailable, never zero. Do not add cache counts to input counts unless the source defines them as disjoint.
- Read available local token history and retain a bounded 30-day normalized history. Deduplicate by source, session, and message identity, including when a file is reread or rotated. Mark incomplete periods and stale data. History that predates available records is unavailable.
- Show all discoverable main Claude and Codex sessions grouped by project, with expandable child agents. Each row has tool, task, state, update time, and progress when explicitly reported. Supported states are running, waiting for input, blocked, completed, idle, and unknown. Inactivity alone does not prove completion or blockage. Missing child-agent information is marked unsupported rather than shown as no children.
- Builder default for session history: show every currently running, waiting, or explicitly blocked session regardless of age; show idle/unknown sessions updated within 24 hours, and completed sessions from the last hour. A filter exposes retained session summaries from the last 30 days. Retain normalized summaries for 30 days without deleting or modifying assistant transcript files. Historical import is bounded to this period; do not scan years of transcripts at startup.
- Show activity in selected GitHub repositories, including authorized private repositories: commits, pull requests, issues, releases, and automated check results. A pull request is a proposed repository change. Seed the editable selection from local development repositories with GitHub remotes; do not automatically include every accessible repository. Use provider event identities and stable ordering to avoid duplicates across feeds. State coverage gaps when an event source cannot supply an event type or private history.
- Show recent preferences, decisions, open items, and resolved items from both assistants' workflow-memory. Start read-only, preserve event identities and dates, filter by category, and expand details on demand. Read through the memory system's supported interface or a versioned adapter; never hand-edit its event or generated view files. Do not publish this feed in wiki examples or remote web hosting.
- Provide navigation to source pages, projects, terminals, sessions, and memory entries where supported. A configuration switch disables source-opening and launch actions; settings, scrolling, filtering, expanding rows, and layout editing remain available. Navigation does not send assistant prompts, run project code, modify memory, or alter repositories.
- Start at login through a per-user desktop entry, with an enable/disable setting. Enforce one collector instance per user, while allowing multiple widget hosts.

### Appearance and layout
Default to a terminal style: monospace text, charcoal background, green accent, clear separators, and textual states as well as colors. Supply light, dark, and follow-system modes. Styles are replaceable Rust renderers using shared theme values, not hard-coded collector behavior. Adding a visual style must not change data collection or the widget contract.

The customization tab controls global and per-widget colors, accent, text size, compact/expanded presentation, background opacity, saved presets, and layout import/export. Background opacity is separate from text opacity so text stays readable. Settings preview before saving and offer cancel and restore defaults. Imported settings are validated and cannot execute commands or enable network access implicitly.

Widgets can be reordered by dragging or keyboard controls, resized, hidden, and detached into their own windows. Save panel geometry, widget order and size, and monitor association. Recover on-screen after monitor removal or resolution changes. Default to behind normal application windows; offer normal-window and always-on-top modes when supported. Default navigation is enabled. Moving and resizing require edit mode so normal use does not accidentally change the layout.

## How it does it
### Widget tray contract
The tray is a generic host, not a dashboard with permanent provider slots. An add-widget control lists registered compatible widget types. Each instance has its own stable identity, type/version, provider or source selection, configuration, layout position, and state. Users can add, remove, replace, reorder, hide, resize, or detach any instance without removing unrelated widgets or rewriting the tray. Adding a different compatible widget type requires registration through the contract, not changes to the tray's layout logic. Initial registration uses trusted Rust components shipped with the application; arbitrary downloaded executable plugins remain outside scope.

Account meters and token graphs are separate widgets even when the default layout places them next to each other. Removing a Claude graph must leave the Claude account meter and other widgets working; replacing GitHub activity with another compatible widget must persist after restart. Shared data adapters can serve several instances, but a widget's removal does not erase source history or other instances' settings. Removing every widget leaves an empty tray with an add-widget action. Tray settings, the widget catalog, and source diagnostics are host controls; all data displays belong to removable widgets.

Acceptance includes removing and restoring each initial widget independently, replacing one widget with a different registered type, running two independently configured instances of one type, and restoring the exact arrangement after restart. Prove that the same widget can move to a different compatible host while keeping its data contract and instance configuration. This clarification is Giovanni's explicit requirement, not an optional later expansion.

### Encapsulation and dependency direction
Use a Rust workspace, a repository containing separately buildable packages. Proposed package names are builder choices, not a public compatibility promise:

1. `status-quo-core`: time, source identities, usage windows, token buckets, session state, activity items, memory items, freshness, and validation. No graphical framework, desktop calls, credentials, or network dependencies.
2. `status-quo-contract`: versioned widget input/output messages, configuration schema, feature capabilities, and host actions. A capability says whether a host can place windows, open links, or provide a particular feed. Incompatible contract versions fail visibly instead of silently misreading data.
3. `status-quo-sources`: independent Claude, Codex, GitHub, task-progress, and memory adapters. An adapter translates an outside format into core records. Each has its own timeout, backoff, health state, and cancellation. One unavailable provider cannot freeze another or the interface.
4. `status-quo-store`: normalized history and settings with transactional writes, schema migration, bounded retention, and deterministic reload. Store source identifiers and counters, not credentials or complete conversations. The concrete storage library is an implementation choice documented before use.
5. `status-quo-collector`: shared background collector and bounded event subscriptions. Local transport uses a per-user socket with user-only permissions. No public listener or hosted dashboard by default. Subscribers recover from interruption by taking a snapshot and resuming from a cursor.
6. `status-quo-widgets`: reusable Rust widget components and terminal renderer. Components consume contract snapshots and emit declared actions; they cannot read files, contact providers, launch processes, or place desktop windows directly.
7. `status-quo-panel`: reusable arrangement, tabs, customization, focus, and keyboard interaction. It knows widgets through registration and capabilities, not provider-specific branching.
8. `status-quo-kde`: first host wrapper, responsible for desktop integration, rendering surface, placement, monitor changes, startup, and opening approved targets. A separate example host proves reuse without importing this package.

Core and contract are at the center. Collectors, storage, and renderers depend inward on them. Desktop code stays at the outer boundary. Boundary tests enforce this direction. Existing shared tools, including task progress and pipeline-check, are consumed by pinned interfaces or versions; do not copy their implementations into this repository.

### Universal interface: explicit meaning
Universal means the same widget data, configuration, and actions can be carried between host environments through the versioned contract. It does not mean desktop window features exist in a browser. Hosts advertise capabilities; the panel explains unsupported features and offers the supported behavior.

Proposed renderer: egui, a Rust interface library; its eframe integration supports native and browser builds. Pin a verified version after approval. Browser output uses WebAssembly, a browser-executable compilation target; tool-generated loader files are build output, not a second hand-authored application language. Browser hosts cannot read local assistant credentials or files. Demonstrate reusable rendering with deterministic sample data; live web integration requires an explicit local bridge and origin restrictions before it is enabled.

The current laptop was observed running KDE on Wayland with XWayland available on display `:0`. Wayland is the display protocol controlling application surfaces; XWayland runs applications using the older X11 protocol inside that session. Do not design from the older assumption of a native X11 desktop. The KDE wrapper should try native supported desktop integration first, and may use an XWayland-backed surface when it is the verified way to achieve placement and stacking. Do not change Giovanni's global desktop settings to force support. Wrapper capability checks and real stacking tests decide which mode is available.

Before broadening Linux claims, test supported X11 and Wayland host paths. Other compositors may restrict absolute placement, background layers, or transparency. Report these as specific unsupported capabilities with a usable normal-window fallback. Never claim that KDE validation proves every Linux desktop works.

### Source strategy and failure behavior
First investigate existing assistant status feeds and task-progress contracts. Extend their hooks only when necessary and reversibly, preserving existing terminal status displays. A hook is a small action triggered by an assistant lifecycle event. Every emitted session record needs a version, session identity, project identity, sequence/time, task label, state, and optional parent and progress. A project identity comes from a resolved repository path or another explicit project root; ambiguous sessions remain in an unassigned group.

Claude account limits must come from a verified account status source. The reference widget documents its account usage endpoint and cached statusline input. Prefer an existing reliable local feed when available; otherwise use an isolated adapter with conservative polling and exponential retry delays. Codex's local application server is a candidate source for account limits; verify its installed protocol before selecting it. Usage reads must not invoke a model or consume paid generation. Reuse only declared assistant credential sources, read-only; never scan unrelated configuration directories, renew credentials, or write authentication files.

Token totals come from local usage records, not account quota percentages. Preserve raw counter meaning and source coverage. An adapter encountering malformed or partial records reports the count and source identity, retries partial writes safely, and quarantines or explicitly skips invalid records without silently losing data. Do not log prompt text or credential contents. Scrub fixtures before committing them.

GitHub reads reuse existing authorized access. Initial defaults: conditional refresh every five minutes, bounded page sizes and history, and backoff on rate limits. Assistant limit polling starts at five minutes unless an existing fresh feed updates sooner. Local session events update promptly through watched records or hooks; use bounded rescans as recovery, not continuous full-history scans. Refresh timing is configurable within safe provider limits. Show last successful update and retry status separately for every source.

Workflow-memory remains authoritative for its own meaning and visibility rules. Use its normalized supported output where possible; do not infer a current preference just from an old raw event. No memory mutation from Status Quo in this scope.

### Configuration, security, and resource targets
Keep per-user settings and caches in standard Linux user configuration/data locations. Save credentials nowhere in Status Quo. Write settings atomically, validate schema versions, and preserve the last valid configuration when an import or write fails. Navigation accepts supported schemes and argument-separated process launches, never shell interpolation of source text. No analytics, news, webhooks, automatic model summaries, or public service are in scope.

Proposed acceptance targets, to be measured on Giovanni's laptop: under 1% average of one processor core while idle over five minutes; under 200 MiB resident memory for collector plus one panel with 30 days of representative history; session update within two seconds of receipt of a local lifecycle event; ordinary interaction response within 100 ms. A missed target is a defect to investigate, not permission to omit features. Revisit a target with Giovanni if meeting it would change the scope. Hidden panels do not continuously redraw; collectors use bounded queues and capped history.

## Where it sits in the pipeline
Scope-driven development means a named deliverable with explicit acceptance checks, rather than a time-box that silently drops features. The baseline is this design plus [[status-quo-layout]]. Requirements receive stable `SQ-*` identities before code. Every requirement gets a meaningful failing test before implementation where automation can observe the behavior; real compositor and visual checks supplement automated tests.

Start from [[project-template]] after design approval, in the proposed private `yujideluca/status-quo` repository. Copy the approved design to `docs/2026-11-01-status-quo-design.md`; keep the wiki explanation aligned. Record structural choices as architecture decision records, documents explaining the choice and its alternatives. Use the pinned pipeline-check runner and a single quality gate, the command deciding whether a revision passes. The gate includes formatting, compiler diagnostics, tests, requirement traceability, dependency boundaries, and relevant failure/mutation checks. Mutation checks deliberately change behavior to confirm that tests detect defects. Define meaningful limits in the project configuration; do not weaken them to pass.

### Numbered delivery plan
1. **Contract and panel scope.** Establish requirements, repository, contracts, simulated providers, reusable panel, KDE wrapper, and terminal style. Accept when sample widgets arrange, resize, detach, and persist; customization saves/cancels/imports correctly; real windows obey the chosen stacking mode and recover after resize/restart/monitor changes.
2. **Usage scope.** Implement verified Claude/Codex sources, history, graphs, quotas, resets, source health, and polling. Accept when displayed live limits match independently read provider values; graph buckets reconcile to deduplicated fixtures; missing/rotated/partial files, midnight, time zones, restart, and rate limiting behave correctly. Test offline recovery without losing last-known data.
3. **Sessions scope.** Add main sessions, project groups, subagents, explicit state, task progress, and optional navigation. Accept through simultaneous Claude/Codex main sessions with child agents, waiting/blocked/completed transitions, crashes, and reconnects; prove existing terminal progress still works. Mark unverifiable or unsupported states explicitly.
4. **GitHub and memory scope.** Add editable repository selection, activity filtering and paging, and both assistants' read-only memory feed. Accept when records reconcile to authorized source reads, duplicates are suppressed, private access failures are visible, and navigation-off suppresses launches without disabling local controls.
5. **Reuse scope.** Run the same widget components and panel in a separate Rust example host without KDE imports. Compile and render a browser example with sample data. Verify contract compatibility handling, unsupported capabilities, and a new style registration without collector changes. Native KDE panel applet embedding and public live-web hosting are future wrappers, outside this first delivery.
6. **Release scope.** Run the complete gate, memory and performance checks, and real desktop verification across reload, idle, concurrent updates, compositor modes available for testing, and monitor changes. Package for the laptop, verify login startup and uninstall behavior, document Linux compatibility evidence and remaining gaps, complete maintenance review, and publish the wiki in English, German, and Brazilian Portuguese.

After approval, continue through these scopes without stopping for routine milestone approvals. Stop only for a blocker that cannot be resolved autonomously or a change requiring Giovanni's scope decision. Keep task progress truthful and report findings as they arise. A failing check triggers investigation; it is not a reason to proceed to release.

Every implementation commit describes changed files, reasons, requirements, and any outside-repository changes. Required continuous integration, the automated checks run on each push, must pass on the exact commit before promotion to `main`. A temporary branch exists only while those checks run; remove it and its worktree after promotion. Do not force-push or bypass protection. End with maintenance review, wiki publication, and memory-use recording.

### Out of scope
No sending prompts, account management, purchases, memory edits, arbitrary third-party executable plugins, Windows/macOS release, public dashboard, native Plasma-panel applet, or additional visual style pack in the first delivery. Future wrapper and style interfaces are in scope; claiming universal environment compatibility without tests is not.

## Who receives its output
Giovanni reviews this design and [[status-quo-layout]] once before implementation. A builder uses the approved baseline to produce the project requirements and complete every delivery scope. Future wrapper authors use the documented contract rather than duplicating collectors.

## Notes
This draft records Giovanni's choices from the Status Quo conversation of 2026-11-01. Proposed technical details and resource thresholds are builder recommendations pending approval of the full design. No Status Quo executable, repository, authentication adapter, or live collection has been created during this design task.

References consulted: [reference widget](https://github.com/bozdemir/claude-usage-widget/blob/main/README.md), [egui architecture](https://github.com/emilk/egui/blob/main/ARCHITECTURE.md), and [KDE widget setup](https://develop.kde.org/docs/plasma/widget/setup/). A native applet is distinct from a standalone window integrated with the KDE desktop.
