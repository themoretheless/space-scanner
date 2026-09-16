use crate::rules::{expand_path_template, DeleteMode, Risk, Rule, RulePack};
use crate::safety::{is_never_touch_normalized, is_old_enough, is_safe_rule_root, normalize};
use anyhow::Result;
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Serialize;
use std::path::{Path, PathBuf};
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
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanReport {
    pub findings: Vec<Finding>,
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

/// Scan an explicit rule list (e.g. from [`crate::manager::RuleManager::scan_rules`]).
pub fn scan_rules<'a>(
    rules: impl IntoIterator<Item = &'a Rule>,
    opts: &ScanOptions,
) -> Result<ScanReport> {
    scan_rules_with_progress(rules, opts, |_| {})
}

/// Scan rules while periodically reporting rule/root/entry progress.
pub fn scan_rules_with_progress<'a>(
    rules: impl IntoIterator<Item = &'a Rule>,
    opts: &ScanOptions,
    mut on_progress: impl FnMut(JunkScanProgress),
) -> Result<ScanReport> {
    let rules: Vec<&Rule> = rules.into_iter().collect();
    let rules_scanned = rules.len();
    let mut progress = JunkScanProgress {
        rules_total: rules_scanned,
        roots_total: rules.iter().map(|r| r.paths.len()).sum(),
        ..Default::default()
    };
    let mut findings = Vec::new();
    let mut skipped_roots = Vec::new();
    let mut matched_rules = std::collections::BTreeSet::new();

    on_progress(progress.clone());
    let grouped_rule_indexes = scan_grouped_project_rules(
        &rules,
        opts,
        &mut findings,
        &mut skipped_roots,
        &mut matched_rules,
        &mut progress,
        &mut on_progress,
    )?;
    for (rule_index, rule) in rules.into_iter().enumerate() {
        progress.current_rule = rule.name.clone();
        progress.rules_done = rule_index;
        on_progress(progress.clone());
        if grouped_rule_indexes.contains(&rule_index) {
            progress.rules_done = rule_index + 1;
            on_progress(progress.clone());
            continue;
        }
        let min_age = opts
            .min_age_override
            .or(opts.prefs_min_age.get(&rule.id).copied())
            .unwrap_or(rule.min_age_days);
        let include = build_globset(&rule.include_globs, true)?;
        let exclude = build_globset(&rule.exclude_globs, false)?;
        let exclude_dirs = build_directory_exclude_globset(&rule.exclude_globs)?;
        let required_ancestor = build_globset(&rule.required_ancestor_globs, false)?;

        for template in &rule.paths {
            let Some(expanded_root) = expand_path_template(template) else {
                skipped_roots.push(format!("{template} (unexpanded)"));
                progress.roots_done += 1;
                on_progress(progress.clone());
                continue;
            };
            progress.current_path = expanded_root.clone();
            on_progress(progress.clone());
            if !expanded_root.exists() {
                progress.roots_done += 1;
                on_progress(progress.clone());
                continue;
            }
            let Ok(root) = normalize(&expanded_root) else {
                skipped_roots.push(format!("{} (cannot normalize)", expanded_root.display()));
                progress.roots_done += 1;
                on_progress(progress.clone());
                continue;
            };
            if !is_safe_rule_root(&root) {
                skipped_roots.push(format!("{} (blocked by safety)", root.display()));
                progress.roots_done += 1;
                on_progress(progress.clone());
                continue;
            }

            match rule.delete_mode {
                DeleteMode::Path => {
                    if let Some(f) = consider_path(
                        &root,
                        &root,
                        rule,
                        min_age,
                        &include,
                        &exclude,
                        &required_ancestor,
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
                        &include,
                        &exclude,
                        &exclude_dirs,
                        &required_ancestor,
                        &mut findings,
                        &mut matched_rules,
                        &mut progress,
                        &mut on_progress,
                    )?;
                }
            }
            progress.roots_done += 1;
            on_progress(progress.clone());
        }
        progress.rules_done = rule_index + 1;
        on_progress(progress.clone());
    }

    findings.sort_by(|a, b| a.path.cmp(&b.path));
    findings.dedup_by(|a, b| a.path == b.path);

    let total_bytes = findings
        .iter()
        .filter(|f| !f.report_only)
        .map(|f| f.bytes)
        .sum();
    Ok(ScanReport {
        findings,
        total_bytes,
        rules_matched: matched_rules.len(),
        rules_scanned,
        skipped_roots,
    })
}

struct CompiledProjectRule<'a> {
    rule: &'a Rule,
    min_age: u64,
    include: GlobSet,
    exclude: GlobSet,
    exclude_dirs: GlobSet,
    required_ancestor: GlobSet,
}

#[derive(Default)]
struct GroupedRoot {
    rule_indexes: Vec<usize>,
    root_occurrences: usize,
}

fn scan_grouped_project_rules(
    rules: &[&Rule],
    opts: &ScanOptions,
    findings: &mut Vec<Finding>,
    skipped_roots: &mut Vec<String>,
    matched_rules: &mut std::collections::BTreeSet<String>,
    progress: &mut JunkScanProgress,
    on_progress: &mut impl FnMut(JunkScanProgress),
) -> Result<std::collections::BTreeSet<usize>> {
    let grouped_indexes: std::collections::BTreeSet<usize> = rules
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
        return Ok(std::collections::BTreeSet::new());
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
                min_age: opts
                    .min_age_override
                    .or(opts.prefs_min_age.get(&rule.id).copied())
                    .unwrap_or(rule.min_age_days),
                include: build_globset(&rule.include_globs, true)?,
                exclude: build_globset(&rule.exclude_globs, false)?,
                exclude_dirs: build_directory_exclude_globset(&rule.exclude_globs)?,
                required_ancestor: build_globset(&rule.required_ancestor_globs, false)?,
            },
        );
        for template in &rule.paths {
            let Some(expanded_root) = expand_path_template(template) else {
                skipped_roots.push(format!("{template} (unexpanded)"));
                progress.roots_done += 1;
                continue;
            };
            if !expanded_root.exists() {
                progress.roots_done += 1;
                continue;
            }
            let Ok(root) = normalize(&expanded_root) else {
                skipped_roots.push(format!("{} (cannot normalize)", expanded_root.display()));
                progress.roots_done += 1;
                continue;
            };
            if !is_safe_rule_root(&root) {
                skipped_roots.push(format!("{} (blocked by safety)", root.display()));
                progress.roots_done += 1;
                continue;
            }
            let group = roots.entry(root).or_default();
            group.rule_indexes.push(index);
            group.root_occurrences += 1;
        }
    }

    progress.current_rule = format!("Project outputs ({} rules)", grouped_indexes.len());
    on_progress(progress.clone());
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
        on_progress(progress.clone());
    }
    size_grouped_findings(findings, &pending_sizes, progress, on_progress);
    Ok(grouped_indexes)
}

fn collect_grouped_project_root(
    root: &Path,
    rules: &[&CompiledProjectRule<'_>],
    findings: &mut Vec<Finding>,
    matched_rules: &mut std::collections::BTreeSet<String>,
    pending_sizes: &mut Vec<(usize, PathBuf)>,
    progress: &mut JunkScanProgress,
    on_progress: &mut impl FnMut(JunkScanProgress),
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

    while let Some(entry) = walker.next() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        progress.entries_seen = progress.entries_seen.saturating_add(1);
        if progress.entries_seen.is_multiple_of(64) {
            progress.current_path = path.to_path_buf();
            on_progress(progress.clone());
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
        if eligible
            .iter()
            .all(|compiled| is_excluded(path, root, &compiled.exclude_dirs))
        {
            walker.skip_current_dir();
            continue;
        }

        let matching: Vec<_> = eligible
            .into_iter()
            .filter(|compiled| !is_excluded(path, root, &compiled.exclude))
            .filter(|compiled| path_matches(path, root, &compiled.include))
            .filter(|compiled| is_old_enough(path, compiled.min_age))
            .filter(|compiled| {
                has_required_ancestor_marker(path, root, &compiled.required_ancestor)
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
            .any(|compiled| compiled.rule.report_only || compiled.rule.risk == Risk::ReportOnly);
        let finding = Finding {
            rule_id: selected.rule.id.clone(),
            category: selected.rule.category.clone(),
            risk: selected.rule.risk.as_str().to_string(),
            path: path.to_path_buf(),
            bytes: 0,
            is_dir: true,
            report_only,
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
    on_progress: &mut impl FnMut(JunkScanProgress),
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
            on_progress(progress.clone());
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

fn path_matches(path: &Path, root: &Path, patterns: &GlobSet) -> bool {
    if patterns.is_empty() {
        return true;
    }
    let rel = path.strip_prefix(root).unwrap_or(path);
    patterns.is_match(rel) || patterns.is_match(path)
}

#[allow(clippy::too_many_arguments)]
fn collect_under(
    root: &Path,
    rule: &Rule,
    min_age: u64,
    include: &GlobSet,
    exclude: &GlobSet,
    exclude_dirs: &GlobSet,
    required_ancestor: &GlobSet,
    findings: &mut Vec<Finding>,
    matched_rules: &mut std::collections::BTreeSet<String>,
    progress: &mut JunkScanProgress,
    on_progress: &mut impl FnMut(JunkScanProgress),
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
            on_progress(progress.clone());
        }
        if path == root {
            continue;
        }
        if entry.file_type().is_symlink() {
            continue;
        }
        if entry.file_type().is_dir() {
            progress.folders_seen = progress.folders_seen.saturating_add(1);
        } else if entry.file_type().is_file() {
            progress.files_seen = progress.files_seen.saturating_add(1);
        }
        if !path.starts_with(root) {
            if entry.file_type().is_dir() {
                walker.skip_current_dir();
            }
            continue;
        }
        if is_never_touch_normalized(path) {
            if entry.file_type().is_dir() {
                walker.skip_current_dir();
            }
            continue;
        }
        if entry.file_type().is_dir() && is_excluded(path, root, exclude_dirs) {
            walker.skip_current_dir();
            continue;
        }
        if deep && entry.file_type().is_dir() {
            if rule.match_directories {
                if let Some(f) = consider_path(
                    path,
                    root,
                    rule,
                    min_age,
                    include,
                    exclude,
                    required_ancestor,
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
        if let Some(f) = consider_path(
            path,
            root,
            rule,
            min_age,
            include,
            exclude,
            required_ancestor,
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

fn is_excluded(path: &Path, root: &Path, exclude: &GlobSet) -> bool {
    let rel = path.strip_prefix(root).unwrap_or(path);
    exclude.is_match(rel) || exclude.is_match(path)
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

#[allow(clippy::too_many_arguments)]
fn consider_path(
    path: &Path,
    root: &Path,
    rule: &Rule,
    min_age: u64,
    include: &GlobSet,
    exclude: &GlobSet,
    required_ancestor: &GlobSet,
    progress: &mut JunkScanProgress,
    on_progress: &mut impl FnMut(JunkScanProgress),
) -> Option<Finding> {
    if !is_old_enough(path, min_age) {
        return None;
    }

    let rel = path.strip_prefix(root).unwrap_or(path);
    if is_excluded(path, root, exclude) {
        return None;
    }
    let rel_str = rel.to_string_lossy();
    // Also test with full path for absolute globs
    let full_str = path.to_string_lossy();
    if !include.is_empty()
        && !include.is_match(rel_str.as_ref())
        && !include.is_match(full_str.as_ref())
    {
        return None;
    }
    if !has_required_ancestor_marker(path, root, required_ancestor) {
        return None;
    }

    let meta = path.symlink_metadata().ok()?;
    // Do not follow symlinks for deletion candidates
    if meta.file_type().is_symlink() {
        return None;
    }

    let is_dir = meta.is_dir();
    let bytes = if is_dir {
        dir_size(path, progress, on_progress)
    } else {
        meta.len()
    };

    // Skip empty noise
    if bytes == 0 && !is_dir {
        return None;
    }

    let report_only = rule.report_only || matches!(rule.risk, Risk::ReportOnly);

    Some(Finding {
        rule_id: rule.id.clone(),
        category: rule.category.clone(),
        risk: rule.risk.as_str().to_string(),
        path: path.to_path_buf(),
        bytes,
        is_dir,
        report_only,
    })
}

fn has_required_ancestor_marker(path: &Path, root: &Path, markers: &GlobSet) -> bool {
    if markers.is_empty() {
        return true;
    }
    let mut dir = path.parent();
    while let Some(current) = dir {
        if std::fs::read_dir(current).ok().is_some_and(|entries| {
            entries.filter_map(|entry| entry.ok()).any(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| markers.is_match(name))
            })
        }) {
            return true;
        }
        if current == root {
            break;
        }
        dir = current.parent();
    }
    false
}

fn dir_size(
    path: &Path,
    progress: &mut JunkScanProgress,
    on_progress: &mut impl FnMut(JunkScanProgress),
) -> u64 {
    let mut bytes = 0u64;
    for entry in WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .skip(1)
    {
        progress.entries_seen = progress.entries_seen.saturating_add(1);
        if entry.file_type().is_dir() {
            progress.folders_seen = progress.folders_seen.saturating_add(1);
        } else if entry.file_type().is_file() {
            progress.files_seen = progress.files_seen.saturating_add(1);
            if let Ok(meta) = entry.metadata() {
                bytes = bytes.saturating_add(meta.len());
            }
        }
        if progress.entries_seen.is_multiple_of(128) {
            progress.current_path = entry.path().to_path_buf();
            on_progress(progress.clone());
        }
    }
    bytes
}

fn build_globset(patterns: &[String], default_all_if_empty: bool) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    if patterns.is_empty() && default_all_if_empty {
        // empty include = match all (handled by is_empty check in consider_path)
        return Ok(builder.build()?);
    }
    for p in patterns {
        builder.add(Glob::new(p)?);
    }
    Ok(builder.build()?)
}

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
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn matches_bin_obj_only_inside_dotnet_project() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("space-csharp-{unique}"));
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
        };
        let exclude = build_globset(&rule.exclude_globs, false).unwrap();
        let exclude_dirs = build_directory_exclude_globset(&rule.exclude_globs).unwrap();
        assert!(!is_excluded(&project.join("bin"), &root, &exclude));
        assert!(exclude_dirs.is_match("Sample/node_modules"));
        assert!(is_excluded(
            &project.join("node_modules"),
            &root,
            &exclude_dirs
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
        fs::remove_dir_all(root).unwrap();
    }
}
