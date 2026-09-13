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

/// System fonts so Cyrillic / CJK / symbols (·) render instead of tofu.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    let candidates: &[(&str, &str)] = &[
        ("sf_pro", "/System/Library/Fonts/SFNS.ttf"),
        ("arial", "/System/Library/Fonts/Supplemental/Arial.ttf"),
    ];

    let mut loaded: Vec<String> = Vec::new();
    for (name, path) in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert((*name).to_owned(), Arc::new(FontData::from_owned(bytes)));
            loaded.push((*name).to_owned());
        }
    }

    if loaded.is_empty() {
        return;
    }

    // Prefer SF Pro, then Arial as glyph fallbacks (Cyrillic, ·, etc.).
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        if let Some(list) = fonts.families.get_mut(&family) {
            for name in loaded.iter().rev() {
                list.insert(0, name.clone());
            }
        }
    }

    ctx.set_fonts(fonts);
}

/// Files by extension; folders get stable distinct hues (not flat gray).
pub fn color_for(name: &str, is_dir: bool) -> Color32 {
    if is_dir {
        return color_for_dir(name);
    }
    color_for_ext(extension_of(name))
}

fn color_for_dir(name: &str) -> Color32 {
    let mut h: u32 = 2166136261;
    for b in name.as_bytes() {
        h ^= u32::from(*b);
        h = h.wrapping_mul(16777619);
    }
    // Distinct but calm — readable on charcoal.
    hsv((h % 360) as f32, 0.38, 0.55)
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
