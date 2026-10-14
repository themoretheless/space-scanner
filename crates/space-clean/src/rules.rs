use anyhow::{bail, Context, Result};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::SystemTime;

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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
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
    /// Effective labels for filtering / bulk prefs (`browser`, `electron`, `npm`, …).
    /// After [`crate::tags::enrich_rule_tags`] this holds the explicit YAML tags
    /// merged with [`Rule::inferred_tags`]; YAML may leave it empty.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Tags derived from id / category / paths at load time. Kept apart from the
    /// explicit tags so a persisted catalog can re-infer instead of freezing them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inferred_tags: Vec<String>,
}

impl Rule {
    /// Report-only rules are listed but never cleaned.
    pub fn is_report_only(&self) -> bool {
        self.report_only || self.risk == Risk::ReportOnly
    }
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

/// Shared `directories` handle for cache / config locations of this product.
pub(crate) fn app_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("com", "themoretheless", "SpaceScanner")
}

/// Recursively collect `*.yaml` / `*.yml` files under `root`, sorted, without
/// following symlinks. Shared by the rule and protection loaders; `space-tools`
/// can reuse it for repository maintenance.
pub fn yaml_files_under(root: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| x.eq_ignore_ascii_case("yaml") || x.eq_ignore_ascii_case("yml"))
                .unwrap_or(false)
        })
        .map(|entry| entry.into_path())
        .collect();
    files.sort();
    files
}

/// Cleanup rule files: every YAML except `_*.yaml` drafts / protection packs
/// and `README.yaml`.
fn collect_rule_files(dir: &Path) -> Vec<PathBuf> {
    yaml_files_under(dir)
        .into_iter()
        .filter(|path| {
            !path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with('_') || n.eq_ignore_ascii_case("README.yaml"))
                .unwrap_or(false)
        })
        .collect()
}

/// Unique canonical rule files across all directories, in deterministic order.
fn unique_rule_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen_dirs = HashSet::new();
    let mut seen_files = HashSet::new();
    let mut files = Vec::new();
    for dir in dirs {
        let canon_dir = dir.canonicalize().unwrap_or_else(|_| dir.clone());
        if !seen_dirs.insert(canon_dir) || !dir.is_dir() {
            continue;
        }
        for file in collect_rule_files(dir) {
            let file = file.canonicalize().unwrap_or(file);
            if seen_files.insert(file.clone()) {
                files.push(file);
            }
        }
    }
    files
}

// ---------------------------------------------------------------------------
// Source fingerprint: stat fast path, content hash only when (len, mtime) moved.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceEntry {
    path: PathBuf,
    digest: [u8; 32],
}

type SourceFingerprint = Vec<SourceEntry>;

type StampTable = HashMap<PathBuf, (FileStamp, [u8; 32])>;

/// In-process (and persisted) stat → digest memo so unchanged files are never
/// re-read. Fingerprints compare digests, so same-length edits with a preserved
/// mtime are the one case that goes unnoticed; this is the documented trade-off
/// for not reading hundreds of megabytes on every call.
static STAMPS: Mutex<Option<StampTable>> = Mutex::new(None);

fn stamp_cache_path() -> Option<PathBuf> {
    Some(app_dirs()?.cache_dir().join("catalog").join("stamps-v1.bin"))
}

fn read_stamp_table(path: &Path) -> Option<StampTable> {
    use bincode::Options;
    let bytes = fs::read(path).ok()?;
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

fn write_stamp_table(path: &Path, table: &StampTable) -> Result<()> {
    let payload = bincode::serialize(table)?;
    let mut bytes = Vec::with_capacity(payload.len() + 32);
    bytes.extend_from_slice(blake3::hash(&payload).as_bytes());
    bytes.extend_from_slice(&payload);
    write_atomic(path, &bytes)
}

/// Write `bytes` to `path` through a temporary sibling and a rename.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    fs::create_dir_all(path.parent().context("atomic write: no parent")?)?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn file_stamp(path: &Path) -> Result<FileStamp> {
    let meta = fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    Ok(FileStamp {
        len: meta.len(),
        modified: meta.modified().ok(),
    })
}

fn source_fingerprint(dirs: &[PathBuf]) -> Result<SourceFingerprint> {
    let files = unique_rule_files(dirs);
    let stamps: Vec<FileStamp> = files
        .iter()
        .map(|file| file_stamp(file))
        .collect::<Result<_>>()?;

    let mut guard = STAMPS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let table = guard.get_or_insert_with(|| {
        stamp_cache_path()
            .and_then(|path| read_stamp_table(&path))
            .unwrap_or_default()
    });

    let misses: Vec<usize> = files
        .iter()
        .zip(&stamps)
        .enumerate()
        .filter(|(_, (file, stamp))| table.get(*file).map(|(seen, _)| seen) != Some(stamp))
        .map(|(index, _)| index)
        .collect();
    let hashed: Vec<Result<[u8; 32]>> = misses
        .par_iter()
        .map(|&index| {
            let bytes = fs::read(&files[index])
                .with_context(|| format!("read {}", files[index].display()))?;
            Ok(*blake3::hash(&bytes).as_bytes())
        })
        .collect();
    for (index, digest) in misses.iter().zip(hashed) {
        table.insert(files[*index].clone(), (stamps[*index].clone(), digest?));
    }
    if !misses.is_empty() {
        if let Some(path) = stamp_cache_path() {
            let _ = write_stamp_table(&path, table);
        }
    }

    Ok(files
        .into_iter()
        .map(|path| {
            let digest = table[&path].1;
            SourceEntry { path, digest }
        })
        .collect())
}

// Include parser/enrichment sources so changes to defaults or inferred tags invalidate
// persisted catalogs even when the application version has not changed.
fn catalog_cache_path(fingerprint: &SourceFingerprint) -> Option<(PathBuf, blake3::Hash)> {
    let base = app_dirs()?;
    let mut hash = blake3::Hasher::new();
    let mut identity = blake3::Hasher::new();
    hash.update(b"catalog-bincode-v2");
    hash.update(include_bytes!("rules.rs"));
    hash.update(include_bytes!("tags.rs"));
    for entry in fingerprint {
        let path = bincode::serialize(&entry.path).ok()?;
        hash.update(&(path.len() as u64).to_le_bytes());
        hash.update(&path);
        identity.update(&(path.len() as u64).to_le_bytes());
        identity.update(&path);
        hash.update(&entry.digest);
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
    let payload = bincode::serialize(pack)?;
    let mut bytes = Vec::with_capacity(payload.len() + 64);
    bytes.extend_from_slice(key.as_bytes());
    bytes.extend_from_slice(blake3::hash(&payload).as_bytes());
    bytes.extend_from_slice(&payload);
    write_atomic(path, &bytes)
}

static SHARED_PACK: Mutex<Option<(SourceFingerprint, Arc<RulePack>)>> = Mutex::new(None);

// ---------------------------------------------------------------------------
// Rule index: built once per shared pack, looked up by Arc identity.
// ---------------------------------------------------------------------------

/// Lookup tables over a [`RulePack`] so id / category / tag queries do not
/// scan the whole catalog. Obtain through [`RulePack::index`].
#[derive(Debug, Default)]
pub struct RuleIndex {
    sorted_by_id: bool,
    by_id: HashMap<String, usize>,
    by_category: BTreeMap<String, Vec<usize>>,
    /// tag → (rules, host rules)
    tag_counts: BTreeMap<String, (usize, usize)>,
    host: Vec<usize>,
    host_categories: usize,
    host_kinds: usize,
}

impl RuleIndex {
    pub fn build(pack: &RulePack) -> Self {
        let rules = &pack.rules;
        let sorted_by_id = rules.windows(2).all(|pair| pair[0].id < pair[1].id);
        let by_id = if sorted_by_id {
            HashMap::new()
        } else {
            rules
                .iter()
                .enumerate()
                .map(|(index, rule)| (rule.id.clone(), index))
                .collect()
        };
        let mut by_category: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut tag_counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        let mut host = Vec::new();
        let mut host_categories = HashSet::new();
        let mut host_kinds = HashSet::new();
        for (index, rule) in rules.iter().enumerate() {
            let on_host = rule.platform.matches_host();
            by_category
                .entry(rule.category.clone())
                .or_default()
                .push(index);
            for tag in &rule.tags {
                let counts = tag_counts.entry(tag.clone()).or_insert((0, 0));
                counts.0 += 1;
                counts.1 += usize::from(on_host);
            }
            if on_host {
                host.push(index);
                host_categories.insert(rule.category.as_str());
                host_kinds.insert(rule.kind);
            }
        }
        Self {
            sorted_by_id,
            by_id,
            by_category,
            tag_counts,
            host,
            host_categories: host_categories.len(),
            host_kinds: host_kinds.len(),
        }
    }

    pub fn position(&self, pack: &RulePack, id: &str) -> Option<usize> {
        if self.sorted_by_id {
            pack.rules
                .binary_search_by(|rule| rule.id.as_str().cmp(id))
                .ok()
        } else {
            self.by_id.get(id).copied()
        }
    }

    /// Catalog positions of rules whose platform matches this host.
    pub fn host_positions(&self) -> &[usize] {
        &self.host
    }

    pub fn category_positions(&self, category: &str) -> &[usize] {
        self.by_category
            .get(category)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn has_category(&self, category: &str) -> bool {
        self.by_category.contains_key(category)
    }

    pub fn has_tag(&self, tag: &str) -> bool {
        self.tag_counts.contains_key(tag)
    }

    /// Distinct tags among rules for this host.
    pub fn host_tag_count(&self) -> usize {
        self.tag_counts
            .values()
            .filter(|(_, host)| *host > 0)
            .count()
    }

    pub fn host_category_count(&self) -> usize {
        self.host_categories
    }

    pub fn host_kind_count(&self) -> usize {
        self.host_kinds
    }
}

type IndexTable = Vec<(Weak<RulePack>, Arc<RuleIndex>)>;
static INDEXES: Mutex<IndexTable> = Mutex::new(Vec::new());

impl RulePack {
    /// Index for this shared pack, built on first use and cached for the
    /// lifetime of the `Arc`. The cache holds a `Weak`, so the allocation
    /// address cannot be reused by another pack while the entry exists.
    pub fn index(self: &Arc<Self>) -> Arc<RuleIndex> {
        let mut table = INDEXES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        table.retain(|(weak, _)| weak.strong_count() > 0);
        if let Some((_, index)) = table
            .iter()
            .find(|(weak, _)| std::ptr::eq(weak.as_ptr(), Arc::as_ptr(self)))
        {
            return Arc::clone(index);
        }
        let index = Arc::new(RuleIndex::build(self));
        table.push((Arc::downgrade(self), Arc::clone(&index)));
        index
    }

    /// Share immutable catalogs while detecting edits, additions and removals.
    /// Files are re-read only when their size or mtime changed.
    pub fn load_many_shared(dirs: &[PathBuf]) -> Result<Arc<Self>> {
        let fingerprint = source_fingerprint(dirs)?;
        {
            let cache = SHARED_PACK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some((previous, pack)) = cache.as_ref() {
                if *previous == fingerprint {
                    return Ok(Arc::clone(pack));
                }
            }
        }
        // Parse without holding the lock so concurrent readers are not blocked.
        let disk_path = catalog_cache_path(&fingerprint);
        let cached = disk_path
            .as_ref()
            .and_then(|(path, key)| read_catalog_cache(path, key));
        let disk_hit = cached.is_some();
        let pack = Arc::new(cached.map(Ok).unwrap_or_else(|| Self::load_many(dirs))?);
        let _ = pack.index();
        // Do not cache a catalog if its files changed while it was being parsed.
        let mut cache = SHARED_PACK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
        let sources = unique_rule_files(dirs);
        let parsed: Vec<Result<Vec<Rule>>> = sources
            .par_iter()
            .map(|file| parse_rule_file(file))
            .collect();

        let mut rules: Vec<Rule> = Vec::new();
        let mut ids: HashSet<String> = HashSet::new();
        for (file, parsed) in sources.iter().zip(parsed) {
            for rule in parsed? {
                if !ids.insert(rule.id.clone()) {
                    bail!("{}: duplicate rule id '{}'", file.display(), rule.id);
                }
                rules.push(rule);
            }
        }
        if rules.is_empty() {
            anyhow::bail!("no rules loaded from {}", path_list(dirs));
        }
        rules.par_sort_unstable_by(|a, b| a.id.cmp(&b.id));
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

    /// Loaded packs are sorted by id, so try a binary search first; a hit is
    /// correct for any order, and only a miss on an unsorted pack falls back
    /// to a linear scan.
    pub fn get(&self, id: &str) -> Option<&Rule> {
        if let Ok(index) = self
            .rules
            .binary_search_by(|rule| rule.id.as_str().cmp(id))
        {
            if self.rules[index].id == id {
                return Some(&self.rules[index]);
            }
        }
        self.rules.iter().find(|r| r.id == id)
    }
}

fn parse_rule_file(file: &Path) -> Result<Vec<Rule>> {
    let text = fs::read_to_string(file).with_context(|| format!("read {}", file.display()))?;
    let parsed: RuleFile =
        serde_yaml::from_str(&text).with_context(|| format!("parse {}", file.display()))?;
    let mut rules = parsed.rules;
    for rule in &mut rules {
        rule.id = rule.id.trim().to_string();
        rule.name = rule.name.trim().to_string();
        rule.category = rule.category.trim().to_string();
        validate_rule(rule, file)?;
        enrich_rule_kind(rule);
        crate::tags::enrich_rule_tags(rule);
    }
    Ok(rules)
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
    home: PathBuf,
    /// `$NAME` variables, case-sensitive like a POSIX shell.
    dollar: HashMap<String, PathBuf>,
    /// `%NAME%` variables, upper-cased like the Windows environment.
    percent: HashMap<String, PathBuf>,
}

impl PathTemplateExpander {
    pub(crate) fn new() -> Option<Self> {
        let base_dirs = directories::BaseDirs::new()?;
        let home = base_dirs.home_dir().to_path_buf();
        let mut dollar: HashMap<String, PathBuf> = HashMap::new();
        let mut percent: HashMap<String, PathBuf> = HashMap::new();

        dollar.insert("HOME".into(), home.clone());
        dollar.insert("TMPDIR".into(), std::env::temp_dir());
        dollar.insert("TEMP".into(), std::env::temp_dir());
        dollar.insert("TMP".into(), std::env::temp_dir());
        let cache = base_dirs.cache_dir().to_path_buf();
        dollar.insert("XDG_CACHE_HOME".into(), cache.clone());
        dollar.insert("CACHE".into(), cache);
        let data_local = base_dirs.data_local_dir().to_path_buf();
        dollar.insert("XDG_DATA_HOME".into(), data_local.clone());
        percent.insert("LOCALAPPDATA".into(), data_local.clone());
        dollar.insert("LOCALAPPDATA".into(), data_local);
        let data = base_dirs.data_dir().to_path_buf();
        dollar.insert("XDG_CONFIG_HOME".into(), base_dirs.config_dir().to_path_buf());
        percent.insert("APPDATA".into(), data.clone());
        dollar.insert("APPDATA".into(), data);
        for (name, var) in [
            ("USERPROFILE", "USERPROFILE"),
            ("WINDIR", "WINDIR"),
            ("SYSTEMROOT", "SystemRoot"),
            ("SYSTEMDRIVE", "SystemDrive"),
            ("PROGRAMDATA", "PROGRAMDATA"),
            ("PROGRAMFILES(X86)", "ProgramFiles(x86)"),
        ] {
            if let Some(value) = std::env::var_os(var) {
                percent.insert(name.into(), PathBuf::from(value));
            }
        }
        if let Ok(temp) = std::env::var("TEMP").or_else(|_| std::env::var("TMP")) {
            let t = PathBuf::from(temp);
            percent.insert("TEMP".into(), t.clone());
            percent.insert("TMP".into(), t);
        }
        if let Ok(pf) = std::env::var("ProgramFiles").or_else(|_| std::env::var("PROGRAMFILES")) {
            percent.insert("PROGRAMFILES".into(), PathBuf::from(pf));
        }
        // Common Windows defaults when env is missing (path still platform-filtered).
        #[cfg(windows)]
        {
            if std::env::var_os("LOCALAPPDATA").is_none() {
                percent.insert("LOCALAPPDATA".into(), home.join("AppData").join("Local"));
                percent.insert("APPDATA".into(), home.join("AppData").join("Roaming"));
            }
            if std::env::var_os("TEMP").is_none() {
                let t = home.join("AppData").join("Local").join("Temp");
                percent.insert("TEMP".into(), t.clone());
                percent.insert("TMP".into(), t);
            }
        }

        Some(Self {
            home,
            dollar,
            percent,
        })
    }

    /// Tokenize `$NAME` / `%NAME%` on identifier boundaries, expand `~` only as
    /// the first path component, and reject anything that is not absolute.
    pub(crate) fn expand(&self, template: &str) -> Option<PathBuf> {
        let template = template.trim();
        let mut out = String::with_capacity(template.len() + 32);
        let bytes = template.as_bytes();
        let mut i = 0;
        if bytes.first() == Some(&b'~') && matches!(bytes.get(1), None | Some(b'/') | Some(b'\\')) {
            out.push_str(&self.home.to_string_lossy());
            i = 1;
        }
        while i < bytes.len() {
            match bytes[i] {
                b'$' => {
                    let start = i + 1;
                    let mut end = start;
                    while end < bytes.len() && is_identifier_byte(bytes[end]) {
                        end += 1;
                    }
                    if end == start {
                        return None;
                    }
                    let value = self.dollar.get(&template[start..end])?;
                    out.push_str(&value.to_string_lossy());
                    i = end;
                }
                b'%' => {
                    let start = i + 1;
                    let end = template[start..]
                        .find(['%', '/', '\\'])
                        .map(|offset| start + offset)?;
                    if bytes[end] != b'%' || end == start {
                        return None;
                    }
                    let name = template[start..end].to_ascii_uppercase();
                    let value = self.percent.get(&name)?;
                    out.push_str(&value.to_string_lossy());
                    i = end + 1;
                }
                _ => {
                    let ch = template[i..].chars().next()?;
                    out.push(ch);
                    i += ch.len_utf8();
                }
            }
        }
        let path = PathBuf::from(out);
        if !path.is_absolute() {
            return None;
        }
        Some(path)
    }
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
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
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        let path = root.join("rules.yaml");
        let source = "rules:\n- id: first\n  name: first\n  description: fixture\n  paths: [/tmp/cache]\n  min_age_days: 3\n";
        fs::write(&path, source).unwrap();
        let dirs = [root.to_path_buf()];
        let first = RulePack::load_many_shared(&dirs).unwrap();
        let again = RulePack::load_many_shared(&dirs).unwrap();
        assert!(Arc::ptr_eq(&first, &again));
        assert!(Arc::ptr_eq(&first.index(), &again.index()));
        // Same-length edit: the stat fast path notices it through the mtime.
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, source.replace("days: 3", "days: 4")).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(modified + std::time::Duration::from_secs(5)),
            )
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
    }

    #[test]
    fn persisted_catalog_checks_key_payload_and_recovers_by_replacement() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        fs::write(
            root.join("rules.yaml"),
            "rules:\n- id: cache\n  name: Cache\n  description: fixture\n  paths: [/tmp/cache]\n",
        )
        .unwrap();
        let pack = RulePack::load_many(std::slice::from_ref(&root.to_path_buf())).unwrap();
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
    }

    #[test]
    fn stamp_table_round_trips_and_rejects_corruption() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("stamps.bin");
        let mut table = StampTable::new();
        table.insert(
            PathBuf::from("/x/a.yaml"),
            (
                FileStamp {
                    len: 3,
                    modified: Some(SystemTime::UNIX_EPOCH),
                },
                [7; 32],
            ),
        );
        write_stamp_table(&path, &table).unwrap();
        assert_eq!(read_stamp_table(&path).unwrap(), table);
        let mut bytes = fs::read(&path).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        fs::write(&path, bytes).unwrap();
        assert!(read_stamp_table(&path).is_none());
    }

    #[test]
    fn parallel_load_is_deterministic_and_rejects_duplicates() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        for (file, id) in [("b.yaml", "zeta"), ("a.yaml", "alpha"), ("c.yaml", "mid")] {
            fs::write(
                root.join(file),
                format!("rules:\n- id: {id}\n  name: {id}\n  description: fixture\n  paths: [/tmp/{id}]\n"),
            )
            .unwrap();
        }
        let pack = RulePack::load_many(&[root.to_path_buf()]).unwrap();
        let ids: Vec<_> = pack.rules.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["alpha", "mid", "zeta"]);
        assert_eq!(pack.sources.len(), 3);
        assert!(pack.get("mid").is_some());
        assert!(pack.get("missing").is_none());
        // Passing the directory twice (also as a relative-ish alias) does not duplicate.
        let twice = RulePack::load_many(&[root.to_path_buf(), root.join(".")]).unwrap();
        assert_eq!(twice.rules.len(), 3);
        fs::write(
            root.join("d.yaml"),
            "rules:\n- id: alpha\n  name: dup\n  description: fixture\n  paths: [/tmp/x]\n",
        )
        .unwrap();
        let err = RulePack::load_many(&[root.to_path_buf()]).unwrap_err();
        assert!(err.to_string().contains("duplicate rule id 'alpha'"));
    }

    #[test]
    fn index_resolves_ids_categories_and_host_counts() {
        let rules: Vec<Rule> = serde_json::from_value(serde_json::json!([
            {"id": "b", "name": "B", "paths": ["/tmp/b"], "category": "logs", "tags": ["x"]},
            {"id": "a", "name": "A", "paths": ["/tmp/a"], "category": "cache", "tags": ["x", "y"]},
            {"id": "c", "name": "C", "paths": ["/tmp/c"], "category": "cache",
             "platform": if cfg!(windows) { "linux" } else { "windows" }, "tags": ["z"]}
        ]))
        .unwrap();
        let unsorted = Arc::new(RulePack {
            rules: rules.clone(),
            sources: vec![],
        });
        let index = unsorted.index();
        assert!(!index.sorted_by_id);
        assert_eq!(index.position(&unsorted, "a"), Some(1));
        assert_eq!(index.position(&unsorted, "zz"), None);
        assert_eq!(index.category_positions("cache"), &[1, 2]);
        assert_eq!(index.host_positions(), &[0, 1]);
        assert!(index.has_tag("z"));
        assert_eq!(index.host_tag_count(), 2);
        assert_eq!(index.host_category_count(), 2);
        assert!(Arc::ptr_eq(&index, &unsorted.index()));
        let mut sorted_rules = rules;
        sorted_rules.sort_by(|a, b| a.id.cmp(&b.id));
        let sorted = Arc::new(RulePack {
            rules: sorted_rules,
            sources: vec![],
        });
        assert!(sorted.index().sorted_by_id);
        assert_eq!(sorted.index().position(&sorted, "c"), Some(2));
        assert_eq!(sorted.get("c").map(|r| r.id.as_str()), Some("c"));
    }

    #[test]
    fn expands_home() {
        let p = expand_path_template("$HOME/Library/Caches").unwrap();
        assert!(p.to_string_lossy().contains("Library"));
        assert_eq!(expand_path_template("~/x"), expand_path_template("$HOME/x"));
    }

    #[test]
    fn expand_respects_identifier_boundaries_and_rejects_relative_results() {
        let expander = PathTemplateExpander::new().unwrap();
        let home = expander.home.to_string_lossy().into_owned();
        // `$HOMEBREW_PREFIX` is one unknown variable, not `$HOME` + `BREW_PREFIX`.
        assert_eq!(expander.expand("$HOMEBREW_PREFIX/Cellar"), None);
        assert_eq!(
            expander.expand("$HOME/.cache"),
            Some(PathBuf::from(format!("{home}/.cache")))
        );
        assert_eq!(
            expander.expand("${HOME}/.cache"),
            None,
            "brace syntax is not supported and must not be silently kept"
        );
        // `~` only as the first component; DOS short names stay literal.
        let short = expander.expand("$HOME/PROGRA~1/cache").unwrap();
        assert!(short.to_string_lossy().ends_with("/PROGRA~1/cache"));
        assert_eq!(expander.expand("~user/x"), None);
        // Relative templates are refused even without variables.
        assert_eq!(expander.expand("cache"), None);
        assert_eq!(expander.expand("./cache"), None);
        // `%NAME%` is case-insensitive and must be closed on the same component.
        assert_eq!(
            expander.expand("%localappdata%/x"),
            expander.expand("%LOCALAPPDATA%/x")
        );
        assert_eq!(expander.expand("%LOCALAPPDATA/x"), None);
        assert_eq!(expander.expand("%NO_SUCH_VARIABLE_XYZ%/x"), None);
        assert_eq!(expander.expand("$"), None);
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
            inferred_tags: vec![],
        };
        let err = validate_rule(&rule, &PathBuf::from("rules.yaml")).unwrap_err();
        assert!(err.to_string().contains("missing non-empty id"));

        rule.id = "valid-id".into();
        rule.name = "Valid name".into();
        rule.category = "caches".into();
        rule.paths = vec!["$HOME/.cache/example".into()];
        let err = validate_rule(&rule, &PathBuf::from("rules.yaml")).unwrap_err();
        assert!(err.to_string().contains("missing non-empty description"));
        assert!(!rule.is_report_only());
        rule.risk = Risk::ReportOnly;
        assert!(rule.is_report_only());
    }
}
