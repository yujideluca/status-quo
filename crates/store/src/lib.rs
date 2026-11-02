//! Atomic, private application state. Credentials and conversation text never belong here.
use fs2::FileExt;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sq_core::{Config, ConfigAction};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
const MAX_FILE: u64 = 32 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SavedConfig {
    pub revision: u64,
    pub config: Config,
}
pub struct Store {
    root: PathBuf,
    configuration: Mutex<SavedConfig>,
}
impl Store {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&root).map_err(|e| format!("Create state directory: {e}"))?;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
        let path = root.join("config.json");
        let configuration = if path.exists() {
            let saved: SavedConfig = read_json(&path)?;
            saved.config.validate()?;
            saved
        } else {
            let saved = SavedConfig {
                revision: 0,
                config: Config::default(),
            };
            atomic_json(&path, &saved)?;
            saved
        };
        Ok(Self {
            root,
            configuration: Mutex::new(configuration),
        })
    }
    pub fn configuration(&self) -> Result<SavedConfig, String> {
        self.configuration
            .lock()
            .map(|s| s.clone())
            .map_err(|_| "Configuration lock poisoned".into())
    }
    pub fn configure(&self, revision: u64, action: ConfigAction) -> Result<SavedConfig, String> {
        let mut current = self
            .configuration
            .lock()
            .map_err(|_| "Configuration lock poisoned")?;
        if revision != current.revision {
            return Err("Configuration changed in another window; reload before saving".into());
        }
        let mut next = current.clone();
        next.config.apply(action)?;
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or("Configuration revision exhausted")?;
        atomic_json(&self.root.join("config.json"), &next)?;
        *current = next.clone();
        Ok(next)
    }
    pub fn import(&self, revision: u64, bytes: &[u8]) -> Result<SavedConfig, String> {
        if bytes.len() as u64 > MAX_FILE {
            return Err("Configuration import exceeds size limit".into());
        }
        let config: Config = serde_json::from_slice(bytes)
            .map_err(|e| format!("Invalid configuration import: {e}"))?;
        self.configure(revision, ConfigAction::Replace(Box::new(config)))
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
}
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let file = File::open(path).map_err(|e| format!("Read {}: {e}", path.display()))?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_FILE {
        return Err(format!("{} exceeds state size limit", path.display()));
    }
    serde_json::from_reader(file).map_err(|e| {
        format!(
            "Invalid state in {}: {e}; original retained",
            path.display()
        )
    })
}
pub fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let parent = path.parent().ok_or("State file needs a parent directory")?;
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FILE {
        return Err("State exceeds size limit".into());
    }
    let temporary = parent.join(format!(
        ".sq-{}-{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> Result<(), String> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        fs::rename(&temporary, path).map_err(|e| e.to_string())?;
        File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
/// Kernel lock releases on process death; keep the inode, avoiding stale PID races.
pub struct CollectorLock {
    file: File,
}
impl CollectorLock {
    pub fn acquire(root: &Path) -> Result<Self, String> {
        fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(root.join("collector.lock"))
            .map_err(|e| e.to_string())?;
        file.try_lock_exclusive()
            .map_err(|_| "Another collector owns this state directory")?;
        Ok(Self { file })
    }
}
impl Drop for CollectorLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}
