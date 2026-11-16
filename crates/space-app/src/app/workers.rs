use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use space_core::{rescan_subtree_with_options, NodeId, ScanEvent, SizeMode, SpaceTree};

use crate::downloads;
use crate::largest_files::{InsightData, LargestRow};
use crate::tm_clean;
use crate::volumes;

use super::{
    capture_rescan_paths, is_live_node, prune_deleted_from_tree, BasketItem, BgMsg, DeleteSource,
    SpaceApp, TrashOutcome,
};

pub(super) fn spawn_bg(name: &str, f: impl FnOnce() + Send + 'static) -> std::io::Result<()> {
    thread::Builder::new()
        .name(name.into())
        .spawn(f)
        .map(|_| ())
}

impl SpaceApp {
    pub(super) fn refresh_volumes_async(&mut self) {
        self.volumes_loading = true;
        let tx = self.bg_tx.clone();
        let _ = spawn_bg("space-vols", move || {
            let _ = tx.send(BgMsg::Volumes(volumes::list_volumes()));
        });
    }

    pub(super) fn poll_bg(&mut self, ctx: &egui::Context) {
        let mut got = false;
        let mut start_scan_path = None;
        while let Ok(msg) = self.bg_rx.try_recv() {
            got = true;
            match msg {
                BgMsg::Volumes(v) => {
                    self.volumes = v;
                    self.volumes_loading = false;
                }
                BgMsg::Snapshots(s) => {
                    self.tm_snapshots = s;
                    self.tm_loading = false;
                }
                BgMsg::Recent(r) => self.recent = r,
                BgMsg::CacheResult { path, loaded } => {
                    self.busy = false;
                    match loaded {
                        Some((mut tree, stale, offline)) => {
                            if self.size_mode != SizeMode::OnDisk {
                                Arc::make_mut(&mut tree).sort_children_by(self.size_mode);
                            }
                            self.focus = tree.root;
                            self.review.clear();
                            self.history_back.clear();
                            self.history_fwd.clear();
                            self.reset_tree_view_state();
                            self.cache_stale = stale;
                            self.cache_offline = offline;
                            self.from_cache = true;
                            self.skipped_expanded = false;
                            self.status = if offline {
                                crate::tr!(
                                    "Offline disk · cached {} items · {}",
                                    tree.item_count,
                                    crate::i18n::bytes(tree.total_size_mode(self.size_mode))
                                )
                            } else if stale {
                                crate::tr!(
                                    "Cached index (stale) · {} items · {} — Rescan recommended",
                                    tree.item_count,
                                    crate::i18n::bytes(tree.total_size_mode(self.size_mode))
                                )
                            } else {
                                crate::tr!(
                                    "Loaded cache · {} items · {}",
                                    tree.item_count,
                                    crate::i18n::bytes(tree.total_size_mode(self.size_mode))
                                )
                            };
                            self.caches.invalidate_all();
                            self.tree = Some(tree);
                            self.progress = None;
                            self.invalidate_largest();
                            self.schedule_largest();
                        }
                        None => {
                            start_scan_path = Some(path);
                        }
                    }
                }
                BgMsg::CacheSaved(recent) => {
                    self.recent = recent;
                }
                BgMsg::DownloadsDone(items) => {
                    self.downloads.set_items(items);
                }
                BgMsg::RescanDone(Ok((tree, saved))) => {
                    self.busy = false;
                    self.restore_rescan_tree(tree, saved);
                }
                BgMsg::RescanDone(Err(e)) => {
                    self.busy = false;
                    self.status = crate::tr!("Incremental rescan failed: {}", e);
                }
                BgMsg::DeleteProgress(progress) => {
                    self.delete_progress = Some(progress);
                }
                BgMsg::DeleteDone(Ok(outcome)) => {
                    self.busy = false;
                    self.delete_progress = None;
                    self.delete_started = None;
                    self.confirm_delete = false;
                    let method = outcome.method;
                    self.delete_method = space_clean::DeleteMethod::Trash;
                    self.delete_source = DeleteSource::Basket;
                    let n = outcome.deleted_paths.len();
                    self.status = match (method, outcome.errors.is_empty()) {
                        (space_clean::DeleteMethod::Permanent, true) => crate::tr!(
                            "Deleted {} permanently · freed {}",
                            crate::widgets::count_noun(n, "item"),
                            crate::i18n::bytes(outcome.freed_bytes)
                        ),
                        (space_clean::DeleteMethod::Permanent, false) => crate::tr!(
                            "Deleted {} permanently · {} failed",
                            crate::widgets::count_noun(n, "item"),
                            outcome.errors.len()
                        ),
                        (space_clean::DeleteMethod::Trash, true) => crate::tr!(
                            "Moved {} items to Trash · {}",
                            n,
                            crate::i18n::bytes(outcome.processed_bytes)
                        ),
                        (space_clean::DeleteMethod::Trash, false) => {
                            crate::tr!("Moved {} items · {} failed", n, outcome.errors.len())
                        }
                    };
                    self.junk.remove_paths(&outcome.deleted_paths);
                    self.downloads.remove_paths(&outcome.deleted_paths);
                    if let Some(tree_arc) = &mut self.tree {
                        let tree_mut = Arc::make_mut(tree_arc);
                        prune_deleted_from_tree(tree_mut, &outcome.deleted_paths);
                        while !is_live_node(tree_mut, self.focus) {
                            let Some(parent) = tree_mut.try_node(self.focus).and_then(|n| n.parent)
                            else {
                                self.focus = tree_mut.root;
                                break;
                            };
                            self.focus = parent;
                        }
                        self.review.retain(|&id| is_live_node(tree_mut, id));
                        self.expanded.retain(|&id| is_live_node(tree_mut, id));
                        self.history_back
                            .retain(|&id| is_live_node(tree_mut, id) && id != self.focus);
                        self.history_fwd
                            .retain(|&id| is_live_node(tree_mut, id) && id != self.focus);
                        self.selected = self.selected.filter(|&id| is_live_node(tree_mut, id));
                        self.selected_set.retain(|&id| is_live_node(tree_mut, id));
                        self.selection_anchor = self
                            .selection_anchor
                            .filter(|&id| is_live_node(tree_mut, id));
                        self.hovered = self.hovered.filter(|&id| is_live_node(tree_mut, id));
                        self.tree_revealed =
                            self.tree_revealed.filter(|&id| is_live_node(tree_mut, id));
                        self.geom = None;
                        self.caches.invalidate_all();
                    }
                    self.invalidate_largest();
                    self.schedule_largest();
                    if let Some(tree) = self.tree.clone() {
                        self.save_cache_async(tree);
                    }
                    self.refresh_volumes_async();
                }
                BgMsg::DeleteDone(Err(e)) => {
                    self.busy = false;
                    self.delete_progress = None;
                    self.delete_started = None;
                    self.confirm_delete = false;
                    let method = self.delete_method;
                    self.delete_method = space_clean::DeleteMethod::Trash;
                    self.delete_source = DeleteSource::Basket;
                    self.status = match method {
                        space_clean::DeleteMethod::Permanent => {
                            crate::tr!("Delete failed: {}", e)
                        }
                        space_clean::DeleteMethod::Trash => crate::tr!("Trash failed: {}", e),
                    };
                }
                BgMsg::TmCleanDone(report) => {
                    self.busy = false;
                    self.confirm_tm_clean = false;
                    if report.deleted == 0 && report.errors.is_empty() {
                        self.status = format!(
                            "Snapshots: nothing to remove ({}/{} kept)",
                            report.kept, report.before
                        );
                    } else if report.errors.is_empty() {
                        self.status = format!(
                            "Removed {} snapshot(s), kept {} · reclaiming APFS space…",
                            report.deleted, report.kept
                        );
                        self.show_tm_panel = false;
                    } else {
                        self.status = format!(
                            "Snapshot cleanup: deleted {} of {}, {} error(s)",
                            report.deleted,
                            report.before,
                            report.errors.len()
                        );
                    }
                    self.refresh_volumes_async();
                    self.refresh_snapshots();
                }
                BgMsg::RuleManagerLoaded(result) => {
                    self.rule_mgr_loading = false;
                    match result {
                        Ok(manager) => {
                            self.rule_mgr = Some(manager);
                            self.rules.error = None;
                        }
                        Err(error) => self.rules.error = Some(error),
                    }
                }
                BgMsg::JunkScanProgress(progress) => {
                    if self.junk.loading {
                        self.junk.progress = progress;
                    }
                }
                BgMsg::JunkScanDone(Ok(report)) => {
                    self.junk.set_report(report);
                    self.status = crate::tr!(
                        "Junk · {} · {} items",
                        crate::i18n::bytes(self.junk.report().map(|r| r.total_bytes).unwrap_or(0)),
                        self.junk.report().map(|r| r.findings.len()).unwrap_or(0)
                    );
                }
                BgMsg::JunkScanDone(Err(e)) => {
                    self.junk.loading = false;
                    self.junk.scan_started = None;
                    self.junk.error = Some(e.clone());
                    self.status = crate::tr!("Junk scan failed: {}", e);
                }
                BgMsg::Largest { request, data } => {
                    self.largest.complete_data(request, data);
                }
            }
        }
        if let Some(path) = start_scan_path {
            self.begin_filesystem_scan(path);
        }
        if self.volumes_loading
            || self.tm_loading
            || self.busy
            || self.scan.is_some()
            || self.junk.loading
            || self.downloads.loading
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        } else if self.largest.is_loading() {
            // Don't spin the UI at 20fps while a background walk finishes.
            ctx.request_repaint_after(Duration::from_millis(250));
        } else if got {
            ctx.request_repaint();
        }
    }

    pub(super) fn schedule_largest(&mut self) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        let focus = self.focus;
        let mode = self.size_mode;
        let Some(request) = self.largest.begin(focus, mode) else {
            return;
        };
        let limit = self.settings.largest_limit.clamp(10, 500);
        let tx = self.bg_tx.clone();
        let result = spawn_bg("space-largest", move || {
            let to_rows = |ids: Vec<NodeId>| -> Vec<LargestRow> {
                ids.into_iter()
                    .filter_map(|id| {
                        let n = tree.try_node(id)?;
                        Some((id, n.name.clone(), n.size(mode)))
                    })
                    .collect()
            };
            let files = to_rows(tree.largest_files_bounded(focus, mode, limit, 8_000));
            let folders = to_rows(tree.largest_dirs_bounded(focus, mode, limit, 8_000));
            let mut oldest_ids = tree.oldest_large_files(focus, mode, 1024 * 1024, limit);
            if oldest_ids.is_empty() {
                oldest_ids = tree.oldest_large_files(focus, mode, 1, limit);
            }
            let oldest = oldest_ids
                .into_iter()
                .filter_map(|id| {
                    let n = tree.try_node(id)?;
                    Some((id, n.name.clone(), n.size(mode), n.mtime_unix))
                })
                .collect();
            let duplicates = tree
                .duplicate_name_size(focus, mode, 1, limit)
                .into_iter()
                .map(|g| (g.name, g.size, g.nodes))
                .collect();
            let types = tree
                .group_by_ext(focus, mode, limit)
                .into_iter()
                .map(|g| (g.ext, g.total_bytes, g.file_count))
                .collect();
            let data = InsightData {
                files,
                folders,
                oldest,
                duplicates,
                types,
            };
            let _ = tx.send(BgMsg::Largest { request, data });
        });
        if let Err(error) = result {
            self.largest.failed(request);
            self.status = format!("Could not start largest-file search: {error}");
        }
    }

    pub(super) fn save_cache_async(&self, tree: Arc<SpaceTree>) {
        let Some(cache) = self.cache.clone() else {
            return;
        };
        let tx = self.bg_tx.clone();
        let _ = spawn_bg("space-cache-save", move || {
            let _ = cache.save(tree.as_ref());
            let recent = cache.list_recent().unwrap_or_default();
            let _ = tx.send(BgMsg::CacheSaved(recent));
        });
    }

    pub(super) fn incremental_rescan_focus(&mut self) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        if self.busy {
            return;
        }
        let focus = self.focus;
        let focus_name = tree
            .try_node(focus)
            .map(|n| n.name.clone())
            .unwrap_or_default();
        self.busy = true;
        self.status = crate::tr!("Incremental rescan of {}…", focus_name);
        let saved = capture_rescan_paths(
            &tree,
            focus,
            self.selected,
            &self.expanded,
            &self.review,
            &self.history_back,
            &self.history_fwd,
        );
        let opts = self.settings.scan_options();
        let tx = self.bg_tx.clone();
        let _ = spawn_bg("space-rescan", move || {
            let result = rescan_subtree_with_options(tree.as_ref(), focus, opts)
                .map(|t| (Arc::new(t), saved));
            let _ = tx.send(BgMsg::RescanDone(result));
        });
    }

    pub(super) fn poll_scan(&mut self, ctx: &egui::Context) {
        let Some(scan) = &self.scan else { return };
        let mut done_tree = None;
        let mut err = None;
        let mut cancelled = false;
        let mut last_prog = None;

        while let Ok(ev) = scan.events.try_recv() {
            match ev {
                ScanEvent::Progress(p) => last_prog = Some(p),
                ScanEvent::Done(t) => done_tree = Some(t),
                ScanEvent::Cancelled => cancelled = true,
                ScanEvent::Error(e) => err = Some(e),
            }
        }

        if let Some(p) = last_prog {
            self.status = crate::tr!(
                "Scanning… {} · {}{}",
                crate::i18n::bytes(p.bytes_seen),
                p.files_seen,
                if p.skipped_errors > 0 {
                    crate::tr!(" · {} skipped", p.skipped_errors)
                } else {
                    String::new()
                }
            );
            self.progress = Some(p);
        }

        if cancelled {
            self.status = crate::i18n::t("Scan cancelled.").into();
            self.scan = None;
            self.progress = None;
            return;
        }

        if let Some(e) = err {
            self.status = crate::tr!("Scan error: {}", e);
            self.scan = None;
            self.progress = None;
            return;
        }

        if let Some(mut tree) = done_tree {
            if self.size_mode != SizeMode::OnDisk {
                tree.sort_children_by(self.size_mode);
            }
            let skip = tree.skipped_errors;
            let hl = tree.hardlinks_deduped;
            self.status = crate::tr!(
                "Indexed {} items · {}{}{}",
                tree.item_count,
                crate::i18n::bytes(tree.total_size_mode(self.size_mode)),
                if skip > 0 {
                    crate::tr!(" · {} skipped", skip)
                } else {
                    String::new()
                },
                if hl > 0 {
                    crate::tr!(" · {} hardlinks deduped", hl)
                } else {
                    String::new()
                }
            );
            self.focus = tree.root;
            self.review.clear();
            self.history_back.clear();
            self.history_fwd.clear();
            self.reset_tree_view_state();
            self.from_cache = false;
            self.cache_stale = false;
            self.cache_offline = false;
            self.skipped_expanded = false;
            let arc = Arc::new(*tree);
            self.save_cache_async(arc.clone());
            self.caches.invalidate_all();
            self.tree = Some(arc);
            self.scan = None;
            self.progress = None;
            self.invalidate_largest();
            self.schedule_largest();
        } else {
            ctx.request_repaint();
        }
    }

    pub(super) fn refresh_snapshots(&mut self) {
        self.tm_loading = true;
        let tx = self.bg_tx.clone();
        let _ = spawn_bg("space-snaps", move || {
            let _ = tx.send(BgMsg::Snapshots(tm_clean::list_local_snapshots()));
        });
    }

    pub(super) fn apply_tm_clean(&mut self) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status = crate::i18n::t("Deleting old Time Machine snapshots…").into();
        let tx = self.bg_tx.clone();
        let keep = if self.tm_snapshots.iter().filter(|s| s.is_tm()).count() > 1 {
            1usize
        } else {
            0usize
        };
        let _ = spawn_bg("space-tm-clean", move || {
            let report = tm_clean::delete_old_tm_locals(keep);
            let _ = tx.send(BgMsg::TmCleanDone(report));
        });
    }

    pub(super) fn start_rule_mgr_load(&mut self) {
        if self.rule_mgr.is_some() || self.rule_mgr_loading {
            return;
        }
        self.rule_mgr_loading = true;
        let tx = self.bg_tx.clone();
        if let Err(error) = spawn_bg("space-rule-load", move || {
            let result =
                space_clean::RuleManager::load_default().map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::RuleManagerLoaded(result));
        }) {
            self.rule_mgr_loading = false;
            self.rules.error = Some(error.to_string());
        }
    }

    pub(crate) fn start_downloads_scan(&mut self) {
        self.downloads.loading = true;
        let tx = self.bg_tx.clone();
        let _ = spawn_bg("space-downloads", move || {
            let _ = tx.send(BgMsg::DownloadsDone(downloads::scan_default_downloads()));
        });
    }

    pub(super) fn start_junk_scan(&mut self) {
        self.junk.loading = true;
        self.junk.error = None;
        self.junk.progress = space_clean::JunkScanProgress::default();
        self.junk.scan_started = Some(Instant::now());
        let mut prefs = self
            .rule_mgr
            .as_ref()
            .map(|m| m.prefs.clone())
            .unwrap_or_else(space_clean::RulePrefs::load);
        let flags = self.settings.flags.clone();
        let tx = self.bg_tx.clone();
        let _ = spawn_bg("space-junk-scan", move || {
            let result = (|| {
                let pack =
                    space_clean::RulePack::load_many_shared(&space_clean::default_rules_dirs())?;
                prefs.migrate_catalog_aliases(&space_clean::default_rules_dirs())?;
                flags.apply_to_rule_prefs(&mut prefs, &pack);
                let mgr = space_clean::RuleManager { pack, prefs };
                let opts = space_clean::ScanOptions {
                    categories: Vec::new(),
                    include_disabled: false,
                    min_age_override: None,
                    prefs_min_age: mgr.prefs.min_age.clone(),
                };
                let rules = mgr.scan_rules(&[]);
                let progress_tx = tx.clone();
                space_clean::scan_rules_with_progress(rules, &opts, move |progress| {
                    let _ = progress_tx.send(BgMsg::JunkScanProgress(progress));
                })
            })()
            .map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::JunkScanDone(result));
        });
    }

    /// Single-batch deletion worker across Quick clean, Downloads, Folders, or direct targets.
    pub(crate) fn apply_basket(&mut self) {
        if self.busy {
            return;
        }
        let items = self.active_delete_items();
        if items.is_empty() {
            self.confirm_delete = false;
            self.delete_source = DeleteSource::Basket;
            return;
        }
        let method = self.delete_method;
        self.busy = true;
        self.confirm_delete = true;
        self.delete_progress = Some(space_clean::CleanProgress {
            items_total: items.len(),
            ..Default::default()
        });
        self.delete_started = Some(Instant::now());
        self.status = match method {
            space_clean::DeleteMethod::Trash => crate::i18n::t("Moving to Trash…").into(),
            space_clean::DeleteMethod::Permanent => crate::i18n::t("Deleting permanently…").into(),
        };
        let tx = self.bg_tx.clone();
        let thread_name = match method {
            space_clean::DeleteMethod::Trash => "space-trash",
            space_clean::DeleteMethod::Permanent => "space-perm-delete",
        };
        let _ = spawn_bg(thread_name, move || {
            let result = run_delete_batch(items, method, &tx);
            let _ = tx.send(BgMsg::DeleteDone(result));
        });
    }
}

fn run_delete_batch(
    items: Vec<BasketItem>,
    method: space_clean::DeleteMethod,
    tx: &crossbeam_channel::Sender<BgMsg>,
) -> Result<TrashOutcome, String> {
    let mut deleted_paths = Vec::new();
    let mut errors = Vec::new();
    let mut progress = space_clean::CleanProgress {
        items_total: items.len(),
        ..Default::default()
    };
    let _ = tx.send(BgMsg::DeleteProgress(progress.clone()));
    let mut candidates = Vec::new();
    for item in items {
        progress.current_path = item.path.clone();
        if space_clean::safety::is_never_touch(&item.path) {
            errors.push(format!("Protected: {}", item.path.display()));
            progress.items_done += 1;
            progress.error_count = errors.len();
            let _ = tx.send(BgMsg::DeleteProgress(progress.clone()));
            continue;
        }
        match method {
            space_clean::DeleteMethod::Trash => {
                if std::fs::symlink_metadata(&item.path).is_err() {
                    deleted_paths.push(item.path);
                    progress.items_done += 1;
                    let _ = tx.send(BgMsg::DeleteProgress(progress.clone()));
                    continue;
                }
            }
            space_clean::DeleteMethod::Permanent => {
                let (rule_root, min_age_days) = item
                    .junk_guard
                    .as_ref()
                    .map(|(root, age)| (root.as_deref(), *age))
                    .unwrap_or((None, 0));
                match space_clean::revalidate_path(
                    &item.path,
                    item.is_dir,
                    rule_root,
                    min_age_days,
                    false,
                ) {
                    Ok(true) => {}
                    Ok(false) => {
                        deleted_paths.push(item.path);
                        progress.items_done += 1;
                        let _ = tx.send(BgMsg::DeleteProgress(progress.clone()));
                        continue;
                    }
                    Err(reason) => {
                        errors.push(format!("{}: {reason}", item.path.display()));
                        progress.items_done += 1;
                        progress.error_count = errors.len();
                        let _ = tx.send(BgMsg::DeleteProgress(progress.clone()));
                        continue;
                    }
                }
            }
        }
        candidates.push(item);
    }

    let mut freed_bytes = 0_u64;
    let mut processed_bytes = 0_u64;
    match method {
        space_clean::DeleteMethod::Trash => {
            candidates.sort_by(|a, b| {
                a.path
                    .parent()
                    .cmp(&b.path.parent())
                    .then_with(|| a.path.cmp(&b.path))
            });
            for chunk in candidates.chunks(space_clean::TRASH_BATCH_SIZE) {
                if let Some(first) = chunk.first() {
                    progress.current_path = first.path.clone();
                    let _ = tx.send(BgMsg::DeleteProgress(progress.clone()));
                }
                let paths: Vec<PathBuf> = chunk.iter().map(|item| item.path.clone()).collect();
                let outcomes = space_clean::trash_paths_batch(&paths);
                for (item, outcome) in chunk.iter().cloned().zip(outcomes) {
                    progress.current_path = item.path.clone();
                    match outcome {
                        Ok(()) => {
                            deleted_paths.push(item.path);
                            progress.items_done += 1;
                            progress.bytes_processed =
                                progress.bytes_processed.saturating_add(item.bytes);
                            if item.is_dir {
                                progress.folders_done += 1;
                            } else {
                                progress.files_done += 1;
                            }
                        }
                        Err(e) => {
                            errors.push(format!("{}: {e}", item.path.display()));
                            progress.items_done += 1;
                            progress.error_count = errors.len();
                        }
                    }
                }
                let _ = tx.send(BgMsg::DeleteProgress(progress.clone()));
            }
            processed_bytes = progress.bytes_processed;
        }
        space_clean::DeleteMethod::Permanent => {
            let perm_candidates: Vec<space_clean::DeleteCandidate> = candidates
                .iter()
                .map(|item| space_clean::DeleteCandidate {
                    path: item.path.clone(),
                    bytes: item.bytes,
                    is_dir: item.is_dir,
                })
                .collect();
            let outcomes = space_clean::delete_candidates_permanently_with_progress(
                &perm_candidates,
                progress.clone(),
                |p| {
                    progress = p.clone();
                    let _ = tx.send(BgMsg::DeleteProgress(p));
                },
            );
            for (item, outcome) in candidates.into_iter().zip(outcomes) {
                match outcome {
                    Ok(_) => {
                        deleted_paths.push(item.path);
                        processed_bytes = processed_bytes.saturating_add(item.bytes);
                        freed_bytes = freed_bytes.saturating_add(item.bytes);
                    }
                    Err(e) => errors.push(format!("{}: {e}", item.path.display())),
                }
            }
            processed_bytes = processed_bytes.max(progress.bytes_processed);
            freed_bytes = freed_bytes.max(progress.bytes_freed);
        }
    }
    if deleted_paths.is_empty() && !errors.is_empty() {
        return Err(errors.join("\n"));
    }

    Ok(TrashOutcome {
        deleted_paths,
        processed_bytes,
        freed_bytes,
        method,
        errors,
    })
}

#[cfg(test)]
mod permanent_delete_tests {
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(label: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("space-app-{label}-{}-{nanos}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn run_delete_batch_permanent_removes_readonly_tree_and_preserves_symlink_target() {
        let dir = TestDir::new("perm-delete");
        let outside = dir.path().join("outside.txt");
        fs::write(&outside, b"keep").unwrap();

        let target = dir.path().join("victim");
        let nested = target.join("sub");
        fs::create_dir_all(&nested).unwrap();
        let ro_file = nested.join("ro.bin");
        fs::write(&ro_file, vec![7_u8; 64]).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, nested.join("link")).unwrap();
            fs::set_permissions(&ro_file, fs::Permissions::from_mode(0o444)).unwrap();
            fs::set_permissions(&nested, fs::Permissions::from_mode(0o555)).unwrap();
        }

        let (tx, rx) = crossbeam_channel::unbounded();
        let outcome = run_delete_batch(
            vec![BasketItem {
                path: target.clone(),
                bytes: 64,
                is_dir: true,
                junk_guard: None,
            }],
            space_clean::DeleteMethod::Permanent,
            &tx,
        )
        .expect("permanent batch succeeds");

        assert!(!target.exists());
        assert!(outside.exists());
        assert_eq!(outcome.deleted_paths, vec![target]);
        assert_eq!(outcome.freed_bytes, 64);
        assert_eq!(outcome.method, space_clean::DeleteMethod::Permanent);
        assert!(outcome.errors.is_empty());
        let progress_msgs: Vec<_> = rx
            .try_iter()
            .filter(|m| matches!(m, BgMsg::DeleteProgress(_)))
            .collect();
        assert!(!progress_msgs.is_empty());
    }

    #[test]
    fn basket_item_from_finding_preserves_rule_guard() {
        let finding = space_clean::Finding {
            rule_id: "r1".into(),
            category: "Caches".into(),
            risk: "safe".into(),
            path: PathBuf::from("/tmp/example-cache"),
            bytes: 128,
            is_dir: true,
            report_only: false,
            root: Some(PathBuf::from("/tmp")),
            min_age_days: 7,
            modified_unix: None,
        };
        let item = SpaceApp::basket_item_from_finding(&finding);
        assert!(item.is_dir);
        assert_eq!(item.bytes, 128);
        assert_eq!(item.junk_guard, Some((Some(PathBuf::from("/tmp")), 7)));
    }
}
