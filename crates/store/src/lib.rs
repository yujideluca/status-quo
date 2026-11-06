//! Atomic, private application state. Credentials and conversation text never belong here.
use fs2::FileExt;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sq_core::{Config, ConfigAction};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};
/// Owner-only files and folders. Unix gets mode bits; on Windows the per-user profile folders
/// that hold all state are already private to the user, so these are no-ops there.
pub mod private {
    use std::{fs::OpenOptions, io, path::Path};
    pub trait PrivateOpen {
        /// Create new files with the given Unix mode (ignored where modes do not exist).
        fn private(&mut self, mode: u32) -> &mut Self;
    }
    impl PrivateOpen for OpenOptions {
        #[cfg(unix)]
        fn private(&mut self, mode: u32) -> &mut Self {
            std::os::unix::fs::OpenOptionsExt::mode(self, mode)
        }
        #[cfg(not(unix))]
        fn private(&mut self, _mode: u32) -> &mut Self {
            self
        }
    }
    /// Make a completed rename durable by flushing its folder. Windows cannot open a folder as a
    /// file, and NTFS keeps renames in its journal, so there it does nothing.
    pub fn sync_dir(path: &Path) -> io::Result<()> {
        #[cfg(unix)]
        {
            std::fs::File::open(path).and_then(|f| f.sync_all())
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Ok(())
        }
    }
    /// Make a folder readable by its owner only.
    pub fn private_dir(path: &Path) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Ok(())
        }
    }
    /// Make a file readable by its owner only.
    pub fn private_file(path: &Path) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Ok(())
        }
    }
}
use private::PrivateOpen;
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
const MAX_FILE: u64 = 32 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SavedConfig {
    pub revision: u64,
    pub config: Config,
}
const LEGACY_PROGRESS_DEFAULT: &str = "~/.local/share/codex-pbar/tasks";
pub struct Store {
    root: PathBuf,
    configuration: Mutex<SavedConfig>,
}
impl Store {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&root).map_err(|e| format!("Create state directory: {e}"))?;
        private::private_dir(&root).map_err(|e| e.to_string())?;
        let path = root.join("config.json");
        let configuration = if path.exists() {
            let mut saved: SavedConfig = read_json(&path)?;
            // Earlier versions defaulted the progress folder to the codex-pbar task files. That
            // is now an optional extra; the old default means "not chosen" (SQ-38).
            if saved.config.sources.progress == LEGACY_PROGRESS_DEFAULT {
                saved.config.sources.progress.clear();
            }
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
            .private(0o600)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        fs::rename(&temporary, path).map_err(|e| e.to_string())?;
        private::sync_dir(parent).map_err(|e| e.to_string())?;
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
            .private(0o600)
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
