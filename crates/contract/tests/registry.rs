use sq_contract::*;
use sq_core::{Config, Snapshot, WidgetInstance};
struct Clock;
impl Widget for Clock {
    fn descriptor(&self) -> Descriptor {
        Descriptor {
            kind: "clock".into(),
            title: "Clock".into(),
            providers: vec!["local".into()],
        }
    }
    fn view(&self, _: &WidgetInstance, _: &Snapshot, now: i64) -> View {
        View {
            title: "Clock".into(),
            nodes: vec![Node::Text(now.to_string())],
        }
    }
}
#[test] // SQ-14 SQ-1: an unrelated widget registers without changing a tray or provider enum.
fn unknown_widget_roundtrips_and_can_be_replaced_with_registered_extension() {
    let mut registry = Registry::default();
    registry.register(Box::new(Clock)).unwrap();
    assert!(registry.register(Box::new(Clock)).is_err());
    let mut config = Config::default();
    let id = config.add_widget("clock", "local");
    let view = registry.view(config.widget(&id).unwrap(), &Snapshot::default(), 42);
    assert_eq!(view.nodes, vec![Node::Text("42".into())]);
    let json = serde_json::to_vec(&view).unwrap();
    assert_eq!(serde_json::from_slice::<View>(&json).unwrap(), view);
    config.widget_mut(&id).unwrap().kind = "future".into();
    assert!(matches!(
        registry
            .view(config.widget(&id).unwrap(), &Snapshot::default(), 42)
            .nodes[0],
        Node::Notice { .. }
    ));
}
#[test] // SQ-13: reject incompatible versions and oversized messages before decoding content.
fn ipc_requires_version_and_bounded_frame() {
    let request = Request {
        version: PROTOCOL_VERSION,
        body: Command::Snapshot { after: Some(9) },
    };
    assert_eq!(
        decode_request(&serde_json::to_vec(&request).unwrap()).unwrap(),
        request
    );
    assert!(decode_request(br#"{"version":999,"body":{"Snapshot":{"after":null}}}"#).is_err());
    assert!(decode_request(&vec![b' '; MAX_FRAME + 1]).is_err());
}
