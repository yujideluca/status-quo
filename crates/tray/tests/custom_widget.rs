//! The widget of docs/guide/build-a-widget.md, compiled and run. If this stops compiling, the
//! guide is out of date.
// DOC-START
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
// DOC-END
use sq_tray::Tray;

// REQ SQ-40
#[test]
fn the_guide_widget_registers_renders_and_is_refused_twice() {
    let mut tray = Tray::new(Config::default());
    tray.registry.register(Box::new(Note)).unwrap();
    assert!(tray.registry.register(Box::new(Note)).is_err());
    let id = tray.config.add_widget("note", "local");
    let widget = tray.config.widget_mut(&id).unwrap();
    widget.options.insert("text".into(), "Hello".into());
    widget.options.insert("percent".into(), "250".into());
    let widget = tray.config.widget(&id).unwrap().clone();
    let view = tray.registry.view(&widget, &Snapshot::default(), 0);
    assert_eq!(view.nodes[0], Node::Text("Hello".into()));
    assert!(matches!(
        view.nodes[1],
        Node::Meter {
            used: Some(v),
            ..
        } if v == 100.0
    ));
    // Without the option the widget explains what to set; a host that never registered it
    // keeps the settings and says so.
    let mut bare = widget.clone();
    bare.options.clear();
    assert!(
        matches!(&tray.registry.view(&bare, &Snapshot::default(), 0).nodes[0], Node::Text(t) if t.contains("Set the text"))
    );
    let other = Tray::new(Config::default());
    let fallback = other.registry.view(&widget, &Snapshot::default(), 0);
    assert!(matches!(&fallback.nodes[0], Node::Notice { .. }));
    config_still_valid(&tray.config);
}
fn config_still_valid(config: &Config) {
    config.validate().unwrap();
}
