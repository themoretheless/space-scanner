//! Persisted app settings + feature flags (keeps 200-feature surface configurable).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use space_core::SizeMode;

use crate::views::{EntryKindFilter, ViewMode};

/// How sizes are written: binary (KiB, 1024) or decimal (kB, 1000).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum SizeUnits {
    #[default]
    Iec = 0,
    Si = 1,
}

impl SizeUnits {
    pub const ALL: [SizeUnits; 2] = [SizeUnits::Iec, SizeUnits::Si];

    pub fn label(self) -> &'static str {
        match self {
            SizeUnits::Iec => "Binary (KiB, MiB)",
            SizeUnits::Si => "Decimal (kB, MB)",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FeatureFlags {
    /// Reserved: the details panel always shows the share of the parent.
    pub show_percent_parent: bool,
    pub debounce_filter_ms: u64,
    pub show_shortcuts_hint: bool,
    /// Rings drawn by the sunburst (1..=4).
    pub sunburst_depth: u8,
    /// Folder levels nested inside treemap tiles (1..=4).
    pub treemap_nest: u8,
    /// Rows of bands the cascade draws below the focus (2..=8).
    pub cascade_depth: u8,
    /// Band height multiplier for the cascade (0.6..=1.6).
    pub cascade_row_scale: f32,
    /// Reserved: the file list under the map was folded into the contents pane.
    pub show_files_under_map: bool,
    /// Include browser cookie rules in junk scans (off by default; signs you out of sites).
    #[serde(default)]
    pub scan_browser_cookies: bool,
    /// Include browser history rules in junk scans (off by default).
    #[serde(default)]
    pub scan_browser_history: bool,
}

impl Default for FeatureFlags {
    fn default() -> Self {
        Self {
            show_percent_parent: true,
            debounce_filter_ms: 150,
            show_shortcuts_hint: true,
            sunburst_depth: 2,
            treemap_nest: 2,
            cascade_depth: 5,
            cascade_row_scale: 1.0,
            show_files_under_map: true,
            scan_browser_cookies: false,
            scan_browser_history: false,
        }
    }
}

impl FeatureFlags {
    /// Master switches for cookies/history: off keeps those tags disabled even if Enable all is on.
    pub fn apply_to_rule_prefs(&self, prefs: &mut space_clean::RulePrefs, pack: &space_clean::RulePack) {
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
#[serde(default)]
pub struct AppSettings {
    pub default_view: ViewMode,
    pub default_size_mode: SizeMode,
    pub flags: FeatureFlags,
    pub window_width: f32,
    pub window_height: f32,
    /// Interface language; `None` follows the system locale.
    pub language: Option<crate::i18n::Lang>,
    /// Visual skin; switched live from the More menu.
    pub skin: crate::theme::Skin,
    /// Contents pane shows the largest-file list instead of the folder.
    pub show_largest: bool,
    /// Narrow windows show the map instead of the list.
    pub compact_map: bool,
    /// Folders / files / all in the contents pane.
    pub filter_kind: EntryKindFilter,
    /// Last viewed folder per scan root (root path → folder path).
    pub last_focus: BTreeMap<String, String>,
    /// Binary or decimal size units everywhere sizes are shown.
    pub size_units: SizeUnits,
    /// No zoom, crossfade or hover animation.
    pub reduce_motion: bool,
    /// How long the pointer rests before a hover hint appears.
    pub tooltip_delay_ms: u64,
    /// Rows in the largest-files list.
    pub largest_limit: usize,
    /// Largest list shows folders (subtree totals) instead of files.
    pub largest_dirs: bool,
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
            show_largest: false,
            compact_map: false,
            filter_kind: EntryKindFilter::All,
            last_focus: BTreeMap::new(),
            size_units: SizeUnits::Iec,
            reduce_motion: false,
            tooltip_delay_ms: 350,
            largest_limit: 100,
            largest_dirs: false,
        }
    }
}

fn config_dir() -> Option<PathBuf> {
    let base = directories::ProjectDirs::from("com", "themoretheless", "SpaceScanner")?;
    Some(base.config_dir().to_path_buf())
}

fn settings_path() -> Option<PathBuf> {
    Some(config_dir()?.join("settings.json"))
}

/// Where the panic hook writes the last crash.
pub fn panic_log_path() -> Option<PathBuf> {
    Some(config_dir()?.join("panic.log"))
}

/// The previous run's panic message, removed once read.
pub fn take_crash_notice() -> Option<String> {
    let path = panic_log_path()?;
    let text = fs::read_to_string(&path).ok()?;
    let _ = fs::remove_file(&path);
    let text = text.trim();
    (!text.is_empty()).then(|| text.chars().take(600).collect())
}

impl AppSettings {
    pub fn load() -> Self {
        let Some(path) = settings_path() else {
            return Self::default();
        };
        Self::load_from(&path)
    }

    /// Unreadable settings are set aside as `settings.json.bad` so the next
    /// save does not silently overwrite what the user had.
    fn load_from(path: &Path) -> Self {
        let Ok(bytes) = fs::read(path) else {
            return Self::default();
        };
        match serde_json::from_slice(&bytes) {
            Ok(settings) => settings,
            Err(_) => {
                let mut bad = path.as_os_str().to_owned();
                bad.push(".bad");
                let _ = fs::rename(path, bad);
                Self::default()
            }
        }
    }

    pub fn save(&self) {
        let Some(path) = settings_path() else {
            return;
        };
        let _ = self.save_to(&path);
    }

    /// Write to a sibling temp file and rename so a crash mid-write cannot
    /// leave a truncated settings file.
    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "space-app-settings-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_fields_take_defaults_and_round_trip_through_the_atomic_write() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("settings.json");
        fs::write(&path, br#"{"default_view":"cascade"}"#).unwrap();
        let mut loaded = AppSettings::load_from(&path);
        assert_eq!(loaded.default_view, ViewMode::Cascade);
        assert_eq!(loaded.flags.debounce_filter_ms, 150);
        loaded.show_largest = true;
        loaded.last_focus.insert("/r".into(), "/r/a".into());
        loaded.save_to(&path).unwrap();
        assert!(!dir.join("settings.json.tmp").exists());
        let again = AppSettings::load_from(&path);
        assert!(again.show_largest);
        assert_eq!(again.last_focus.get("/r").map(String::as_str), Some("/r/a"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_settings_are_set_aside_not_overwritten() {
        let dir = temp_dir("corrupt");
        let path = dir.join("settings.json");
        fs::write(&path, b"{not json").unwrap();
        let loaded = AppSettings::load_from(&path);
        assert_eq!(loaded.window_width, 1280.0);
        assert!(!path.exists());
        assert_eq!(fs::read(dir.join("settings.json.bad")).unwrap(), b"{not json");
        let _ = fs::remove_dir_all(&dir);
    }
}
