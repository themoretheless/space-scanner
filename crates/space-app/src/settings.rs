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
    /// Treemap tiles in ordered strips (keeps the size order readable)
    /// instead of the squarified layout.
    #[serde(default)]
    pub treemap_strip: bool,
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
            treemap_strip: false,
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

/// How the Disk usage workspace arranges the same data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceLayout {
    /// Size-ranked tree beside the map, details on the right.
    #[default]
    Split,
    /// Finder-style columns with details on the right.
    Columns,
    /// One dense tree table with checkboxes, no map.
    Table,
}

impl WorkspaceLayout {
    pub const ALL: [WorkspaceLayout; 3] = [Self::Split, Self::Columns, Self::Table];

    pub fn label(self) -> &'static str {
        match self {
            Self::Split => "List and map",
            Self::Columns => "Columns",
            Self::Table => "Table",
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
    /// Dark or light colours.
    #[serde(default)]
    pub appearance: crate::theme::Appearance,
    /// Disk usage arrangement, independent of the colours.
    #[serde(default)]
    pub layout: WorkspaceLayout,
    /// Pre-redesign "skin" (it set colours and layout together). Read once, never written.
    #[serde(default, rename = "skin", skip_serializing)]
    legacy_skin: Option<String>,
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
    /// High-contrast palette: stronger text and borders, no translucent fills.
    #[serde(default)]
    pub high_contrast: bool,
    /// How long the pointer rests before a hover hint appears.
    pub tooltip_delay_ms: u64,
    /// Open with the view and size mode last used instead of the defaults.
    pub remember_last_view: bool,
    /// View and size mode at the last exit (used when `remember_last_view`).
    pub last_view: Option<ViewMode>,
    pub last_size_mode: Option<SizeMode>,
    /// Saved scans older than this are removed at startup (0 = keep forever).
    pub cache_ttl_days: u64,
    /// Least-recently-used saved scans are removed once the cache exceeds
    /// this many MB (0 = no limit).
    pub cache_max_mb: u64,
    /// Rows in the largest-files list.
    pub largest_limit: usize,
    /// Largest list shows folders (subtree totals) instead of files.
    pub largest_dirs: bool,
    /// Report shown in place of the largest list (`None` = largest list).
    #[serde(default)]
    pub largest_report: Option<crate::reports::ReportKind>,
    /// Minimum size of the Oldest report, in MiB.
    #[serde(default = "default_oldest_min_mb")]
    pub oldest_min_mb: u64,
    /// Files below this size are rolled into one node per folder.
    #[serde(default = "default_small_file_threshold")]
    pub scan_small_file_threshold: u64,
    /// Scan worker threads; `None` = automatic.
    #[serde(default)]
    pub scan_threads: Option<usize>,
    /// Folders (paths) and glob patterns the scanner does not descend into.
    #[serde(default)]
    pub scan_exclude: Vec<String>,
}

fn default_oldest_min_mb() -> u64 {
    100
}

fn default_small_file_threshold() -> u64 {
    space_core::DEFAULT_SMALL_FILE_THRESHOLD
}

/// Small-file threshold presets offered in Settings.
pub const SMALL_FILE_PRESETS: [u64; 5] = [0, 64 * 1024, 512 * 1024, 4 * 1024 * 1024, 32 * 1024 * 1024];

/// True when an exclude entry is a glob pattern rather than a folder path.
pub fn is_glob(entry: &str) -> bool {
    entry.contains(['*', '?', '[', '{'])
}

/// Add `entry` to the exclude list unless it is empty or already there
/// (trailing separators ignored). Returns whether the list changed.
pub fn add_exclude(list: &mut Vec<String>, entry: &str) -> bool {
    let trimmed = entry.trim();
    let normal = |s: &str| -> String {
        let t = s.trim_end_matches(['/', '\\']);
        if t.is_empty() { s.to_string() } else { t.to_string() }
    };
    if trimmed.is_empty() {
        return false;
    }
    let entry = normal(trimmed);
    if list.iter().any(|e| normal(e) == entry) {
        return false;
    }
    list.push(entry);
    true
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
            appearance: crate::theme::Appearance::default(),
            layout: WorkspaceLayout::default(),
            legacy_skin: None,
            show_largest: false,
            compact_map: false,
            filter_kind: EntryKindFilter::All,
            last_focus: BTreeMap::new(),
            size_units: SizeUnits::Iec,
            reduce_motion: false,
            high_contrast: false,
            tooltip_delay_ms: 350,
            remember_last_view: true,
            last_view: None,
            last_size_mode: None,
            cache_ttl_days: 90,
            cache_max_mb: 0,
            largest_limit: 100,
            largest_dirs: false,
            largest_report: None,
            oldest_min_mb: default_oldest_min_mb(),
            scan_small_file_threshold: default_small_file_threshold(),
            scan_threads: None,
            scan_exclude: Vec::new(),
        }
    }
}

impl AppSettings {
    /// Scanner options built from the Scanning settings.
    pub fn scan_options(&self) -> space_core::ScanOptions {
        let mut opts = space_core::ScanOptions::default()
            .small_file_threshold(self.scan_small_file_threshold)
            .threads(self.scan_threads.map(|n| n.clamp(1, 32)));
        for entry in &self.scan_exclude {
            if is_glob(entry) {
                opts = opts.exclude_glob(entry.clone());
            } else {
                opts = opts.exclude(PathBuf::from(entry));
            }
        }
        opts
    }

    /// Scans with other options build a differently shaped tree, so they
    /// neither read nor write the saved-scan cache.
    pub fn scan_options_are_default(&self) -> bool {
        self.scan_small_file_threshold == space_core::DEFAULT_SMALL_FILE_THRESHOLD
            && self.scan_threads.is_none()
            && self.scan_exclude.is_empty()
    }

    /// Map an old skin onto the two settings that replaced it.
    fn migrate(mut self) -> Self {
        use crate::theme::Appearance;
        if let Some(skin) = self.legacy_skin.take() {
            (self.appearance, self.layout) = match skin.as_str() {
                "Columns" => (Appearance::Light, WorkspaceLayout::Columns),
                "Table" => (Appearance::Light, WorkspaceLayout::Table),
                _ => (Appearance::Dark, WorkspaceLayout::Split),
            };
        }
        self
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
        match serde_json::from_slice::<Self>(&bytes) {
            Ok(settings) => settings.migrate(),
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

    use crate::theme::Appearance;

    #[test]
    fn old_skins_become_appearance_and_layout() {
        let json = r#"{"default_view":"sunburst","default_size_mode":"OnDisk","flags":{"show_percent_parent":true,"debounce_filter_ms":150,"show_shortcuts_hint":true,"sunburst_depth":2},"window_width":1280.0,"window_height":800.0,"skin":"Table"}"#;
        let s = serde_json::from_str::<AppSettings>(json).unwrap().migrate();
        assert_eq!(s.appearance, Appearance::Light);
        assert_eq!(s.layout, WorkspaceLayout::Table);
        let saved = serde_json::to_string(&s).unwrap();
        assert!(
            !saved.contains("\"skin\""),
            "legacy key is not written back"
        );
    }

    #[test]
    fn scan_options_round_trip_and_default_when_missing() {
        let old: AppSettings = serde_json::from_str(r#"{"default_view":"sunburst"}"#).unwrap();
        assert_eq!(old.scan_small_file_threshold, space_core::DEFAULT_SMALL_FILE_THRESHOLD);
        assert_eq!(old.scan_threads, None);
        assert!(old.scan_exclude.is_empty());
        assert_eq!(old.oldest_min_mb, 100);
        assert!(old.scan_options_are_default());

        let dir = temp_dir("scanopts");
        let path = dir.join("settings.json");
        let s = AppSettings {
            scan_small_file_threshold: 64 * 1024,
            scan_threads: Some(4),
            scan_exclude: vec!["/Volumes/Backup".into(), "*.photoslibrary".into()],
            largest_report: Some(crate::reports::ReportKind::Duplicates),
            oldest_min_mb: 1024,
            ..AppSettings::default()
        };
        s.save_to(&path).unwrap();
        let back = AppSettings::load_from(&path);
        assert_eq!(back.scan_small_file_threshold, 64 * 1024);
        assert_eq!(back.scan_threads, Some(4));
        assert_eq!(back.scan_exclude, s.scan_exclude);
        assert_eq!(back.largest_report, Some(crate::reports::ReportKind::Duplicates));
        assert_eq!(back.oldest_min_mb, 1024);
        assert!(!back.scan_options_are_default());
        let opts = back.scan_options();
        assert_eq!(opts.exclude, vec![PathBuf::from("/Volumes/Backup")]);
        assert_eq!(opts.exclude_globs, vec!["*.photoslibrary".to_string()]);
        assert_eq!(opts.threads, Some(4));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn exclude_list_dedupes_and_ignores_blanks() {
        let mut list = Vec::new();
        assert!(add_exclude(&mut list, " /a/b/ "));
        assert!(!add_exclude(&mut list, "/a/b"));
        assert!(!add_exclude(&mut list, "/a/b//"));
        assert!(!add_exclude(&mut list, "   "));
        assert!(add_exclude(&mut list, "node_modules"));
        assert!(add_exclude(&mut list, "/"));
        assert!(!add_exclude(&mut list, "/"));
        assert_eq!(list, vec!["/a/b".to_string(), "node_modules".into(), "/".into()]);
        assert!(is_glob("*.tmp") && !is_glob("/a/b"));
    }

    #[test]
    fn new_settings_keep_their_values() {
        let s = AppSettings {
            appearance: Appearance::Light,
            layout: WorkspaceLayout::Columns,
            ..AppSettings::default()
        };
        let back: AppSettings =
            serde_json::from_str::<AppSettings>(&serde_json::to_string(&s).unwrap())
                .unwrap()
                .migrate();
        assert_eq!(back.appearance, Appearance::Light);
        assert_eq!(back.layout, WorkspaceLayout::Columns);
    }
}
