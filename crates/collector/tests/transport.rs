use sq_collector::*;
use sq_contract::*;
use sq_core::*;
use std::{fs, thread, time::Duration};
// REQ SQ-13 SQ-12 SQ-18: independent clients see conflicts and unchanged revisions.
#[test]
fn transport_rejects_bad_frames_and_preserves_configuration_between_clients() {
    let root = std::env::temp_dir().join(format!("sq-transport-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let server = Service::new(root.clone()).unwrap();
    let path = server.socket_path();
    let stop = server.stop_handle();
    let handle = thread::spawn(move || server.serve(false).unwrap());
    let client = Client::new(path.clone());
    // Wait until the server answers, not just until its socket file exists: under load the file
    // appears before the server accepts, which made this test fail on busy first runs.
    for _ in 0..500 {
        if matches!(client.request(Command::Ping), Ok(Reply::Pong)) {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(matches!(
        client.request(Command::Ping).unwrap(),
        Reply::Pong
    ));
    let Reply::Configuration { revision, config } = client.request(Command::Configuration).unwrap()
    else {
        panic!("configuration")
    };
    assert_eq!(config.widgets.len(), 7);
    assert!(matches!(
        client
            .request(Command::Configure {
                revision,
                action: ConfigAction::Remove("widget-1".into())
            })
            .unwrap(),
        Reply::Configuration { .. }
    ));
    assert!(matches!(
        client
            .request(Command::Configure {
                revision,
                action: ConfigAction::Replace(Box::new(*config))
            })
            .unwrap(),
        Reply::Error(_)
    ));
    let Reply::Snapshot(snapshot) = client.request(Command::Snapshot { after: None }).unwrap()
    else {
        panic!("snapshot")
    };
    assert!(matches!(
        client
            .request(Command::Snapshot {
                after: Some(snapshot.revision)
            })
            .unwrap(),
        Reply::Unchanged { .. }
    ));
    use std::{
        io::{Read, Write},
        os::unix::net::UnixStream,
    };
    let mut stream = UnixStream::connect(&path).unwrap();
    stream
        .write_all(&((MAX_FRAME + 1) as u32).to_be_bytes())
        .unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut response = [0; 4];
    assert_eq!(stream.read(&mut response).unwrap(), 0);
    stop.store(true, std::sync::atomic::Ordering::Release);
    handle.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}
