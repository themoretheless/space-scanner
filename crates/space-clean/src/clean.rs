use crate::scan::ScanReport;
use anyhow::{Context, Result};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
pub struct CleanResult {
    pub deleted: Vec<CleanedItem>,
    pub skipped_report_only: usize,
    pub errors: Vec<String>,
    pub freed_bytes: u64,
    pub permanent: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CleanedItem {
    pub path: PathBuf,
    pub bytes: u64,
    pub method: String,
}

#[derive(Debug, Clone, Default)]
pub struct CleanProgress {
    pub items_done: usize,
    pub items_total: usize,
    pub files_done: usize,
    pub folders_done: usize,
    pub bytes_freed: u64,
    pub error_count: usize,
    pub current_path: PathBuf,
}

// Finder authorizes each delete request separately. Keep the whole operation
// in one request on macOS so the system asks for permission at most once.
#[cfg(target_os = "macos")]
pub const TRASH_BATCH_SIZE: usize = usize::MAX;
#[cfg(not(target_os = "macos"))]
pub const TRASH_BATCH_SIZE: usize = 64;

/// Move one batch with a single platform Trash operation. If the batch fails,
/// retry remaining paths individually so one bad item does not block the rest.
pub fn trash_paths_batch(paths: &[PathBuf]) -> Vec<Result<(), String>> {
    if paths.is_empty() {
        return Vec::new();
    }
    match trash::delete_all(paths) {
        Ok(()) => paths.iter().map(|_| Ok(())).collect(),
        Err(batch_error) => paths
            .iter()
            .map(|path| {
                // A platform batch may have moved some entries before reporting
                // an error. Missing now means that entry completed successfully.
                if !path.exists() {
                    Ok(())
                } else {
                    #[cfg(target_os = "macos")]
                    {
                        // Retrying through Finder would show another password
                        // dialog for every remaining item.
                        Err(batch_error.to_string())
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        trash::delete(path).map_err(|e| e.to_string())
                    }
                }
            })
            .collect(),
    }
}

pub fn clean(report: &ScanReport, permanent: bool) -> Result<CleanResult> {
    clean_with_progress(report, permanent, |_| {})
}

pub fn clean_with_progress(
    report: &ScanReport,
    permanent: bool,
    mut on_progress: impl FnMut(CleanProgress),
) -> Result<CleanResult> {
    let mut items: Vec<_> = report
        .findings
        .iter()
        .filter(|f| !f.report_only)
        .cloned()
        .collect();

    // Keep the shallowest selected unit. Moving a parent already includes all
    // selected descendants and avoids thousands of redundant Trash calls.
    items.sort_by(|a, b| {
        a.path
            .components()
            .count()
            .cmp(&b.path.components().count())
            .then_with(|| a.path.cmp(&b.path))
    });
    let mut coalesced = Vec::with_capacity(items.len());
    for item in items {
        if coalesced
            .iter()
            .any(|parent: &crate::scan::Finding| item.path.starts_with(&parent.path))
        {
            continue;
        }
        coalesced.push(item);
    }
    let items = coalesced;

    let skipped_report_only = report.findings.iter().filter(|f| f.report_only).count();
    let mut deleted = Vec::new();
    let mut errors = Vec::new();
    let mut freed = 0u64;
    let mut progress = CleanProgress {
        items_total: items.len(),
        ..Default::default()
    };
    on_progress(progress.clone());

    let mut candidates = Vec::new();
    for item in items {
        progress.current_path = item.path.clone();
        if !item.path.exists() {
            progress.items_done += 1;
            on_progress(progress.clone());
            continue;
        }
        // Re-check safety at delete time
        if crate::safety::is_never_touch(&item.path) {
            errors.push(format!("blocked by safety: {}", item.path.display()));
            progress.error_count = errors.len();
            progress.items_done += 1;
            on_progress(progress.clone());
            continue;
        }
        candidates.push(item);
    }
    candidates.sort_by(|a, b| {
        a.path
            .parent()
            .cmp(&b.path.parent())
            .then_with(|| a.path.cmp(&b.path))
    });

    if permanent {
        for item in candidates {
            progress.current_path = item.path.clone();
            match permanent_delete(&item.path) {
                Ok(()) => {
                    record_deleted(item, "permanent", &mut deleted, &mut freed, &mut progress)
                }
                Err(e) => {
                    errors.push(format!("{}: {e}", item.path.display()));
                    progress.error_count = errors.len();
                    progress.items_done += 1;
                }
            }
            on_progress(progress.clone());
        }
    } else {
        for chunk in candidates.chunks(TRASH_BATCH_SIZE) {
            if let Some(first) = chunk.first() {
                progress.current_path = first.path.clone();
                on_progress(progress.clone());
            }
            let paths: Vec<PathBuf> = chunk.iter().map(|item| item.path.clone()).collect();
            let outcomes = trash_paths_batch(&paths);
            for (item, outcome) in chunk.iter().cloned().zip(outcomes) {
                progress.current_path = item.path.clone();
                match outcome {
                    Ok(()) => {
                        record_deleted(item, "trash", &mut deleted, &mut freed, &mut progress)
                    }
                    Err(e) => {
                        errors.push(format!("{}: {e}", item.path.display()));
                        progress.error_count = errors.len();
                        progress.items_done += 1;
                    }
                }
            }
            on_progress(progress.clone());
        }
    }

    Ok(CleanResult {
        deleted,
        skipped_report_only,
        errors,
        freed_bytes: freed,
        permanent,
    })
}

fn record_deleted(
    item: crate::scan::Finding,
    method: &str,
    deleted: &mut Vec<CleanedItem>,
    freed: &mut u64,
    progress: &mut CleanProgress,
) {
    *freed = (*freed).saturating_add(item.bytes);
    progress.items_done += 1;
    progress.bytes_freed = *freed;
    if item.is_dir {
        progress.folders_done += 1;
    } else {
        progress.files_done += 1;
    }
    deleted.push(CleanedItem {
        path: item.path,
        bytes: item.bytes,
        method: method.to_string(),
    });
}

fn permanent_delete(path: &std::path::Path) -> Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)
            .with_context(|| format!("remove_dir_all {}", path.display()))?;
    } else {
        std::fs::remove_file(path).with_context(|| format!("remove_file {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::Finding;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn reports_live_clean_progress() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("space-clean-progress-{unique}"));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("junk.tmp");
        fs::write(&path, b"1234").unwrap();
        let report = ScanReport {
            findings: vec![Finding {
                rule_id: "test".into(),
                category: "temp".into(),
                risk: "low".into(),
                path: path.clone(),
                bytes: 4,
                is_dir: false,
                report_only: false,
            }],
            total_bytes: 4,
            rules_matched: 1,
            rules_scanned: 1,
            skipped_roots: vec![],
        };
        let mut updates = Vec::new();
        let result = clean_with_progress(&report, true, |p| updates.push(p)).unwrap();
        let last = updates.last().unwrap();
        assert_eq!(last.items_done, 1);
        assert_eq!(last.files_done, 1);
        assert_eq!(last.bytes_freed, 4);
        assert_eq!(result.freed_bytes, 4);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn coalesces_selected_descendants_into_parent() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("space-clean-coalesce-{unique}"));
        let parent = root.join("cache");
        let child = parent.join("nested.tmp");
        fs::create_dir_all(&parent).unwrap();
        fs::write(&child, b"1234").unwrap();
        let finding = |path: PathBuf, bytes, is_dir| Finding {
            rule_id: "test".into(),
            category: "temp".into(),
            risk: "low".into(),
            path,
            bytes,
            is_dir,
            report_only: false,
        };
        let report = ScanReport {
            findings: vec![finding(child, 4, false), finding(parent.clone(), 4, true)],
            total_bytes: 8,
            rules_matched: 1,
            rules_scanned: 1,
            skipped_roots: vec![],
        };
        let mut updates = Vec::new();
        let result = clean_with_progress(&report, true, |p| updates.push(p)).unwrap();

        assert_eq!(result.deleted.len(), 1);
        assert_eq!(result.deleted[0].path, parent);
        assert_eq!(result.freed_bytes, 4);
        assert_eq!(updates.last().unwrap().items_total, 1);
        assert!(!root.join("cache").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
