use sq_core::*;

fn hue_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

// REQ SQ-37
#[test]
fn seed_takes_the_dominant_colourful_hue_and_ignores_black_white_and_grey() {
    let mut pixels = vec![[0, 0, 0]; 500];
    pixels.extend(vec![[255, 255, 255]; 300]);
    pixels.extend(vec![[128, 128, 128]; 200]);
    pixels.extend(vec![[30, 70, 200]; 120]); // blue
    pixels.extend(vec![[200, 40, 40]; 40]); // a little red
    let seed = seed_from_pixels(&pixels).unwrap();
    assert!(hue_distance(seed.hue, 223.0) < 10.0, "hue {}", seed.hue);
    assert!(seed.chroma > 0.5);
}

// REQ SQ-37
#[test]
fn red_hue_across_the_zero_seam_stays_red() {
    let pixels: Vec<[u8; 3]> = (0..400)
        .map(|i| {
            if i % 2 == 0 {
                [220, 30, 50]
            } else {
                [220, 50, 30]
            }
        })
        .collect();
    let seed = seed_from_pixels(&pixels).unwrap();
    assert!(hue_distance(seed.hue, 0.0) < 15.0, "hue {}", seed.hue);
}

// REQ SQ-37
#[test]
fn greyscale_or_empty_input_never_invents_a_hue() {
    assert_eq!(seed_from_pixels(&[]), None);
    let grey: Vec<[u8; 3]> = (0..=255).map(|v| [v, v, v]).collect();
    assert_eq!(seed_from_pixels(&grey).unwrap().chroma, 0.0);
    let neutral = Palette::from_wallpaper(&seed_from_pixels(&grey).unwrap(), true);
    for c in [neutral.background, neutral.text, neutral.muted] {
        assert!(
            c[0].abs_diff(c[1]) <= 1 && c[1].abs_diff(c[2]) <= 1,
            "{c:?} is tinted"
        );
    }
}

// REQ SQ-37
#[test]
fn every_hue_gives_a_readable_palette_in_both_looks() {
    for hue in (0..360).step_by(3) {
        for chroma in [0.0, 0.3, 0.7, 1.0] {
            for dark in [true, false] {
                let seed = WallpaperSeed {
                    hue: hue as f32,
                    chroma,
                };
                let p = Palette::from_wallpaper(&seed, dark);
                let bg = p.background;
                let luma = |c: [u8; 3]| contrast(c, [0, 0, 0]);
                assert_eq!(luma(bg) < 4.0, dark, "background of hue {hue} dark={dark}");
                assert!(contrast(p.text, bg) >= 7.0, "text {hue} {chroma} {dark}");
                for (name, c) in [
                    ("accent", p.accent),
                    ("muted", p.muted),
                    ("warning", p.warning),
                    ("error", p.error),
                ] {
                    assert!(contrast(c, bg) >= 4.5, "{name} {hue} {chroma} {dark}");
                }
            }
        }
    }
}

// REQ SQ-37
#[test]
fn accent_carries_the_wallpaper_hue_and_palette_is_deterministic() {
    let seed = WallpaperSeed {
        hue: 120.0,
        chroma: 0.8,
    };
    let a = Palette::from_wallpaper(&seed, true);
    assert_eq!(a, Palette::from_wallpaper(&seed, true));
    assert_ne!(a, Palette::from_wallpaper(&seed, false));
    assert!(
        a.accent[1] > a.accent[0] && a.accent[1] > a.accent[2],
        "{:?}",
        a.accent
    );
}

// REQ SQ-37 SQ-12
#[test]
fn follow_wallpaper_is_off_by_default_round_trips_and_old_files_still_load() {
    assert!(!Theme::default().follow_wallpaper);
    let theme = Theme {
        follow_wallpaper: true,
        ..Theme::default()
    };
    let text = serde_json::to_string(&theme).unwrap();
    assert_eq!(serde_json::from_str::<Theme>(&text).unwrap(), theme);
    let old = r#"{"style":"terminal","opacity":0.5}"#;
    assert!(!serde_json::from_str::<Theme>(old).unwrap().follow_wallpaper);
    let mut config = Config::default();
    config.theme.follow_wallpaper = true;
    config.widgets[0].theme = Some(theme);
    config.validate().unwrap();
}
