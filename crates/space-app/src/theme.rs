//! Visual language: deep charcoal + amber accent — map-first, quiet chrome.

use std::sync::Arc;

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Margin, Stroke, Visuals};

pub const BG: Color32 = Color32::from_rgb(14, 15, 17);
pub const PANEL: Color32 = Color32::from_rgb(21, 23, 26);
pub const PANEL_2: Color32 = Color32::from_rgb(28, 31, 35);
pub const BORDER: Color32 = Color32::from_rgb(42, 46, 52);
pub const TEXT: Color32 = Color32::from_rgb(236, 233, 226);
/// Secondary text — lifted to keep 4.5:1 on PANEL.
pub const MUTED: Color32 = Color32::from_rgb(154, 160, 168);
pub const ACCENT: Color32 = Color32::from_rgb(240, 173, 78);
/// Text on ACCENT fills.
pub const ACCENT_INK: Color32 = Color32::from_rgb(26, 18, 7);
/// "Safe to remove" signal.
pub const SAFE: Color32 = Color32::from_rgb(92, 200, 168);
pub const DIR: Color32 = Color32::from_rgb(48, 52, 58);
pub const DANGER: Color32 = Color32::from_rgb(210, 90, 80);

pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);

    let mut visuals = Visuals::dark();
    visuals.window_fill = PANEL;
    visuals.panel_fill = BG;
    visuals.extreme_bg_color = PANEL_2;
    visuals.override_text_color = Some(TEXT);
    visuals.widgets.noninteractive.bg_fill = PANEL;
    visuals.widgets.inactive.bg_fill = PANEL_2;
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(32, 36, 42);
    visuals.widgets.active.bg_fill = Color32::from_rgb(38, 42, 48);
    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(240, 173, 78, 48);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, MUTED);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.inactive.weak_bg_fill = PANEL_2;
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(32, 36, 42);
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.menu_corner_radius = CornerRadius::same(8);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(6);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(8);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(8);
    visuals.widgets.active.corner_radius = CornerRadius::same(8);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.window_margin = Margin::same(14);
    style.spacing.indent = 12.0;
    style.interaction.show_tooltips_only_when_still = false;
    ctx.set_style(style);
}

/// Geist (bundled, OFL) for UI text; system Arial behind it covers Cyrillic / symbols.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "geist".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/Geist-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "geist-mono".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/GeistMono-Regular.ttf"
        ))),
    );

    // Arial (~0.7MB) covers Cyrillic without loading SFNS (7.5MB) which can jetsam the process.
    let fallback = std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf").ok();
    if let Some(bytes) = fallback {
        fonts
            .font_data
            .insert("arial".to_owned(), Arc::new(FontData::from_owned(bytes)));
    }

    for (family, primary) in [
        (FontFamily::Proportional, "geist"),
        (FontFamily::Monospace, "geist-mono"),
    ] {
        if let Some(list) = fonts.families.get_mut(&family) {
            list.insert(0, primary.to_owned());
            if fonts.font_data.contains_key("arial") {
                list.insert(1, "arial".to_owned());
            }
        }
    }

    ctx.set_fonts(fonts);
}

/// Well-spaced folder hues — brighter, clearer (charcoal map + amber family).
const DIR_HUES: &[f32] = &[
    35.0,  // amber (accent family) — the largest sibling
    215.0, // blue
    162.0, // teal
    266.0, // violet
    9.0,   // coral
    48.0,  // gold
    195.0, // sky
    300.0, // magenta
    140.0, // green
    235.0, // indigo
    20.0,  // orange
    180.0, // cyan
];

/// Minimum squared RGB distance so adjacent wedges read as different.
const MIN_COLOR_DIST2: i32 = 55 * 55;

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
        return pick_ranked_color(name, sibling_index, avoid);
    }
    let base = color_for_ext(extension_of(name));
    if avoid
        .iter()
        .all(|c| color_dist2(base, *c) >= MIN_COLOR_DIST2)
    {
        return base;
    }
    // Same extension as neighbor — nudge hue so wedges still separate.
    pick_ranked_color(name, sibling_index.wrapping_add(3), avoid)
}

/// Child ring: related to parent, but siblings still clearly distinct.
pub fn color_child_of(parent: Color32, sibling_index: usize, avoid: &[Color32]) -> Color32 {
    let (h, s0, v0) = rgb_to_hsv(parent);
    // Step around the wheel so neighbors don't melt together.
    let step = 38.0;
    for offset in 0..10 {
        let i = sibling_index + offset;
        let h2 = (h + (i as f32) * step - step * 0.35).rem_euclid(360.0);
        let s = (s0 * 0.75 + 0.18 + (i % 3) as f32 * 0.06).clamp(0.40, 0.78);
        let v = (v0 * 0.85 + 0.12 + (i % 4) as f32 * 0.04).clamp(0.45, 0.80);
        let c = hsv(h2, s, v);
        if avoid.iter().all(|a| color_dist2(c, *a) >= MIN_COLOR_DIST2) {
            return c;
        }
    }
    hsv(
        (h + 95.0 + sibling_index as f32 * 47.0).rem_euclid(360.0),
        0.58,
        0.66,
    )
}

/// Colors for one ring of siblings in size order — shared by the map and the contents list.
pub fn sibling_palette(items: &[(&str, bool)]) -> Vec<Color32> {
    let mut out: Vec<Color32> = Vec::with_capacity(items.len());
    for (i, &(name, is_dir)) in items.iter().enumerate() {
        let mut avoid: Vec<Color32> = Vec::with_capacity(3);
        if let Some(&prev) = out.last() {
            avoid.push(prev);
        }
        if is_dir {
            avoid.extend(out.iter().copied().take(2));
        }
        out.push(color_for_adjacent(name, is_dir, i, &avoid));
    }
    if out.len() >= 2 {
        let first = out[0];
        let prev = out[out.len() - 2];
        let last_i = out.len() - 1;
        let (name, is_dir) = items[last_i];
        out[last_i] = color_for_adjacent(name, is_dir, last_i, &[prev, first]);
    }
    out
}

/// Hue by rank among siblings, bright enough to read on the charcoal map.
fn pick_ranked_color(name: &str, sibling_index: usize, avoid: &[Color32]) -> Color32 {
    let n = DIR_HUES.len();
    for offset in 0..n {
        let slot = (sibling_index + offset) % n;
        let v = 0.90 - ((sibling_index / n + offset) % 3) as f32 * 0.06;
        let s = 0.58 + (slot % 3) as f32 * 0.04;
        let c = hsv(DIR_HUES[slot], s, v);
        if avoid.iter().all(|a| color_dist2(c, *a) >= MIN_COLOR_DIST2) {
            return c;
        }
    }
    pick_dir_color(name, sibling_index, avoid)
}

fn color_for_dir(name: &str) -> Color32 {
    pick_dir_color(name, 0, &[])
}

fn pick_dir_color(name: &str, sibling_index: usize, avoid: &[Color32]) -> Color32 {
    let preferred = (fnv1a(name) as usize).wrapping_add(sibling_index.wrapping_mul(3));
    let n = DIR_HUES.len();
    for offset in 0..n {
        let slot = (preferred + offset) % n;
        let v = 0.58 + ((sibling_index + offset) % 4) as f32 * 0.04;
        let s = 0.52 + ((sibling_index + slot) % 3) as f32 * 0.05;
        let c = hsv(DIR_HUES[slot], s.clamp(0.45, 0.72), v.clamp(0.48, 0.78));
        if avoid.iter().all(|a| color_dist2(c, *a) >= MIN_COLOR_DIST2) {
            return c;
        }
    }
    let conflict = avoid.last().copied().unwrap_or(DIR);
    let h = (fnv1a(name) % 360) as f32;
    let forced = hsv(
        (h + 140.0 + sibling_index as f32 * 37.0) % 360.0,
        0.58,
        0.65,
    );
    if color_dist2(forced, conflict) >= MIN_COLOR_DIST2 / 2 {
        forced
    } else {
        hsv((h + 200.0) % 360.0, 0.60, 0.68)
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
        "json" | "xml" | "yml" | "yaml" | "toml" | "csv" | "sql" => {
            Color32::from_rgb(140, 170, 100)
        }
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
pub fn legend_for_names<'a>(
    names: impl Iterator<Item = (&'a str, bool)>,
) -> Vec<(String, Color32)> {
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

fn rgb_to_hsv(c: Color32) -> (f32, f32, f32) {
    let r = c.r() as f32 / 255.0;
    let g = c.g() as f32 / 255.0;
    let b = c.b() as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let v = max;
    let s = if max <= 1e-6 { 0.0 } else { d / max };
    let h = if d <= 1e-6 {
        0.0
    } else if (max - r).abs() < 1e-6 {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if (max - g).abs() < 1e-6 {
        60.0 * (((b - r) / d) + 2.0)
    } else {
        60.0 * (((r - g) / d) + 4.0)
    };
    (h.rem_euclid(360.0), s, v)
}
