//! Read-only native adapters. Pure parsers are shared with fixture verification.
mod parse;
#[cfg(test)]
#[path = "../../scratch.rs"]
mod scratch;
pub use parse::*;
mod logs;
pub use logs::*;
mod accounts;
pub use accounts::*;
mod github;
pub use github::*;
mod local;
pub use local::*;
mod lifecycle;
pub use lifecycle::*;
mod system;
pub use system::*;
mod gpu;
pub use gpu::*;
mod wallpaper;
pub use wallpaper::*;

/// A number that stays the same while a path names the same file and changes when the file is
/// replaced: the inode on Unix, the volume and file index on Windows. (Not the creation time:
/// Windows gives a file recreated under the same name within seconds the old creation time.)
pub fn file_identity(file: &std::fs::File, metadata: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        let _ = file;
        std::os::unix::fs::MetadataExt::ino(metadata)
    }
    #[cfg(windows)]
    {
        let _ = metadata;
        windows_file_index(file)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (file, metadata);
        0
    }
}
#[cfg(windows)]
fn windows_file_index(file: &std::fs::File) -> u64 {
    use std::os::windows::io::AsRawHandle;
    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct HandleInformation {
        attributes: u32,
        created: FileTime,
        accessed: FileTime,
        written: FileTime,
        volume_serial: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(
            file: *mut std::ffi::c_void,
            information: *mut HandleInformation,
        ) -> i32;
    }
    let mut info = HandleInformation::default();
    // SAFETY: the handle belongs to the open `file`; `info` is a valid, writable structure.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return 0;
    }
    ((u64::from(info.index_high) << 32) | u64::from(info.index_low))
        ^ (u64::from(info.volume_serial) << 48)
}
/// The program to start for a tool name. Unix shells find `gh` or `codex` by name. Windows only
/// finds `.exe` files that way, but npm installs tools as `.cmd` files, so `PATH` is searched
/// for `.exe`, `.cmd` and `.bat` there.
pub fn program(name: &str) -> std::path::PathBuf {
    if !cfg!(windows) || name.contains(['/', '\\']) {
        return std::path::PathBuf::from(name);
    }
    find_program(name, &std::env::var_os("PATH").unwrap_or_default())
        .unwrap_or_else(|| std::path::PathBuf::from(name))
}
/// Search `path` (a `PATH`-style list) for `name` with a Windows executable extension.
pub fn find_program(name: &str, path: &std::ffi::OsStr) -> Option<std::path::PathBuf> {
    std::env::split_paths(path).find_map(|dir| {
        ["exe", "cmd", "bat"]
            .iter()
            .map(|ext| dir.join(format!("{name}.{ext}")))
            .find(|candidate| candidate.is_file())
    })
}
