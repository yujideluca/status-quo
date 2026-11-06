//! Colours taken from the wallpaper. Pure: the host samples the picture and hands over pixels.
use crate::Palette;

/// The one hue a wallpaper contributes. `chroma` is 0 for a greyscale picture: then there is no
/// hue and the palette stays neutral instead of inventing one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WallpaperSeed {
    /// Degrees, 0 up to 360.
    pub hue: f32,
    /// 0 for no usable hue, up to 1 for a fully saturated one.
    pub chroma: f32,
}

/// What the host knows about the wallpaper right now.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum WallpaperColors {
    /// Not looked at yet.
    #[default]
    Pending,
    Ready(WallpaperSeed),
    /// The wallpaper could not be found or read; the text says why.
    Unavailable(String),
}

const BINS: usize = 36;

/// Dominant hue of the pixels, weighting each by how colourful and how far from black or white
/// it is. `None` when there are no pixels. Greyscale input gives `chroma == 0`.
pub fn seed_from_pixels(pixels: &[[u8; 3]]) -> Option<WallpaperSeed> {
    if pixels.is_empty() {
        return None;
    }
    // Unit-vector sums per hue bin keep the average hue correct across the 0/360 seam.
    let mut weight = [0.0f64; BINS];
    let mut sat = [0.0f64; BINS];
    let mut x = [0.0f64; BINS];
    let mut y = [0.0f64; BINS];
    for &rgb in pixels {
        let (h, s, l) = to_hsl(rgb);
        let w = f64::from(s) * f64::from(1.0 - (2.0 * l - 1.0).abs());
        if w <= 0.0 {
            continue;
        }
        let bin = ((h / 360.0 * BINS as f32) as usize).min(BINS - 1);
        weight[bin] += w;
        sat[bin] += w * f64::from(s);
        x[bin] += w * f64::from(h.to_radians().cos());
        y[bin] += w * f64::from(h.to_radians().sin());
    }
    let total: f64 = weight.iter().sum();
    // Almost no colourful pixels (about 2 % of them at a medium colour): a greyscale picture.
    if total < pixels.len() as f64 * 0.01 {
        return Some(WallpaperSeed {
            hue: 0.0,
            chroma: 0.0,
        });
    }
    let best = (0..BINS)
        .max_by(|&a, &b| weight[a].total_cmp(&weight[b]))
        .unwrap_or(0);
    let hue = (y[best].atan2(x[best]).to_degrees() as f32).rem_euclid(360.0);
    let chroma = (sat[best] / weight[best]) as f32;
    Some(WallpaperSeed {
        hue,
        chroma: chroma.clamp(0.0, 1.0),
    })
}

impl Palette {
    /// A palette for the dark or light look built from the wallpaper seed. Every foreground
    /// colour is moved until it reads on the background (WCAG contrast).
    pub fn from_wallpaper(seed: &WallpaperSeed, dark: bool) -> Palette {
        let c = seed.chroma.clamp(0.0, 1.0);
        let h = seed.hue;
        let accent_sat = if c == 0.0 { 0.0 } else { c.clamp(0.45, 0.9) };
        let (background, text, accent, muted, warning, error) = if dark {
            (
                from_hsl(h, 0.30 * c, 0.09),
                from_hsl(h, 0.20 * c, 0.90),
                from_hsl(h, accent_sat, 0.68),
                from_hsl(h, 0.14 * c, 0.65),
                from_hsl(42.0, 0.70, 0.66),
                from_hsl(0.0, 0.70, 0.68),
            )
        } else {
            (
                from_hsl(h, 0.35 * c, 0.95),
                from_hsl(h, 0.30 * c, 0.12),
                from_hsl(h, accent_sat, 0.32),
                from_hsl(h, 0.15 * c, 0.38),
                from_hsl(38.0, 0.80, 0.30),
                from_hsl(0.0, 0.65, 0.38),
            )
        };
        let fit = |rgb, min| readable(rgb, background, min, dark);
        Palette {
            background,
            text: fit(text, 7.0),
            accent: fit(accent, 4.5),
            muted: fit(muted, 4.5),
            warning: fit(warning, 4.5),
            error: fit(error, 4.5),
        }
    }
}

/// WCAG contrast ratio of two colours, 1 up to 21.
pub fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}
fn luminance(rgb: [u8; 3]) -> f32 {
    let lin = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2])
}
/// Move the lightness of `rgb` away from the background until the contrast reaches `min`.
fn readable(rgb: [u8; 3], background: [u8; 3], min: f32, dark: bool) -> [u8; 3] {
    let (h, s, mut l) = to_hsl(rgb);
    let mut out = rgb;
    while contrast(out, background) < min && (0.0..=1.0).contains(&l) {
        l += if dark { 0.01 } else { -0.01 };
        out = from_hsl(h, s, l.clamp(0.0, 1.0));
        if l <= 0.0 || l >= 1.0 {
            break;
        }
    }
    out
}
fn to_hsl(rgb: [u8; 3]) -> (f32, f32, f32) {
    let [r, g, b] = rgb.map(|v| f32::from(v) / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s.clamp(0.0, 1.0), l)
}
fn from_hsl(h: f32, s: f32, l: f32) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r, g, b].map(|v| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u8)
}
