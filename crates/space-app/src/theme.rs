//! Visual language: deep charcoal + amber accent, no purple glassmorphism.

use std::sync::Arc;

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke, Visuals};

pub const BG: Color32 = Color32::from_rgb(14, 16, 18);
pub const PANEL: Color32 = Color32::from_rgb(22, 25, 28);
pub const PANEL_2: Color32 = Color32::from_rgb(28, 32, 36);
pub const BORDER: Color32 = Color32::from_rgb(48, 54, 60);
pub const TEXT: Color32 = Color32::from_rgb(232, 230, 224);
pub const MUTED: Color32 = Color32::from_rgb(140, 146, 152);
pub const ACCENT: Color32 = Color32::from_rgb(232, 168, 72);
pub const DIR: Color32 = Color32::from_rgb(52, 58, 66);

pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);

    let mut visuals = Visuals::dark();
    visuals.window_fill = BG;
    visuals.panel_fill = PANEL;
    visuals.extreme_bg_color = PANEL_2;
    visuals.override_text_color = Some(TEXT);
    visuals.widgets.noninteractive.bg_fill = PANEL;
    visuals.widgets.inactive.bg_fill = PANEL_2;
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(36, 40, 46);
    visuals.widgets.active.bg_fill = Color32::from_rgb(42, 48, 54);
    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(232, 168, 72, 60);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, MUTED);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.menu_corner_radius = CornerRadius::same(6);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    ctx.set_style(style);
}

/// System fonts so Cyrillic / symbols render — keep small to avoid atlas/OOM kills.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    // Arial (~0.7MB) covers Cyrillic without loading SFNS (7.5MB) which can jetsam the process.
    let path = "/System/Library/Fonts/Supplemental/Arial.ttf";
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    fonts
        .font_data
        .insert("arial".to_owned(), Arc::new(FontData::from_owned(bytes)));

    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        if let Some(list) = fonts.families.get_mut(&family) {
            list.insert(0, "arial".to_owned());
        }
    }

    ctx.set_fonts(fonts);
}

/// Well-spaced folder hues (no two neighbors in this list are close).
const DIR_HUES: &[f32] = &[
    18.0, 48.0, 78.0, 112.0, 148.0, 178.0, 205.0, 232.0, 262.0, 310.0, 338.0, 8.0,
];

/// Minimum squared RGB distance so adjacent wedges read as different.
const MIN_COLOR_DIST2: i32 = 48 * 48;

/// Files by extension; folders get stable distinct hues (not flat gray).
pub fn color_for(name: &str, is_dir: bool) -> Color32 {
    if is_dir {
        return color_for_dir(name);
    }
    color_for_ext(extension_of(name))
}

/// Color for a sibling among neighbors — never matches `avoid` (parent / prev / wrap).
pub fn color_for_adjacent(
    name: &str,
    is_dir: bool,
    sibling_index: usize,
    avoid: &[Color32],
) -> Color32 {
    if is_dir {
        return pick_dir_color(name, sibling_index, avoid);
    }
    let base = color_for_ext(extension_of(name));
    if avoid.iter().all(|c| color_dist2(base, *c) >= MIN_COLOR_DIST2) {
        return base;
    }
    // Same extension as neighbor — nudge hue so wedges still separate.
    pick_dir_color(name, sibling_index.wrapping_add(3), avoid)
}

fn color_for_dir(name: &str) -> Color32 {
    pick_dir_color(name, 0, &[])
}

fn pick_dir_color(name: &str, sibling_index: usize, avoid: &[Color32]) -> Color32 {
    let preferred = (fnv1a(name) as usize).wrapping_add(sibling_index.wrapping_mul(3));
    let n = DIR_HUES.len();
    for offset in 0..n {
        let slot = (preferred + offset) % n;
        // Alternate value slightly so even distant palette reuse stays readable.
        let v = 0.50 + ((sibling_index + offset) % 4) as f32 * 0.035;
        let s = 0.40 + ((sibling_index + slot) % 3) as f32 * 0.04;
        let c = hsv(DIR_HUES[slot], s, v);
        if avoid.iter().all(|a| color_dist2(c, *a) >= MIN_COLOR_DIST2) {
            return c;
        }
    }
    // Last resort: force a hue halfway from the strongest conflict.
    let conflict = avoid.last().copied().unwrap_or(DIR);
    let h = (fnv1a(name) % 360) as f32;
    let forced = hsv((h + 140.0 + sibling_index as f32 * 37.0) % 360.0, 0.48, 0.58);
    if color_dist2(forced, conflict) >= MIN_COLOR_DIST2 / 2 {
        forced
    } else {
        hsv((h + 200.0) % 360.0, 0.50, 0.60)
    }
}

fn fnv1a(name: &str) -> u32 {
    let mut h: u32 = 2166136261;
    for b in name.as_bytes() {
        h ^= u32::from(*b);
        h = h.wrapping_mul(16777619);
    }
    h
}

fn color_dist2(a: Color32, b: Color32) -> i32 {
    let dr = a.r() as i32 - b.r() as i32;
    let dg = a.g() as i32 - b.g() as i32;
    let db = a.b() as i32 - b.b() as i32;
    dr * dr + dg * dg + db * db
}

pub fn extension_of(name: &str) -> &str {
    let base = name.rsplit('/').next().unwrap_or(name);
    match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() && ext.len() <= 8 => ext,
        _ => "(none)",
    }
}

pub fn color_for_ext(ext: &str) -> Color32 {
    let key = ext.to_ascii_lowercase();
    match key.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "svg" | "bmp" | "tiff" => {
            Color32::from_rgb(72, 160, 210)
        }
        "mp4" | "mov" | "mkv" | "avi" | "webm" | "m4v" => Color32::from_rgb(210, 90, 110),
        "mp3" | "wav" | "flac" | "aac" | "m4a" | "ogg" => Color32::from_rgb(180, 120, 220),
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "dmg" | "iso" => {
            Color32::from_rgb(200, 140, 60)
        }
        "rs" | "go" | "c" | "h" | "cpp" | "hpp" | "py" | "js" | "ts" | "tsx" | "jsx" | "swift"
        | "kt" | "java" => Color32::from_rgb(90, 180, 130),
        "pdf" | "doc" | "docx" | "txt" | "md" | "rtf" | "pages" => Color32::from_rgb(220, 180, 80),
        "app" | "exe" | "dll" | "so" | "dylib" | "bin" => Color32::from_rgb(100, 140, 190),
        "json" | "xml" | "yml" | "yaml" | "toml" | "csv" | "sql" => Color32::from_rgb(140, 170, 100),
        "ttf" | "otf" | "woff" | "woff2" => Color32::from_rgb(160, 130, 100),
        "(none)" => Color32::from_rgb(110, 115, 120),
        _ => {
            let mut h: u32 = 2166136261;
            for b in key.as_bytes() {
                h ^= u32::from(*b);
                h = h.wrapping_mul(16777619);
            }
            hsv((h % 360) as f32, 0.48, 0.68)
        }
    }
}

/// Stable palette entries for a legend (ext, color).
pub fn legend_for_names<'a>(names: impl Iterator<Item = (&'a str, bool)>) -> Vec<(String, Color32)> {
    let mut seen = std::collections::BTreeMap::<String, Color32>::new();
    for (name, is_dir) in names {
        if is_dir {
            continue;
        }
        let ext = extension_of(name).to_ascii_lowercase();
        seen.entry(ext.clone())
            .or_insert_with(|| color_for_ext(&ext));
    }
    seen.into_iter().take(12).collect()
}

fn hsv(h: f32, s: f32, v: f32) -> Color32 {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    Color32::from_rgb(
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}
