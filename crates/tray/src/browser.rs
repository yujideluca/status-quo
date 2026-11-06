//! The folder navigator of Customize (SQ-39). The tray never touches the file system: the host
//! implements `FolderBrowser`, the tray owns the window, the buttons and the path rules.
use std::sync::Arc;

/// What a host offers so the tray can show folders. Listings are read-only.
pub trait FolderBrowser: Send + Sync {
    /// The user's home folder, absolute.
    fn home(&self) -> String;
    /// Where browsing can start from: `/` on Unix, the drives on Windows.
    fn roots(&self) -> Vec<String>;
    /// Names of the sub-folders of `folder`, or the reason it cannot be read.
    fn folders(&self, folder: &str) -> Result<Vec<String>, String>;
}
pub type SharedBrowser = Arc<dyn FolderBrowser>;

/// Which setting a chosen folder goes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowseTarget {
    Claude,
    Codex,
    Memory,
    Progress,
    /// Home folder of the extra account at this index.
    Account(usize),
    /// Added to the list of project roots.
    ProjectRoot,
}

/// The open window: where it is and what it found there.
pub struct Picker {
    pub target: BrowseTarget,
    pub current: String,
    pub folders: Result<Vec<String>, String>,
}

fn windows_style(path: &str) -> bool {
    path.contains('\\') || path.as_bytes().get(1) == Some(&b':')
}
fn sep(path: &str) -> char {
    if windows_style(path) {
        '\\'
    } else {
        '/'
    }
}
/// `~` and `~/x` made absolute against `home`; other paths unchanged.
pub fn expand_home(path: &str, home: &str) -> String {
    if path == "~" {
        home.to_string()
    } else if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        join(home, rest)
    } else {
        path.to_string()
    }
}
/// The folder one level up, or `None` at a root (`/`, `C:\`).
pub fn parent(path: &str) -> Option<String> {
    let t = path.trim_end_matches(['/', '\\']);
    if t.is_empty() || (t.len() == 2 && t.as_bytes()[1] == b':') {
        return None;
    }
    let up = &t[..t.rfind(['/', '\\'])?];
    if up.is_empty() {
        return Some("/".into());
    }
    if up.len() == 2 && up.as_bytes()[1] == b':' {
        return Some(format!("{up}\\"));
    }
    Some(up.to_string())
}
pub fn join(folder: &str, name: &str) -> String {
    let s = sep(folder);
    if folder.ends_with(['/', '\\']) {
        format!("{folder}{name}")
    } else {
        format!("{folder}{s}{name}")
    }
}
/// The form stored in the configuration: `~/…` inside the home folder (so a shared setting
/// works for another user), the absolute path elsewhere.
pub fn to_config_path(chosen: &str, home: &str) -> String {
    let home = home.trim_end_matches(['/', '\\']);
    if chosen.trim_end_matches(['/', '\\']) == home {
        return "~".into();
    }
    if let Some(rest) = chosen.strip_prefix(home) {
        if rest.starts_with(['/', '\\']) {
            return format!("~/{}", rest[1..].replace('\\', "/"));
        }
    }
    chosen.to_string()
}
/// Start of the navigator: the setting's current folder, or the nearest parent that can be
/// listed, or home.
pub fn start_folder(browser: &dyn FolderBrowser, setting: &str) -> String {
    let home = browser.home();
    let mut folder = if setting.trim().is_empty() {
        home.clone()
    } else {
        expand_home(setting.trim(), &home)
    };
    for _ in 0..64 {
        if browser.folders(&folder).is_ok() {
            return folder;
        }
        match parent(&folder) {
            Some(up) => folder = up,
            None => break,
        }
    }
    home
}
impl Picker {
    pub fn open(browser: &dyn FolderBrowser, target: BrowseTarget, setting: &str) -> Self {
        let current = start_folder(browser, setting);
        Self {
            target,
            folders: browser.folders(&current),
            current,
        }
    }
    pub fn go(&mut self, browser: &dyn FolderBrowser, folder: String) {
        self.folders = browser.folders(&folder);
        self.current = folder;
    }
}
