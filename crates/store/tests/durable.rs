#[path = "../../scratch.rs"]
mod scratch;
use scratch::Scratch;
use sq_core::*;
use sq_store::*;
use std::{fs, sync::Arc, thread};
fn root() -> Scratch {
    Scratch::new("sq-store-test")
}
#[test] // SQ-12: failed import and stale writer cannot overwrite good settings.
fn stale_writer_and_bad_import_preserve_disk_state() {
    let scratch = root();
    let dir = scratch.to_path_buf();
    let store = Store::new(dir.clone()).unwrap();
    let initial = store.configuration().unwrap();
    let changed = store
        .configure(initial.revision, ConfigAction::Remove("widget-1".into()))
        .unwrap();
    assert!(store
        .configure(initial.revision, ConfigAction::Replace(Box::default()))
        .is_err());
    assert_eq!(
        Store::new(dir.clone()).unwrap().configuration().unwrap(),
        changed
    );
    assert!(store.import(changed.revision, b"{torn").is_err());
    assert_eq!(store.configuration().unwrap(), changed);
    fs::remove_dir_all(dir).unwrap();
}
#[test] // SQ-12: readers see a whole old or new document during concurrent updates.
fn atomic_updates_remain_readable_and_lock_excludes_second_collector() {
    let scratch = root();
    let dir = scratch.to_path_buf();
    let store = Arc::new(Store::new(dir.clone()).unwrap());
    let lock = CollectorLock::acquire(&dir).unwrap();
    assert!(CollectorLock::acquire(&dir).is_err());
    let reader = {
        let path = dir.join("config.json");
        thread::spawn(move || {
            for _ in 0..500 {
                if path.exists() {
                    let data = fs::read(&path).unwrap();
                    let _: SavedConfig = serde_json::from_slice(&data).unwrap();
                }
            }
        })
    };
    for _ in 0..100 {
        let old = store.configuration().unwrap();
        store
            .configure(old.revision, ConfigAction::Replace(Box::new(old.config)))
            .unwrap();
    }
    reader.join().unwrap();
    drop(lock);
    assert!(CollectorLock::acquire(&dir).is_ok());
    fs::remove_dir_all(dir).unwrap();
}
#[test] // SQ-12: malformed or future files are reported, not silently replaced.
fn corrupt_existing_file_is_retained_and_reported() {
    let scratch = root();
    let dir = scratch.to_path_buf();
    fs::write(dir.join("config.json"), b"broken").unwrap();
    assert!(Store::new(dir.clone()).is_err());
    assert_eq!(fs::read(dir.join("config.json")).unwrap(), b"broken");
    fs::remove_dir_all(dir).unwrap();
}
#[test] // SQ-38: the old default progress folder of earlier versions means "not chosen".
fn legacy_default_progress_folder_is_cleared_on_load_and_other_choices_are_kept() {
    let scratch = root();
    let dir = scratch.join("legacy");
    fs::create_dir_all(&dir).unwrap();
    for (saved, expected) in [
        ("~/.local/share/codex-pbar/tasks", ""),
        ("~/my/progress", "~/my/progress"),
    ] {
        let mut config = Config::default();
        config.sources.progress = saved.into();
        let text = serde_json::json!({"revision": 3, "config": config}).to_string();
        fs::write(dir.join("config.json"), text).unwrap();
        let store = Store::new(dir.clone()).unwrap();
        let loaded = store.configuration().unwrap();
        assert_eq!(loaded.config.sources.progress, expected);
        assert_eq!(loaded.revision, 3);
    }
}
