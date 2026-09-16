use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Deletion danger level for a cleaning rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Risk {
    Low,
    Medium,
    High,
    ReportOnly,
}

impl Risk {
    pub fn as_str(self) -> &'static str {
        match self {
            Risk::Low => "low",
            Risk::Medium => "medium",
            Risk::High => "high",
            Risk::ReportOnly => "report-only",
        }
    }

    /// Short danger label for UI / CLI.
    pub fn danger_label(self) -> &'static str {
        match self {
            Risk::Low => "safe",
            Risk::Medium => "caution",
            Risk::High => "dangerous",
            Risk::ReportOnly => "report-only",
        }
    }

    /// Rank for `--max-risk` filters (higher = more dangerous).
    pub fn rank(self) -> u8 {
        match self {
            Risk::Low => 0,
            Risk::Medium => 1,
            Risk::High => 2,
            Risk::ReportOnly => 3,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "low" | "safe" => Some(Risk::Low),
            "medium" | "caution" | "med" => Some(Risk::Medium),
            "high" | "dangerous" | "danger" => Some(Risk::High),
            "report-only" | "report" | "ro" => Some(Risk::ReportOnly),
            _ => None,
        }
    }

    pub fn all() -> &'static [Risk] {
        &[Risk::Low, Risk::Medium, Risk::High, Risk::ReportOnly]
    }
}

/// What kind of junk the rule targets (logs, temp, cache, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum JunkKind {
    Cache,
    Logs,
    Temp,
    Downloads,
    Leftovers,
    Packages,
    Browser,
    System,
    AppData,
    Media,
    Databases,
    Models,
    Vm,
    Dev,
    #[default]
    Other,
}

impl JunkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            JunkKind::Cache => "cache",
            JunkKind::Logs => "logs",
            JunkKind::Temp => "temp",
            JunkKind::Downloads => "downloads",
            JunkKind::Leftovers => "leftovers",
            JunkKind::Packages => "packages",
            JunkKind::Browser => "browser",
            JunkKind::System => "system",
            JunkKind::AppData => "app-data",
            JunkKind::Media => "media",
            JunkKind::Databases => "databases",
            JunkKind::Models => "models",
            JunkKind::Vm => "vm",
            JunkKind::Dev => "dev",
            JunkKind::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "cache" | "caches" => Some(JunkKind::Cache),
            "logs" | "log" => Some(JunkKind::Logs),
            "temp" | "tmp" | "temporary" => Some(JunkKind::Temp),
            "downloads" | "download" => Some(JunkKind::Downloads),
            "leftovers" | "leftover" | "orphan" => Some(JunkKind::Leftovers),
            "packages" | "package" | "package-cache" => Some(JunkKind::Packages),
            "browser" | "browser-cache" => Some(JunkKind::Browser),
            "system" => Some(JunkKind::System),
            "app-data" | "appdata" => Some(JunkKind::AppData),
            "media" | "media-cache" | "media-library" => Some(JunkKind::Media),
            "databases" | "database" | "db" => Some(JunkKind::Databases),
            "models" | "ml" | "ml-models" => Some(JunkKind::Models),
            "vm" | "vm-disk" | "vm-cache" => Some(JunkKind::Vm),
            "dev" | "developer" => Some(JunkKind::Dev),
            "other" | "misc" | "report" => Some(JunkKind::Other),
            _ => None,
        }
    }

    pub fn all() -> &'static [JunkKind] {
        &[
            JunkKind::Cache,
            JunkKind::Logs,
            JunkKind::Temp,
            JunkKind::Downloads,
            JunkKind::Leftovers,
            JunkKind::Packages,
            JunkKind::Browser,
            JunkKind::System,
            JunkKind::AppData,
            JunkKind::Media,
            JunkKind::Databases,
            JunkKind::Models,
            JunkKind::Vm,
            JunkKind::Dev,
            JunkKind::Other,
        ]
    }

    /// Infer junk kind from category / id / paths when YAML omits `kind`.
    pub fn infer(rule: &Rule) -> Self {
        let cat = rule.category.to_ascii_lowercase();
        if let Some(k) = Self::from_category(&cat) {
            return k;
        }
        let id = rule.id.to_ascii_lowercase();
        let name = rule.name.to_ascii_lowercase();
        let paths = rule.paths.join("\n").to_ascii_lowercase();
        let hay = format!("{id}\n{name}\n{paths}");
        if hay.contains("browser") || hay.contains("chrome") || hay.contains("firefox") {
            return JunkKind::Browser;
        }
        if hay.contains("/log") || hay.contains("logs") || hay.contains(".log") {
            return JunkKind::Logs;
        }
        if hay.contains("temp") || hay.contains("tmpdir") || hay.contains("/tmp") {
            return JunkKind::Temp;
        }
        if hay.contains("download") {
            return JunkKind::Downloads;
        }
        if hay.contains("node_modules") || hay.contains("package") || hay.contains("pip") {
            return JunkKind::Packages;
        }
        if hay.contains("cache") {
            return JunkKind::Cache;
        }
        JunkKind::Other
    }

    fn from_category(cat: &str) -> Option<Self> {
        match cat {
            "logs" => Some(JunkKind::Logs),
            "temp" => Some(JunkKind::Temp),
            "leftovers" => Some(JunkKind::Leftovers),
            "package-cache" => Some(JunkKind::Packages),
            "browser-cache" => Some(JunkKind::Browser),
            "system" => Some(JunkKind::System),
            "app-data" => Some(JunkKind::AppData),
            "databases" => Some(JunkKind::Databases),
            "ml-models" => Some(JunkKind::Models),
            "dev" => Some(JunkKind::Dev),
            "games-downloads" => Some(JunkKind::Downloads),
            "media-library" | "sample-library" | "media-cache" => Some(JunkKind::Media),
            "vm-disk" | "vm-cache" => Some(JunkKind::Vm),
            "app-cache" | "caches" | "shader-cache" | "game-cache" | "ml-cache" | "wine-cache" => {
                Some(JunkKind::Cache)
            }
            "report" | "misc" => Some(JunkKind::Other),
            _ => {
                if cat.contains("log") {
                    Some(JunkKind::Logs)
                } else if cat.contains("temp") {
                    Some(JunkKind::Temp)
                } else if cat.contains("browser") {
                    Some(JunkKind::Browser)
                } else if cat.contains("cache") {
                    Some(JunkKind::Cache)
                } else {
                    None
                }
            }
        }
    }
}

/// Platform filter for a rule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Platform {
    #[default]
    Any,
    Macos,
    Linux,
    Windows,
}

impl Platform {
    pub fn matches_host(self) -> bool {
        match self {
            Platform::Any => true,
            Platform::Macos => cfg!(target_os = "macos"),
            Platform::Linux => cfg!(target_os = "linux"),
            Platform::Windows => cfg!(target_os = "windows"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Platform::Any => "any",
            Platform::Macos => "macos",
            Platform::Linux => "linux",
            Platform::Windows => "windows",
        }
    }
}

/// How to treat matched paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum DeleteMode {
    /// Delete files/dirs under the path, keep the root directory.
    #[default]
    Contents,
    /// Delete the matched path itself.
    Path,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default)]
    pub platform: Platform,
    #[serde(default = "default_risk")]
    pub risk: Risk,
    /// Junk type: logs, temp, cache, … — inferred from category when omitted.
    #[serde(default)]
    pub kind: JunkKind,
    #[serde(default = "default_true")]
    pub default_enabled: bool,
    /// Skip files newer than this many days (0 = no age filter).
    #[serde(default = "default_min_age")]
    pub min_age_days: u64,
    /// Path templates with $HOME, $TMPDIR, $XDG_CACHE_HOME, %LOCALAPPDATA%, etc.
    pub paths: Vec<String>,
    #[serde(default)]
    pub include_globs: Vec<String>,
    #[serde(default)]
    pub exclude_globs: Vec<String>,
    /// Require at least one matching marker in the candidate's ancestor tree.
    /// Useful for project output rules such as C# `bin`/`obj`.
    #[serde(default)]
    pub required_ancestor_globs: Vec<String>,
    /// Emit a matched directory as one cleanable unit instead of its files.
    #[serde(default)]
    pub match_directories: bool,
    #[serde(default = "default_max_depth")]
    pub max_depth: usize,
    #[serde(default)]
    pub delete_mode: DeleteMode,
    /// If true, only list; never delete even with --yes (unless --all and future override).
    #[serde(default)]
    pub report_only: bool,
    /// Free-form labels for filtering / bulk prefs (`browser`, `electron`, `npm`, …).
    /// Empty in YAML is fine — tags are inferred on load and merged in.
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_category() -> String {
    "misc".into()
}
fn default_risk() -> Risk {
    Risk::Low
}
fn default_true() -> bool {
    true
}
fn default_min_age() -> u64 {
    3
}
fn default_max_depth() -> usize {
    12
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuleFile {
    #[serde(default)]
    pub pack: Option<String>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone)]
pub struct RulePack {
    pub rules: Vec<Rule>,
    pub sources: Vec<PathBuf>,
}

impl RulePack {
    pub fn load_many(dirs: &[PathBuf]) -> Result<Self> {
        let mut seen_dirs = Vec::new();
        let mut by_id: BTreeMap<String, Rule> = BTreeMap::new();
        let mut sources = Vec::new();

        for dir in dirs {
            let canon = dir.canonicalize().unwrap_or_else(|_| dir.clone());
            if seen_dirs.iter().any(|d| d == &canon) {
                continue;
            }
            seen_dirs.push(canon.clone());
            if !dir.is_dir() {
                continue;
            }
            let mut files = collect_rule_files(dir)?;
            files.sort();
            for file in files {
                let text = fs::read_to_string(&file)
                    .with_context(|| format!("read {}", file.display()))?;
                let parsed: RuleFile = serde_yaml::from_str(&text)
                    .with_context(|| format!("parse {}", file.display()))?;
                sources.push(file.clone());
                for mut rule in parsed.rules {
                    rule.id = rule.id.trim().to_string();
                    rule.name = rule.name.trim().to_string();
                    rule.category = rule.category.trim().to_string();
                    validate_rule(&rule, &file)?;
                    enrich_rule_kind(&mut rule);
                    crate::tags::enrich_rule_tags(&mut rule);
                    if by_id.contains_key(&rule.id) {
                        bail!("{}: duplicate rule id '{}'", file.display(), rule.id);
                    }
                    by_id.insert(rule.id.clone(), rule);
                }
            }
        }

        let rules: Vec<Rule> = by_id.into_values().collect();
        if rules.is_empty() {
            anyhow::bail!("no rules loaded from {}", path_list(dirs));
        }
        Ok(RulePack { rules, sources })
    }

    pub fn active_rules(&self, categories: &[String], include_disabled: bool) -> Vec<&Rule> {
        self.rules
            .iter()
            .filter(|r| r.platform.matches_host())
            .filter(|r| include_disabled || r.default_enabled)
            .filter(|r| {
                if categories.is_empty() {
                    true
                } else {
                    categories.iter().any(|c| c == &r.category || c == &r.id)
                }
            })
            .collect()
    }

    pub fn categories(&self, include_disabled: bool) -> BTreeMap<String, usize> {
        let mut map = BTreeMap::new();
        for r in &self.rules {
            if !r.platform.matches_host() {
                continue;
            }
            if !include_disabled && !r.default_enabled {
                continue;
            }
            *map.entry(r.category.clone()).or_insert(0) += 1;
        }
        map
    }

    pub fn get(&self, id: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.id == id)
    }
}

/// Recursively collect `*.yaml` / `*.yml` under a rules root (skips `_*.yaml` drafts).
fn collect_rule_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| x.eq_ignore_ascii_case("yaml") || x.eq_ignore_ascii_case("yml"))
            .unwrap_or(false);
        if !ext {
            continue;
        }
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with('_') || n.eq_ignore_ascii_case("README.yaml"))
            .unwrap_or(false)
        {
            continue;
        }
        files.push(path.to_path_buf());
    }
    Ok(files)
}

fn validate_rule(rule: &Rule, source: &Path) -> Result<()> {
    let rule_id = rule.id.trim();
    if rule_id.is_empty() {
        bail!("{}: rule missing non-empty id", source.display());
    }
    if rule.name.trim().is_empty() {
        bail!(
            "{}: rule '{}' missing non-empty name",
            source.display(),
            rule_id
        );
    }
    if rule.description.trim().is_empty() {
        bail!(
            "{}: rule '{}' missing non-empty description",
            source.display(),
            rule_id
        );
    }
    if rule.category.trim().is_empty() {
        bail!(
            "{}: rule '{}' missing non-empty category",
            source.display(),
            rule_id
        );
    }
    if rule.paths.is_empty() {
        bail!(
            "{}: rule '{}' must define at least one path",
            source.display(),
            rule_id
        );
    }
    for path in &rule.paths {
        if path.trim().is_empty() {
            bail!(
                "{}: rule '{}' contains an empty path entry",
                source.display(),
                rule_id
            );
        }
    }
    Ok(())
}

fn path_list(dirs: &[PathBuf]) -> String {
    dirs.iter()
        .map(|d| d.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Expand path templates to concrete absolute paths.
pub fn expand_path_template(template: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let mut s = template.to_string();

    let replacements: Vec<(&str, PathBuf)> = {
        let mut v = vec![
            ("$HOME", home.clone()),
            ("~", home.clone()),
            ("$TMPDIR", std::env::temp_dir()),
            ("$TEMP", std::env::temp_dir()),
            ("$TMP", std::env::temp_dir()),
        ];
        if let Some(cache) = dirs::cache_dir() {
            v.push(("$XDG_CACHE_HOME", cache.clone()));
            v.push(("$CACHE", cache));
        }
        if let Some(data) = dirs::data_local_dir() {
            v.push(("$XDG_DATA_HOME", data.clone()));
            v.push(("%LOCALAPPDATA%", data.clone()));
            v.push(("$LOCALAPPDATA", data));
        }
        if let Some(data) = dirs::data_dir() {
            v.push((
                "$XDG_CONFIG_HOME",
                dirs::config_dir().unwrap_or(data.clone()),
            ));
            v.push(("%APPDATA%", data.clone()));
            v.push(("$APPDATA", data));
        }
        if let Ok(userprofile) = std::env::var("USERPROFILE") {
            v.push(("%USERPROFILE%", PathBuf::from(userprofile)));
        }
        if let Ok(windir) = std::env::var("WINDIR") {
            v.push(("%WINDIR%", PathBuf::from(windir)));
        }
        if let Ok(systemdrive) = std::env::var("SystemDrive") {
            v.push(("%SystemDrive%", PathBuf::from(systemdrive)));
        }
        // Windows env aliases (also useful when cross-compiling paths in YAML).
        if let Ok(temp) = std::env::var("TEMP").or_else(|_| std::env::var("TMP")) {
            let t = PathBuf::from(temp);
            v.push(("%TEMP%", t.clone()));
            v.push(("%TMP%", t));
        }
        if let Ok(pd) = std::env::var("PROGRAMDATA") {
            v.push(("%PROGRAMDATA%", PathBuf::from(pd)));
        }
        if let Ok(pf) = std::env::var("ProgramFiles").or_else(|_| std::env::var("PROGRAMFILES")) {
            v.push(("%PROGRAMFILES%", PathBuf::from(pf)));
        }
        if let Ok(pf86) = std::env::var("ProgramFiles(x86)") {
            v.push(("%PROGRAMFILES(X86)%", PathBuf::from(pf86)));
        }
        // Common Windows defaults when env is missing (path still platform-filtered).
        #[cfg(windows)]
        {
            if std::env::var_os("LOCALAPPDATA").is_none() {
                if let Some(home) = dirs::home_dir() {
                    v.push(("%LOCALAPPDATA%", home.join("AppData").join("Local")));
                    v.push(("%APPDATA%", home.join("AppData").join("Roaming")));
                }
            }
            if std::env::var_os("TEMP").is_none() {
                if let Some(home) = dirs::home_dir() {
                    let t = home.join("AppData").join("Local").join("Temp");
                    v.push(("%TEMP%", t.clone()));
                    v.push(("%TMP%", t));
                }
            }
        }
        v
    };

    // Longest keys first so %LOCALAPPDATA% beats partials
    let mut keys = replacements;
    keys.sort_by_key(|item| std::cmp::Reverse(item.0.len()));

    for (key, path) in &keys {
        if s.contains(key) {
            s = s.replace(key, &path.to_string_lossy());
        }
    }

    // Reject unexpanded variables
    if s.contains('$') || s.contains('%') {
        return None;
    }

    let p = PathBuf::from(s);
    if p.exists() {
        Some(p)
    } else {
        // Still return path so scan can report missing roots quietly
        Some(p)
    }
}

pub fn host_platform_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "unknown"
    }
}

/// Fill `kind` when YAML left it as default `other` but category/paths imply something else.
pub fn enrich_rule_kind(rule: &mut Rule) {
    if rule.kind == JunkKind::Other {
        let inferred = JunkKind::infer(rule);
        if inferred != JunkKind::Other {
            rule.kind = inferred;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_home() {
        let p = expand_path_template("$HOME/Library/Caches").unwrap();
        assert!(p.to_string_lossy().contains("Library"));
    }

    #[test]
    fn rejects_rule_without_required_fields() {
        let mut rule = Rule {
            id: "   ".into(),
            name: "   ".into(),
            description: "".into(),
            category: "".into(),
            platform: Platform::Any,
            risk: Risk::Low,
            kind: JunkKind::Other,
            default_enabled: true,
            min_age_days: 0,
            paths: vec![],
            include_globs: vec![],
            exclude_globs: vec![],
            required_ancestor_globs: vec![],
            match_directories: false,
            max_depth: 12,
            delete_mode: DeleteMode::Contents,
            report_only: false,
            tags: vec![],
        };
        let err = validate_rule(&rule, &PathBuf::from("rules.yaml")).unwrap_err();
        assert!(err.to_string().contains("missing non-empty id"));

        rule.id = "valid-id".into();
        rule.name = "Valid name".into();
        rule.category = "caches".into();
        rule.paths = vec!["$HOME/.cache/example".into()];
        let err = validate_rule(&rule, &PathBuf::from("rules.yaml")).unwrap_err();
        assert!(err.to_string().contains("missing non-empty description"));
    }
}
