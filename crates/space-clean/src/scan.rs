use crate::rules::{DeleteMode, PathTemplateExpander, Risk, Rule, RulePack};
use crate::safety::{
    canonical_root, is_never_touch_normalized, is_safe_rule_root_normalized, mtime_is_old_enough,
    newest_mtime,
};
use anyhow::Result;
use globset::{Candidate, Glob, GlobSet, GlobSetBuilder};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use walkdir::WalkDir;

#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    pub categories: Vec<String>,
    pub include_disabled: bool,
    pub min_age_override: Option<u64>,
    /// Optional per-rule min_age from [`crate::manager::RulePrefs`].
    pub prefs_min_age: std::collections::BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub rule_id: String,
    pub category: String,
    pub risk: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub is_dir: bool,
    pub report_only: bool,
    /// Canonical rule root the path was found under. Cleaning refuses a path
    /// that no longer resolves inside it. `None` for findings built by hand.
    #[serde(default)]
    pub root: Option<PathBuf>,
    /// Age threshold the finding was judged with; re-checked before deletion.
    #[serde(default)]
    pub min_age_days: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanReport {
    pub findings: Vec<Finding>,
    /// Deletable bytes with nested findings counted once.
    pub total_bytes: u64,
    pub rules_matched: usize,
    pub rules_scanned: usize,
    pub skipped_roots: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct JunkScanProgress {
    pub rules_done: usize,
    pub rules_total: usize,
    pub roots_done: usize,
    pub roots_total: usize,
    pub entries_seen: u64,
    pub files_seen: u64,
    pub folders_seen: u64,
    pub findings_found: usize,
    pub files_found: usize,
    pub folders_found: usize,
    pub bytes_found: u64,
    pub current_rule: String,
    pub current_path: PathBuf,
}

pub fn scan(pack: &RulePack, opts: &ScanOptions) -> Result<ScanReport> {
    let rules = pack.active_rules(&opts.categories, opts.include_disabled);
    scan_rules(rules, opts)
}

/// Ids of the rules with at least one root that exists on this machine.
/// One `stat` per root and no traversal, so the rules window can call it
/// while open; a rule absent here would find nothing in a scan either.
pub fn rules_present_on_disk<'a>(
    rules: impl IntoIterator<Item = &'a Rule>,
) -> std::collections::HashSet<String> {
    let Some(expander) = PathTemplateExpander::new() else {
        return Default::default();
    };
    rules
        .into_iter()
        .filter(|rule| {
            rule.paths
                .iter()
                .filter_map(|template| expander.expand(template))
                .any(|root| root.symlink_metadata().is_ok())
        })
        .map(|rule| rule.id.clone())
        .collect()
}

/// Scan an explicit rule list (e.g. from [`crate::manager::RuleManager::scan_rules`]).
pub fn scan_rules<'a>(
    rules: impl IntoIterator<Item = &'a Rule>,
    opts: &ScanOptions,
) -> Result<ScanReport> {
    scan_rules_impl(
        rules,
        opts,
        ProgressReporter::<fn(JunkScanProgress)>::new(None),
    )
}

/// Scan rules while periodically reporting rule/root/entry progress.
pub fn scan_rules_with_progress<'a>(
    rules: impl IntoIterator<Item = &'a Rule>,
    opts: &ScanOptions,
    on_progress: impl FnMut(JunkScanProgress),
) -> Result<ScanReport> {
    scan_rules_impl(rules, opts, ProgressReporter::new(Some(on_progress)))
}

struct ProgressReporter<F> {
    callback: Option<F>,
    last: std::time::Instant,
}

impl<F: FnMut(JunkScanProgress)> ProgressReporter<F> {
    fn new(callback: Option<F>) -> Self {
        Self {
            callback,
            last: std::time::Instant::now(),
        }
    }

    fn emit(&mut self, progress: &JunkScanProgress) {
        if self.callback.is_some() && self.last.elapsed() >= std::time::Duration::from_millis(50) {
            self.force(progress);
        }
    }

    fn force(&mut self, progress: &JunkScanProgress) {
        if let Some(callback) = self.callback.as_mut() {
            callback(progress.clone());
            self.last = std::time::Instant::now();
        }
    }
}

/// Accumulated results shared by ordinary and grouped rule traversal.
#[derive(Default)]
struct ScanResults {
    findings: Vec<Finding>,
    skipped_roots: Vec<String>,
    matched_rules: BTreeSet<String>,
}

impl ScanResults {
    fn into_report(mut self, rules_scanned: usize) -> ScanReport {
        self.findings.sort_by(|a, b| a.path.cmp(&b.path));
        self.findings.dedup_by(|a, b| a.path == b.path);
        // A finding inside another finding is deleted with its parent, so it
        // must not be counted twice.
        let deletable: Vec<&Finding> = self
            .findings
            .iter()
            .filter(|finding| !finding.report_only)
            .collect();
        let total_bytes = crate::clean::coalesce_nested_paths(deletable, |f| f.path.as_path())
            .iter()
            .fold(0u64, |total, finding| total.saturating_add(finding.bytes));
        ScanReport {
            findings: self.findings,
            total_bytes,
            rules_matched: self.matched_rules.len(),
            rules_scanned,
            skipped_roots: self.skipped_roots,
        }
    }
}

/// Compiled matchers of one rule; identical pattern lists share one `GlobSet`.
#[derive(Clone)]
struct RuleGlobs {
    include: Arc<GlobSet>,
    exclude: Arc<GlobSet>,
    exclude_dirs: Arc<GlobSet>,
    required_ancestor: Arc<GlobSet>,
}

impl RuleGlobs {
    fn compile(cache: &mut GlobCache, rule: &Rule) -> Result<Self> {
        Ok(Self {
            include: cache.get(&rule.include_globs)?,
            exclude: cache.get(&rule.exclude_globs)?,
            exclude_dirs: cache.directories(&rule.exclude_globs)?,
            required_ancestor: cache.get(&rule.required_ancestor_globs)?,
        })
    }
}

/// Everything that decides which paths a traversal yields and how findings
/// are attributed. Compiled glob sets are shared per unique pattern list, so
/// their addresses identify the patterns for the duration of a scan.
#[derive(Hash, PartialEq, Eq)]
struct TraversalKey {
    root: PathBuf,
    min_age: u64,
    include: usize,
    exclude: usize,
    required_ancestor: usize,
    match_directories: bool,
    max_depth: usize,
    delete_mode: DeleteMode,
    report_only: bool,
    risk: Risk,
    category: String,
}

impl TraversalKey {
    fn new(root: &Path, rule: &Rule, min_age: u64, globs: &RuleGlobs) -> Self {
        Self {
            root: root.to_path_buf(),
            min_age,
            include: Arc::as_ptr(&globs.include) as usize,
            exclude: Arc::as_ptr(&globs.exclude) as usize,
            required_ancestor: Arc::as_ptr(&globs.required_ancestor) as usize,
            match_directories: rule.match_directories,
            max_depth: rule.max_depth,
            delete_mode: rule.delete_mode,
            report_only: rule.report_only,
            risk: rule.risk,
            category: rule.category.clone(),
        }
    }
}

/// Memo of "does this directory contain a marker" per (marker set, directory),
/// valid for one root traversal.
#[derive(Default)]
struct MarkerCache(HashMap<(usize, PathBuf), bool>);

impl MarkerCache {
    fn dir_has_marker(&mut self, dir: &Path, markers: &GlobSet) -> bool {
        let key = (markers as *const GlobSet as usize, dir.to_path_buf());
        if let Some(&known) = self.0.get(&key) {
            return known;
        }
        let found = std::fs::read_dir(dir).ok().is_some_and(|entries| {
            entries.filter_map(|entry| entry.ok()).any(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| markers.is_match(name))
            })
        });
        self.0.insert(key, found);
        found
    }
}

fn min_age_for(opts: &ScanOptions, rule: &Rule) -> u64 {
    opts.min_age_override
        .or(opts.prefs_min_age.get(&rule.id).copied())
        .unwrap_or(rule.min_age_days)
}

/// Resolve a rule root once: `Ok(None)` when it does not exist (silently
/// skipped), `Err` with a reason when it exists but cannot be used.
fn resolve_root(
    expander: Option<&PathTemplateExpander>,
    template: &str,
) -> std::result::Result<Option<PathBuf>, String> {
    let Some(expanded_root) = expander.and_then(|expander| expander.expand(template)) else {
        return Err(format!("{template} (unexpanded)"));
    };
    let root = match canonical_root(&expanded_root) {
        Ok(Some(root)) => root,
        Ok(None) => return Ok(None),
        Err(error) => {
            return Err(format!(
                "{} (cannot normalize: {error})",
                expanded_root.display()
            ))
        }
    };
    if !is_safe_rule_root_normalized(&root) {
        return Err(format!("{} (blocked by safety)", root.display()));
    }
    Ok(Some(root))
}

fn scan_rules_impl<'a>(
    rules: impl IntoIterator<Item = &'a Rule>,
    opts: &ScanOptions,
    mut on_progress: ProgressReporter<impl FnMut(JunkScanProgress)>,
) -> Result<ScanReport> {
    let rules: Vec<&Rule> = rules.into_iter().collect();
    let rules_scanned = rules.len();
    let expander = PathTemplateExpander::new();
    let mut progress = JunkScanProgress {
        rules_total: rules_scanned,
        roots_total: rules.iter().map(|r| r.paths.len()).sum(),
        ..Default::default()
    };
    let mut results = ScanResults::default();
    if let Some(error) = crate::protection::load_error() {
        results.skipped_roots.push(format!(
            "protection pack unavailable, every path treated as protected: {error}"
        ));
    }
    let mut glob_cache = GlobCache::default();
    // Reuse identical traversal predicates, preserving rule order and first-match
    // attribution. Tags and labels may differ without changing filesystem work.
    let mut traversals: HashMap<TraversalKey, std::ops::Range<usize>> = HashMap::new();

    on_progress.force(&progress);
    let grouped_rule_indexes = scan_grouped_project_rules(
        &rules,
        opts,
        expander.as_ref(),
        &mut glob_cache,
        &mut results,
        &mut progress,
        &mut on_progress,
    )?;
    let ScanResults {
        findings,
        skipped_roots,
        matched_rules,
    } = &mut results;
    for (rule_index, rule) in rules.into_iter().enumerate() {
        progress.current_rule = rule.name.clone();
        progress.rules_done = rule_index;
        on_progress.emit(&progress);
        if grouped_rule_indexes.contains(&rule_index) {
            progress.rules_done = rule_index + 1;
            on_progress.emit(&progress);
            continue;
        }
        let min_age = min_age_for(opts, rule);
        let globs = RuleGlobs::compile(&mut glob_cache, rule)?;

        for template in &rule.paths {
            let root = match resolve_root(expander.as_ref(), template) {
                Ok(Some(root)) => root,
                Ok(None) => {
                    progress.roots_done += 1;
                    on_progress.emit(&progress);
                    continue;
                }
                Err(reason) => {
                    skipped_roots.push(reason);
                    progress.roots_done += 1;
                    on_progress.emit(&progress);
                    continue;
                }
            };
            progress.current_path = root.clone();
            on_progress.emit(&progress);

            let traversal_key = TraversalKey::new(&root, rule, min_age, &globs);
            if let Some(previous) = traversals.get(&traversal_key) {
                if !previous.is_empty() {
                    matched_rules.insert(rule.id.clone());
                }
                // Findings already carry the earlier rule's attribution, which
                // stable path sorting and deduplication would have retained.
                progress.roots_done += 1;
                on_progress.emit(&progress);
                continue;
            }
            let findings_start = findings.len();
            let mut markers = MarkerCache::default();
            match rule.delete_mode {
                DeleteMode::Path => {
                    if let Some(f) = consider_path(
                        &root,
                        &root,
                        rule,
                        min_age,
                        &globs,
                        &mut markers,
                        &mut progress,
                        &mut on_progress,
                    ) {
                        matched_rules.insert(rule.id.clone());
                        record_finding_progress(&f, &mut progress);
                        findings.push(f);
                    }
                }
                DeleteMode::Contents => {
                    collect_under(
                        &root,
                        rule,
                        min_age,
                        &globs,
                        &mut markers,
                        findings,
                        matched_rules,
                        &mut progress,
                        &mut on_progress,
                    )?;
                }
            }
            traversals.insert(traversal_key, findings_start..findings.len());
            progress.roots_done += 1;
            on_progress.emit(&progress);
        }
        progress.rules_done = rule_index + 1;
        on_progress.emit(&progress);
    }

    let report = results.into_report(rules_scanned);
    on_progress.force(&progress);
    Ok(report)
}

struct CompiledProjectRule<'a> {
    rule: &'a Rule,
    min_age: u64,
    globs: RuleGlobs,
}

#[derive(Default)]
struct GroupedRoot {
    rule_indexes: Vec<usize>,
    root_occurrences: usize,
}

#[allow(clippy::too_many_arguments)]
fn scan_grouped_project_rules(
    rules: &[&Rule],
    opts: &ScanOptions,
    expander: Option<&PathTemplateExpander>,
    glob_cache: &mut GlobCache,
    results: &mut ScanResults,
    progress: &mut JunkScanProgress,
    on_progress: &mut ProgressReporter<impl FnMut(JunkScanProgress)>,
) -> Result<BTreeSet<usize>> {
    let ScanResults {
        findings,
        skipped_roots,
        matched_rules,
    } = results;
    let grouped_indexes: BTreeSet<usize> = rules
        .iter()
        .enumerate()
        .filter(|(_, rule)| {
            rule.delete_mode == DeleteMode::Contents
                && rule.match_directories
                && !rule.required_ancestor_globs.is_empty()
        })
        .map(|(index, _)| index)
        .collect();
    if grouped_indexes.is_empty() {
        return Ok(BTreeSet::new());
    }

    let mut compiled = std::collections::BTreeMap::new();
    let mut roots: std::collections::BTreeMap<PathBuf, GroupedRoot> =
        std::collections::BTreeMap::new();
    for &index in &grouped_indexes {
        let rule = rules[index];
        compiled.insert(
            index,
            CompiledProjectRule {
                rule,
                min_age: min_age_for(opts, rule),
                globs: RuleGlobs::compile(glob_cache, rule)?,
            },
        );
        for template in &rule.paths {
            let root = match resolve_root(expander, template) {
                Ok(Some(root)) => root,
                Ok(None) => {
                    progress.roots_done += 1;
                    continue;
                }
                Err(reason) => {
                    skipped_roots.push(reason);
                    progress.roots_done += 1;
                    continue;
                }
            };
            let group = roots.entry(root).or_default();
            group.rule_indexes.push(index);
            group.root_occurrences += 1;
        }
    }

    progress.current_rule = format!("Project outputs ({} rules)", grouped_indexes.len());
    on_progress.emit(progress);
    let mut pending_sizes = Vec::new();
    for (root, mut group) in roots {
        group.rule_indexes.sort_unstable();
        group.rule_indexes.dedup();
        let active: Vec<&CompiledProjectRule<'_>> = group
            .rule_indexes
            .iter()
            .filter_map(|index| compiled.get(index))
            .collect();
        collect_grouped_project_root(
            &root,
            &active,
            findings,
            matched_rules,
            &mut pending_sizes,
            progress,
            on_progress,
        )?;
        progress.roots_done += group.root_occurrences;
        on_progress.emit(progress);
    }
    size_grouped_findings(findings, &pending_sizes, progress, on_progress);
    Ok(grouped_indexes)
}

fn collect_grouped_project_root(
    root: &Path,
    rules: &[&CompiledProjectRule<'_>],
    findings: &mut Vec<Finding>,
    matched_rules: &mut BTreeSet<String>,
    pending_sizes: &mut Vec<(usize, PathBuf)>,
    progress: &mut JunkScanProgress,
    on_progress: &mut ProgressReporter<impl FnMut(JunkScanProgress)>,
) -> Result<()> {
    let max_depth = rules
        .iter()
        .map(|compiled| compiled.rule.max_depth)
        .max()
        .unwrap_or(1);
    let mut walker = WalkDir::new(root)
        .follow_links(false)
        .max_depth(max_depth)
        .into_iter();
    let mut markers = MarkerCache::default();

    while let Some(entry) = walker.next() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        progress.entries_seen = progress.entries_seen.saturating_add(1);
        if progress.entries_seen.is_multiple_of(64) {
            progress.current_path = path.to_path_buf();
            on_progress.emit(progress);
        }
        if path == root || entry.file_type().is_symlink() {
            continue;
        }
        if entry.file_type().is_file() {
            progress.files_seen = progress.files_seen.saturating_add(1);
            continue;
        }
        if !entry.file_type().is_dir() {
            continue;
        }
        progress.folders_seen = progress.folders_seen.saturating_add(1);
        if !path.starts_with(root) || is_never_touch_normalized(path) {
            walker.skip_current_dir();
            continue;
        }
        let depth = entry.depth();
        let eligible: Vec<_> = rules
            .iter()
            .copied()
            .filter(|compiled| depth <= compiled.rule.max_depth)
            .collect();
        if eligible.is_empty() {
            walker.skip_current_dir();
            continue;
        }
        let rel = path.strip_prefix(root).unwrap_or(path);
        let rel = Candidate::new(rel);
        let full = Candidate::new(path);
        // Excluded directories are pruned: nothing under them is considered.
        if eligible.iter().all(|compiled| {
            matches_either(&compiled.globs.exclude_dirs, &rel, &full)
                || matches_either(&compiled.globs.exclude, &rel, &full)
        }) {
            walker.skip_current_dir();
            continue;
        }

        let mut newest: Option<Option<SystemTime>> = None;
        let matching: Vec<_> = eligible
            .into_iter()
            .filter(|compiled| !matches_either(&compiled.globs.exclude, &rel, &full))
            .filter(|compiled| {
                compiled.globs.include.is_empty()
                    || matches_either(&compiled.globs.include, &rel, &full)
            })
            .filter(|compiled| {
                has_required_ancestor_marker(
                    path,
                    root,
                    &compiled.globs.required_ancestor,
                    &mut markers,
                )
            })
            .filter(|compiled| {
                compiled.min_age == 0 || {
                    let modified = *newest.get_or_insert_with(|| {
                        let own = entry.metadata().ok().and_then(|m| m.modified().ok());
                        newest_mtime(path, own)
                    });
                    mtime_is_old_enough(modified, compiled.min_age)
                }
            })
            .collect();
        if matching.is_empty() {
            continue;
        }

        let selected = matching
            .iter()
            .copied()
            .max_by_key(|compiled| compiled.rule.risk.rank())
            .expect("non-empty matching rules");
        for compiled in &matching {
            matched_rules.insert(compiled.rule.id.clone());
        }
        let report_only = matching
            .iter()
            .any(|compiled| compiled.rule.is_report_only());
        let finding = Finding {
            rule_id: selected.rule.id.clone(),
            category: selected.rule.category.clone(),
            risk: selected.rule.risk.as_str().to_string(),
            path: path.to_path_buf(),
            bytes: 0,
            is_dir: true,
            report_only,
            root: Some(root.to_path_buf()),
            min_age_days: selected.min_age,
        };
        record_finding_progress(&finding, progress);
        pending_sizes.push((findings.len(), finding.path.clone()));
        findings.push(finding);
        walker.skip_current_dir();
    }
    Ok(())
}

fn size_grouped_findings(
    findings: &mut [Finding],
    pending: &[(usize, PathBuf)],
    progress: &mut JunkScanProgress,
    on_progress: &mut ProgressReporter<impl FnMut(JunkScanProgress)>,
) {
    if pending.is_empty() {
        return;
    }
    let workers = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(4)
        .min(pending.len());
    let chunk_size = pending.len().div_ceil(workers);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        for chunk in pending.chunks(chunk_size) {
            let tx = tx.clone();
            scope.spawn(move || {
                for (finding_index, path) in chunk {
                    let bytes = dir_size_quiet(path);
                    let _ = tx.send((*finding_index, path.clone(), bytes));
                }
            });
        }
        drop(tx);
        for (finding_index, path, bytes) in rx {
            findings[finding_index].bytes = bytes;
            if !findings[finding_index].report_only {
                progress.bytes_found = progress.bytes_found.saturating_add(bytes);
            }
            progress.current_path = path;
            on_progress.emit(progress);
        }
    });
}

fn dir_size_quiet(path: &Path) -> u64 {
    WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| entry.metadata().ok())
        .fold(0u64, |total, metadata| total.saturating_add(metadata.len()))
}

/// Patterns are tested against the root-relative path and the full path so
/// absolute globs work too.
fn matches_either(patterns: &GlobSet, rel: &Candidate<'_>, full: &Candidate<'_>) -> bool {
    patterns.is_match_candidate(rel) || patterns.is_match_candidate(full)
}

#[allow(clippy::too_many_arguments)]
fn collect_under(
    root: &Path,
    rule: &Rule,
    min_age: u64,
    globs: &RuleGlobs,
    markers: &mut MarkerCache,
    findings: &mut Vec<Finding>,
    matched_rules: &mut BTreeSet<String>,
    progress: &mut JunkScanProgress,
    on_progress: &mut ProgressReporter<impl FnMut(JunkScanProgress)>,
) -> Result<()> {
    // Prefer top-level cleanable units under the rule root (CleanMyMac/BleachBit style)
    // so sizes are not double-counted across parents and children.
    // If include_globs are set, walk deeper. Rules may opt into matching whole
    // directories (for example C# bin/obj) instead of individual files.
    let deep = !rule.include_globs.is_empty();
    let max_depth = if deep { rule.max_depth } else { 1 };

    let mut walker = WalkDir::new(root)
        .follow_links(false)
        .max_depth(max_depth)
        .into_iter();

    while let Some(entry) = walker.next() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        progress.entries_seen = progress.entries_seen.saturating_add(1);
        if progress.entries_seen.is_multiple_of(64) {
            progress.current_path = path.to_path_buf();
            on_progress.emit(progress);
        }
        if path == root {
            continue;
        }
        let file_type = entry.file_type();
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            progress.folders_seen = progress.folders_seen.saturating_add(1);
        } else if file_type.is_file() {
            progress.files_seen = progress.files_seen.saturating_add(1);
        }
        if !path.starts_with(root) || is_never_touch_normalized(path) {
            if file_type.is_dir() {
                walker.skip_current_dir();
            }
            continue;
        }
        if file_type.is_dir() {
            let rel = Candidate::new(path.strip_prefix(root).unwrap_or(path));
            let full = Candidate::new(path);
            // Excluded directories (`**/node_modules`, `**/.git/**`) are pruned.
            if matches_either(&globs.exclude_dirs, &rel, &full)
                || matches_either(&globs.exclude, &rel, &full)
            {
                walker.skip_current_dir();
                continue;
            }
            if deep {
                if rule.match_directories {
                    if let Some(f) = consider_path(
                        path,
                        root,
                        rule,
                        min_age,
                        globs,
                        markers,
                        progress,
                        on_progress,
                    ) {
                        matched_rules.insert(rule.id.clone());
                        record_finding_progress(&f, progress);
                        findings.push(f);
                        walker.skip_current_dir();
                    }
                }
                continue;
            }
        }
        if let Some(f) = consider_path(
            path,
            root,
            rule,
            min_age,
            globs,
            markers,
            progress,
            on_progress,
        ) {
            matched_rules.insert(rule.id.clone());
            record_finding_progress(&f, progress);
            findings.push(f);
        }
    }
    Ok(())
}

fn record_finding_progress(finding: &Finding, progress: &mut JunkScanProgress) {
    if finding.report_only {
        return;
    }
    progress.findings_found += 1;
    progress.bytes_found = progress.bytes_found.saturating_add(finding.bytes);
    if finding.is_dir {
        progress.folders_found += 1;
    } else {
        progress.files_found += 1;
    }
}

/// One `symlink_metadata` per candidate; directories are sized and aged in a
/// single walk.
#[allow(clippy::too_many_arguments)]
fn consider_path(
    path: &Path,
    root: &Path,
    rule: &Rule,
    min_age: u64,
    globs: &RuleGlobs,
    markers: &mut MarkerCache,
    progress: &mut JunkScanProgress,
    on_progress: &mut ProgressReporter<impl FnMut(JunkScanProgress)>,
) -> Option<Finding> {
    let meta = path.symlink_metadata().ok()?;
    // Do not follow symlinks for deletion candidates
    if meta.file_type().is_symlink() {
        return None;
    }

    let rel = Candidate::new(path.strip_prefix(root).unwrap_or(path));
    let full = Candidate::new(path);
    if matches_either(&globs.exclude, &rel, &full) {
        return None;
    }
    if !globs.include.is_empty() && !matches_either(&globs.include, &rel, &full) {
        return None;
    }
    if !has_required_ancestor_marker(path, root, &globs.required_ancestor, markers) {
        return None;
    }

    let is_dir = meta.is_dir();
    let own_modified = meta.modified().ok();
    let (bytes, modified) = if is_dir {
        let stats = dir_stats(path, progress, on_progress);
        let newest = match (own_modified, stats.newest) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        (stats.bytes, newest)
    } else {
        (meta.len(), own_modified)
    };
    if !mtime_is_old_enough(modified, min_age) {
        return None;
    }

    // Skip empty noise
    if bytes == 0 && !is_dir {
        return None;
    }

    Some(Finding {
        rule_id: rule.id.clone(),
        category: rule.category.clone(),
        risk: rule.risk.as_str().to_string(),
        path: path.to_path_buf(),
        bytes,
        is_dir,
        report_only: rule.is_report_only(),
        root: Some(root.to_path_buf()),
        min_age_days: min_age,
    })
}

fn has_required_ancestor_marker(
    path: &Path,
    root: &Path,
    markers: &GlobSet,
    cache: &mut MarkerCache,
) -> bool {
    if markers.is_empty() {
        return true;
    }
    let mut dir = path.parent();
    while let Some(current) = dir {
        if cache.dir_has_marker(current, markers) {
            return true;
        }
        if current == root {
            break;
        }
        dir = current.parent();
    }
    false
}

struct DirStats {
    bytes: u64,
    newest: Option<SystemTime>,
}

fn dir_stats(
    path: &Path,
    progress: &mut JunkScanProgress,
    on_progress: &mut ProgressReporter<impl FnMut(JunkScanProgress)>,
) -> DirStats {
    let mut stats = DirStats {
        bytes: 0,
        newest: None,
    };
    for entry in WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .skip(1)
    {
        progress.entries_seen = progress.entries_seen.saturating_add(1);
        let file_type = entry.file_type();
        if file_type.is_dir() {
            progress.folders_seen = progress.folders_seen.saturating_add(1);
        } else if file_type.is_file() {
            progress.files_seen = progress.files_seen.saturating_add(1);
        }
        if let Ok(meta) = entry.metadata() {
            if file_type.is_file() {
                stats.bytes = stats.bytes.saturating_add(meta.len());
            }
            if let Ok(modified) = meta.modified() {
                stats.newest = Some(stats.newest.map_or(modified, |n| n.max(modified)));
            }
        }
        if progress.entries_seen.is_multiple_of(128) {
            progress.current_path = entry.path().to_path_buf();
            on_progress.emit(progress);
        }
    }
    stats
}

/// Per-scan cache: share compiled patterns across rules, including empty sets.
/// Compilation still precedes root checks so malformed patterns remain errors.
#[derive(Default)]
struct GlobCache {
    patterns: HashMap<Vec<String>, Arc<GlobSet>>,
    directories: HashMap<Vec<String>, Arc<GlobSet>>,
}

impl GlobCache {
    fn get(&mut self, patterns: &[String]) -> Result<Arc<GlobSet>> {
        if let Some(compiled) = self.patterns.get(patterns) {
            return Ok(Arc::clone(compiled));
        }
        let compiled = Arc::new(build_globset(patterns)?);
        self.patterns
            .insert(patterns.to_vec(), Arc::clone(&compiled));
        Ok(compiled)
    }

    fn directories(&mut self, patterns: &[String]) -> Result<Arc<GlobSet>> {
        if let Some(compiled) = self.directories.get(patterns) {
            return Ok(Arc::clone(compiled));
        }
        let compiled = Arc::new(build_directory_exclude_globset(patterns)?);
        self.directories
            .insert(patterns.to_vec(), Arc::clone(&compiled));
        Ok(compiled)
    }
}

fn build_globset(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        builder.add(Glob::new(p)?);
    }
    Ok(builder.build()?)
}

/// `dir/**` excludes everything inside `dir`, so `dir` itself can be pruned.
fn build_directory_exclude_globset(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        if let Some(directory_pattern) = pattern.strip_suffix("/**") {
            builder.add(Glob::new(directory_pattern)?);
        }
    }
    Ok(builder.build()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{JunkKind, Platform};
    use crate::safety::normalize;
    use std::fs;

    #[test]
    fn progress_is_rate_limited_and_always_reports_empty_scan_boundaries() {
        let mut snapshots = Vec::new();
        let report = scan_rules_with_progress(std::iter::empty(), &ScanOptions::default(), |p| {
            snapshots.push(p)
        })
        .unwrap();
        assert!(report.findings.is_empty());
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots.last().unwrap().rules_done, 0);
        let mut delivered = Vec::new();
        let mut reporter =
            ProgressReporter::new(Some(|p: JunkScanProgress| delivered.push(p.rules_done)));
        let mut progress = JunkScanProgress::default();
        reporter.force(&progress);
        progress.rules_done = 1;
        reporter.emit(&progress);
        reporter.last = std::time::Instant::now() - std::time::Duration::from_millis(51);
        reporter.emit(&progress);
        progress.rules_done = 2;
        reporter.force(&progress);
        assert_eq!(delivered, [0, 1, 2]);
    }

    #[test]
    fn cached_patterns_preserve_matching_and_invalid_pattern_errors() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.log"), b"log").unwrap();
        fs::write(root.join("b.tmp"), b"tmp!").unwrap();
        fs::write(root.join("keep.log"), b"keep").unwrap();
        let mut rule: Rule = serde_yaml::from_str(
            r#"
id: logs
name: logs
description: pattern cache regression
platform: any
paths: []
min_age_days: 0
include_globs: ['**/*.log']
exclude_globs: ['**/keep.log']
"#,
        )
        .unwrap();
        rule.paths = vec![root.to_string_lossy().into_owned()];
        let mut temp = rule.clone();
        temp.id = "temp".into();
        temp.include_globs = vec!["**/*.tmp".into()];
        let report = scan_rules([&rule, &temp], &ScanOptions::default()).unwrap();
        assert_eq!(report.findings.len(), 2);
        assert_eq!(report.total_bytes, 7);
        assert_eq!(report.rules_matched, 2);
        let canonical = normalize(root).unwrap();
        assert!(report
            .findings
            .iter()
            .all(|f| f.root.as_deref() == Some(canonical.as_path()) && f.min_age_days == 0));
        let duplicate = scan_rules([&rule, &rule], &ScanOptions::default()).unwrap();
        assert_eq!(duplicate.findings.len(), 1);
        assert_eq!(duplicate.total_bytes, 3);
        // A root that does not exist is skipped silently, not reported.
        rule.paths = vec![root.join("missing").to_string_lossy().into_owned()];
        let missing = scan_rules([&rule], &ScanOptions::default()).unwrap();
        assert!(missing.findings.is_empty());
        assert!(missing.skipped_roots.is_empty());
        rule.include_globs = vec!["[".into()];
        assert!(scan_rules([&rule], &ScanOptions::default()).is_err());
        // Relative or unexpandable roots are reported.
        rule.include_globs.clear();
        rule.paths = vec!["relative/cache".into(), "$NOPE_VARIABLE/x".into()];
        let unexpanded = scan_rules([&rule], &ScanOptions::default()).unwrap();
        assert_eq!(unexpanded.skipped_roots.len(), 2);
        assert!(unexpanded.skipped_roots[0].contains("unexpanded"));
    }

    #[test]
    fn presence_needs_one_existing_root() {
        let dir = tempfile::tempdir().unwrap();
        let here = dir.path().to_string_lossy().into_owned();
        let gone = dir.path().join("missing").to_string_lossy().into_owned();
        let rule = |id: &str, paths: Vec<String>| {
            let mut rule: Rule = serde_yaml::from_str(&format!("id: {id}\nname: {id}\npaths: []\n")).unwrap();
            rule.paths = paths;
            rule
        };
        let rules = [
            rule("both", vec![gone.clone(), here.clone()]),
            rule("absent", vec![gone]),
            rule("empty", Vec::new()),
        ];
        let present = rules_present_on_disk(&rules);
        assert!(present.contains("both"));
        assert_eq!(present.len(), 1);
    }

    #[test]
    fn repeated_traversals_preserve_attribution_preferences_and_exclusions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("excluded")).unwrap();
        fs::write(root.join("a.log"), b"abc").unwrap();
        fs::write(root.join("excluded/hidden.log"), b"hidden").unwrap();
        let mut first: Rule = serde_yaml::from_str("id: first\nname: first\npaths: []\nmin_age_days: 0\ninclude_globs: ['**/*.log']\nexclude_globs: ['**/excluded/**']\n").unwrap();
        first.paths = vec![root.to_string_lossy().into_owned()];
        let mut second = first.clone();
        second.id = "second".into();
        second.tags = vec!["different-preference-tag".into()];
        let mut single_progress = JunkScanProgress::default();
        let single =
            scan_rules_with_progress([&first], &ScanOptions::default(), |p| single_progress = p)
                .unwrap();
        let mut progress = JunkScanProgress::default();
        let shared =
            scan_rules_with_progress([&first, &second], &ScanOptions::default(), |p| progress = p)
                .unwrap();
        assert_eq!(
            serde_json::to_value(&single.findings).unwrap(),
            serde_json::to_value(&shared.findings).unwrap()
        );
        assert_eq!(shared.rules_matched, 2);
        assert_eq!(shared.total_bytes, 3);
        assert_eq!(progress.entries_seen, single_progress.entries_seen);
        assert_eq!(progress.rules_done, 2);
        assert_eq!(progress.roots_done, 2);
        let mut options = ScanOptions::default();
        options.prefs_min_age.insert("first".into(), 10);
        let aged = scan_rules([&first, &second], &options).unwrap();
        assert_eq!(aged.findings[0].rule_id, "second");
        assert_eq!(aged.rules_matched, 1);
        second.exclude_globs.clear();
        let distinct = scan_rules([&first, &second], &ScanOptions::default()).unwrap();
        assert_eq!(distinct.findings.len(), 2);
    }

    #[test]
    fn excluded_directories_are_pruned_even_without_a_contents_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("project/node_modules/pkg")).unwrap();
        fs::create_dir_all(root.join("project/src")).unwrap();
        fs::write(root.join("project/node_modules/pkg/debug.log"), b"nm").unwrap();
        fs::write(root.join("project/src/debug.log"), b"src").unwrap();
        let mut rule: Rule = serde_yaml::from_str(
            "id: logs\nname: logs\ndescription: prune\npaths: []\nmin_age_days: 0\ninclude_globs: ['**/*.log']\nexclude_globs: ['**/node_modules']\n",
        )
        .unwrap();
        rule.paths = vec![root.to_string_lossy().into_owned()];
        let mut progress = JunkScanProgress::default();
        let report =
            scan_rules_with_progress([&rule], &ScanOptions::default(), |p| progress = p).unwrap();
        assert_eq!(report.findings.len(), 1);
        assert!(report.findings[0].path.ends_with("src/debug.log"));
        // node_modules itself is seen, nothing below it is.
        assert_eq!(progress.files_seen, 1, "{progress:?}");
        assert_eq!(progress.folders_seen, 3);
    }

    #[test]
    fn nested_findings_count_once_in_total_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("cache/sub")).unwrap();
        fs::write(root.join("cache/sub/a.bin"), b"12345").unwrap();
        fs::write(root.join("cache/b.bin"), b"67").unwrap();
        fs::write(root.join("loose.bin"), b"8").unwrap();
        // Top-level units: `cache` (7 bytes) and `loose.bin` (1 byte).
        let mut units: Rule = serde_yaml::from_str(
            "id: units\nname: units\ndescription: top-level\npaths: []\nmin_age_days: 0\n",
        )
        .unwrap();
        units.paths = vec![root.to_string_lossy().into_owned()];
        // Deep files: `cache/sub/a.bin`, `cache/b.bin`, `loose.bin`.
        let mut files = units.clone();
        files.id = "files".into();
        files.include_globs = vec!["**/*.bin".into()];
        let report = scan_rules([&units, &files], &ScanOptions::default()).unwrap();
        assert_eq!(report.findings.len(), 4);
        assert_eq!(
            report.findings.iter().map(|f| f.bytes).sum::<u64>(),
            15,
            "raw sum double counts nested entries"
        );
        assert_eq!(report.total_bytes, 8);
        // Report-only parents do not absorb deletable children.
        let mut report_units = units.clone();
        report_units.report_only = true;
        let mixed = scan_rules([&files, &report_units], &ScanOptions::default()).unwrap();
        assert_eq!(mixed.total_bytes, 8);
    }

    #[test]
    fn directory_units_are_young_when_anything_inside_is_recent() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("stale/deep")).unwrap();
        fs::create_dir_all(root.join("active/deep")).unwrap();
        let ancient = SystemTime::now() - std::time::Duration::from_secs(40 * 24 * 3600);
        let backdate = |path: &Path| {
            fs::File::options()
                .write(path.is_file())
                .read(true)
                .open(path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(ancient))
                .unwrap();
        };
        for sub in ["stale", "active"] {
            fs::write(root.join(sub).join("deep/old.bin"), b"old!").unwrap();
            backdate(&root.join(sub).join("deep/old.bin"));
        }
        fs::write(root.join("active/deep/new.bin"), b"new").unwrap();
        for path in [
            "stale/deep",
            "stale",
            "active/deep",
            "active",
        ] {
            backdate(&root.join(path));
        }
        let mut rule: Rule = serde_yaml::from_str(
            "id: units\nname: units\ndescription: dir age\npaths: []\nmin_age_days: 7\n",
        )
        .unwrap();
        rule.paths = vec![root.to_string_lossy().into_owned()];
        let report = scan_rules([&rule], &ScanOptions::default()).unwrap();
        let paths: Vec<_> = report
            .findings
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(paths, ["stale"], "{report:?}");
        assert_eq!(report.findings[0].min_age_days, 7);
    }

    #[test]
    fn matches_bin_obj_only_inside_dotnet_project() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let project = root.join("Sample");
        let unrelated = root.join("Unrelated");
        fs::create_dir_all(project.join("bin/Debug")).unwrap();
        fs::create_dir_all(project.join("obj")).unwrap();
        fs::create_dir_all(project.join("node_modules/pkg/bin")).unwrap();
        fs::create_dir_all(unrelated.join("bin")).unwrap();
        fs::write(project.join("Sample.csproj"), "<Project />").unwrap();
        fs::write(project.join("bin/Debug/Sample.dll"), b"dll").unwrap();
        fs::write(project.join("obj/assets.json"), b"{}").unwrap();
        fs::write(
            project.join("node_modules/pkg/bin/generated.dll"),
            b"excluded",
        )
        .unwrap();
        fs::write(unrelated.join("bin/keep.txt"), b"keep").unwrap();

        let rule = Rule {
            id: "test-dotnet-output".into(),
            name: "C# output".into(),
            description: "Test C# output".into(),
            category: "dev".into(),
            platform: Platform::Any,
            risk: Risk::Low,
            kind: JunkKind::Dev,
            default_enabled: true,
            min_age_days: 0,
            paths: vec![root.to_string_lossy().into_owned()],
            include_globs: vec!["**/bin".into(), "**/obj".into()],
            exclude_globs: vec!["**/.git/**".into(), "**/node_modules/**".into()],
            required_ancestor_globs: vec!["*.csproj".into(), "*.sln".into()],
            match_directories: true,
            max_depth: 8,
            delete_mode: DeleteMode::Contents,
            report_only: false,
            tags: vec![],
            inferred_tags: vec![],
        };
        let exclude = build_globset(&rule.exclude_globs).unwrap();
        let exclude_dirs = build_directory_exclude_globset(&rule.exclude_globs).unwrap();
        let candidate = |path: &Path| {
            let rel = path.strip_prefix(root).unwrap().to_path_buf();
            (rel, path.to_path_buf())
        };
        let (rel, full) = candidate(&project.join("bin"));
        assert!(!matches_either(
            &exclude,
            &Candidate::new(&rel),
            &Candidate::new(&full)
        ));
        assert!(exclude_dirs.is_match("Sample/node_modules"));
        let (rel, full) = candidate(&project.join("node_modules"));
        assert!(matches_either(
            &exclude_dirs,
            &Candidate::new(&rel),
            &Candidate::new(&full)
        ));

        let mut last_progress = JunkScanProgress::default();
        let report = scan_rules_with_progress([&rule], &ScanOptions::default(), |progress| {
            last_progress = progress;
        })
        .unwrap();
        let paths: Vec<_> = report.findings.iter().map(|f| &f.path).collect();
        let canonical_project = normalize(&project).unwrap();
        let canonical_unrelated = normalize(&unrelated).unwrap();
        assert!(paths.contains(&&canonical_project.join("bin")));
        assert!(paths.contains(&&canonical_project.join("obj")));
        assert!(!paths.contains(&&canonical_project.join("node_modules/pkg/bin")));
        assert!(!paths.contains(&&canonical_unrelated.join("bin")));
        assert_eq!(last_progress.rules_done, 1);
        assert_eq!(last_progress.rules_total, 1);
        // Sizing matched directories no longer inflates traversal counters.
        assert_eq!(last_progress.files_seen, 2);
        assert!(last_progress.folders_seen >= 3);
        assert_eq!(last_progress.findings_found, 2);
        assert_eq!(last_progress.files_found, 0);
        assert_eq!(last_progress.folders_found, 2);
        assert_eq!(last_progress.bytes_found, 5);

        let mut bin_rule = rule.clone();
        bin_rule.id = "test-dotnet-bin".into();
        bin_rule.include_globs = vec!["**/bin".into()];
        let mut obj_rule = rule.clone();
        obj_rule.id = "test-dotnet-obj".into();
        obj_rule.include_globs = vec!["**/obj".into()];
        let mut grouped_progress = JunkScanProgress::default();
        let grouped = scan_rules_with_progress(
            [&bin_rule, &obj_rule],
            &ScanOptions::default(),
            |progress| {
                grouped_progress = progress;
            },
        )
        .unwrap();
        assert_eq!(grouped.findings.len(), 2);
        assert_eq!(grouped.rules_matched, 2);
        assert_eq!(grouped_progress.rules_done, 2);
        assert_eq!(grouped_progress.roots_done, 2);
        assert!(grouped_progress.entries_seen <= last_progress.entries_seen + 1);

        let mut report_rule = rule.clone();
        report_rule.report_only = true;
        let mut report_progress = JunkScanProgress::default();
        let report_only =
            scan_rules_with_progress([&report_rule], &ScanOptions::default(), |progress| {
                report_progress = progress;
            })
            .unwrap();
        assert_eq!(report_only.findings.len(), 2);
        assert_eq!(report_only.total_bytes, 0);
        assert_eq!(report_progress.findings_found, 0);
        assert_eq!(report_progress.bytes_found, 0);

        // Grouped project outputs and ordinary files contribute to the same report.
        fs::write(root.join("trace.log"), b"log!").unwrap();
        let mut logs = bin_rule.clone();
        logs.id = "test-project-logs".into();
        logs.match_directories = false;
        logs.required_ancestor_globs.clear();
        logs.include_globs = vec!["**/*.log".into()];
        let mut mixed_progress = JunkScanProgress::default();
        let mixed = scan_rules_with_progress(
            [&bin_rule, &logs, &obj_rule],
            &ScanOptions::default(),
            |progress| mixed_progress = progress,
        )
        .unwrap();
        assert_eq!(mixed.findings.len(), 3);
        assert_eq!(mixed.total_bytes, 9);
        assert_eq!(mixed.rules_matched, 3);
        assert_eq!(mixed.rules_scanned, 3);
        assert_eq!(mixed_progress.rules_done, 3);
        assert_eq!(mixed_progress.roots_done, 3);
        assert_eq!(mixed_progress.bytes_found, 9);
        assert!(mixed.skipped_roots.is_empty());
    }
}
