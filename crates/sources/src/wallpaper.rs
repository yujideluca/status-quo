//! Finds the picture on the desktop and reads its colours (SQ-37). The parsers are pure and
//! tested on every platform; `locate` asks the real desktop.
use sq_core::{seed_from_pixels, WallpaperSeed};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Largest picture file that is decoded.
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
const EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "bmp", "webp", "jxl", "avif"];

/// What the locators may look at: the home folder, environment variables and short read-only
/// commands. Injected so fixtures can stand in for a desktop.
pub struct Probe<'a> {
    pub home: &'a Path,
    pub var: &'a dyn Fn(&str) -> Option<String>,
    pub run: &'a dyn Fn(&str, &[&str]) -> Option<String>,
}

/// The current wallpaper file of this desktop, or why it cannot be found.
pub fn locate() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or("no home folder")?;
    locate_with(
        &Probe {
            home: &home,
            var: &|name| std::env::var(name).ok().filter(|v| !v.is_empty()),
            run: &run_command,
        },
        cfg!(windows),
    )
}

pub fn locate_with(probe: &Probe, windows: bool) -> Result<PathBuf, String> {
    if let Some(file) = (probe.var)("STATUS_QUO_WALLPAPER") {
        return resolve_image(Path::new(&file))
            .ok_or_else(|| format!("STATUS_QUO_WALLPAPER names no picture: {file}"));
    }
    let mut tried = Vec::new();
    let sources: Vec<(&str, Source)> = if windows {
        vec![("Windows", windows_wallpaper)]
    } else {
        ordered_linux_sources(&(probe.var)("XDG_CURRENT_DESKTOP").unwrap_or_default())
    };
    for (name, source) in sources {
        if let Some(path) = source(probe).and_then(|p| resolve_image(&p)) {
            return Ok(path);
        }
        tried.push(name);
    }
    Err(format!(
        "no wallpaper found (looked at {})",
        tried.join(", ")
    ))
}

type Source = fn(&Probe) -> Option<PathBuf>;
/// Desktops listed in `XDG_CURRENT_DESKTOP` go first so a stale file of another desktop that was
/// once installed cannot win.
fn ordered_linux_sources(desktop: &str) -> Vec<(&'static str, Source)> {
    let mut all: Vec<(&'static str, &[&str], Source)> = vec![
        ("KDE Plasma", &["kde"], plasma_wallpaper),
        (
            "GNOME and Cinnamon and MATE settings",
            &[
                "gnome", "unity", "budgie", "pantheon", "cinnamon", "mate", "deepin", "ubuntu",
            ],
            gsettings_wallpaper,
        ),
        ("Xfce", &["xfce"], xfce_wallpaper),
        ("LXQt and LXDE", &["lxqt", "lxde"], lxde_wallpaper),
        ("Hyprland hyprpaper", &["hyprland"], hyprpaper_wallpaper),
        (
            "nitrogen and feh",
            &["i3", "bspwm", "openbox", "awesome", "dwm"],
            setter_wallpaper,
        ),
    ];
    let desktop = desktop.to_lowercase();
    let (hinted, rest): (Vec<_>, Vec<_>) = all
        .drain(..)
        .partition(|(_, tags, _)| tags.iter().any(|t| desktop.contains(t)));
    hinted
        .into_iter()
        .chain(rest)
        .map(|(name, _, source)| (name, source))
        .collect()
}

fn config_dir(probe: &Probe) -> PathBuf {
    (probe.var)("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| probe.home.join(".config"))
}
fn plasma_wallpaper(probe: &Probe) -> Option<PathBuf> {
    let text =
        std::fs::read_to_string(config_dir(probe).join("plasma-org.kde.plasma.desktop-appletsrc"))
            .ok()?;
    parse_plasma_wallpaper(&text).map(|v| uri_to_path(&v))
}
fn gsettings_wallpaper(probe: &Probe) -> Option<PathBuf> {
    let dark = (probe.run)(
        "gsettings",
        &["get", "org.gnome.desktop.interface", "color-scheme"],
    )
    .is_some_and(|v| v.contains("dark"));
    let mut keys: Vec<(&str, &str)> = Vec::new();
    for schema in [
        "org.gnome.desktop.background",
        "org.cinnamon.desktop.background",
    ] {
        if dark {
            keys.push((schema, "picture-uri-dark"));
        }
        keys.push((schema, "picture-uri"));
    }
    keys.push(("org.mate.background", "picture-filename"));
    keys.into_iter().find_map(|(schema, key)| {
        let value = parse_gsettings_string(&(probe.run)("gsettings", &["get", schema, key])?)?;
        let path = uri_to_path(&value);
        resolve_image(&path)
    })
}
fn xfce_wallpaper(probe: &Probe) -> Option<PathBuf> {
    let list = (probe.run)("xfconf-query", &["-c", "xfce4-desktop", "-l"])?;
    list.lines()
        .filter(|l| l.trim().ends_with("/last-image"))
        .find_map(|property| {
            let value = (probe.run)(
                "xfconf-query",
                &["-c", "xfce4-desktop", "-p", property.trim()],
            )?;
            resolve_image(Path::new(value.trim()))
        })
}
fn lxde_wallpaper(probe: &Probe) -> Option<PathBuf> {
    let config = config_dir(probe);
    [
        (config.join("pcmanfm-qt/lxqt/settings.conf"), "Wallpaper"),
        (
            config.join("pcmanfm/LXDE/desktop-items-0.conf"),
            "wallpaper",
        ),
        (
            config.join("pcmanfm/default/desktop-items-0.conf"),
            "wallpaper",
        ),
    ]
    .into_iter()
    .find_map(|(file, key)| {
        let text = std::fs::read_to_string(file).ok()?;
        let value = ini_value(&text, key)?;
        resolve_image(Path::new(&value))
    })
}
fn hyprpaper_wallpaper(probe: &Probe) -> Option<PathBuf> {
    let text = std::fs::read_to_string(config_dir(probe).join("hypr/hyprpaper.conf")).ok()?;
    parse_hyprpaper(&text, probe.home)
}
fn setter_wallpaper(probe: &Probe) -> Option<PathBuf> {
    if let Ok(text) = std::fs::read_to_string(config_dir(probe).join("nitrogen/bg-saved.cfg")) {
        if let Some(value) = ini_value(&text, "file") {
            return Some(PathBuf::from(value));
        }
    }
    parse_fehbg(&std::fs::read_to_string(probe.home.join(".fehbg")).ok()?)
}
fn windows_wallpaper(probe: &Probe) -> Option<PathBuf> {
    let registry = (probe.run)(
        "reg",
        &["query", r"HKCU\Control Panel\Desktop", "/v", "WallPaper"],
    )
    .and_then(|out| parse_reg_wallpaper(&out))
    .map(PathBuf::from)
    .and_then(|p| resolve_image(&p));
    registry.or_else(|| {
        let appdata = (probe.var)("APPDATA")?;
        resolve_image(
            &["Microsoft", "Windows", "Themes", "TranscodedWallpaper"]
                .iter()
                .fold(PathBuf::from(&appdata), |path, part| path.join(part)),
        )
    })
}

/// A file is used as it is (whatever its extension: Windows stores a JPEG without one); a
/// folder is a KDE wallpaper package or a folder of pictures: its smallest picture is used,
/// because only the colours matter and a small file costs less to read.
pub fn resolve_image(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return Some(path.to_path_buf());
    }
    if !path.is_dir() {
        return None;
    }
    for sub in ["contents/images", "images", ""] {
        let dir = path.join(sub);
        let mut best: Option<(u64, PathBuf)> = None;
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let file = entry.path();
            let ext = file.extension()?.to_str()?.to_lowercase();
            if !file.is_file() || !EXTENSIONS.contains(&ext.as_str()) {
                continue;
            }
            let area = size_from_name(&file).unwrap_or(u64::MAX - 1);
            if best.as_ref().is_none_or(|(a, p)| (area, &file) < (*a, p)) {
                best = Some((area, file));
            }
        }
        if let Some((_, file)) = best {
            return Some(file);
        }
    }
    None
}
fn size_from_name(file: &Path) -> Option<u64> {
    let stem = file.file_stem()?.to_str()?;
    let (w, h) = stem.split_once('x')?;
    Some(w.parse::<u64>().ok()? * h.parse::<u64>().ok()?)
}

/// The `Image=` value of the first desktop wallpaper of Plasma's applets file.
pub fn parse_plasma_wallpaper(text: &str) -> Option<String> {
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line.contains("[Wallpaper][org.kde.image][General]")
                && line.starts_with("[Containments]");
        } else if inside {
            if let Some(value) = line.strip_prefix("Image=") {
                if !value.trim().is_empty() {
                    return Some(value.trim().to_string());
                }
            }
        }
    }
    None
}
/// `'file:///a/b.jpg'` as printed by `gsettings get` → `file:///a/b.jpg`.
pub fn parse_gsettings_string(output: &str) -> Option<String> {
    let value = output.trim();
    let value = value.strip_prefix("@s ").unwrap_or(value);
    let value = value.trim_matches(|c| c == '\'' || c == '"').trim();
    (!value.is_empty()).then(|| value.to_string())
}
/// `file:///a%20b` → `/a b`; a plain path stays as it is.
pub fn uri_to_path(value: &str) -> PathBuf {
    let Some(rest) = value.strip_prefix("file://") else {
        return PathBuf::from(value);
    };
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            ) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    let text = String::from_utf8_lossy(&out).into_owned();
    // `file:///C:/x.png` carries a slash before the drive letter.
    let drive = text.as_bytes();
    if drive.len() > 2 && drive[0] == b'/' && drive[2] == b':' && drive[1].is_ascii_alphabetic() {
        return PathBuf::from(&text[1..]);
    }
    PathBuf::from(text)
}
/// First `key=value` of an ini-like file, sections ignored; the key is matched exactly.
pub fn ini_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (k, v) = line.split_once('=')?;
        let v = v.trim().trim_matches('"');
        (k.trim() == key && !v.is_empty()).then(|| v.to_string())
    })
}
/// The picture of a `~/.fehbg` script (`feh --no-fehbg --bg-fill '/a/b.jpg'`).
pub fn parse_fehbg(text: &str) -> Option<PathBuf> {
    text.lines()
        .filter(|l| l.trim_start().starts_with("feh"))
        .find_map(|line| {
            split_shell(line)
                .into_iter()
                .skip(1)
                .rfind(|t| !t.starts_with('-'))
                .map(PathBuf::from)
        })
}
fn split_shell(line: &str) -> Vec<String> {
    let (mut out, mut cur, mut quote, mut has) = (Vec::new(), String::new(), None, false);
    for c in line.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                has = true;
            }
            (None, c) if c.is_whitespace() => {
                if has || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if has || !cur.is_empty() {
        out.push(cur);
    }
    out
}
/// The first `wallpaper = monitor,path` (or `preload = path`) of hyprpaper's configuration.
pub fn parse_hyprpaper(text: &str, home: &Path) -> Option<PathBuf> {
    let expand = |p: &str| {
        let p = p.trim();
        match p.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => PathBuf::from(p),
        }
    };
    for key in ["wallpaper", "preload"] {
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            if k.trim() != key {
                continue;
            }
            let path = if key == "wallpaper" {
                v.split_once(',').map_or(v, |(_, p)| p)
            } else {
                v
            };
            let path = expand(path);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}
/// The data of `reg query "HKCU\Control Panel\Desktop" /v WallPaper`.
pub fn parse_reg_wallpaper(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let line = line.trim();
        let rest = line.strip_prefix("WallPaper")?.trim_start();
        let value = rest
            .strip_prefix("REG_SZ")
            .or_else(|| rest.strip_prefix("REG_EXPAND_SZ"))?;
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// Colours of the picture: decoded with limits, shrunk to 64 pixels and reduced to one hue.
pub fn load_seed(path: &Path) -> Result<WallpaperSeed, String> {
    let meta =
        std::fs::metadata(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("{} is too large to read", path.display()));
    }
    let mut reader = image::ImageReader::open(path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?
        .with_guessed_format()
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(30_000);
    limits.max_image_height = Some(30_000);
    limits.max_alloc = Some(768 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| {
        let kind = path.extension().and_then(|e| e.to_str()).unwrap_or("this");
        format!("the {kind} picture cannot be read here: {e}")
    })?;
    let small = image.thumbnail(64, 64).to_rgb8();
    let pixels: Vec<[u8; 3]> = small.pixels().map(|p| p.0).collect();
    seed_from_pixels(&pixels).ok_or_else(|| "the picture has no pixels".to_string())
}

/// Runs a short read-only command; missing program, failure or a hang beyond three seconds
/// all mean "no answer".
fn run_command(program: &str, args: &[&str]) -> Option<String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000); // no console window
    let mut child = command.spawn().ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait().ok()? {
            Some(status) => {
                if !status.success() {
                    return None;
                }
                let mut out = String::new();
                std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
                return Some(out);
            }
            None if start.elapsed() > Duration::from_secs(3) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}
