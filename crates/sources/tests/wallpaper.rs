use sq_sources::*;
use std::{cell::RefCell, collections::HashMap, fs, path::Path, path::PathBuf};

fn temp(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("sq-wall-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}
fn png(path: &Path, rgb: [u8; 3], size: (u32, u32)) {
    image::RgbImage::from_pixel(size.0, size.1, image::Rgb(rgb))
        .save_with_format(path, image::ImageFormat::Png)
        .unwrap();
}
/// A desktop made of environment variables and canned command answers.
struct Fake {
    vars: HashMap<&'static str, String>,
    commands: HashMap<String, String>,
    asked: RefCell<Vec<String>>,
}
impl Fake {
    fn new() -> Self {
        Fake {
            vars: HashMap::new(),
            commands: HashMap::new(),
            asked: RefCell::new(Vec::new()),
        }
    }
    fn locate(&self, home: &Path, windows: bool) -> Result<PathBuf, String> {
        let var = |n: &str| self.vars.get(n).cloned();
        let run = |p: &str, a: &[&str]| {
            let key = format!("{p} {}", a.join(" "));
            self.asked.borrow_mut().push(key.clone());
            self.commands.get(&key).cloned()
        };
        locate_with(
            &Probe {
                home,
                var: &var,
                run: &run,
            },
            windows,
        )
    }
}

// REQ SQ-37
#[test]
fn parsers_read_each_desktop_format() {
    let rc = "[Containments][2]\nplugin=org.kde.panel\n[Containments][1][Wallpaper][org.kde.image][General]\nImage=file:///usr/share/wallpapers/Orionids/\nSlidePaths=/x\n";
    assert_eq!(
        parse_plasma_wallpaper(rc).as_deref(),
        Some("file:///usr/share/wallpapers/Orionids/")
    );
    assert_eq!(
        parse_plasma_wallpaper("[Containments][1]\nImage=nope\n"),
        None
    );
    assert_eq!(
        parse_gsettings_string("'file:///a/b.jpg'\n").as_deref(),
        Some("file:///a/b.jpg")
    );
    assert_eq!(parse_gsettings_string("''\n"), None);
    assert_eq!(
        uri_to_path("file:///a/my%20pic%C3%A9.jpg"),
        PathBuf::from("/a/my picé.jpg")
    );
    assert_eq!(
        uri_to_path("file:///C:/Users/x/a.png"),
        PathBuf::from("C:/Users/x/a.png")
    );
    assert_eq!(
        uri_to_path("/plain/path.png"),
        PathBuf::from("/plain/path.png")
    );
    assert_eq!(uri_to_path("file:///bad%zz%"), PathBuf::from("/bad%zz%"));
    assert_eq!(
        ini_value("[a]\nWallpaper=/w.png\n", "Wallpaper").as_deref(),
        Some("/w.png")
    );
    assert_eq!(ini_value("[a]\nwallpaper=/w.png\n", "Wallpaper"), None);
    assert_eq!(
        parse_fehbg("#!/bin/sh\nfeh --no-fehbg --bg-fill '/home/u/my pics/a.jpg'\n"),
        Some(PathBuf::from("/home/u/my pics/a.jpg"))
    );
    let reg = "\nHKEY_CURRENT_USER\\Control Panel\\Desktop\n    WallPaper    REG_SZ    C:\\Users\\u\\Pictures\\a b.jpg\n\n";
    assert_eq!(
        parse_reg_wallpaper(reg).as_deref(),
        Some("C:\\Users\\u\\Pictures\\a b.jpg")
    );
    assert_eq!(parse_reg_wallpaper("    WallPaper    REG_SZ    \n"), None);
}

// REQ SQ-37
#[test]
fn kde_package_folder_resolves_to_its_smallest_picture() {
    let root = temp("kde");
    let home = root.join("home");
    let package = root.join("Orionids/contents/images");
    fs::create_dir_all(&package).unwrap();
    png(&package.join("5120x2880.png"), [10, 10, 200], (4, 4));
    png(&package.join("1440x2960.png"), [10, 10, 200], (4, 4));
    fs::create_dir_all(home.join(".config")).unwrap();
    fs::write(
        home.join(".config/plasma-org.kde.plasma.desktop-appletsrc"),
        format!(
            "[Containments][1][Wallpaper][org.kde.image][General]\nImage=file://{}/\n",
            root.join("Orionids").display()
        ),
    )
    .unwrap();
    let mut fake = Fake::new();
    fake.vars.insert("XDG_CURRENT_DESKTOP", "KDE".into());
    assert_eq!(
        fake.locate(&home, false).unwrap(),
        package.join("1440x2960.png")
    );
}

// REQ SQ-37
#[test]
fn gnome_uses_the_dark_picture_in_a_dark_session_and_the_desktop_hint_decides_the_order() {
    let root = temp("gnome");
    let (light, dark) = (root.join("l.png"), root.join("d.png"));
    png(&light, [200, 200, 10], (2, 2));
    png(&dark, [10, 10, 200], (2, 2));
    let mut fake = Fake::new();
    fake.vars
        .insert("XDG_CURRENT_DESKTOP", "ubuntu:GNOME".into());
    fake.commands.insert(
        "gsettings get org.gnome.desktop.background picture-uri".into(),
        format!("'file://{}'\n", light.display()),
    );
    fake.commands.insert(
        "gsettings get org.gnome.desktop.background picture-uri-dark".into(),
        format!("'file://{}'\n", dark.display()),
    );
    assert_eq!(fake.locate(&root, false).unwrap(), light);
    fake.commands.insert(
        "gsettings get org.gnome.desktop.interface color-scheme".into(),
        "'prefer-dark'\n".into(),
    );
    assert_eq!(fake.locate(&root, false).unwrap(), dark);
    // GNOME is hinted, so Xfce and KDE are never asked before it succeeds.
    assert!(!fake
        .asked
        .borrow()
        .iter()
        .any(|c| c.starts_with("xfconf-query")));
}

// REQ SQ-37
#[test]
fn xfce_lxqt_feh_nitrogen_and_hyprpaper_are_read() {
    let root = temp("others");
    let pic = root.join("w.png");
    png(&pic, [10, 200, 10], (2, 2));
    let cfg = |rel: &str, text: String| {
        let file = root.join(".config").join(rel);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    };
    let mut fake = Fake::new();
    fake.commands.insert(
        "xfconf-query -c xfce4-desktop -l".into(),
        "/backdrop/screen0/monitor0/workspace0/color-style\n/backdrop/screen0/monitor0/workspace0/last-image\n".into(),
    );
    fake.commands.insert(
        "xfconf-query -c xfce4-desktop -p /backdrop/screen0/monitor0/workspace0/last-image".into(),
        format!("{}\n", pic.display()),
    );
    assert_eq!(fake.locate(&root, false).unwrap(), pic);
    let fake = Fake::new();
    assert!(fake.locate(&root, false).is_err());
    cfg(
        "pcmanfm-qt/lxqt/settings.conf",
        format!("[Desktop]\nWallpaper={}\n", pic.display()),
    );
    assert_eq!(fake.locate(&root, false).unwrap(), pic);
    fs::remove_file(root.join(".config/pcmanfm-qt/lxqt/settings.conf")).unwrap();
    cfg(
        "nitrogen/bg-saved.cfg",
        format!("[xin_-1]\nfile={}\n", pic.display()),
    );
    assert_eq!(fake.locate(&root, false).unwrap(), pic);
    fs::remove_file(root.join(".config/nitrogen/bg-saved.cfg")).unwrap();
    fs::write(
        root.join(".fehbg"),
        format!("feh --bg-fill '{}'\n", pic.display()),
    )
    .unwrap();
    assert_eq!(fake.locate(&root, false).unwrap(), pic);
    fs::remove_file(root.join(".fehbg")).unwrap();
    cfg(
        "hypr/hyprpaper.conf",
        format!("preload = {0}\nwallpaper = eDP-1,{0}\n", pic.display()),
    );
    assert_eq!(fake.locate(&root, false).unwrap(), pic);
}

// REQ SQ-37 SQ-36
#[test]
fn windows_reads_the_registry_then_the_transcoded_file_and_an_override_wins_everywhere() {
    let root = temp("win");
    let picture = root.join("pic.png");
    png(&picture, [200, 10, 10], (2, 2));
    // On Linux a backslash is a file-name character; the fixture only needs the same name.
    let transcoded_file = root.join(r"Microsoft\Windows\Themes\TranscodedWallpaper");
    png(&transcoded_file, [10, 10, 10], (2, 2));
    let mut fake = Fake::new();
    fake.vars.insert("APPDATA", root.display().to_string());
    assert_eq!(fake.locate(&root, true).unwrap(), transcoded_file);
    fake.commands.insert(
        r"reg query HKCU\Control Panel\Desktop /v WallPaper".into(),
        format!("    WallPaper    REG_SZ    {}\n", picture.display()),
    );
    assert_eq!(fake.locate(&root, true).unwrap(), picture);
    // Not a Windows question on Linux, and the override beats every desktop.
    let override_file = root.join("o.png");
    png(&override_file, [1, 2, 3], (2, 2));
    fake.vars
        .insert("STATUS_QUO_WALLPAPER", override_file.display().to_string());
    assert_eq!(fake.locate(&root, true).unwrap(), override_file);
    assert_eq!(fake.locate(&root, false).unwrap(), override_file);
    fake.vars.insert(
        "STATUS_QUO_WALLPAPER",
        root.join("missing.png").display().to_string(),
    );
    assert!(fake
        .locate(&root, false)
        .unwrap_err()
        .contains("STATUS_QUO_WALLPAPER"));
}

// REQ SQ-37
#[test]
fn nothing_found_says_where_it_looked() {
    let root = temp("none");
    let error = Fake::new().locate(&root, false).unwrap_err();
    assert!(
        error.contains("no wallpaper found") && error.contains("KDE Plasma"),
        "{error}"
    );
}

// REQ SQ-37
#[test]
fn pictures_are_decoded_to_a_hue_and_bad_files_are_refused_with_a_reason() {
    let root = temp("decode");
    let blue = root.join("blue.png");
    png(&blue, [20, 60, 220], (300, 200));
    let seed = load_seed(&blue).unwrap();
    assert!((seed.hue - 226.0).abs() < 8.0, "hue {}", seed.hue);
    let grey = root.join("grey");
    png(&grey, [120, 120, 120], (40, 40)); // no extension, like Windows' TranscodedWallpaper
    assert_eq!(load_seed(&grey).unwrap().chroma, 0.0);
    let jpg = root.join("red.jpg");
    image::RgbImage::from_pixel(64, 64, image::Rgb([220, 30, 30]))
        .save_with_format(&jpg, image::ImageFormat::Jpeg)
        .unwrap();
    let hue = load_seed(&jpg).unwrap().hue;
    assert!(!(12.0..=348.0).contains(&hue), "hue {hue}");
    let junk = root.join("junk.jpg");
    fs::write(&junk, b"not a picture").unwrap();
    assert!(load_seed(&junk).unwrap_err().contains("cannot be read"));
    assert!(load_seed(&root.join("missing.png")).is_err());
}
