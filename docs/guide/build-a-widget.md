# Build your own widget

Back to the [tour](../../README.md). By the end of this page you will have a new widget in your **+ Add widget** menu, with its own size, colours and settings.

## The idea in one picture

A widget never draws anything. It returns a small *description* of what to draw (text, a meter, a chart, a list), and the tray draws it. That is why one widget can run in the desktop panel, in a second Rust host and in the browser sample without change.

| You write | The tray gives you |
|---|---|
| `descriptor()`: the widget's kind, title and the accounts it understands | a place in the **+ Add widget** menu |
| `view()`: the pieces to show right now | layout, fonts, colours, resizing, scrolling, detach, remove, save and cancel |

The kind (`"note"` below) is the widget's permanent name inside saved settings. A kind that is empty or already taken is refused when you register it.

## Step 1: the smallest widget

This is the whole widget. It is compiled and run by the project's tests (`crates/tray/tests/custom_widget.rs`), so what you read here works.

```rust
use sq_contract::*;
use sq_core::*;

/// A widget that shows a line of text and, when asked, a progress meter. It needs no
/// collector data: everything comes from the options of its instance.
struct Note;
impl Widget for Note {
    fn descriptor(&self) -> Descriptor {
        Descriptor {
            kind: "note".into(),
            title: "Note".into(),
            providers: vec!["local".into()],
        }
    }
    fn view(&self, instance: &WidgetInstance, _snapshot: &Snapshot, _now: i64) -> View {
        let text = instance
            .options
            .get("text")
            .cloned()
            .unwrap_or_else(|| "Set the text option of this widget".into());
        let mut nodes = vec![Node::Text(text)];
        if let Some(percent) = instance
            .options
            .get("percent")
            .and_then(|v| v.parse::<f64>().ok())
        {
            nodes.push(Node::Meter {
                label: "Progress".into(),
                used: Some(percent.clamp(0.0, 100.0)),
                resets_at: None,
            });
        }
        View {
            title: "Note".into(),
            nodes,
        }
    }
}
```

`WidgetInstance` is *this particular copy* of the widget on screen: its `options` (the `name = value` pairs from Customize), its provider, size and position. `Snapshot` is everything the collector knows (see step 4). `now` is the current time, so a widget never reads a clock itself and can be tested at any time of day.

## Step 2: register it

Register the widget once, where the tray is created. For the desktop program that is `App::new` in `crates/desktop/src/main.rs`, right after `Tray::new(config)`:

```rust
tray.registry.register(Box::new(Note)).expect("unique widget kind");
```

Rebuild and run. Open **+ Add widget**: `local / Note` is there. Add it, open **Customize → Widgets → local / note**, add the option `text = Hello` and save.

Put the type in its own file (`mod note;`) so upgrades of Status Quo merge cleanly. The browser sample registers its own example the same way (`crates/web/src/lib.rs`).

## Step 3: the pieces you can show

| `Node` | Shows | Key fields |
|---|---|---|
| `Text(String)` | a line of text | |
| `Notice { tone, text }` | a small note in a muted, accent, warning or error colour | `Tone::Muted`, `Accent`, `Warning`, `Error` |
| `Meter { label, used, resets_at }` | a bar from 0 to 100 | `used: None` shows *unavailable*, not zero |
| `Chart { series, unit, scale, compact }` | a line chart | `ChartScale::FromZero`, `Fixed { min, max }`, `Fit { min_span }` |
| `Row { id, label, detail, action, children }` | an expandable row, optionally opening something | `action: Some(HostAction::OpenUrl(..))` |

Two rules keep widgets honest. **Unknown is not zero:** use `None`. **Never read files or start programs in `view()`:** the portable crates may not, and the project's boundary check fails if they do. If your widget needs outside data, that belongs in a collector source, not in the widget.

## Step 4: use the collector's data

`view()` receives the collector's `Snapshot`. Its fields:

| Field | Holds |
|---|---|
| `usage` | five-hour and weekly limits per account key |
| `tokens` | token buckets per account key |
| `sessions` | assistant sessions with state, task, progress and parent |
| `activity` | GitHub items |
| `memory` | workflow-memory entries |
| `system`, `gpu` | recent CPU, memory and graphics samples |
| `health` | per source: fresh, stale, empty, unavailable or error, and why |

Filter by `instance.provider` so your widget honours the account chosen for it, as the built-in ones do (see [accounts](accounts.md)). Declare which providers you understand in `descriptor()`: `claude`, `codex` and `github` expand to every account you configured; anything else (like `local` above) is used as written.

A completely new *source of data* (a service of your own) needs a worker in `crates/collector` and a reader in `crates/sources`; those are read-only by design and have their own tests. Plugins loaded at run time do not exist yet, and `Snapshot.extensions` is reserved but unused.

## Step 5: test it

Test the view without any window. The project's test registers the widget, adds an instance, sets options and checks the nodes:

```rust
let mut tray = Tray::new(Config::default());
tray.registry.register(Box::new(Note)).unwrap();
let id = tray.config.add_widget("note", "local");
tray.config.widget_mut(&id).unwrap().options.insert("text".into(), "Hello".into());
let widget = tray.config.widget(&id).unwrap().clone();
let view = tray.registry.view(&widget, &Snapshot::default(), 0);
assert_eq!(view.nodes[0], Node::Text("Hello".into()));
```

Also test the unhappy path (missing option, unparsable number, no data). A host that never registered your widget shows *"This host has no compatible renderer for this widget. Its settings are retained."* and keeps your settings, so moving a configuration between machines never loses a widget.

Tag requirement-driven tests with a `// REQ SQ-…` comment as the rest of the project does ([development notes](../DEVELOPMENT.md)).
