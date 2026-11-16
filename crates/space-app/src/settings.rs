//! Persisted app settings + feature flags (keeps 200-feature surface configurable).

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use space_core::SizeMode;

use crate::views::ViewMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ByteUnit {
    #[default]
    Binary,
    Si,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TreemapMode {
    #[default]
    Squarify,
    Strip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureFlags {
    pub show_percent_parent: bool,
    pub debounce_filter_ms: u64,
    pub show_shortcuts_hint: bool,
    pub sunburst_depth: u8,
    /// File list strip under the central diagram.
    #[serde(default = "default_true")]
    pub show_files_under_map: bool,
    /// Include browser cookie rules in junk scans (off by default; signs you out of sites).
    #[serde(default)]
    pub scan_browser_cookies: bool,
    /// Include browser history rules in junk scans (off by default).
    #[serde(default)]
    pub scan_browser_history: bool,
}

fn default_true() -> bool {
    true
}

fn default_two() -> usize {
    2
}

fn default_cache_ttl_days() -> u32 {
    14
}

fn default_small_file_kb() -> u64 {
    512
}

fn default_largest_limit() -> usize {
    100
}

fn default_cascade_depth() -> usize {
    5
}

fn default_cascade_row_height() -> f32 {
    48.0
}

impl Default for FeatureFlags {
    fn default() -> Self {
        Self {
            show_percent_parent: true,
            debounce_filter_ms: 150,
            show_shortcuts_hint: true,
            sunburst_depth: 2,
            show_files_under_map: true,
            scan_browser_cookies: false,
            scan_browser_history: false,
        }
    }
}

impl FeatureFlags {
    /// Master switches for cookies/history: off keeps those tags disabled even if Enable all is on.
    pub fn apply_to_rule_prefs(
        &self,
        prefs: &mut space_clean::RulePrefs,
        pack: &space_clean::RulePack,
    ) {
        for (on, tag) in [
            (self.scan_browser_cookies, "cookies"),
            (self.scan_browser_history, "history"),
        ] {
            prefs.tags_on.remove(tag);
            prefs.tags_off.remove(tag);
            if on {
                prefs.tags_on.insert(tag.into());
            } else {
                prefs.tags_off.insert(tag.into());
                for rule in &pack.rules {
                    if rule.tags.iter().any(|value| value == tag) {
                        prefs.set_rule(&rule.id, false);
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub default_view: ViewMode,
    pub default_size_mode: SizeMode,
    pub flags: FeatureFlags,
    pub window_width: f32,
    pub window_height: f32,
    /// Interface language; `None` follows the system locale.
    #[serde(default)]
    pub language: Option<crate::i18n::Lang>,
    /// Visual skin; switched live from the More menu.
    #[serde(default)]
    pub skin: crate::theme::Skin,
    #[serde(default)]
    pub high_contrast: bool,
    #[serde(default)]
    pub reduce_motion: bool,
    #[serde(default = "default_true")]
    pub confirm_trash: bool,
    #[serde(default = "default_true")]
    pub confirm_permanent_delete: bool,
    #[serde(default = "default_cache_ttl_days")]
    pub cache_ttl_days: u32,
    #[serde(default = "default_small_file_kb")]
    pub small_file_threshold_kb: u64,
    #[serde(default)]
    pub skip_hidden: bool,
    #[serde(default)]
    pub cross_mount: bool,
    #[serde(default)]
    pub exclude_names: Vec<String>,
    #[serde(default = "default_two")]
    pub sunburst_rings: usize,
    #[serde(default = "default_two")]
    pub treemap_depth: usize,
    #[serde(default)]
    pub treemap_mode: TreemapMode,
    #[serde(default)]
    pub byte_unit: ByteUnit,
    #[serde(default)]
    pub pinned_roots: Vec<PathBuf>,
    /// Worker count (`0` = automatic).
    #[serde(default)]
    pub scan_threads: u8,
    #[serde(default = "default_largest_limit")]
    pub largest_limit: usize,
    #[serde(default = "default_cascade_depth")]
    pub cascade_depth: usize,
    #[serde(default = "default_cascade_row_height")]
    pub cascade_row_height: f32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            default_view: ViewMode::Sunburst,
            default_size_mode: SizeMode::OnDisk,
            flags: FeatureFlags::default(),
            window_width: 1280.0,
            window_height: 800.0,
            language: None,
            skin: crate::theme::Skin::default(),
            high_contrast: false,
            reduce_motion: false,
            confirm_trash: true,
            confirm_permanent_delete: true,
            cache_ttl_days: default_cache_ttl_days(),
            small_file_threshold_kb: default_small_file_kb(),
            skip_hidden: false,
            cross_mount: false,
            exclude_names: Vec::new(),
            sunburst_rings: default_two(),
            treemap_depth: default_two(),
            treemap_mode: TreemapMode::default(),
            byte_unit: ByteUnit::default(),
            pinned_roots: Vec::new(),
            scan_threads: 0,
            largest_limit: default_largest_limit(),
            cascade_depth: default_cascade_depth(),
            cascade_row_height: default_cascade_row_height(),
        }
    }
}

fn settings_path() -> Option<PathBuf> {
    let base = directories::ProjectDirs::from("com", "themoretheless", "SpaceScanner")?;
    Some(base.config_dir().join("settings.json"))
}

impl AppSettings {
    pub fn load() -> Self {
        let Some(path) = settings_path() else {
            return Self::default();
        };
        let Ok(bytes) = fs::read(&path) else {
            return Self::default();
        };
        let mut settings: Self = serde_json::from_slice(&bytes).unwrap_or_default();
        settings.clamp();
        settings
    }

    pub fn save(&self) {
        let Some(path) = settings_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(self) {
            let _ = fs::write(path, bytes);
        }
    }

    /// Clamps persisted numeric settings to safe UI & engine bounds.
    pub fn clamp(&mut self) {
        self.sunburst_rings = self.sunburst_rings.clamp(1, 4);
        self.flags.sunburst_depth = self.sunburst_rings.min(u8::MAX as usize) as u8;
        self.treemap_depth = self.treemap_depth.clamp(1, 4);
        self.cascade_depth = self.cascade_depth.clamp(2, 8);
        self.cascade_row_height = self.cascade_row_height.clamp(28.0, 72.0);
        self.cache_ttl_days = self.cache_ttl_days.clamp(1, 365);
        self.small_file_threshold_kb = self.small_file_threshold_kb.clamp(0, 1_048_576);
        self.scan_threads = self.scan_threads.min(16);
        self.largest_limit = self.largest_limit.clamp(10, 500);
        self.flags.debounce_filter_ms = self.flags.debounce_filter_ms.clamp(0, 2_000);
        self.window_width = self.window_width.clamp(640.0, 7680.0);
        self.window_height = self.window_height.clamp(480.0, 4320.0);
    }

    /// Converts user scan settings into [`space_core::ScanOptions`].
    pub fn scan_options(&self) -> space_core::ScanOptions {
        space_core::ScanOptions {
            small_file_threshold: self.small_file_threshold_kb.saturating_mul(1024),
            skip_hidden: self.skip_hidden,
            cross_mount: self.cross_mount,
            exclude_names: self.exclude_names.clone(),
            threads: (self.scan_threads > 0).then_some(self.scan_threads as usize),
            force_read_dir: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_settings_json_deserializes_with_defaults() {
        let legacy = r#"{
            "default_view": "Sunburst",
            "default_size_mode": "OnDisk",
            "flags": {
                "show_percent_parent": true,
                "debounce_filter_ms": 150,
                "show_shortcuts_hint": true,
                "sunburst_depth": 2
            },
            "window_width": 1280.0,
            "window_height": 800.0
        }"#;
        let mut parsed: AppSettings = serde_json::from_str(legacy).expect("parse legacy json");
        parsed.clamp();
        assert!(!parsed.high_contrast);
        assert!(!parsed.reduce_motion);
        assert!(parsed.confirm_trash);
        assert!(parsed.confirm_permanent_delete);
        assert_eq!(parsed.cache_ttl_days, 14);
        assert_eq!(parsed.small_file_threshold_kb, 512);
        assert_eq!(parsed.sunburst_rings, 2);
        assert_eq!(parsed.treemap_depth, 2);
        assert_eq!(parsed.treemap_mode, TreemapMode::Squarify);
        assert_eq!(parsed.byte_unit, ByteUnit::Binary);
        assert!(parsed.pinned_roots.is_empty());
        assert_eq!(parsed.scan_options().small_file_threshold, 512 * 1024);
    }
}
