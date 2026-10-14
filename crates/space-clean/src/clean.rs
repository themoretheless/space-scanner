use crate::scan::ScanReport;
use anyhow::{Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct CleanResult {
    pub deleted: Vec<CleanedItem>,
    pub skipped_report_only: usize,
    pub errors: Vec<String>,
    /// Bytes successfully moved to Trash or permanently removed.
    pub processed_bytes: u64,
    /// Bytes actually released from the filesystem (zero for Trash moves).
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
    pub bytes_processed: u64,
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

#[cfg(target_os = "macos")]
const MACOS_TRASH_SCRIPT: &str = r#"
on run argv
    set manifestPath to item 1 of argv
    set rawPaths to read POSIX file manifestPath as «class utf8»
    set previousDelimiters to AppleScript's text item delimiters
    set AppleScript's text item delimiters to character id 0
    set pathItems to text items of rawPaths
    set AppleScript's text item delimiters to previousDelimiters
    set targets to {}
    repeat with pathItem in pathItems
        if length of (contents of pathItem) > 0 then
            set end of targets to POSIX file (contents of pathItem)
        end if
    end repeat
    if (count of targets) > 0 then
        tell application "Finder" to delete targets
    end if
end run
"#;

pub fn coalesce_nested_paths<T>(mut items: Vec<T>, path_of: impl Fn(&T) -> &Path) -> Vec<T> {
    items.sort_by(|a, b| path_of(a).cmp(path_of(b)));
    let mut selected = std::collections::HashSet::with_capacity(items.len());
    let mut coalesced = Vec::with_capacity(items.len());
    for item in items {
        let path = path_of(&item);
        if selected.contains(path)
            || path
                .ancestors()
                .skip(1)
                .any(|ancestor| selected.contains(ancestor))
        {
            continue;
        }
        selected.insert(path.to_path_buf());
        coalesced.push(item);
    }
    coalesced
}

/// Move one batch with a single platform Trash operation. If the batch fails,
/// retry remaining paths individually so one bad item does not block the rest.
#[cfg(not(target_os = "macos"))]
pub fn trash_paths_batch(paths: &[PathBuf]) -> Vec<Result<(), String>> {
    if paths.is_empty() {
        return Vec::new();
    }
    match trash::delete_all(paths) {
        Ok(()) => paths.iter().map(|_| Ok(())).collect(),
        Err(_batch_error) => paths
            .iter()
            .map(|path| {
                // A platform batch may have moved some entries before reporting
                // an error. Missing now means that entry completed successfully.
                if std::fs::symlink_metadata(path).is_err() {
                    Ok(())
                } else {
                    trash::delete(path).map_err(|e| e.to_string())
                }
            })
            .collect(),
    }
}

/// Finder's implementation in `trash` puts the complete AppleScript in one
/// process argument, which fails at ARG_MAX for large selections. Pass paths
/// through a NUL-delimited manifest instead; the AppleScript and argv stay
/// constant-size and Finder still receives one delete operation.
#[cfg(target_os = "macos")]
pub fn trash_paths_batch(paths: &[PathBuf]) -> Vec<Result<(), String>> {
    use std::io::Write;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    if paths.is_empty() {
        return Vec::new();
    }
    let mut outcomes = vec![Ok(()); paths.len()];
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let manifest = std::env::temp_dir().join(format!(
        "space-scanner-trash-{}-{unique}.paths",
        std::process::id()
    ));
    let write_result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&manifest)?;
        for (index, path) in paths.iter().enumerate() {
            let Some(path) = path.to_str() else {
                outcomes[index] = Err("path is not valid UTF-8".into());
                continue;
            };
            file.write_all(path.as_bytes())?;
            file.write_all(&[0])?;
        }
        file.flush()
    })();
    if let Err(error) = write_result {
        return paths
            .iter()
            .map(|_| Err(format!("create Trash manifest: {error}")))
            .collect();
    }

    let command_result = Command::new("osascript")
        .args(["-e", MACOS_TRASH_SCRIPT])
        .arg(&manifest)
        .output();
    let _ = std::fs::remove_file(&manifest);
    let error = match command_result {
        Ok(output) if output.status.success() => {
            for (index, path) in paths.iter().enumerate() {
                if outcomes[index].is_ok() && std::fs::symlink_metadata(path).is_ok() {
                    outcomes[index] =
                        Err("Finder reported success but the path still exists".into());
                }
            }
            return outcomes;
        }
        Ok(output) => format!(
            "Finder Trash operation failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(error) => format!("start Finder Trash operation: {error}"),
    };
    for (index, path) in paths.iter().enumerate() {
        if outcomes[index].is_ok() && std::fs::symlink_metadata(path).is_ok() {
            outcomes[index] = Err(error.clone());
        }
    }
    outcomes
}

pub fn clean(report: &ScanReport, permanent: bool) -> Result<CleanResult> {
    clean_with_progress(report, permanent, |_| {})
}

pub fn clean_with_progress(
    report: &ScanReport,
    permanent: bool,
    mut on_progress: impl FnMut(CleanProgress),
) -> Result<CleanResult> {
    let items: Vec<_> = report
        .findings
        .iter()
        .filter(|f| !f.report_only)
        .cloned()
        .collect();

    // Keep the shallowest selected unit. Check all path ancestors rather than
    // only the previous lexicographic item: a sibling such as `cache-old`
    // can otherwise hide `cache` before `cache/nested` is considered.
    let items = coalesce_nested_paths(items, |item| &item.path);

    let skipped_report_only = report.findings.iter().filter(|f| f.report_only).count();
    let mut deleted = Vec::new();
    let mut errors = Vec::new();
    let mut processed = 0u64;
    let mut freed = 0u64;
    let mut progress = CleanProgress {
        items_total: items.len(),
        ..Default::default()
    };
    on_progress(progress.clone());

    let mut candidates = Vec::new();
    for item in items {
        progress.current_path = item.path.clone();
        if std::fs::symlink_metadata(&item.path).is_err() {
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
                Ok(()) => record_deleted(
                    item,
                    "permanent",
                    &mut deleted,
                    &mut processed,
                    &mut freed,
                    &mut progress,
                ),
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
                    Ok(()) => record_deleted(
                        item,
                        "trash",
                        &mut deleted,
                        &mut processed,
                        &mut freed,
                        &mut progress,
                    ),
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
        processed_bytes: processed,
        freed_bytes: freed,
        permanent,
    })
}

fn record_deleted(
    item: crate::scan::Finding,
    method: &str,
    deleted: &mut Vec<CleanedItem>,
    processed: &mut u64,
    freed: &mut u64,
    progress: &mut CleanProgress,
) {
    *processed = (*processed).saturating_add(item.bytes);
    if method == "permanent" {
        *freed = (*freed).saturating_add(item.bytes);
    }
    progress.items_done += 1;
    progress.bytes_processed = *processed;
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
        assert_eq!(last.bytes_processed, 4);
        assert_eq!(last.bytes_freed, 4);
        assert_eq!(result.processed_bytes, 4);
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
        let sibling = root.join("cache-old");
        fs::create_dir_all(&parent).unwrap();
        fs::write(&child, b"1234").unwrap();
        fs::write(&sibling, b"x").unwrap();
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
            findings: vec![
                finding(child, 4, false),
                finding(sibling.clone(), 1, false),
                finding(parent.clone(), 4, true),
            ],
            total_bytes: 9,
            rules_matched: 1,
            rules_scanned: 1,
            skipped_roots: vec![],
        };
        let mut updates = Vec::new();
        let result = clean_with_progress(&report, true, |p| updates.push(p)).unwrap();

        assert_eq!(result.deleted.len(), 2);
        assert!(result.deleted.iter().any(|item| item.path == parent));
        assert!(result.deleted.iter().any(|item| item.path == sibling));
        assert_eq!(result.processed_bytes, 5);
        assert_eq!(result.freed_bytes, 5);
        assert_eq!(updates.last().unwrap().items_total, 2);
        assert!(!root.join("cache").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn trash_bytes_are_not_reported_as_freed_disk_space() {
        let item = Finding {
            rule_id: "test".into(),
            category: "temp".into(),
            risk: "low".into(),
            path: PathBuf::from("/tmp/example"),
            bytes: 42,
            is_dir: false,
            report_only: false,
        };
        let mut deleted = Vec::new();
        let mut processed = 0;
        let mut freed = 0;
        let mut progress = CleanProgress::default();
        record_deleted(
            item,
            "trash",
            &mut deleted,
            &mut processed,
            &mut freed,
            &mut progress,
        );
        assert_eq!(processed, 42);
        assert_eq!(freed, 0);
        assert_eq!(progress.bytes_processed, 42);
        assert_eq!(progress.bytes_freed, 0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finder_manifest_script_parses_nul_delimited_paths() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let manifest = std::env::temp_dir().join(format!("space-trash-script-{unique}.paths"));
        fs::write(&manifest, b"/tmp/one\0/tmp/two\0").unwrap();
        let parse_only = MACOS_TRASH_SCRIPT.replace(
            "tell application \"Finder\" to delete targets",
            "return count of targets",
        );
        let output = std::process::Command::new("osascript")
            .args(["-e", &parse_only])
            .arg(&manifest)
            .output()
            .unwrap();
        let _ = fs::remove_file(manifest);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "2");
    }
}
