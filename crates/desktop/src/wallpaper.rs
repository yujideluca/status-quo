//! Keeps the tray informed about the wallpaper colours (SQ-37). Looks only while some theme asks
//! to follow the wallpaper, and decodes the picture again only when its file or path changed.
use sq_core::WallpaperColors;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

const RECHECK: Duration = Duration::from_secs(30);

pub fn spawn(
    wanted: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    publish: impl Fn(WallpaperColors) + Send + 'static,
) {
    thread::spawn(move || {
        let mut shown = WallpaperColors::Pending;
        let mut last: Option<(PathBuf, Option<SystemTime>)> = None;
        let mut checked: Option<Instant> = None;
        let mut was_wanted = false;
        while !stop.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(500));
            let wanted_now = wanted.load(Ordering::Acquire);
            let due = wanted_now && (!was_wanted || checked.is_none_or(|t| t.elapsed() >= RECHECK));
            was_wanted = wanted_now;
            if !due {
                continue;
            }
            checked = Some(Instant::now());
            let colors = match sq_sources::desktop() {
                Ok(sq_sources::Desktop::Picture(path)) => {
                    let signature = (
                        path.clone(),
                        std::fs::metadata(&path).and_then(|m| m.modified()).ok(),
                    );
                    if last.as_ref() == Some(&signature) && shown != WallpaperColors::Pending {
                        continue;
                    }
                    let colors = match sq_sources::load_seed(&path) {
                        Ok(seed) => WallpaperColors::Ready(seed),
                        Err(error) => WallpaperColors::Unavailable(error),
                    };
                    last = Some(signature);
                    colors
                }
                // No picture, one colour (Windows): that colour is the whole wallpaper.
                Ok(sq_sources::Desktop::Colour(colour)) => {
                    last = None;
                    match sq_core::seed_from_pixels(&[colour]) {
                        Some(seed) => WallpaperColors::Ready(seed),
                        None => WallpaperColors::Unavailable("the desktop has no colour".into()),
                    }
                }
                Err(error) => {
                    last = None;
                    WallpaperColors::Unavailable(error)
                }
            };
            if colors != shown {
                shown = colors.clone();
                publish(colors);
            }
        }
    });
}
