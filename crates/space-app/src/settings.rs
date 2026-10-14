//! Persisted app settings + feature flags (keeps 200-feature surface configurable).

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use space_core::SizeMode;

use crate::views::ViewMode;

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
pub struct AppSettings {
    pub default_view: ViewMode,
    pub default_size_mode: SizeMode,
    pub flags: FeatureFlags,
    pub window_width: f32,
    pub window_height: f32,
    /// Interface language; `None` follows the system locale.
    #[serde(default)]
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
        }
    }
}

impl AppSettings {
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
        serde_json::from_slice::<Self>(&bytes)
            .map(Self::migrate)
            .unwrap_or_default()
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
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn new_settings_keep_their_values() {
        let mut s = AppSettings::default();
        s.appearance = Appearance::Light;
        s.layout = WorkspaceLayout::Columns;
        let back: AppSettings =
            serde_json::from_str::<AppSettings>(&serde_json::to_string(&s).unwrap())
                .unwrap()
                .migrate();
        assert_eq!(back.appearance, Appearance::Light);
        assert_eq!(back.layout, WorkspaceLayout::Columns);
    }
}
