//! Visual language: graphite or paper ground, one amber accent, quiet chrome.
//!
//! Appearance (dark or light) only changes colours. The Disk usage layout is a
//! separate setting (`settings::WorkspaceLayout`), so every layout works in both.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Margin, Stroke, Visuals};

/// Colour scheme of the whole interface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Appearance {
    /// Graphite ground, amber accent.
    #[default]
    Dark,
    /// Paper ground, darker amber accent for contrast on white.
    Light,
}

impl Appearance {
    pub const ALL: [Appearance; 2] = [Appearance::Dark, Appearance::Light];

    pub fn label(self) -> &'static str {
        match self {
            Appearance::Dark => "Dark",
            Appearance::Light => "Light",
        }
    }

    fn palette(self) -> &'static Palette {
        match self {
            Appearance::Dark => &DARK,
            Appearance::Light => &LIGHT,
        }
    }
}

struct Palette {
    dark: bool,
    bg: Color32,
    panel: Color32,
    panel_2: Color32,
    border: Color32,
    text: Color32,
    muted: Color32,
    accent: Color32,
    accent_ink: Color32,
    safe: Color32,
    dir: Color32,
    danger: Color32,
}

const DARK: Palette = Palette {
    dark: true,
    bg: Color32::from_rgb(0x12, 0x14, 0x18),
    panel: Color32::from_rgb(0x18, 0x1B, 0x20),
    panel_2: Color32::from_rgb(0x22, 0x26, 0x2D),
    border: Color32::from_rgb(0x2A, 0x2F, 0x37),
    text: Color32::from_rgb(0xEC, 0xEF, 0xF4),
    // 4.5:1 on the panel colour.
    muted: Color32::from_rgb(0x9C, 0xA3, 0xAD),
    accent: Color32::from_rgb(0xF0, 0xAD, 0x4E),
    accent_ink: Color32::from_rgb(0x1B, 0x13, 0x07),
    safe: Color32::from_rgb(0x5C, 0xC8, 0xA8),
    dir: Color32::from_rgb(0x30, 0x34, 0x3A),
    danger: Color32::from_rgb(0xE8, 0x70, 0x5F),
};

const LIGHT: Palette = Palette {
    dark: false,
    bg: Color32::from_rgb(0xF6, 0xF5, 0xF1),
    panel: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    panel_2: Color32::from_rgb(0xEF, 0xED, 0xE8),
    border: Color32::from_rgb(0xE1, 0xDD, 0xD5),
    text: Color32::from_rgb(0x1E, 0x1C, 0x19),
    muted: Color32::from_rgb(0x67, 0x63, 0x5B),
    // Darker than the dark-mode amber so white text on it keeps 4.5:1.
    accent: Color32::from_rgb(0xA8, 0x5A, 0x0B),
    accent_ink: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    safe: Color32::from_rgb(0x1F, 0x74, 0x58),
    dir: Color32::from_rgb(0xC9, 0xC4, 0xBA),
    danger: Color32::from_rgb(0xB3, 0x32, 0x1F),
};

static APPEARANCE: AtomicU8 = AtomicU8::new(0);

pub fn appearance() -> Appearance {
    Appearance::ALL[APPEARANCE.load(Ordering::Relaxed) as usize % Appearance::ALL.len()]
}

fn pal() -> &'static Palette {
    appearance().palette()
}

/// True for the dark appearance; map colours use it to keep contrast on each ground.
pub fn is_dark() -> bool {
    pal().dark
}

// Palette accessors keep the old constant names so call sites read the same.
#[allow(non_snake_case)]
pub fn BG() -> Color32 {
    pal().bg
}
#[allow(non_snake_case)]
pub fn PANEL() -> Color32 {
    pal().panel
}
/// Raised surfaces: hover fills, stat tiles, the selected nav item.
#[allow(non_snake_case)]
pub fn PANEL_2() -> Color32 {
    pal().panel_2
}
#[allow(non_snake_case)]
pub fn BORDER() -> Color32 {
    pal().border
}
#[allow(non_snake_case)]
pub fn TEXT() -> Color32 {
    pal().text
}
/// Secondary text.
#[allow(non_snake_case)]
pub fn MUTED() -> Color32 {
    pal().muted
}
#[allow(non_snake_case)]
pub fn ACCENT() -> Color32 {
    pal().accent
}
/// Text on ACCENT() fills.
#[allow(non_snake_case)]
pub fn ACCENT_INK() -> Color32 {
    pal().accent_ink
}
/// "Safe to remove" signal.
#[allow(non_snake_case)]
pub fn SAFE() -> Color32 {
    pal().safe
}
#[allow(non_snake_case)]
pub fn DIR() -> Color32 {
    pal().dir
}
#[allow(non_snake_case)]
pub fn DANGER() -> Color32 {
    pal().danger
}
/// Dims everything behind a modal dialog or a bottom sheet.
#[allow(non_snake_case)]
pub fn SCRIM() -> Color32 {
    if pal().dark {
        Color32::from_black_alpha(168)
    } else {
        Color32::from_black_alpha(96)
    }
}

/// Type scale (px). Captions are the smallest text in the app.
pub mod type_scale {
    pub const CAPTION: f32 = 12.0;
    pub const MONO: f32 = 13.0;
    pub const BODY: f32 = 14.0;
    pub const LEAD: f32 = 16.0;
    pub const TITLE: f32 = 18.0;
    pub const PAGE: f32 = 22.0;
    pub const DISPLAY: f32 = 28.0;
}

/// Spacing and shape scale (px).
pub mod metrics {
    pub const S2: f32 = 8.0;
    pub const S3: f32 = 12.0;
    pub const S4: f32 = 16.0;
    pub const S6: f32 = 24.0;
    /// Workspace toolbar (location, search, layout).
    pub const APP_BAR_H: f32 = 56.0;
    /// Filter chips under the toolbar.
    pub const FILTER_BAR_H: f32 = 44.0;
    pub const CONTROL_H: f32 = 32.0;
    pub const CHIP_H: f32 = 28.0;
    pub const RADIUS_CONTROL: u8 = 6;
    pub const RADIUS_CARD: u8 = 10;
    pub const RADIUS_DIALOG: u8 = 14;
    pub const SIDEBAR_W: f32 = 224.0;
    pub const RAIL_W: f32 = 60.0;
    pub const SIDE_W: f32 = 288.0;
    pub const TREE_W: f32 = 360.0;
    pub const TREE_ROW_H: f32 = 32.0;
    pub const TREE_INDENT: f32 = 16.0;
}

/// Medium weight, for labels that need a little more presence than body text.
pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}

/// Semibold weight, for titles, numbers that matter and the current location.
pub fn bold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Install fonts and the starting appearance. Call once.
pub fn apply(ctx: &egui::Context, appearance: Appearance, reduce_motion: bool, tooltip_delay_ms: u64) {
    install_fonts(ctx);
    base_style(ctx);
    set_appearance(ctx, appearance);
    apply_interaction(ctx, reduce_motion, tooltip_delay_ms);
}

/// Motion and hover-hint timing from the settings; switched live.
pub fn apply_interaction(ctx: &egui::Context, reduce_motion: bool, tooltip_delay_ms: u64) {
    crate::anim::set_reduce_motion(reduce_motion);
    let mut style = (*ctx.style()).clone();
    style.animation_time = if reduce_motion { 0.0 } else { 0.1 };
    style.interaction.tooltip_delay = tooltip_delay_ms as f32 / 1000.0;
    style.interaction.tooltip_grace_time = if tooltip_delay_ms == 0 { 0.0 } else { 0.2 };
    ctx.set_style(style);
}

/// Switch the palette at runtime; every frame reads it, so nothing restarts.
pub fn set_appearance(ctx: &egui::Context, appearance: Appearance) {
    APPEARANCE.store(
        Appearance::ALL
            .iter()
            .position(|&a| a == appearance)
            .unwrap_or(0) as u8,
        Ordering::Relaxed,
    );
    let p = pal();
    let hover = p.panel_2.lerp_to_gamma(p.border, 0.5);
    let active = p.border;
    let control_stroke = p.border.lerp_to_gamma(p.muted, 0.35);
    let mut visuals = if p.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    visuals.window_fill = if p.dark {
        Color32::from_rgb(0x1C, 0x1F, 0x25)
    } else {
        PANEL()
    };
    visuals.panel_fill = BG();
    visuals.extreme_bg_color = if p.dark { PANEL() } else { BG() };
    visuals.faint_bg_color = PANEL_2();
    visuals.override_text_color = Some(TEXT());
    visuals.hyperlink_color = ACCENT();
    visuals.widgets.noninteractive.bg_fill = PANEL();
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER());
    visuals.widgets.inactive.bg_fill = PANEL_2();
    visuals.widgets.hovered.bg_fill = hover;
    visuals.widgets.active.bg_fill = active;
    visuals.selection.bg_fill = p.accent.gamma_multiply(0.28);
    visuals.selection.stroke = Stroke::new(1.0_f32, p.accent);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, MUTED());
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT());
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT());
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT());
    visuals.widgets.inactive.weak_bg_fill = PANEL_2();
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, control_stroke);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, p.muted);
    visuals.widgets.hovered.weak_bg_fill = hover;
    visuals.widgets.open.weak_bg_fill = PANEL_2();
    visuals.window_stroke = Stroke::new(
        1.0_f32,
        if p.dark {
            Color32::from_rgb(0x3A, 0x3F, 0x48)
        } else {
            BORDER()
        },
    );
    visuals.window_corner_radius = CornerRadius::same(metrics::RADIUS_DIALOG);
    visuals.menu_corner_radius = CornerRadius::same(metrics::RADIUS_CARD);
    visuals.window_shadow = egui::Shadow {
        offset: [0, 16],
        blur: 48,
        spread: 0,
        color: Color32::from_black_alpha(if p.dark { 150 } else { 40 }),
    };
    visuals.popup_shadow = egui::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(if p.dark { 120 } else { 30 }),
    };
    for w in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(metrics::RADIUS_CONTROL);
    }
    ctx.set_visuals(visuals);
}

fn base_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.window_margin = Margin::same(20);
    style.spacing.menu_margin = Margin::same(6);
    style.spacing.indent = 12.0;
    style.spacing.interact_size.y = 28.0;
    // Tooltips wait for the pointer to rest, so scrolling a list stays quiet.
    style.interaction.show_tooltips_only_when_still = true;
    style.interaction.tooltip_delay = 0.35;
    // egui's default Small is 9 px; nothing in this app should be smaller than 12.
    style.text_styles.insert(
        egui::TextStyle::Small,
        egui::FontId::proportional(type_scale::CAPTION),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        egui::FontId::monospace(type_scale::MONO),
    );
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::new(type_scale::TITLE, bold()),
    );
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::proportional(type_scale::BODY),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::new(type_scale::BODY, medium()),
    );
    ctx.set_style(style);
}

/// Geist (bundled, OFL) in three weights; it covers Latin and Cyrillic. egui's
/// built-in fonts stay behind it for symbols and emoji.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        (
            "geist",
            &include_bytes!("../assets/fonts/Geist-Regular.ttf")[..],
        ),
        (
            "geist-medium",
            &include_bytes!("../assets/fonts/Geist-Medium.ttf")[..],
        ),
        (
            "geist-semibold",
            &include_bytes!("../assets/fonts/Geist-SemiBold.ttf")[..],
        ),
        (
            "geist-mono",
            &include_bytes!("../assets/fonts/GeistMono-Regular.ttf")[..],
        ),
    ] {
        fonts
            .font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    }

    let fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    for (family, primary) in [
        (FontFamily::Proportional, "geist"),
        (FontFamily::Monospace, "geist-mono"),
    ] {
        if let Some(list) = fonts.families.get_mut(&family) {
            list.insert(0, primary.to_owned());
        }
    }
    for (family, primary) in [(medium(), "geist-medium"), (bold(), "geist-semibold")] {
        let mut list = vec![primary.to_owned()];
        list.extend(fallback.iter().cloned());
        fonts.families.insert(family, list);
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

/// Hue by rank among siblings: bright on graphite, deeper on paper so tiles and
/// swatches keep their edge against a white ground.
fn pick_ranked_color(name: &str, sibling_index: usize, avoid: &[Color32]) -> Color32 {
    let n = DIR_HUES.len();
    let (top, sat) = if is_dark() {
        (0.90, 0.58)
    } else {
        (0.80, 0.66)
    };
    for offset in 0..n {
        let slot = (sibling_index + offset) % n;
        let v = top - ((sibling_index / n + offset) % 3) as f32 * 0.06;
        let s = sat + (slot % 3) as f32 * 0.04;
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
    let conflict = avoid.last().copied().unwrap_or(DIR());
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
