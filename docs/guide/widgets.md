# The widgets

Back to the [tour](../../README.md). A widget is one independent display: it has a *kind* (what it shows), a *provider* (whose data), its own options, size, position and, if you like, its own colours. You can add a kind several times.

Add one with **+ Add widget** (the menu lists `provider / kind` pairs). Change its settings in **Customize → Widgets**, where each entry can be moved, replaced, resized, hidden, detached into its own window or removed, and has an **options** list of `name = value` pairs.

A value that cannot be read is shown as **unavailable**. It is never drawn as zero, because zero would be a claim.

## `quota`: account usage

Percent of the **five-hour** and **weekly** limits used, with the time each resets. The percentage comes from the provider's own status, never from counting local tokens. Providers: `claude`, `codex` (and extra accounts such as `claude:work`). Options: none.

## `tokens`: token history

A graph of input, output, cache-read and cache-write tokens. The history is kept for 30 days.

| Option | Values | Default |
|---|---|---|
| `range` | `24h`, `7d`, `30d` | `24h` |
| `bucket` | `hourly`, `daily` | `hourly` |
| `cumulative` | `true` | per bucket |

Buttons inside the widget switch range and view without opening Customize.

## `sessions`: what your assistants are doing

Sessions grouped by project, subagents nested under their parent. States are explicit: running, waiting, blocked, completed, idle, or unknown. Silence proves neither completion nor a block, so an inactive session is not marked finished. When a session keeps a task list, the row shows `done/total` and the step in progress; Status Quo reads that list from the session's own transcript (Claude `TaskCreate`/`TaskUpdate`/`TodoWrite`, Codex `update_plan`) and shows nothing when there is none.

Providers: `both`, `claude`, `codex`. Option: `history = true` keeps finished sessions in the list (otherwise completed ones disappear after an hour and others after 24 hours).

## `activity`: recent GitHub activity

Commits, pull requests, issues, releases and checks of the repositories you select (Customize → *Selected GitHub repositories*, one `owner/name` per line). Provider: `github` or `github:name`. Option: `repository = owner/name` to show only that repository. A row opens the item in your browser when navigation is enabled.

## `memory`: workflow memory

Recent preference, decision and open/resolved entries from your workflow-memory notes folder (read-only). Providers: `both`, `claude`, `codex`. Option: `type` to show one kind of entry.

## `system`, `ram`, `gpu`: your machine

Light by design: no program is started to measure them.

* `system` reads CPU share and package temperature every 5 seconds from the kernel (Linux) or Windows, and graphs the last ten minutes.
* `ram` shows memory used and total, and swap.
* `gpu` shows every graphics card, of any maker, several at once. A second card that has put itself to sleep to save power is *not* woken up to be measured; the widget says it is powered down. On Intel only the share of time the chip is awake can be read without administrator rights, and it is labelled *Awake*.

Compact charts in these widgets share the widget's height, use a fixed 0 to 100 scale for percentages, and a fitted scale for temperature and clock.

## Next

Plug in more than one account: [accounts guide](accounts.md). Missing a widget? [Build your own](build-a-widget.md).
