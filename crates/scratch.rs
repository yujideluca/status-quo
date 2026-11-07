//! Scratch folders for tests that delete themselves (shared by every crate through `#[path]`).
//!
//! A test that builds `temp_dir()/name-PID` by hand leaves the folder behind whenever it fails or
//! forgets to remove it, and a rerun has a new PID, so nothing ever removes it; on Giovanni's
//! laptop /tmp is RAM. Every test folder comes from `Scratch::new`, which removes the folder
//! when the guard drops, also while a failed assertion unwinds. Set KEEP_SCRATCH=1 to keep the
//! folders for inspection. `tools/quality_gate.sh` refuses `temp_dir()` anywhere else unless the
//! line says `scratch-ok:` and why.
#![allow(dead_code)]
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(prefix: &str) -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Scratch(path)
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl std::ops::Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<std::ffi::OsStr> for Scratch {
    fn as_ref(&self) -> &std::ffi::OsStr {
        self.0.as_os_str()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if std::env::var_os("KEEP_SCRATCH").is_none() {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
