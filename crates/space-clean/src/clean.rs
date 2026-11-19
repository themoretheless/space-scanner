use crate::scan::{Finding, ScanReport};
use anyhow::Result;
use rayon::prelude::*;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::Duration;

/// How a cleaned item left the filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeleteMethod {
    /// Moved to the platform Trash; bytes are not freed until it is emptied.
    #[default]
    Trash,
    /// Removed outright.
    Permanent,
}

#[derive(Debug, Clone)]
pub struct DeleteCandidate {
    pub path: PathBuf,
    pub bytes: u64,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Default)]
pub struct DeleteStats {
    pub files_deleted: usize,
    pub folders_deleted: usize,
}

impl DeleteMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            DeleteMethod::Trash => "trash",
            DeleteMethod::Permanent => "permanent",
        }
    }

    pub fn frees_bytes(self) -> bool {
        matches!(self, DeleteMethod::Permanent)
    }
}

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
    pub bytes_total: u64,
    pub in_flight_bytes: u64,
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
    set totalCount to count of targets
    if totalCount > 0 then
        try
            tell application "Finder" to delete targets
            return ""
        on error
            set errLines to {}
            set chunkSize to 32
            set chunkStart to 1
            repeat while chunkStart ≤ totalCount
                set chunkEnd to chunkStart + chunkSize - 1
                if chunkEnd > totalCount then set chunkEnd to totalCount
                set subTargets to items chunkStart thru chunkEnd of targets
                try
                    tell application "Finder" to delete subTargets
                on error
                    repeat with k from chunkStart to chunkEnd
                        try
                            tell application "Finder" to delete (item k of targets)
                        on error itemErrMsg number itemErrNum
                            set end of errLines to ((k - 1) as text) & tab & itemErrMsg & " (" & itemErrNum & ")"
                        end try
                    end repeat
                end try
                set chunkStart to chunkEnd + 1
            end repeat
            set previousDelimiters to AppleScript's text item delimiters
            set AppleScript's text item delimiters to linefeed
            set joinedErrors to errLines as text
            set AppleScript's text item delimiters to previousDelimiters
            return joinedErrors
        end try
    end if
    return ""
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
    use std::collections::HashMap;
    use std::io::Write;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    if paths.is_empty() {
        return Vec::new();
    }
    let mut outcomes = vec![Ok(()); paths.len()];
    let mut valid_indices = Vec::with_capacity(paths.len());
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
            valid_indices.push(index);
            file.write_all(path.as_bytes())?;
            file.write_all(&[0])?;
        }
        file.flush()
    })();
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&manifest);
        return paths
            .iter()
            .map(|_| Err(format!("create Trash manifest: {error}")))
            .collect();
    }
    if valid_indices.is_empty() {
        let _ = std::fs::remove_file(&manifest);
        return outcomes;
    }

    let command_result = Command::new("osascript")
        .args(["-e", MACOS_TRASH_SCRIPT])
        .arg(&manifest)
        .output();
    let _ = std::fs::remove_file(&manifest);
    let error = match command_result {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut item_errors = HashMap::new();
            for line in stdout.lines() {
                if let Some((pos_str, reason)) = line.split_once('\t') {
                    if let Ok(pos) = pos_str.trim().parse::<usize>() {
                        item_errors.insert(pos, reason.trim().to_string());
                    }
                }
            }
            let stderr_note = String::from_utf8_lossy(&output.stderr).trim().to_string();
            for (manifest_pos, &orig_index) in valid_indices.iter().enumerate() {
                if outcomes[orig_index].is_ok()
                    && std::fs::symlink_metadata(&paths[orig_index]).is_ok()
                {
                    let msg = if let Some(reason) = item_errors.get(&manifest_pos) {
                        format!("Finder Trash operation failed: {reason}")
                    } else if !stderr_note.is_empty() {
                        format!("Finder Trash operation failed: {stderr_note}")
                    } else {
                        "Finder reported success but the path still exists".to_string()
                    };
                    outcomes[orig_index] = Err(msg);
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
    let items: Vec<&Finding> = report.findings.iter().filter(|f| !f.report_only).collect();

    // Keep the shallowest selected unit. Check all path ancestors rather than
    // only the previous lexicographic item: a sibling such as `cache-old`
    // can otherwise hide `cache` before `cache/nested` is considered.
    let items = coalesce_nested_paths(items, |item| item.path.as_path());

    let skipped_report_only = report.findings.iter().filter(|f| f.report_only).count();
    let mut deleted = Vec::new();
    let mut errors = Vec::new();
    let mut processed = 0u64;
    let mut freed = 0u64;
    let bytes_total = items
        .iter()
        .fold(0u64, |sum, item| sum.saturating_add(item.bytes));
    let mut progress = CleanProgress {
        items_total: items.len(),
        bytes_total,
        ..Default::default()
    };
    on_progress(progress.clone());

    let mut candidates = Vec::new();
    for item in items {
        progress.current_path = item.path.clone();
        match revalidate(item) {
            Ok(true) => candidates.push(item),
            Ok(false) => {
                progress.items_done += 1;
                on_progress(progress.clone());
            }
            Err(reason) => {
                errors.push(format!("{reason}: {}", item.path.display()));
                progress.error_count = errors.len();
                progress.items_done += 1;
                on_progress(progress.clone());
            }
        }
    }
    candidates.sort_by(|a, b| {
        a.path
            .parent()
            .cmp(&b.path.parent())
            .then_with(|| a.path.cmp(&b.path))
    });

    if permanent {
        let delete_candidates: Vec<DeleteCandidate> = candidates
            .iter()
            .map(|item| DeleteCandidate {
                path: item.path.clone(),
                bytes: item.bytes,
                is_dir: item.is_dir,
            })
            .collect();
        let outcomes = delete_candidates_permanently_with_progress(
            &delete_candidates,
            progress.clone(),
            |p| {
                progress = p.clone();
                on_progress(p);
            },
        );
        for (item, outcome) in candidates.into_iter().zip(outcomes) {
            match outcome {
                Ok(_stats) => {
                    processed = processed.saturating_add(item.bytes);
                    freed = freed.saturating_add(item.bytes);
                    deleted.push(CleanedItem {
                        path: item.path.clone(),
                        bytes: item.bytes,
                        method: DeleteMethod::Permanent.as_str().to_string(),
                    });
                }
                Err(e) => {
                    errors.push(format!("{}: {e}", item.path.display()));
                }
            }
        }
    } else {
        for chunk in candidates.chunks(TRASH_BATCH_SIZE) {
            if let Some(first) = chunk.first() {
                progress.current_path = first.path.clone();
                on_progress(progress.clone());
            }
            let paths: Vec<PathBuf> = chunk.iter().map(|item| item.path.clone()).collect();
            let outcomes = trash_paths_batch(&paths);
            for (item, outcome) in chunk.iter().zip(outcomes) {
                progress.current_path = item.path.clone();
                match outcome {
                    Ok(()) => record_deleted(
                        item,
                        DeleteMethod::Trash,
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

/// Re-check a finding at delete time, after the scan may have gone stale:
/// `Ok(false)` when it is already gone, `Err(reason)` when it must not be
/// touched any more.
fn revalidate(item: &Finding) -> std::result::Result<bool, String> {
    revalidate_path(
        &item.path,
        item.is_dir,
        item.root.as_deref(),
        item.min_age_days,
        false,
    )
}

/// Re-check a path at delete time, after a scan may have gone stale:
/// `Ok(false)` when it is already gone, `Err(reason)` when it must not be
/// touched any more.
pub fn revalidate_path(
    path: &Path,
    expected_is_dir: bool,
    rule_root: Option<&Path>,
    min_age_days: u64,
    allow_top_symlink: bool,
) -> std::result::Result<bool, String> {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Ok(false);
    };
    // A symlink swapped in since the scan would make the delete follow it.
    if meta.file_type().is_symlink() {
        if !allow_top_symlink {
            return Err("refused: path is now a symlink".into());
        }
        if expected_is_dir {
            return Err("refused: folder was replaced by a symlink".into());
        }
    } else if meta.is_dir() != expected_is_dir {
        return Err("refused: entry type changed since the scan".into());
    }
    if crate::safety::is_never_touch(path) {
        return Err("blocked by safety".into());
    }
    if let Some(root) = rule_root {
        if !crate::safety::stays_under_root(root, path) {
            return Err(format!(
                "refused: no longer inside rule root {}",
                root.display()
            ));
        }
    }
    if min_age_days > 0 && !crate::safety::is_old_enough(path, min_age_days) {
        return Err("skipped: modified since the scan".into());
    }
    Ok(true)
}

fn record_deleted(
    item: &Finding,
    method: DeleteMethod,
    deleted: &mut Vec<CleanedItem>,
    processed: &mut u64,
    freed: &mut u64,
    progress: &mut CleanProgress,
) {
    *processed = (*processed).saturating_add(item.bytes);
    if method.frees_bytes() {
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
        path: item.path.clone(),
        bytes: item.bytes,
        method: method.as_str().to_string(),
    });
}

struct LiveDeleteCounters<'a> {
    files_done: Option<&'a AtomicUsize>,
    folders_done: Option<&'a AtomicUsize>,
    current_path: Option<&'a Mutex<PathBuf>>,
}

/// Permanently delete `path` (bypassing Trash) using parallel tree traversal
/// and unlinking. Never follows symlinks.
pub fn permanent_delete_fast(path: &Path) -> std::result::Result<DeleteStats, String> {
    permanent_delete_with_counters(
        path,
        &LiveDeleteCounters {
            files_done: None,
            folders_done: None,
            current_path: None,
        },
    )
}

/// Permanently delete `candidates` while streaming live progress updates to
/// `on_progress`. Directory candidates use bounded internal parallelism.
pub fn delete_candidates_permanently_with_progress(
    candidates: &[DeleteCandidate],
    initial_progress: CleanProgress,
    mut on_progress: impl FnMut(CleanProgress),
) -> Vec<std::result::Result<DeleteStats, String>> {
    if candidates.is_empty() {
        return Vec::new();
    }
    let bytes_total = if initial_progress.bytes_total > 0 {
        initial_progress.bytes_total
    } else {
        initial_progress.bytes_processed.saturating_add(
            candidates
                .iter()
                .fold(0u64, |sum, c| sum.saturating_add(c.bytes)),
        )
    };
    if candidates.len() <= 2 && candidates.iter().all(|c| !c.is_dir) {
        let mut progress = initial_progress;
        progress.bytes_total = bytes_total;
        progress.in_flight_bytes = 0;
        let mut outcomes = Vec::with_capacity(candidates.len());
        let no_live = LiveDeleteCounters {
            files_done: None,
            folders_done: None,
            current_path: None,
        };
        for candidate in candidates {
            progress.current_path = candidate.path.clone();
            let outcome = permanent_delete_with_counters(&candidate.path, &no_live);
            match &outcome {
                Ok(stats) => {
                    progress.files_done += stats.files_deleted;
                    progress.folders_deleted_add(stats.folders_deleted);
                    progress.bytes_processed =
                        progress.bytes_processed.saturating_add(candidate.bytes);
                    progress.bytes_freed = progress.bytes_freed.saturating_add(candidate.bytes);
                }
                Err(_) => {
                    progress.error_count += 1;
                }
            }
            progress.items_done += 1;
            on_progress(progress.clone());
            outcomes.push(outcome);
        }
        return outcomes;
    }

    let items_done = AtomicUsize::new(initial_progress.items_done);
    let files_done = AtomicUsize::new(initial_progress.files_done);
    let folders_done = AtomicUsize::new(initial_progress.folders_done);
    let bytes_freed = AtomicU64::new(initial_progress.bytes_freed);
    let error_count = AtomicUsize::new(initial_progress.error_count);
    let current_path = Mutex::new(initial_progress.current_path.clone());
    let byte_state = Mutex::new((
        initial_progress.bytes_processed,
        0u64,
        initial_progress.files_done,
    ));
    let done = AtomicBool::new(false);
    let items_total = initial_progress.items_total;

    let snapshot = || {
        let cur_files = files_done.load(Ordering::Relaxed);
        let (completed_bytes, cand_bytes, start_files) = byte_state
            .lock()
            .map(|st| *st)
            .unwrap_or((initial_progress.bytes_processed, 0, 0));
        let in_flight_bytes = if cand_bytes > 0 {
            let cf = cur_files.saturating_sub(start_files);
            if cf > 0 {
                let ratio = (cf as f64) / (cf as f64 + 8_000.0);
                ((cand_bytes as f64) * ratio * 0.95) as u64
            } else {
                0
            }
        } else {
            0
        };
        CleanProgress {
            items_done: items_done.load(Ordering::Relaxed),
            items_total,
            files_done: cur_files,
            folders_done: folders_done.load(Ordering::Relaxed),
            bytes_processed: completed_bytes,
            bytes_freed: bytes_freed.load(Ordering::Relaxed),
            bytes_total,
            in_flight_bytes,
            error_count: error_count.load(Ordering::Relaxed),
            current_path: current_path
                .try_lock()
                .map(|p| p.clone())
                .unwrap_or_default(),
        }
    };

    let outcomes = std::thread::scope(|scope| {
        let (tx, rx) = mpsc::channel::<()>();
        let items_done_ref = &items_done;
        let files_done_ref = &files_done;
        let folders_done_ref = &folders_done;
        let bytes_freed_ref = &bytes_freed;
        let error_count_ref = &error_count;
        let current_path_ref = &current_path;
        let byte_state_ref = &byte_state;
        let done_ref = &done;

        let handle = scope.spawn(move || {
            let live = LiveDeleteCounters {
                files_done: Some(files_done_ref),
                folders_done: Some(folders_done_ref),
                current_path: Some(current_path_ref),
            };
            let results: Vec<std::result::Result<DeleteStats, String>> = candidates
                .iter()
                .map(|candidate| {
                    set_current_path(current_path_ref, &candidate.path);
                    if let Ok(mut st) = byte_state_ref.lock() {
                        st.1 = candidate.bytes;
                        st.2 = files_done_ref.load(Ordering::Relaxed);
                    }
                    let res = permanent_delete_with_counters(&candidate.path, &live);
                    match &res {
                        Ok(_) => {
                            bytes_freed_ref.fetch_add(candidate.bytes, Ordering::Relaxed);
                            if let Ok(mut st) = byte_state_ref.lock() {
                                st.0 = st.0.saturating_add(candidate.bytes);
                                st.1 = 0;
                                st.2 = 0;
                            }
                        }
                        Err(_) => {
                            error_count_ref.fetch_add(1, Ordering::Relaxed);
                            if let Ok(mut st) = byte_state_ref.lock() {
                                st.1 = 0;
                                st.2 = 0;
                            }
                        }
                    }
                    items_done_ref.fetch_add(1, Ordering::Relaxed);
                    let _ = tx.send(());
                    res
                })
                .collect();
            done_ref.store(true, Ordering::Release);
            let _ = tx.send(());
            results
        });

        let mut last_path = initial_progress.current_path;
        loop {
            match rx.recv_timeout(Duration::from_millis(30)) {
                Ok(()) => while rx.try_recv().is_ok() {},
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            let mut snap = snapshot();
            if snap.current_path.as_os_str().is_empty() {
                snap.current_path = last_path.clone();
            } else {
                last_path = snap.current_path.clone();
            }
            on_progress(snap);
            if done.load(Ordering::Acquire) {
                break;
            }
        }
        handle.join().expect("permanent delete worker panicked")
    });

    let mut final_snap = snapshot();
    final_snap.in_flight_bytes = 0;
    if final_snap.current_path.as_os_str().is_empty() {
        if let Some(last) = candidates.last() {
            final_snap.current_path = last.path.clone();
        }
    }
    on_progress(final_snap);
    outcomes
}

impl CleanProgress {
    pub fn displayed_bytes(&self) -> u64 {
        let live = self.bytes_processed.saturating_add(self.in_flight_bytes);
        if self.bytes_total > 0 {
            live.min(self.bytes_total.max(self.bytes_processed))
        } else {
            live
        }
    }

    pub fn fraction(&self) -> f32 {
        if self.items_total > 0 && self.items_done >= self.items_total {
            return 1.0;
        }
        let item_frac = if self.items_total > 0 {
            (self.items_done as f32 / self.items_total as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        if self.bytes_total > 0 {
            let byte_frac =
                (self.displayed_bytes() as f64 / self.bytes_total as f64).clamp(0.0, 1.0) as f32;
            (0.85 * byte_frac + 0.15 * item_frac).clamp(0.0, 0.999)
        } else {
            item_frac
        }
    }

    fn folders_deleted_add(&mut self, n: usize) {
        self.folders_done += n;
    }
}

fn set_current_path(slot: &Mutex<PathBuf>, path: &Path) {
    if let Ok(mut guard) = slot.try_lock() {
        guard.clear();
        guard.push(path);
    }
}

fn permanent_delete_with_counters(
    path: &Path,
    ctx: &LiveDeleteCounters<'_>,
) -> std::result::Result<DeleteStats, String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(DeleteStats::default()),
        Err(e) => return Err(format!("stat {}: {e}", path.display())),
    };
    let ft = meta.file_type();
    if ft.is_dir() {
        #[cfg(unix)]
        {
            unix_delete_dir_fast(path, ctx)
        }
        #[cfg(not(unix))]
        {
            permanent_delete_dir_parallel(path, ctx)
        }
    } else {
        unlink_non_dir(path, None, ft.is_symlink())?;
        if let Some(counter) = ctx.files_done {
            counter.fetch_add(1, Ordering::Relaxed);
        }
        Ok(DeleteStats {
            files_deleted: 1,
            folders_deleted: 0,
        })
    }
}

#[cfg(unix)]
fn delete_pool() -> &'static rayon::ThreadPool {
    static POOL: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let threads = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(4)
            .clamp(2, 8);
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("space-del-{i}"))
            .build()
            .expect("build delete thread pool")
    })
}

#[cfg(unix)]
fn unix_delete_dir_fast(
    root: &Path,
    ctx: &LiveDeleteCounters<'_>,
) -> std::result::Result<DeleteStats, String> {
    static DEL_SEQ: AtomicU64 = AtomicU64::new(0);
    let (target, renamed) = match (root.parent(), root.file_name()) {
        (Some(parent), Some(name)) => {
            let seq = DEL_SEQ.fetch_add(1, Ordering::Relaxed);
            let staged = parent.join(format!(
                ".space-del-{}-{seq}-{}",
                std::process::id(),
                name.to_string_lossy()
            ));
            if !staged.exists() && std::fs::rename(root, &staged).is_ok() {
                (staged, true)
            } else {
                (root.to_path_buf(), false)
            }
        }
        _ => (root.to_path_buf(), false),
    };

    let first_error: Mutex<Option<String>> = Mutex::new(None);
    let files_deleted = AtomicUsize::new(0);
    let folders_deleted = AtomicUsize::new(0);

    delete_pool().install(|| {
        delete_dir_unix(
            &target,
            None,
            &files_deleted,
            &folders_deleted,
            &first_error,
            ctx,
        );
    });

    if let Some(err) = first_error.into_inner().unwrap_or_else(|p| p.into_inner()) {
        if renamed && target.exists() && !root.exists() {
            let _ = std::fs::rename(&target, root);
        }
        return Err(err);
    }
    Ok(DeleteStats {
        files_deleted: files_deleted.load(Ordering::Relaxed),
        folders_deleted: folders_deleted.load(Ordering::Relaxed),
    })
}

#[cfg(unix)]
fn delete_dir_unix(
    dir: &Path,
    parent_inside: Option<&Path>,
    files_deleted: &AtomicUsize,
    folders_deleted: &AtomicUsize,
    first_error: &Mutex<Option<String>>,
    ctx: &LiveDeleteCounters<'_>,
) {
    use std::ffi::{CStr, CString, OsStr};
    use std::os::unix::ffi::OsStrExt;

    struct DirGuard(*mut libc::DIR);
    impl Drop for DirGuard {
        fn drop(&mut self) {
            unsafe {
                libc::closedir(self.0);
            }
        }
    }

    if let Some(slot) = ctx.current_path {
        set_current_path(slot, dir);
    }

    let Ok(c_dir) = CString::new(dir.as_os_str().as_bytes()) else {
        record_first_error(first_error, format!("invalid path: {}", dir.display()));
        return;
    };

    let mut fd = unsafe {
        libc::open(
            c_dir.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::NotFound {
            return;
        }
        if err.kind() == std::io::ErrorKind::PermissionDenied {
            ensure_writable_dir(dir);
            fd = unsafe {
                libc::open(
                    c_dir.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC,
                )
            };
        }
        if fd < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() != std::io::ErrorKind::NotFound {
                record_first_error(first_error, format!("open {}: {err}", dir.display()));
            }
            return;
        }
    }

    let dirp = unsafe { libc::fdopendir(fd) };
    if dirp.is_null() {
        let err = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        record_first_error(first_error, format!("fdopendir {}: {err}", dir.display()));
        return;
    }
    let dir_guard = DirGuard(dirp);
    let dir_fd = unsafe { libc::dirfd(dir_guard.0) };

    let mut subdirs: Vec<PathBuf> = Vec::new();
    let mut local_files = 0usize;
    let mut made_dir_writable = false;

    loop {
        let entry = unsafe { libc::readdir(dir_guard.0) };
        if entry.is_null() {
            break;
        }
        let d_type = unsafe { (*entry).d_type };
        let name_ptr = unsafe { (*entry).d_name.as_ptr() };
        let name_bytes = unsafe { CStr::from_ptr(name_ptr).to_bytes() };
        if name_bytes == b"." || name_bytes == b".." {
            continue;
        }

        let (is_subdir, is_symlink) = match d_type {
            libc::DT_DIR => (true, false),
            libc::DT_LNK => (false, true),
            libc::DT_UNKNOWN => {
                let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
                let rc = unsafe {
                    libc::fstatat(dir_fd, name_ptr, st.as_mut_ptr(), libc::AT_SYMLINK_NOFOLLOW)
                };
                if rc < 0 {
                    let err = std::io::Error::last_os_error();
                    if err.kind() != std::io::ErrorKind::NotFound {
                        record_first_error(
                            first_error,
                            format!(
                                "stat {}: {err}",
                                dir.join(OsStr::from_bytes(name_bytes)).display()
                            ),
                        );
                    }
                    continue;
                }
                let mode = unsafe { st.assume_init().st_mode } & libc::S_IFMT;
                (mode == libc::S_IFDIR, mode == libc::S_IFLNK)
            }
            _ => (false, false),
        };

        if is_subdir {
            subdirs.push(dir.join(OsStr::from_bytes(name_bytes)));
            continue;
        }

        let mut rc = unsafe { libc::unlinkat(dir_fd, name_ptr, 0) };
        if rc < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::NotFound {
                continue;
            }
            if err.kind() == std::io::ErrorKind::PermissionDenied {
                if !made_dir_writable {
                    unsafe {
                        libc::fchmod(dir_fd, 0o700);
                    }
                    made_dir_writable = true;
                }
                if !is_symlink {
                    #[cfg(target_os = "macos")]
                    let flags = libc::AT_SYMLINK_NOFOLLOW;
                    #[cfg(not(target_os = "macos"))]
                    let flags = 0;
                    unsafe {
                        libc::fchmodat(dir_fd, name_ptr, 0o600, flags);
                    }
                }
                rc = unsafe { libc::unlinkat(dir_fd, name_ptr, 0) };
                if rc < 0 {
                    let err = std::io::Error::last_os_error();
                    if err.kind() != std::io::ErrorKind::NotFound {
                        record_first_error(
                            first_error,
                            format!(
                                "remove {}: {err}",
                                dir.join(OsStr::from_bytes(name_bytes)).display()
                            ),
                        );
                    }
                    continue;
                }
            } else {
                record_first_error(
                    first_error,
                    format!(
                        "remove {}: {err}",
                        dir.join(OsStr::from_bytes(name_bytes)).display()
                    ),
                );
                continue;
            }
        }

        local_files += 1;
        if local_files >= 256 {
            files_deleted.fetch_add(local_files, Ordering::Relaxed);
            if let Some(counter) = ctx.files_done {
                counter.fetch_add(local_files, Ordering::Relaxed);
            }
            local_files = 0;
        }
    }

    if local_files > 0 {
        files_deleted.fetch_add(local_files, Ordering::Relaxed);
        if let Some(counter) = ctx.files_done {
            counter.fetch_add(local_files, Ordering::Relaxed);
        }
    }

    // Close DIR* before recursing so at most `delete_pool` FDs remain open regardless of depth.
    drop(dir_guard);

    if subdirs.len() == 1 {
        delete_dir_unix(
            &subdirs[0],
            Some(dir),
            files_deleted,
            folders_deleted,
            first_error,
            ctx,
        );
    } else if subdirs.len() >= 2 {
        subdirs.par_iter().for_each(|sub| {
            delete_dir_unix(
                sub,
                Some(dir),
                files_deleted,
                folders_deleted,
                first_error,
                ctx,
            );
        });
    }

    match remove_empty_dir(dir, parent_inside) {
        Ok(()) => {
            folders_deleted.fetch_add(1, Ordering::Relaxed);
            if let Some(counter) = ctx.folders_done {
                counter.fetch_add(1, Ordering::Relaxed);
            }
        }
        Err(e) => {
            record_first_error(first_error, format!("remove_dir {}: {e}", dir.display()));
        }
    }
}

#[cfg(not(unix))]
fn permanent_delete_dir_parallel(
    root: &Path,
    ctx: &LiveDeleteCounters<'_>,
) -> std::result::Result<DeleteStats, String> {
    let mut levels: Vec<Vec<PathBuf>> = vec![vec![root.to_path_buf()]];
    let first_error: Mutex<Option<String>> = Mutex::new(None);
    let files_deleted = AtomicUsize::new(0);
    let folders_deleted = AtomicUsize::new(0);

    loop {
        let current_level = levels.last().expect("at least root level");
        let next_dirs: Vec<PathBuf> = if current_level.len() >= 2 {
            current_level
                .par_iter()
                .flat_map_iter(|dir| process_dir_for_delete(dir, &files_deleted, &first_error, ctx))
                .collect()
        } else if let Some(dir) = current_level.first() {
            process_dir_for_delete(dir, &files_deleted, &first_error, ctx)
        } else {
            Vec::new()
        };
        if next_dirs.is_empty() {
            break;
        }
        levels.push(next_dirs);
    }

    for (depth, level) in levels.iter().enumerate().rev() {
        let remove_one = |dir: &Path| {
            let parent_inside = if depth > 0 { dir.parent() } else { None };
            match remove_empty_dir(dir, parent_inside) {
                Ok(()) => {
                    folders_deleted.fetch_add(1, Ordering::Relaxed);
                    if let Some(counter) = ctx.folders_done {
                        counter.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Err(e) => {
                    record_first_error(&first_error, format!("remove_dir {}: {e}", dir.display()));
                }
            }
        };
        if level.len() >= 8 {
            level.par_iter().for_each(|dir| remove_one(dir));
        } else {
            for dir in level {
                remove_one(dir);
            }
        }
    }

    if let Some(err) = first_error.into_inner().unwrap_or_else(|p| p.into_inner()) {
        return Err(err);
    }
    Ok(DeleteStats {
        files_deleted: files_deleted.load(Ordering::Relaxed),
        folders_deleted: folders_deleted.load(Ordering::Relaxed),
    })
}

#[cfg(not(unix))]
fn process_dir_for_delete(
    dir: &Path,
    files_deleted: &AtomicUsize,
    first_error: &Mutex<Option<String>>,
    ctx: &LiveDeleteCounters<'_>,
) -> Vec<PathBuf> {
    if let Some(slot) = ctx.current_path {
        set_current_path(slot, dir);
    }
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            ensure_writable_dir(dir);
            match std::fs::read_dir(dir) {
                Ok(rd) => rd,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
                Err(e) => {
                    record_first_error(first_error, format!("read_dir {}: {e}", dir.display()));
                    return Vec::new();
                }
            }
        }
        Err(e) => {
            record_first_error(first_error, format!("read_dir {}: {e}", dir.display()));
            return Vec::new();
        }
    };

    let mut subdirs = Vec::new();
    let mut overflow_non_dirs: Vec<(PathBuf, bool)> = Vec::new();
    let mut inline_count = 0usize;
    let mut inline_deleted = 0usize;

    for entry in rd {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                record_first_error(first_error, format!("read_dir {}: {e}", dir.display()));
                continue;
            }
        };
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => match std::fs::symlink_metadata(&path) {
                Ok(m) => m.file_type(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    record_first_error(first_error, format!("stat {}: {e}", path.display()));
                    continue;
                }
            },
        };
        if ft.is_dir() {
            subdirs.push(path);
        } else if inline_count < 64 {
            inline_count += 1;
            match unlink_non_dir(&path, Some(dir), ft.is_symlink()) {
                Ok(()) => inline_deleted += 1,
                Err(err) => record_first_error(first_error, err),
            }
        } else {
            overflow_non_dirs.push((path, ft.is_symlink()));
        }
    }

    if inline_deleted > 0 {
        files_deleted.fetch_add(inline_deleted, Ordering::Relaxed);
        if let Some(counter) = ctx.files_done {
            counter.fetch_add(inline_deleted, Ordering::Relaxed);
        }
    }

    if !overflow_non_dirs.is_empty() {
        overflow_non_dirs.par_chunks(128).for_each(|chunk| {
            let mut chunk_deleted = 0usize;
            for (path, is_symlink) in chunk {
                match unlink_non_dir(path, Some(dir), *is_symlink) {
                    Ok(()) => chunk_deleted += 1,
                    Err(err) => record_first_error(first_error, err),
                }
            }
            if chunk_deleted > 0 {
                files_deleted.fetch_add(chunk_deleted, Ordering::Relaxed);
                if let Some(counter) = ctx.files_done {
                    counter.fetch_add(chunk_deleted, Ordering::Relaxed);
                }
            }
        });
    }

    subdirs
}

fn record_first_error(slot: &Mutex<Option<String>>, msg: String) {
    let mut guard = slot.lock().unwrap_or_else(|p| p.into_inner());
    if guard.is_none() {
        *guard = Some(msg);
    }
}

fn remove_empty_dir(dir: &Path, parent_inside_target: Option<&Path>) -> std::io::Result<()> {
    let remove_non_empty = |err: std::io::Error| -> std::io::Result<()> {
        if err.kind() == std::io::ErrorKind::DirectoryNotEmpty
            && std::fs::symlink_metadata(dir).is_ok_and(|m| !m.file_type().is_symlink())
        {
            return match std::fs::remove_dir_all(dir) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e),
            };
        }
        Err(err)
    };
    match std::fs::remove_dir(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            ensure_writable_dir(dir);
            if let Some(parent) = parent_inside_target {
                ensure_writable_dir(parent);
            }
            match std::fs::remove_dir(dir) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => remove_non_empty(e),
            }
        }
        Err(e) => remove_non_empty(e),
    }
}

fn remove_non_dir_once(path: &Path, is_symlink: bool) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        #[cfg(windows)]
        Err(_) if is_symlink => match std::fs::remove_dir(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        },
        Err(e) => {
            let _ = is_symlink;
            Err(e)
        }
    }
}

fn unlink_non_dir(
    path: &Path,
    parent_inside_target: Option<&Path>,
    is_symlink: bool,
) -> std::result::Result<(), String> {
    match remove_non_dir_once(path, is_symlink) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            if let Some(parent) = parent_inside_target {
                ensure_writable_dir(parent);
            }
            if !is_symlink {
                ensure_writable_entry(path);
            }
            remove_non_dir_once(path, is_symlink)
                .map_err(|e| format!("remove {}: {e}", path.display()))
        }
        Err(err) => Err(format!("remove {}: {err}", path.display())),
    }
}

fn ensure_writable_dir(path: &Path) {
    ensure_writable_mode(path, 0o700);
}

fn ensure_writable_entry(path: &Path) {
    ensure_writable_mode(path, 0o200);
}

fn ensure_writable_mode(path: &Path, unix_mask: u32) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    if meta.file_type().is_symlink() {
        return;
    }
    let mut perms = meta.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = perms.mode();
        let needed = mode | unix_mask;
        if needed != mode {
            perms.set_mode(needed);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = unix_mask;
        if perms.readonly() {
            perms.set_readonly(false);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn finding(path: PathBuf, bytes: u64, is_dir: bool) -> Finding {
        Finding {
            rule_id: "test".into(),
            category: "temp".into(),
            risk: "low".into(),
            path,
            bytes,
            is_dir,
            report_only: false,
            root: None,
            min_age_days: 0,
            modified_unix: None,
        }
    }

    fn report(findings: Vec<Finding>) -> ScanReport {
        ScanReport {
            total_bytes: findings.iter().map(|f| f.bytes).sum(),
            findings,
            rules_matched: 1,
            rules_scanned: 1,
            skipped_roots: vec![],
        }
    }

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
            findings: vec![finding(path.clone(), 4, false)],
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
        let item = finding(PathBuf::from("/tmp/example"), 42, false);
        let mut deleted = Vec::new();
        let mut processed = 0;
        let mut freed = 0;
        let mut progress = CleanProgress::default();
        record_deleted(
            &item,
            DeleteMethod::Trash,
            &mut deleted,
            &mut processed,
            &mut freed,
            &mut progress,
        );
        assert_eq!(processed, 42);
        assert_eq!(freed, 0);
        assert_eq!(progress.bytes_processed, 42);
        assert_eq!(progress.bytes_freed, 0);
        assert_eq!(deleted[0].method, "trash");
    }

    #[test]
    fn symlink_swapped_in_after_the_scan_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let victim = root.join("precious");
        fs::create_dir_all(&victim).unwrap();
        fs::write(victim.join("keep.txt"), b"keep me").unwrap();
        let junk = root.join("junk");
        fs::create_dir_all(&junk).unwrap();
        fs::write(junk.join("old.bin"), b"junk").unwrap();
        // Scan said `junk` is a plain directory …
        let mut item = finding(junk.clone(), 4, true);
        item.root = Some(root.to_path_buf());
        // … but it was replaced by a symlink to something precious.
        fs::remove_dir_all(&junk).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&victim, &junk).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&victim, &junk).unwrap();
        let report = report(vec![item.clone()]);
        let result = clean(&report, true).unwrap();
        assert!(result.deleted.is_empty());
        assert_eq!(result.errors.len(), 1, "{:?}", result.errors);
        assert!(result.errors[0].contains("symlink"), "{:?}", result.errors);
        assert!(victim.join("keep.txt").exists(), "target must survive");
        assert!(
            junk.symlink_metadata().is_ok(),
            "the link itself is untouched"
        );
        // A file that turned into a directory (or vice versa) is refused too.
        fs::remove_file(&junk).unwrap();
        fs::write(&junk, b"now a file").unwrap();
        let result = clean(&report, true).unwrap();
        assert!(
            result.errors[0].contains("type changed"),
            "{:?}",
            result.errors
        );
        assert!(junk.exists());
    }

    #[test]
    fn delete_revalidates_root_containment_and_age() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let inside = root.join("cache/blob.bin");
        fs::create_dir_all(inside.parent().unwrap()).unwrap();
        fs::write(&inside, b"blob").unwrap();
        // Root recorded by the scan is a different tree: refuse.
        let mut item = finding(inside.clone(), 4, false);
        item.root = Some(root.join("elsewhere"));
        let result = clean(&report(vec![item.clone()]), true).unwrap();
        assert!(
            result.errors[0].contains("rule root"),
            "{:?}",
            result.errors
        );
        assert!(inside.exists());
        // Correct root, but the file was modified after the scan: skip.
        item.root = Some(root.to_path_buf());
        item.min_age_days = 3;
        let result = clean(&report(vec![item.clone()]), true).unwrap();
        assert!(result.errors[0].contains("modified"), "{:?}", result.errors);
        assert!(inside.exists());
        // Legacy finding without a root and no age: deleted.
        item.root = None;
        item.min_age_days = 0;
        let result = clean(&report(vec![item]), true).unwrap();
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(result.deleted.len(), 1);
        assert!(!inside.exists());
    }

    #[test]
    fn report_only_findings_are_counted_not_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let keep = dir.path().join("keep.bin");
        fs::write(&keep, b"k").unwrap();
        let mut item = finding(keep.clone(), 1, false);
        item.report_only = true;
        let result = clean(&report(vec![item]), true).unwrap();
        assert_eq!(result.skipped_report_only, 1);
        assert!(result.deleted.is_empty());
        assert!(keep.exists());
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

    #[cfg(target_os = "macos")]
    #[test]
    fn finder_manifest_script_isolates_per_item_failures() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let manifest = std::env::temp_dir().join(format!("space-trash-fallback-{unique}.paths"));
        fs::write(&manifest, b"/tmp/a\0/tmp/blocked\0/tmp/c\0").unwrap();
        let simulated = MACOS_TRASH_SCRIPT
            .replace(
                "tell application \"Finder\" to delete targets",
                "error \"batch blocked\" number -1741",
            )
            .replace(
                "tell application \"Finder\" to delete subTargets",
                "error \"chunk blocked\" number -1741",
            )
            .replace(
                "tell application \"Finder\" to delete (item k of targets)",
                "if POSIX path of (item k of targets) contains \"blocked\" then error \"Operation not permitted\" number -1741",
            );
        let output = std::process::Command::new("osascript")
            .args(["-e", &simulated])
            .arg(&manifest)
            .output()
            .unwrap();
        let _ = fs::remove_file(manifest);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "1\tOperation not permitted (-1741)"
        );
    }

    #[test]
    fn permanent_delete_removes_deep_parallel_tree_and_readonly_entries() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("cache-tree");
        let mut total_bytes = 0u64;
        for branch in 0..4 {
            let sub = target.join(format!("branch-{branch}/nested"));
            fs::create_dir_all(&sub).unwrap();
            for i in 0..60 {
                let file = sub.join(format!("blob-{i:03}.bin"));
                fs::write(&file, b"payload").unwrap();
                total_bytes += 7;
                if i % 15 == 0 {
                    let mut p = fs::metadata(&file).unwrap().permissions();
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        p.set_mode(0o444);
                    }
                    #[cfg(not(unix))]
                    p.set_readonly(true);
                    fs::set_permissions(&file, p).unwrap();
                }
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut p = fs::metadata(&sub).unwrap().permissions();
                p.set_mode(0o555);
                fs::set_permissions(&sub, p).unwrap();
            }
        }

        let report = report(vec![finding(target.clone(), total_bytes, true)]);
        let mut updates = Vec::new();
        let result = clean_with_progress(&report, true, |p| updates.push(p)).unwrap();
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(result.deleted.len(), 1);
        assert_eq!(result.freed_bytes, total_bytes);
        assert!(!target.exists());
        let last = updates.last().unwrap();
        assert_eq!(last.items_done, 1);
        assert!(last.files_done >= 240);
        assert!(last.folders_done >= 9);
        assert_eq!(last.bytes_freed, total_bytes);
    }

    #[cfg(unix)]
    #[test]
    fn permanent_delete_unlinks_nested_symlinks_without_touching_targets() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let external = dir.path().join("precious-target");
        fs::create_dir_all(&external).unwrap();
        let external_file = external.join("keep.txt");
        fs::write(&external_file, b"keep").unwrap();
        let mut ro = fs::metadata(&external_file).unwrap().permissions();
        ro.set_mode(0o444);
        fs::set_permissions(&external_file, ro).unwrap();

        let target = dir.path().join("junk-dir");
        fs::create_dir_all(&target).unwrap();
        let nested_link = target.join("link-to-external");
        std::os::unix::fs::symlink(&external, &nested_link).unwrap();

        let stats = permanent_delete_fast(&target).unwrap();
        assert!(!target.exists());
        assert_eq!(stats.files_deleted, 1);
        assert_eq!(stats.folders_deleted, 1);
        assert!(external.exists(), "external directory must survive");
        assert!(external_file.exists(), "external file must survive");
        let surviving_mode = fs::metadata(&external_file).unwrap().permissions().mode() & 0o777;
        assert_eq!(surviving_mode, 0o444);

        let top_link = dir.path().join("top-link");
        std::os::unix::fs::symlink(&external, &top_link).unwrap();
        let top_stats = permanent_delete_fast(&top_link).unwrap();
        assert_eq!(top_stats.files_deleted, 1);
        assert_eq!(top_stats.folders_deleted, 0);
        assert!(top_link.symlink_metadata().is_err());
        assert!(external.exists());
        assert!(external_file.exists());
    }
}
