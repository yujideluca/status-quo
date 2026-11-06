//! Hooks retain only the newest sanitized lifecycle observation per session.
use fs2::FileExt;
use sq_sources::LifecycleEvent;
use sq_store::private::PrivateOpen;
use std::{
    fs::{self, OpenOptions},
    path::Path,
};
pub fn record_lifecycle(root: &Path, event: &LifecycleEvent) -> Result<bool, String> {
    if event.version != 1
        || !["claude", "codex"].contains(&event.assistant.as_str())
        || event.id.is_empty()
        || event.id.len() > 200
        || !event
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Err("Invalid lifecycle observation".into());
    }
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    sq_store::private::private_dir(root).map_err(|e| e.to_string())?;
    let directory = root.join("events");
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    sq_store::private::private_dir(&directory).map_err(|e| e.to_string())?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .private(0o600)
        .open(root.join("events.lock"))
        .map_err(|e| e.to_string())?;
    lock.lock_exclusive().map_err(|e| e.to_string())?;
    let path = directory.join(format!("{}-{}.json", event.assistant, event.id));
    if path.exists() {
        let previous: LifecycleEvent = sq_store::read_json(&path)?;
        if (previous.at, previous.sequence) >= (event.at, event.sequence) {
            return Ok(false);
        }
    }
    sq_store::atomic_json(&path, event)?;
    Ok(true)
}
