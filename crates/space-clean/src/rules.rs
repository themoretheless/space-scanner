use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RulePack {
    pub rules: Vec<Rule>,
    pub sources: Vec<PathBuf>,
}

type SourceFingerprint = Vec<(PathBuf, blake3::Hash)>;
static SHARED_PACK: Mutex<Option<(SourceFingerprint, Arc<RulePack>)>> = Mutex::new(None);

fn source_fingerprint(dirs: &[PathBuf]) -> Result<SourceFingerprint> {
    let mut seen = HashSet::new();
    let mut fingerprint = Vec::new();
    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        let mut files = collect_rule_files(dir)?;
        files.sort();
        for file in files {
            let file = file.canonicalize().unwrap_or(file);
            if seen.insert(file.clone()) {
                let bytes = fs::read(&file).with_context(|| format!("read {}", file.display()))?;
                fingerprint.push((file, blake3::hash(&bytes)));
            }
        }
    }
    Ok(fingerprint)
}

// Include parser/enrichment sources so changes to defaults or inferred tags invalidate
// persisted catalogs even when the application version has not changed.
fn catalog_cache_path(fingerprint: &SourceFingerprint) -> Option<(PathBuf, blake3::Hash)> {
    let base = directories::ProjectDirs::from("com", "themoretheless", "SpaceScanner")?;
    let mut hash = blake3::Hasher::new();
    let mut identity = blake3::Hasher::new();
    hash.update(b"catalog-bincode-v1");
    hash.update(include_bytes!("rules.rs"));
    hash.update(include_bytes!("tags.rs"));
    for (path, digest) in fingerprint {
        let path = bincode::serialize(path).ok()?;
        hash.update(&(path.len() as u64).to_le_bytes());
        hash.update(&path);
        identity.update(&(path.len() as u64).to_le_bytes());
        identity.update(&path);
        hash.update(digest.as_bytes());
    }
    Some((
        base.cache_dir()
            .join("catalog")
            .join(format!("{}.bin", identity.finalize().to_hex())),
        hash.finalize(),
    ))
}

fn read_catalog_cache(path: &Path, key: &blake3::Hash) -> Option<RulePack> {
    use bincode::Options;
    let bytes = fs::read(path).ok()?;
    let (stored_key, bytes) = bytes.split_at_checked(32)?;
    if stored_key != key.as_bytes() {
        return None;
    }
    let (checksum, payload) = bytes.split_at_checked(32)?;
    if blake3::hash(payload).as_bytes() != checksum {
        return None;
    }
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(payload.len() as u64)
        .reject_trailing_bytes()
        .deserialize(payload)
        .ok()
}

fn write_catalog_cache(path: &Path, key: &blake3::Hash, pack: &RulePack) -> Result<()> {
    use std::io::Write;
    fs::create_dir_all(path.parent().context("cache parent")?)?;
    let bytes = bincode::serialize(pack)?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(key.as_bytes())?;
        file.write_all(blake3::hash(&bytes).as_bytes())?;
        file.write_all(&bytes)?;
        drop(file);
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

impl RulePack {
    /// Share immutable catalogs while detecting edits, additions and removals.
    /// Hash file contents so same-length edits and preserved timestamps invalidate.
    pub fn load_many_shared(dirs: &[PathBuf]) -> Result<Arc<Self>> {
        let mut cache = SHARED_PACK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let fingerprint = source_fingerprint(dirs)?;
        if let Some((previous, pack)) = cache.as_ref() {
            if *previous == fingerprint {
                return Ok(Arc::clone(pack));
            }
        }
        let disk_path = catalog_cache_path(&fingerprint);
        let cached = disk_path
            .as_ref()
            .and_then(|(path, key)| read_catalog_cache(path, key));
        let disk_hit = cached.is_some();
        let pack = Arc::new(cached.map(Ok).unwrap_or_else(|| Self::load_many(dirs))?);
        // Do not cache a catalog if its files changed while it was being parsed.
        if source_fingerprint(dirs)? == fingerprint {
            if !disk_hit {
                if let Some((path, key)) = disk_path.as_ref() {
                    let _ = write_catalog_cache(path, key, &pack);
                }
            }
            *cache = Some((fingerprint, Arc::clone(&pack)));
        } else {
            *cache = None;
        }
        Ok(pack)
    }

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
                let file = file.canonicalize().unwrap_or(file);
                if sources.iter().any(|seen| seen == &file) {
                    continue;
                }
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
    PathTemplateExpander::new()?.expand(template)
}

/// Snapshot environment-derived substitutions once per scan, not once per root.
pub(crate) struct PathTemplateExpander {
    keys: Vec<(&'static str, PathBuf)>,
}

impl PathTemplateExpander {
    pub(crate) fn new() -> Option<Self> {
        let base_dirs = directories::BaseDirs::new()?;
        let home = base_dirs.home_dir().to_path_buf();
        let replacements: Vec<(&str, PathBuf)> = {
            let mut v = vec![
                ("$HOME", home.clone()),
                ("~", home.clone()),
                ("$TMPDIR", std::env::temp_dir()),
                ("$TEMP", std::env::temp_dir()),
                ("$TMP", std::env::temp_dir()),
            ];
            let cache = base_dirs.cache_dir().to_path_buf();
            v.push(("$XDG_CACHE_HOME", cache.clone()));
            v.push(("$CACHE", cache));
            let data_local = base_dirs.data_local_dir().to_path_buf();
            v.push(("$XDG_DATA_HOME", data_local.clone()));
            v.push(("%LOCALAPPDATA%", data_local.clone()));
            v.push(("$LOCALAPPDATA", data_local));
            let data = base_dirs.data_dir().to_path_buf();
            v.push(("$XDG_CONFIG_HOME", base_dirs.config_dir().to_path_buf()));
            v.push(("%APPDATA%", data.clone()));
            v.push(("$APPDATA", data));
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
            if let Ok(pf) = std::env::var("ProgramFiles").or_else(|_| std::env::var("PROGRAMFILES"))
            {
                v.push(("%PROGRAMFILES%", PathBuf::from(pf)));
            }
            if let Ok(pf86) = std::env::var("ProgramFiles(x86)") {
                v.push(("%PROGRAMFILES(X86)%", PathBuf::from(pf86)));
            }
            // Common Windows defaults when env is missing (path still platform-filtered).
            #[cfg(windows)]
            {
                if std::env::var_os("LOCALAPPDATA").is_none() {
                    v.push(("%LOCALAPPDATA%", home.join("AppData").join("Local")));
                    v.push(("%APPDATA%", home.join("AppData").join("Roaming")));
                }
                if std::env::var_os("TEMP").is_none() {
                    let t = home.join("AppData").join("Local").join("Temp");
                    v.push(("%TEMP%", t.clone()));
                    v.push(("%TMP%", t));
                }
            }
            v
        };

        // Longest keys first so %LOCALAPPDATA% beats partials
        let mut keys = replacements;
        keys.sort_by_key(|item| std::cmp::Reverse(item.0.len()));

        Some(Self { keys })
    }

    pub(crate) fn expand(&self, template: &str) -> Option<PathBuf> {
        let mut s = template.to_string();
        for (key, path) in &self.keys {
            if s.contains(key) {
                s = s.replace(key, &path.to_string_lossy());
            }
        }

        // Reject unexpanded variables
        if s.contains('$') || s.contains('%') {
            return None;
        }

        Some(PathBuf::from(s))
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
    fn shared_catalog_invalidates_on_content_addition_removal_and_errors() {
        let root = std::env::temp_dir().join(format!("space-shared-rules-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let path = root.join("rules.yaml");
        let source = "rules:\n- id: first\n  name: first\n  description: fixture\n  paths: [/tmp/cache]\n  min_age_days: 3\n";
        fs::write(&path, source).unwrap();
        let dirs = [root.clone()];
        let first = RulePack::load_many_shared(&dirs).unwrap();
        let again = RulePack::load_many_shared(&dirs).unwrap();
        assert!(Arc::ptr_eq(&first, &again));
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, source.replace("days: 3", "days: 4")).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
        let changed = RulePack::load_many_shared(&dirs).unwrap();
        assert!(!Arc::ptr_eq(&first, &changed));
        assert_eq!(changed.rules[0].min_age_days, 4);
        fs::write(root.join("extra.yaml"), source.replace("first", "second")).unwrap();
        assert_eq!(RulePack::load_many_shared(&dirs).unwrap().rules.len(), 2);
        fs::remove_file(root.join("extra.yaml")).unwrap();
        assert_eq!(RulePack::load_many_shared(&dirs).unwrap().rules.len(), 1);
        fs::write(&path, "rules: [invalid").unwrap();
        assert!(RulePack::load_many_shared(&dirs).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn persisted_catalog_checks_key_payload_and_recovers_by_replacement() {
        let root = std::env::temp_dir().join(format!("space-binary-cache-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("rules.yaml"),
            "rules:\n- id: cache\n  name: Cache\n  description: fixture\n  paths: [/tmp/cache]\n",
        )
        .unwrap();
        let pack = RulePack::load_many(&[root.clone()]).unwrap();
        let key = blake3::hash(b"source version one");
        let path = root.join("catalog.bin");
        write_catalog_cache(&path, &key, &pack).unwrap();
        let restored = read_catalog_cache(&path, &key).unwrap();
        assert_eq!(
            serde_json::to_value(&pack).unwrap(),
            serde_json::to_value(&restored).unwrap()
        );
        assert!(read_catalog_cache(&path, &blake3::hash(b"source version two")).is_none());
        let mut bytes = fs::read(&path).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        fs::write(&path, bytes).unwrap();
        assert!(read_catalog_cache(&path, &key).is_none());
        fs::write(&path, b"truncated").unwrap();
        assert!(read_catalog_cache(&path, &key).is_none());
        write_catalog_cache(&path, &key, &pack).unwrap();
        assert!(read_catalog_cache(&path, &key).is_some());
        fs::remove_dir_all(root).unwrap();
    }

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
