use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Receiver};
use eframe::egui::{self, Key};
use space_core::{
    rescan_subtree_with_options, scan_path_with_options, CacheMeta, CacheStore, NodeId, ScanEvent,
    ScanHandle, ScanProgress, SizeMode, SpaceTree,
};

use crate::anim::{TransKind, Transition};
use crate::downloads::{self, DownloadItem, DownloadsState};
use crate::junk_panel::JunkPanelState;
use crate::largest_files::{InsightData, InsightTab, LargestFiles, LargestRequest, LargestRow};
use crate::review;
use crate::rules_panel::RulesPanelState;
use crate::settings::AppSettings;
use crate::theme;
use crate::tm_clean::{self, CleanReport, LocalSnapshot};
use crate::views::{GeomCache, ViewFilter, ViewMode};
use crate::volumes::{self, VolumeInfo};

mod ui;

#[derive(Clone, Debug)]
pub(crate) struct BasketItem {
    pub(crate) path: PathBuf,
    pub(crate) bytes: u64,
    pub(crate) is_dir: bool,
}

pub(crate) fn basket_total_bytes(items: &[BasketItem]) -> u64 {
    items
        .iter()
        .fold(0_u64, |sum, item| sum.saturating_add(item.bytes))
}

struct TrashOutcome {
    deleted_paths: Vec<PathBuf>,
    processed_bytes: u64,
    errors: Vec<String>,
}

struct RescanSavedPaths {
    focus: PathBuf,
    selected: Option<PathBuf>,
    expanded: Vec<PathBuf>,
    review: Vec<PathBuf>,
    history_back: Vec<PathBuf>,
    history_fwd: Vec<PathBuf>,
}

enum BgMsg {
    RuleManagerLoaded(Result<space_clean::RuleManager, String>),
    Volumes(Vec<VolumeInfo>),
    Snapshots(Vec<LocalSnapshot>),
    DownloadsDone(Vec<DownloadItem>),
    Recent(Vec<CacheMeta>),
    /// Cache hit (or forced miss path will scan).
    CacheResult {
        path: PathBuf,
        loaded: Option<(Arc<SpaceTree>, bool /*stale*/, bool /*offline*/)>,
    },
    CacheSaved(Vec<CacheMeta>),
    RescanDone(Result<(Arc<SpaceTree>, RescanSavedPaths), String>),
    DeleteProgress(space_clean::CleanProgress),
    DeleteDone(Result<TrashOutcome, String>),
    TmCleanDone(CleanReport),
    JunkScanProgress(space_clean::JunkScanProgress),
    JunkScanDone(Result<space_clean::ScanReport, String>),
    Largest {
        request: LargestRequest,
        data: InsightData,
    },
}

/// Top-level screens: quick cleanup and interactive folder exploration.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Mode {
    Clean,
    Explore,
}

pub struct SpaceApp {
    mode: Mode,
    tree: Option<Arc<SpaceTree>>,
    focus: NodeId,
    selected: Option<NodeId>,
    selected_set: Vec<NodeId>,
    selection_anchor: Option<NodeId>,
    /// Live hover from the central view (tooltip + sidebar highlight).
    hovered: Option<NodeId>,
    view: ViewMode,
    size_mode: SizeMode,
    scan: Option<ScanHandle>,
    progress: Option<ScanProgress>,
    status: String,
    cache: Option<CacheStore>,
    recent: Vec<CacheMeta>,
    from_cache: bool,
    cache_stale: bool,
    cache_offline: bool,
    show_settings: bool,
    show_about: bool,
    show_goto: bool,
    goto_draft: String,
    focus_goto: bool,
    pending_screenshot: Option<PathBuf>,
    last_exported_file: Option<PathBuf>,
    skipped_expanded: bool,
    exclude_draft: String,
    filter: ViewFilter,
    history_back: Vec<NodeId>,
    history_fwd: Vec<NodeId>,
    volumes: Vec<VolumeInfo>,
    volumes_loading: bool,
    review: Vec<NodeId>,
    confirm_delete: bool,
    delete_progress: Option<space_clean::CleanProgress>,
    delete_started: Option<Instant>,
    show_tm_panel: bool,
    confirm_tm_clean: bool,
    tm_snapshots: Vec<LocalSnapshot>,
    tm_loading: bool,
    junk: JunkPanelState,
    pub(crate) downloads: DownloadsState,
    pub(crate) choosing_target: bool,
    rules: RulesPanelState,
    rule_mgr: Option<space_clean::RuleManager>,
    rule_mgr_loading: bool,
    bg_rx: Receiver<BgMsg>,
    bg_tx: crossbeam_channel::Sender<BgMsg>,
    /// True while cache load / rescan / delete runs off-thread.
    busy: bool,
    largest: LargestFiles,
    insight_tab: InsightTab,
    /// Cached view geometry (sunburst mesh / treemap tiles).
    geom: Option<GeomCache>,
    settings: AppSettings,
    filter_draft: String,
    ext_draft: String,
    min_mb_draft: String,
    max_mb_draft: String,
    filter_edited_at: Option<Instant>,
    show_shortcuts: bool,
    focus_filter: bool,
    transition: Option<Transition>,
    pending_open: Option<PathBuf>,
    show_largest: bool,
    /// Folders opened inline in the contents tree.
    expanded: HashSet<NodeId>,
    /// Visible tree rows in display order, as drawn last frame (keyboard nav).
    tree_rows: Vec<NodeId>,
    /// Selection the tree last revealed; a change expands its ancestors.
    tree_revealed: Option<NodeId>,
    /// Scroll the selected row into view on the next frame.
    tree_scroll: bool,
    compact_map: bool,
}

impl SpaceApp {
    pub fn new(cc: &eframe::CreationContext<'_>, mut settings: AppSettings) -> Self {
        settings.skin = settings.skin.normalized_appearance();
        settings.clamp();
        theme::set_high_contrast(settings.high_contrast);
        theme::apply(&cc.egui_ctx, settings.skin);
        theme::apply_motion(&cc.egui_ctx, settings.reduce_motion);
        crate::i18n::set(settings.language.unwrap_or_else(crate::i18n::detect));
        crate::i18n::set_byte_unit(settings.byte_unit);

        let cache = CacheStore::open().ok();
        let (bg_tx, bg_rx) = unbounded();

        {
            let tx = bg_tx.clone();
            thread::Builder::new()
                .name("space-vols".into())
                .spawn(move || {
                    let _ = tx.send(BgMsg::Volumes(volumes::list_volumes()));
                })
                .ok();
        }
        {
            let tx = bg_tx.clone();
            thread::Builder::new()
                .name("space-snaps".into())
                .spawn(move || {
                    let _ = tx.send(BgMsg::Snapshots(tm_clean::list_local_snapshots()));
                })
                .ok();
        }
        if let Some(cache) = cache.clone() {
            let tx = bg_tx.clone();
            let ttl_days = settings.cache_ttl_days;
            thread::Builder::new()
                .name("space-recent".into())
                .spawn(move || {
                    if ttl_days > 0 {
                        let _ = cache.prune_ttl(ttl_days);
                    }
                    let recent = cache.list_recent().unwrap_or_default();
                    let _ = tx.send(BgMsg::Recent(recent));
                })
                .ok();
        }

        let view = settings.default_view;
        let size_mode = settings.default_size_mode;
        let pending_open = folder_arg(std::env::args());
        let mode = if pending_open.is_some() {
            Mode::Explore
        } else {
            Mode::Clean
        };

        let mut app = Self {
            tree: None,
            focus: NodeId(0),
            selected: None,
            selected_set: Vec::new(),
            selection_anchor: None,
            hovered: None,
            view,
            size_mode,
            scan: None,
            progress: None,
            status: String::new(),
            cache,
            recent: Vec::new(),
            from_cache: false,
            cache_stale: false,
            cache_offline: false,
            show_settings: false,
            show_about: false,
            show_goto: false,
            goto_draft: String::new(),
            focus_goto: false,
            pending_screenshot: None,
            last_exported_file: None,
            skipped_expanded: false,
            exclude_draft: settings.exclude_names.join(", "),
            filter: ViewFilter::default(),
            history_back: Vec::new(),
            history_fwd: Vec::new(),
            volumes: volumes::list_volumes_fast(),
            volumes_loading: true,
            review: Vec::new(),
            mode,
            confirm_delete: false,
            delete_progress: None,
            delete_started: None,
            show_tm_panel: false,
            confirm_tm_clean: false,
            tm_snapshots: Vec::new(),
            tm_loading: true,
            junk: JunkPanelState::default(),
            downloads: DownloadsState::default(),
            choosing_target: false,
            rules: RulesPanelState::default(),
            rule_mgr: None,
            rule_mgr_loading: false,
            bg_rx,
            bg_tx,
            busy: false,
            largest: LargestFiles::default(),
            insight_tab: InsightTab::default(),
            geom: None,
            settings,
            filter_draft: String::new(),
            ext_draft: String::new(),
            min_mb_draft: String::new(),
            max_mb_draft: String::new(),
            filter_edited_at: None,
            show_shortcuts: false,
            focus_filter: false,
            transition: None,
            pending_open,
            show_largest: false,
            expanded: HashSet::new(),
            tree_rows: Vec::new(),
            tree_revealed: None,
            tree_scroll: false,
            compact_map: false,
        };
        if app.mode == Mode::Clean {
            app.start_quick_clean_scan();
        }
        app
    }

    fn refresh_volumes_async(&mut self) {
        self.volumes_loading = true;
        let tx = self.bg_tx.clone();
        thread::Builder::new()
            .name("space-vols".into())
            .spawn(move || {
                let _ = tx.send(BgMsg::Volumes(volumes::list_volumes()));
            })
            .ok();
    }

    fn poll_bg(&mut self, ctx: &egui::Context) {
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
                    let n = outcome.deleted_paths.len();
                    self.status = if outcome.errors.is_empty() {
                        crate::tr!(
                            "Moved {} items to Trash · {}",
                            n,
                            crate::i18n::bytes(outcome.processed_bytes)
                        )
                    } else {
                        crate::tr!("Moved {} items · {} failed", n, outcome.errors.len())
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
                    self.status = crate::tr!("Trash failed: {}", e);
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
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        } else if self.largest.is_loading() {
            // Don't spin the UI at 20fps while a background walk finishes.
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        } else if got {
            ctx.request_repaint();
        }
    }

    fn invalidate_largest(&mut self) {
        self.largest.invalidate();
        self.geom = None;
    }

    fn schedule_largest(&mut self) {
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
        let result = thread::Builder::new()
            .name("space-largest".into())
            .spawn(move || {
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

    fn save_cache_async(&self, tree: Arc<SpaceTree>) {
        let Some(cache) = self.cache.clone() else {
            return;
        };
        let tx = self.bg_tx.clone();
        thread::Builder::new()
            .name("space-cache-save".into())
            .spawn(move || {
                let _ = cache.save(tree.as_ref());
                let recent = cache.list_recent().unwrap_or_default();
                let _ = tx.send(BgMsg::CacheSaved(recent));
            })
            .ok();
    }

    fn start_scan(&mut self, path: PathBuf, force: bool) {
        self.mode = Mode::Explore;
        self.choosing_target = false;
        if let Some(scan) = self.scan.take() {
            scan.cancel();
        }

        if !force {
            if let Some(cache) = self.cache.clone() {
                self.busy = true;
                self.status = crate::tr!("Loading cache for {}…", path.display());
                self.tree = None;
                self.progress = None;
                self.invalidate_largest();
                let tx = self.bg_tx.clone();
                let path_bg = path.clone();
                let ttl_days = self.settings.cache_ttl_days;
                thread::Builder::new()
                    .name("space-cache-load".into())
                    .spawn(move || {
                        let loaded = cache
                            .load_with_ttl(&path_bg, ttl_days)
                            .ok()
                            .map(|l| (Arc::new(l.tree), l.stale, l.offline));
                        let _ = tx.send(BgMsg::CacheResult {
                            path: path_bg,
                            loaded,
                        });
                    })
                    .ok();
                return;
            }
        }

        self.begin_filesystem_scan(path);
    }

    fn begin_filesystem_scan(&mut self, path: PathBuf) {
        self.from_cache = false;
        self.cache_stale = false;
        self.cache_offline = false;
        self.skipped_expanded = false;
        self.busy = false;
        self.tree = None;
        self.review.clear();
        self.history_back.clear();
        self.history_fwd.clear();
        self.reset_tree_view_state();
        self.invalidate_largest();
        self.progress = Some(ScanProgress {
            files_seen: 0,
            bytes_seen: 0,
            current_path: path.display().to_string(),
            skipped_errors: 0,
        });
        self.status = crate::tr!("Scanning {}…", path.display());
        self.scan = Some(scan_path_with_options(path, self.settings.scan_options()));
    }

    fn rescan_current(&mut self) {
        let Some(tree) = &self.tree else { return };
        let path = PathBuf::from(&tree.root_path);
        self.start_scan(path, true);
    }

    fn incremental_rescan_focus(&mut self) {
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
        thread::Builder::new()
            .name("space-rescan".into())
            .spawn(move || {
                let result = rescan_subtree_with_options(tree.as_ref(), focus, opts)
                    .map(|t| (Arc::new(t), saved));
                let _ = tx.send(BgMsg::RescanDone(result));
            })
            .ok();
    }

    fn poll_scan(&mut self, ctx: &egui::Context) {
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
            self.tree = Some(arc);
            self.scan = None;
            self.progress = None;
            self.invalidate_largest();
            self.schedule_largest();
        } else {
            ctx.request_repaint();
        }
    }

    fn navigate(&mut self, id: NodeId, push_history: bool) {
        let Some(tree) = &self.tree else { return };
        if let Some(target_node) = tree.try_node(id) {
            if !target_node.is_dir {
                return;
            }
            if id == self.focus {
                return;
            }
            let zoom_in = tree.path_to(id).contains(&self.focus) && id != self.focus;
            // Child of current focus → zoom in; otherwise zoom out / sideways.
            let kind = if zoom_in || target_node.parent.is_some_and(|p| p == self.focus) {
                TransKind::ZoomIn
            } else {
                TransKind::ZoomOut
            };
            if push_history {
                self.history_back.push(self.focus);
                self.history_fwd.clear();
            }
            self.focus = id;
            self.clear_selection();
            self.hovered = None;
            self.transition = if self.settings.reduce_motion {
                None
            } else {
                Some(Transition::start(kind))
            };
            self.invalidate_largest();
            self.schedule_largest();
        }
    }

    fn set_view(&mut self, mode: ViewMode) {
        if self.view != mode {
            self.view = mode;
            self.geom = None;
            self.transition = if self.settings.reduce_motion {
                None
            } else {
                Some(Transition::start(TransKind::ViewSwitch))
            };
        }
    }

    fn copy_focus_path(&mut self) {
        let Some(tree) = &self.tree else { return };
        let id = self.hovered.or(self.selected).unwrap_or(self.focus);
        let path = tree.path_string(id);
        self.status = crate::tr!("Copied {}", path);
        self.clipboard_set(&path);
    }

    fn clipboard_set(&self, text: &str) {
        // egui context clipboard is set from update; stash via arboard-less cmd on macOS.
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("pbcopy")
                .stdin(std::process::Stdio::piped())
                .spawn()
                .and_then(|mut c| {
                    use std::io::Write;
                    if let Some(mut stdin) = c.stdin.take() {
                        stdin.write_all(text.as_bytes())?;
                    }
                    c.wait()?;
                    Ok(())
                });
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = text;
        }
    }

    fn percent_of_parent(&self, id: NodeId) -> Option<f32> {
        let tree = self.tree.as_ref()?;
        let node = tree.try_node(id)?;
        let parent = node.parent?;
        let psize = tree.try_node(parent)?.size(self.size_mode).max(1);
        Some((node.size(self.size_mode) as f32 / psize as f32) * 100.0)
    }

    fn apply_filter_debounce(&mut self) {
        let ms = self.settings.flags.debounce_filter_ms;
        if let Some(at) = self.filter_edited_at {
            if at.elapsed() >= Duration::from_millis(ms) {
                if self.filter.query != self.filter_draft {
                    self.filter.query = self.filter_draft.clone();
                    self.filter.rebuild_regex();
                    self.geom = None;
                }
                self.filter_edited_at = None;
            }
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let input = ctx.input(|i| {
            (
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::ArrowLeft),
                i.key_pressed(Key::ArrowRight),
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Escape),
                i.key_pressed(Key::Home),
                i.key_pressed(Key::End),
                i.key_pressed(Key::Slash),
                i.key_pressed(Key::Questionmark),
                i.modifiers,
                i.key_pressed(Key::Num1),
                i.key_pressed(Key::Num2),
                i.key_pressed(Key::Num3),
                i.key_pressed(Key::Num4),
                i.key_pressed(Key::C),
                i.key_pressed(Key::F),
                i.key_pressed(Key::R),
                i.key_pressed(Key::O),
                i.key_pressed(Key::J),
                i.key_pressed(Key::L),
                i.key_pressed(Key::Y),
                i.key_pressed(Key::Delete),
                i.key_pressed(Key::Backspace),
                i.key_pressed(Key::Space),
                i.key_pressed(Key::Comma),
            )
        });
        let (
            up,
            down,
            left,
            right,
            enter,
            esc,
            home,
            end,
            slash,
            question,
            mods,
            n1,
            n2,
            n3,
            n4,
            key_c,
            key_f,
            key_r,
            key_o,
            key_j,
            key_l,
            key_y,
            key_del,
            key_back,
            space,
            key_comma,
        ) = input;

        let in_explore = self.mode == Mode::Explore && !self.choosing_target && self.tree.is_some();

        if (question || (mods.shift && slash)) && !ctx.wants_keyboard_input() {
            self.show_shortcuts = !self.show_shortcuts;
        }
        if mods.command || mods.ctrl {
            if in_explore {
                if n1 {
                    self.set_view(ViewMode::Sunburst);
                }
                if n2 {
                    self.set_view(ViewMode::Container);
                }
                if n3 {
                    self.set_view(ViewMode::Radial);
                }
                if n4 {
                    self.set_view(ViewMode::Cascade);
                }
                if key_c && mods.shift {
                    self.copy_focus_path();
                }
                if key_f {
                    self.focus_filter = true;
                }
                if left {
                    self.go_back();
                }
                if right {
                    self.go_forward();
                }
            }
            if key_r && self.scan.is_none() && !self.busy {
                if self.mode == Mode::Clean && !self.junk.loading && !self.downloads.loading {
                    self.start_quick_clean_scan();
                } else if in_explore {
                    if mods.shift {
                        self.incremental_rescan_focus();
                    } else {
                        self.rescan_current();
                    }
                }
            }
            if key_o && self.scan.is_none() && !self.busy {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    self.start_scan(path, false);
                }
            }
            if key_j {
                self.open_junk_panel();
            }
            if key_l {
                self.open_goto_dialog();
            }
            if key_y && in_explore && !ctx.wants_keyboard_input() {
                self.quick_look_selected();
            }
            if key_comma {
                self.show_settings = !self.show_settings;
                self.show_about = false;
                self.show_goto = false;
            }
            if (key_del || key_back) && !ctx.wants_keyboard_input() {
                let selected_deletable = if in_explore {
                    self.selected.filter(|&id| {
                        self.tree.as_ref().is_some_and(|tree| {
                            tree.try_node(id)
                                .is_some_and(|n| !n.synthetic && n.parent.is_some())
                        })
                    })
                } else {
                    None
                };
                if self.basket_len() == 0 {
                    if let (Some(id), Some(tree)) = (selected_deletable, self.tree.as_ref()) {
                        review::toggle_in_tree(tree, &mut self.review, id);
                    }
                }
                if self.basket_len() > 0 {
                    self.request_trash();
                }
            }
            return;
        }

        if slash && in_explore && !ctx.wants_keyboard_input() {
            self.focus_filter = true;
        }
        if esc {
            if self.scan.is_some() {
                if let Some(s) = &self.scan {
                    s.cancel();
                }
            } else if self.show_goto {
                self.show_goto = false;
            } else if self.show_settings {
                self.show_settings = false;
            } else if self.show_about {
                self.show_about = false;
            } else if self.show_shortcuts {
                self.show_shortcuts = false;
            } else if self.confirm_delete && self.delete_progress.is_none() {
                self.confirm_delete = false;
            } else if self.rules.open {
                self.rules.open = false;
            } else if self.show_tm_panel {
                self.show_tm_panel = false;
            } else if self.choosing_target && self.tree.is_some() {
                self.choosing_target = false;
            } else if self.filter.is_active() || !self.filter_draft.is_empty() {
                self.reset_filters();
            } else if in_explore {
                // Esc never switches tabs; going up only means something here.
                self.go_up();
            }
        }
        if in_explore && !ctx.wants_keyboard_input() && !mods.command && !mods.ctrl {
            if !self.any_modal_open() {
                if space {
                    self.quick_look_selected();
                    return;
                }
                if key_del || key_back {
                    self.toggle_selected_in_basket();
                    return;
                }
            }
            if self.tree_keys(up, down, left, right, mods.shift) {
                return;
            }
            if left || right {
                self.cycle_map_child(right);
            }
            if up {
                self.go_up();
            }
            if home {
                if let Some(tree) = &self.tree {
                    self.navigate(tree.root, true);
                }
            }
            if end || down {
                // Select / focus largest visible child in narrow map mode.
                if let Some(tree) = self.tree.clone() {
                    let first = tree.children(self.focus).iter().copied().find(|&cid| {
                        !self.filter.is_active() || {
                            let n = tree.node(cid);
                            self.filter
                                .matches(&n.name, n.is_dir, n.size(self.size_mode))
                        }
                    });
                    if let Some(first) = first {
                        self.select_single(first);
                    }
                }
            }
        }
        if enter {
            if self.confirm_delete && self.delete_progress.is_none() && self.basket_len() > 0 {
                self.apply_basket();
                return;
            }
            if self.confirm_tm_clean {
                self.apply_tm_clean();
                return;
            }
            if self.show_shortcuts {
                self.show_shortcuts = false;
                return;
            }
            if in_explore && !ctx.wants_keyboard_input() {
                let id = self.selected.or(self.hovered);
                if let Some(id) = id {
                    let is_dir = self
                        .tree
                        .as_ref()
                        .is_some_and(|t| t.try_node(id).is_some_and(|n| n.is_dir));
                    if is_dir {
                        self.navigate(id, true);
                    } else {
                        self.select_single(id);
                        self.reveal_selected();
                    }
                }
            }
        }
    }

    fn persist_settings_from_viewport(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            if let Some(rect) = i.viewport().inner_rect {
                self.settings.window_width = rect.width();
                self.settings.window_height = rect.height();
            }
        });
        self.settings.default_view = self.view;
        self.settings.default_size_mode = self.size_mode;
    }

    fn go_up(&mut self) {
        if let Some(tree) = &self.tree {
            if let Some(parent) = tree.try_node(self.focus).and_then(|n| n.parent) {
                self.navigate(parent, true);
            }
        }
    }

    fn go_back(&mut self) {
        if let Some(prev) = self.history_back.pop() {
            self.history_fwd.push(self.focus);
            self.focus = prev;
            self.clear_selection();
            self.invalidate_largest();
            self.schedule_largest();
        }
    }

    fn go_forward(&mut self) {
        if let Some(next) = self.history_fwd.pop() {
            self.history_back.push(self.focus);
            self.focus = next;
            self.clear_selection();
            self.invalidate_largest();
            self.schedule_largest();
        }
    }

    fn reset_tree_view_state(&mut self) {
        self.clear_selection();
        self.hovered = None;
        self.expanded.clear();
        self.tree_rows.clear();
        self.tree_revealed = None;
        self.tree_scroll = false;
        self.geom = None;
    }

    fn select_single(&mut self, id: NodeId) {
        self.selected = Some(id);
        self.selected_set.clear();
        self.selected_set.push(id);
        self.selection_anchor = Some(id);
    }

    fn clear_selection(&mut self) {
        self.selected = None;
        self.selected_set.clear();
        self.selection_anchor = None;
    }

    fn toggle_select_node(&mut self, id: NodeId) {
        if let Some(pos) = self.selected_set.iter().position(|&x| x == id) {
            self.selected_set.remove(pos);
            if self.selected == Some(id) {
                self.selected = self.selected_set.last().copied();
            }
        } else {
            self.selected_set.push(id);
            self.selected = Some(id);
        }
        self.selection_anchor = self.selected;
    }

    fn select_range_in_rows(&mut self, to: NodeId) {
        let anchor = self.selection_anchor.or(self.selected);
        if let Some(a) = anchor {
            if let (Some(i0), Some(i1)) = (
                self.tree_rows.iter().position(|&x| x == a),
                self.tree_rows.iter().position(|&x| x == to),
            ) {
                let (lo, hi) = if i0 <= i1 { (i0, i1) } else { (i1, i0) };
                self.selected_set = self.tree_rows[lo..=hi].to_vec();
                self.selected = Some(to);
                return;
            }
        }
        self.select_single(to);
    }

    fn reset_filters(&mut self) {
        self.filter.clear();
        self.filter_draft.clear();
        self.ext_draft.clear();
        self.min_mb_draft.clear();
        self.max_mb_draft.clear();
        self.filter_edited_at = None;
        self.geom = None;
    }

    fn cycle_map_child(&mut self, forward: bool) {
        let Some(tree) = &self.tree else { return };
        let kids: Vec<NodeId> = if !self.filter.is_active() {
            tree.children(self.focus).to_vec()
        } else {
            tree.children(self.focus)
                .iter()
                .copied()
                .filter(|&cid| {
                    let n = tree.node(cid);
                    self.filter
                        .matches(&n.name, n.is_dir, n.size(self.size_mode))
                })
                .collect()
        };
        if kids.is_empty() {
            return;
        }
        let next = match self
            .selected
            .and_then(|s| kids.iter().position(|&c| c == s))
        {
            Some(i) if forward => kids[(i + 1) % kids.len()],
            Some(i) => kids[(i + kids.len() - 1) % kids.len()],
            None if forward => kids[0],
            None => kids[kids.len() - 1],
        };
        self.select_single(next);
    }

    fn restore_rescan_tree(&mut self, mut tree: Arc<SpaceTree>, saved: RescanSavedPaths) {
        if self.size_mode != SizeMode::OnDisk {
            Arc::make_mut(&mut tree).sort_children_by(self.size_mode);
        }
        self.focus = find_path(&tree, &saved.focus).unwrap_or(tree.root);
        self.selected = saved.selected.as_deref().and_then(|p| find_path(&tree, p));
        self.selected_set = self.selected.into_iter().collect();
        self.selection_anchor = self.selected;
        self.expanded = saved
            .expanded
            .iter()
            .filter_map(|p| find_path(&tree, p))
            .collect();
        self.review.clear();
        for p in &saved.review {
            if let Some(id) = find_path(&tree, p) {
                if tree
                    .try_node(id)
                    .is_some_and(|n| !n.synthetic && n.parent.is_some())
                {
                    self.review.push(id);
                }
            }
        }
        self.history_back = saved
            .history_back
            .iter()
            .filter_map(|p| find_path(&tree, p))
            .filter(|&id| id != self.focus)
            .collect();
        self.history_fwd = saved
            .history_fwd
            .iter()
            .filter_map(|p| find_path(&tree, p))
            .filter(|&id| id != self.focus)
            .collect();
        self.hovered = None;
        self.tree_rows.clear();
        self.tree_revealed = None;
        self.tree_scroll = false;
        self.geom = None;
        self.cache_stale = false;
        self.from_cache = false;
        self.status = crate::tr!(
            "Rescanned · {} items · {}",
            tree.item_count,
            crate::i18n::bytes(tree.total_size_mode(self.size_mode))
        );
        self.tree = Some(tree);
        self.invalidate_largest();
        self.schedule_largest();
        if let Some(tree) = self.tree.clone() {
            self.save_cache_async(tree);
        }
    }

    /// A rule path (may start with `~` or contain a glob): select it in Folders
    /// when the open scan indexed it, otherwise reveal it in Finder.
    fn reveal_rule_path(&mut self, raw: &str) {
        let mut path = raw.to_owned();
        if let (Some(rest), Some(home)) = (path.strip_prefix('~'), std::env::var_os("HOME")) {
            path = format!("{}{rest}", home.to_string_lossy());
        }
        // Stop at the first glob component: that folder is what exists.
        let path: PathBuf = PathBuf::from(&path)
            .components()
            .take_while(|c| !c.as_os_str().to_string_lossy().contains(['*', '?', '[']))
            .collect();
        if let Some(id) = self.tree.as_ref().and_then(|t| find_path(t, &path)) {
            let parent = self
                .tree
                .as_ref()
                .and_then(|t| t.try_node(id))
                .and_then(|n| n.parent);
            if let Some(parent) = parent {
                self.navigate(parent, true);
            }
            self.select_single(id);
            self.rules.open = false;
            self.choosing_target = false;
            self.mode = Mode::Explore;
            return;
        }
        crate::widgets::reveal_in_os(&path);
        self.status = crate::tr!("Not in this scan — opened {} in Finder", path.display());
    }

    fn reveal_selected(&self) {
        let Some(tree) = &self.tree else { return };
        let id = self.selected.unwrap_or(self.focus);
        let path = tree.path_string(id);
        crate::widgets::reveal_in_os(std::path::Path::new(&path));
    }

    fn quick_look_selected(&self) {
        let Some(tree) = &self.tree else { return };
        let id = self.selected.or(self.hovered).unwrap_or(self.focus);
        if let Some(n) = tree.try_node(id) {
            if !n.synthetic {
                let path = tree.path_string(id);
                crate::widgets::quick_look(std::path::Path::new(&path));
            }
        }
    }

    fn open_terminal_selected(&self) {
        let Some(tree) = &self.tree else { return };
        let id = self.selected.or(self.hovered).unwrap_or(self.focus);
        if let Some(n) = tree.try_node(id) {
            if !n.synthetic {
                let path = tree.path_string(id);
                crate::widgets::open_in_terminal(std::path::Path::new(&path));
            }
        }
    }

    fn toggle_selected_in_basket(&mut self) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        let ids: Vec<NodeId> = if !self.selected_set.is_empty() {
            self.selected_set.clone()
        } else if let Some(id) = self.selected {
            vec![id]
        } else {
            return;
        };
        for id in ids {
            if tree
                .try_node(id)
                .is_some_and(|n| !n.synthetic && n.parent.is_some())
            {
                review::toggle_in_tree(&tree, &mut self.review, id);
            }
        }
    }

    fn open_goto_dialog(&mut self) {
        self.show_goto = true;
        self.focus_goto = true;
        if self.goto_draft.is_empty() {
            if let Some(tree) = &self.tree {
                self.goto_draft = tree.path_string(self.focus);
            }
        }
    }

    fn any_modal_open(&self) -> bool {
        self.confirm_delete
            || self.confirm_tm_clean
            || self.show_settings
            || self.show_about
            || self.show_shortcuts
            || self.show_goto
            || self.rules.open
            || self.show_tm_panel
    }

    fn refresh_snapshots(&mut self) {
        self.tm_loading = true;
        let tx = self.bg_tx.clone();
        thread::Builder::new()
            .name("space-snaps".into())
            .spawn(move || {
                let _ = tx.send(BgMsg::Snapshots(tm_clean::list_local_snapshots()));
            })
            .ok();
    }

    fn open_tm_panel(&mut self) {
        self.show_tm_panel = true;
        self.refresh_snapshots();
    }

    fn apply_tm_clean(&mut self) {
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
        thread::Builder::new()
            .name("space-tm-clean".into())
            .spawn(move || {
                let report = tm_clean::delete_old_tm_locals(keep);
                let _ = tx.send(BgMsg::TmCleanDone(report));
            })
            .ok();
    }

    fn start_rule_mgr_load(&mut self) {
        if self.rule_mgr.is_some() || self.rule_mgr_loading {
            return;
        }
        self.rule_mgr_loading = true;
        let tx = self.bg_tx.clone();
        if let Err(error) = thread::Builder::new()
            .name("space-rule-load".into())
            .spawn(move || {
                let result =
                    space_clean::RuleManager::load_default().map_err(|error| error.to_string());
                let _ = tx.send(BgMsg::RuleManagerLoaded(result));
            })
        {
            self.rule_mgr_loading = false;
            self.rules.error = Some(error.to_string());
        }
    }

    fn open_rules_panel(&mut self) {
        self.rules.open = true;
        self.rules.error = None;
        self.start_rule_mgr_load();
    }

    fn open_junk_panel(&mut self) {
        self.mode = Mode::Clean;
        self.start_quick_clean_scan();
    }

    pub(crate) fn start_downloads_scan(&mut self) {
        self.downloads.loading = true;
        let tx = self.bg_tx.clone();
        thread::Builder::new()
            .name("space-downloads".into())
            .spawn(move || {
                let _ = tx.send(BgMsg::DownloadsDone(downloads::scan_default_downloads()));
            })
            .ok();
    }

    pub(crate) fn start_quick_clean_scan(&mut self) {
        self.junk.open_and_scan();
        self.start_junk_scan();
        self.start_downloads_scan();
    }

    fn set_mode(&mut self, mode: Mode) {
        if mode == Mode::Clean {
            self.junk.open = true;
            if self.junk.report().is_none() && !self.junk.loading {
                self.start_quick_clean_scan();
            }
        }
        self.mode = mode;
    }

    fn start_junk_scan(&mut self) {
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
        thread::Builder::new()
            .name("space-junk-scan".into())
            .spawn(move || {
                let result = (|| {
                    let pack = space_clean::RulePack::load_many_shared(
                        &space_clean::default_rules_dirs(),
                    )?;
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
            })
            .ok();
    }

    fn review_bytes(&self) -> u64 {
        self.tree
            .as_ref()
            .map(|t| review::queue_bytes(t, &self.review, self.size_mode))
            .unwrap_or(0)
    }

    /// Unified Trash queue combining Quick clean, Downloads, and Folders selections.
    pub(crate) fn basket_items(&self) -> Vec<BasketItem> {
        let mut items = Vec::new();
        if let Some(tree) = &self.tree {
            items.extend(
                self.review
                    .iter()
                    .copied()
                    .filter(|&id| {
                        is_live_node(tree, id)
                            && tree
                                .try_node(id)
                                .is_some_and(|n| !n.synthetic && n.parent.is_some())
                    })
                    .filter_map(|id| {
                        let node = tree.try_node(id)?;
                        Some(BasketItem {
                            path: PathBuf::from(tree.path_string(id)),
                            bytes: node.size(self.size_mode),
                            is_dir: node.is_dir,
                        })
                    }),
            );
        }
        items.extend(self.junk.selected_findings().into_iter().map(|finding| {
            let is_dir = std::fs::symlink_metadata(&finding.path)
                .map(|m| m.is_dir())
                .unwrap_or(false);
            BasketItem {
                path: finding.path,
                bytes: finding.bytes,
                is_dir,
            }
        }));
        items.extend(
            self.downloads
                .selected_items()
                .into_iter()
                .map(|item| BasketItem {
                    path: item.path,
                    bytes: item.bytes,
                    is_dir: item.is_dir,
                }),
        );
        let mut coalesced = space_clean::coalesce_nested_paths(items, |item| item.path.as_path());
        coalesced.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
        coalesced
    }

    pub(crate) fn basket_len(&self) -> usize {
        self.basket_items().len()
    }

    pub(crate) fn basket_bytes(&self) -> u64 {
        basket_total_bytes(&self.basket_items())
    }

    /// "N not in this scan · K snapshots" when the volume holds more than the scan saw.
    fn scan_gap_label(&self) -> Option<String> {
        let tree = self.tree.as_ref()?;
        let vol = volumes::match_volume(&self.volumes, &tree.root_path)?;
        if tree.root_path != vol.mount {
            return None;
        }
        let indexed = tree.total_size_mode(self.size_mode);
        let gap = vol.used().checked_sub(indexed.saturating_add(64 << 20))?;
        let snaps = if self.tm_snapshots.is_empty() {
            vol.snapshot_count as usize
        } else {
            self.tm_snapshots.len()
        };
        Some(if snaps > 0 {
            crate::tr!(
                "{} not in this scan · {} snapshots",
                crate::i18n::bytes(gap + (64 << 20)),
                snaps
            )
        } else {
            crate::tr!("{} not in this scan", crate::i18n::bytes(gap + (64 << 20)))
        })
    }

    pub(crate) fn remove_from_basket(&mut self, path: &std::path::Path) {
        let slice = [path.to_path_buf()];
        self.junk.exclude_paths(&slice);
        self.downloads.exclude_paths(&slice);
        if let Some(tree) = &self.tree {
            self.review
                .retain(|&id| !std::path::Path::new(&tree.path_string(id)).starts_with(path));
        }
    }

    pub(crate) fn request_trash(&mut self) {
        if self.basket_len() == 0 {
            return;
        }
        if self.settings.confirm_trash {
            self.confirm_delete = true;
        } else {
            self.apply_basket();
        }
    }

    pub(crate) fn clear_all_cache(&mut self) {
        if let Some(cache) = &self.cache {
            if let Ok(removed) = cache.wipe_all() {
                self.recent.clear();
                self.status = crate::tr!("Cleared {} cached scans", removed);
            }
        }
    }

    pub(crate) fn remove_cache_entry(&mut self, root: &std::path::Path) {
        if let Some(cache) = &self.cache {
            let _ = cache.invalidate(root);
            self.recent = cache.list_recent().unwrap_or_default();
        }
    }

    /// Single-batch Trash worker across Quick clean, Downloads, and Folders.
    pub(crate) fn apply_basket(&mut self) {
        if self.busy {
            return;
        }
        let items = self.basket_items();
        if items.is_empty() {
            self.confirm_delete = false;
            return;
        }
        self.busy = true;
        self.confirm_delete = true;
        self.delete_progress = Some(space_clean::CleanProgress {
            items_total: items.len(),
            ..Default::default()
        });
        self.delete_started = Some(Instant::now());
        self.status = crate::i18n::t("Moving to Trash…").into();
        let tx = self.bg_tx.clone();
        thread::Builder::new()
            .name("space-trash".into())
            .spawn(move || {
                let result = (|| {
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
                        if std::fs::symlink_metadata(&item.path).is_err() {
                            deleted_paths.push(item.path);
                            progress.items_done += 1;
                            let _ = tx.send(BgMsg::DeleteProgress(progress.clone()));
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
                    for chunk in candidates.chunks(space_clean::TRASH_BATCH_SIZE) {
                        if let Some(first) = chunk.first() {
                            progress.current_path = first.path.clone();
                            let _ = tx.send(BgMsg::DeleteProgress(progress.clone()));
                        }
                        let paths: Vec<PathBuf> =
                            chunk.iter().map(|item| item.path.clone()).collect();
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
                    if deleted_paths.is_empty() && !errors.is_empty() {
                        return Err(errors.join("\n"));
                    }

                    Ok(TrashOutcome {
                        deleted_paths,
                        processed_bytes: progress.bytes_processed,
                        errors,
                    })
                })();
                let _ = tx.send(BgMsg::DeleteDone(result));
            })
            .ok();
    }
}

impl eframe::App for SpaceApp {
    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.settings.default_view = self.view;
        self.settings.default_size_mode = self.size_mode;
        self.settings.save();
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(path) = self.pending_open.take() {
            if path.exists() {
                self.start_scan(path, false);
            } else {
                self.status = crate::tr!("Folder not found: {}", path.display());
            }
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if let Some(path) = dropped.into_iter().next() {
            let dir = if path.is_dir() {
                path
            } else {
                path.parent().map(PathBuf::from).unwrap_or(path)
            };
            self.start_scan(dir, false);
        } else if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            ctx.request_repaint();
        }
        self.poll_bg(ctx);
        self.poll_scan(ctx);
        self.handle_screenshot_events(ctx);
        self.apply_filter_debounce();
        self.handle_keys(ctx);
        self.persist_settings_from_viewport(ctx);

        if let Some(tree) = &self.tree {
            let path = tree.path_string(self.focus);
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                "Space Scanner — {path}"
            )));
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title("Space Scanner".into()));
        }

        self.ui(ctx);
    }
}

fn folder_arg(args: impl IntoIterator<Item = String>) -> Option<PathBuf> {
    args.into_iter()
        .skip(1)
        .find(|a| !a.starts_with('-'))
        .map(PathBuf::from)
}

/// Node for an absolute path inside the scanned tree, walking by name.
fn find_path(tree: &space_core::SpaceTree, path: &std::path::Path) -> Option<NodeId> {
    let rel = path.strip_prefix(&tree.root_path).ok()?;
    let mut id = tree.root;
    for comp in rel.components() {
        let name = comp.as_os_str().to_string_lossy();
        id = *tree
            .children(id)
            .iter()
            .find(|&&c| tree.try_node(c).is_some_and(|n| n.name == name))?;
    }
    Some(id)
}

fn is_live_node(tree: &SpaceTree, id: NodeId) -> bool {
    if tree.try_node(id).is_none() {
        return false;
    }
    let mut cur = id;
    while cur != tree.root {
        let Some(parent) = tree.try_node(cur).and_then(|n| n.parent) else {
            return false;
        };
        if !tree.children(parent).contains(&cur) {
            return false;
        }
        cur = parent;
    }
    true
}

fn prune_deleted_from_tree(tree: &mut SpaceTree, deleted_paths: &[PathBuf]) {
    let mut ids: Vec<NodeId> = deleted_paths
        .iter()
        .filter_map(|p| find_path(tree, p))
        .filter(|&id| id != tree.root)
        .collect();
    ids.sort_by_key(|&id| std::cmp::Reverse(tree.path_to(id).len()));
    ids.dedup();
    for id in ids {
        if is_live_node(tree, id) {
            tree.remove_node(id);
        }
    }
}

fn capture_rescan_paths(
    tree: &SpaceTree,
    focus: NodeId,
    selected: Option<NodeId>,
    expanded: &HashSet<NodeId>,
    review: &[NodeId],
    history_back: &[NodeId],
    history_fwd: &[NodeId],
) -> RescanSavedPaths {
    RescanSavedPaths {
        focus: PathBuf::from(tree.path_string(focus)),
        selected: selected.map(|id| PathBuf::from(tree.path_string(id))),
        expanded: expanded
            .iter()
            .map(|&id| PathBuf::from(tree.path_string(id)))
            .collect(),
        review: review
            .iter()
            .map(|&id| PathBuf::from(tree.path_string(id)))
            .collect(),
        history_back: history_back
            .iter()
            .map(|&id| PathBuf::from(tree.path_string(id)))
            .collect(),
        history_fwd: history_fwd
            .iter()
            .map(|&id| PathBuf::from(tree.path_string(id)))
            .collect(),
    }
}

#[cfg(test)]
mod folder_arg_tests {
    use super::folder_arg;
    use std::path::PathBuf;

    #[test]
    fn skips_flags() {
        let args = ["bin", "--offline", "/tmp/foo"].map(String::from);
        assert_eq!(folder_arg(args), Some(PathBuf::from("/tmp/foo")));
    }

    #[test]
    fn none_without_path() {
        let args = ["bin", "--offline"].map(String::from);
        assert_eq!(folder_arg(args), None);
    }
}

#[cfg(test)]
mod tree_state_tests {
    use super::{capture_rescan_paths, find_path, is_live_node, prune_deleted_from_tree};
    use space_core::{rescan_subtree, scan_path, Node, NodeId, ScanEvent, SizeMode, SpaceTree};
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn prune_deleted_removes_node_marks_descendants_dead_and_subtracts_ancestors() {
        let dir = Node {
            name: "root".into(),
            apparent_size: 300,
            allocated_size: 300,
            file_count: 2,
            is_dir: true,
            synthetic: false,
            mtime_unix: 0,
            children: vec![NodeId(1), NodeId(3)],
            parent: None,
        };
        let sub = Node {
            name: "sub".into(),
            apparent_size: 200,
            allocated_size: 200,
            file_count: 1,
            is_dir: true,
            synthetic: false,
            mtime_unix: 0,
            children: vec![NodeId(2)],
            parent: Some(NodeId(0)),
        };
        let child = Node {
            name: "child.bin".into(),
            apparent_size: 200,
            allocated_size: 200,
            file_count: 1,
            is_dir: false,
            synthetic: false,
            mtime_unix: 0,
            children: vec![],
            parent: Some(NodeId(1)),
        };
        let keep = Node {
            name: "keep.bin".into(),
            apparent_size: 100,
            allocated_size: 100,
            file_count: 1,
            is_dir: false,
            synthetic: false,
            mtime_unix: 0,
            children: vec![],
            parent: Some(NodeId(0)),
        };
        let mut tree = SpaceTree {
            root_path: "/tmp/test_root".into(),
            nodes: vec![dir, sub, child, keep],
            root: NodeId(0),
            scanned_at_unix: 0,
            item_count: 4,
            skipped_errors: 0,
            hardlink_candidates: 0,
            hardlinks_deduped: 0,
            skipped_roots: Vec::new(),
        };

        assert!(is_live_node(&tree, NodeId(1)));
        assert!(is_live_node(&tree, NodeId(2)));

        prune_deleted_from_tree(
            &mut tree,
            &[
                PathBuf::from("/tmp/test_root/sub"),
                PathBuf::from("/tmp/test_root/sub/child.bin"),
            ],
        );

        assert!(is_live_node(&tree, NodeId(0)));
        assert!(is_live_node(&tree, NodeId(3)));
        assert!(!is_live_node(&tree, NodeId(1)));
        assert!(!is_live_node(&tree, NodeId(2)));
        assert_eq!(tree.node(NodeId(0)).size(SizeMode::OnDisk), 100);
        assert_eq!(tree.node(NodeId(0)).file_count, 1);
    }

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(tag: &str) -> Self {
            static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "space-app-test-{}-{}-{n}",
                tag,
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
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
    fn incremental_rescan_preserves_focus_expanded_and_review_paths() {
        let dir = TestDir::new("rescan");
        let sub_a = dir.path().join("alpha");
        let sub_b = dir.path().join("beta");
        fs::create_dir_all(&sub_a).unwrap();
        fs::create_dir_all(&sub_b).unwrap();
        fs::write(sub_a.join("a.bin"), vec![1_u8; 600_000]).unwrap();
        fs::write(sub_b.join("b.bin"), vec![2_u8; 600_000]).unwrap();

        let handle = scan_path(dir.path().to_path_buf());
        let mut scanned = None;
        for ev in handle.events.iter() {
            if let ScanEvent::Done(t) = ev {
                scanned = Some(*t);
                break;
            }
        }
        let tree = scanned.expect("scan completes");
        let id_a = find_path(&tree, &sub_a).unwrap();
        let id_b = find_path(&tree, &sub_b).unwrap();
        let id_b_file = find_path(&tree, &sub_b.join("b.bin")).unwrap();

        let expanded = HashSet::from([id_b]);
        let review = vec![id_b_file];
        let saved = capture_rescan_paths(
            &tree,
            id_a,
            Some(id_a),
            &expanded,
            &review,
            &[tree.root],
            &[],
        );

        fs::write(sub_a.join("new.bin"), vec![3_u8; 700_000]).unwrap();
        let rescanned = rescan_subtree(&tree, id_a).expect("rescan succeeds");

        assert!(find_path(&rescanned, &saved.focus).is_some());
        assert_eq!(
            find_path(&rescanned, &sub_a.join("new.bin"))
                .map(|id| rescanned.node(id).apparent_size),
            Some(700_000)
        );
        assert!(saved
            .expanded
            .iter()
            .all(|p| find_path(&rescanned, p).is_some()));
        assert!(saved
            .review
            .iter()
            .all(|p| find_path(&rescanned, p).is_some()));
    }
}

#[cfg(test)]
mod ui_smoke_tests {
    use super::*;
    use crate::i18n::{self, Lang};
    use crate::largest_files::{InsightData, InsightTab, LargestFiles};
    use crate::theme::{self, Skin};
    use space_core::{Node, NodeId, SizeMode, SpaceTree};

    struct RestoreUiGlobals {
        lang: Lang,
        high_contrast: bool,
        skin: Skin,
    }

    impl Drop for RestoreUiGlobals {
        fn drop(&mut self) {
            i18n::set(self.lang);
            theme::set_high_contrast(self.high_contrast);
            let ctx = egui::Context::default();
            theme::set_skin(&ctx, self.skin);
        }
    }

    fn synthetic_tree() -> SpaceTree {
        let nodes = vec![
            Node {
                name: "root".into(),
                apparent_size: 10_000,
                allocated_size: 12_288,
                file_count: 3,
                is_dir: true,
                synthetic: false,
                mtime_unix: 1_700_000_000,
                children: vec![NodeId(1), NodeId(3)],
                parent: None,
            },
            Node {
                name: "alpha".into(),
                apparent_size: 6_000,
                allocated_size: 8_192,
                file_count: 1,
                is_dir: true,
                synthetic: false,
                mtime_unix: 1_700_000_000,
                children: vec![NodeId(2)],
                parent: Some(NodeId(0)),
            },
            Node {
                name: "big.bin".into(),
                apparent_size: 6_000,
                allocated_size: 8_192,
                file_count: 1,
                is_dir: false,
                synthetic: false,
                mtime_unix: 1_700_000_000,
                children: Vec::new(),
                parent: Some(NodeId(1)),
            },
            Node {
                name: "beta".into(),
                apparent_size: 4_000,
                allocated_size: 4_096,
                file_count: 2,
                is_dir: true,
                synthetic: false,
                mtime_unix: 1_650_000_000,
                children: vec![NodeId(4), NodeId(5)],
                parent: Some(NodeId(0)),
            },
            Node {
                name: "old.zip".into(),
                apparent_size: 3_000,
                allocated_size: 3_072,
                file_count: 1,
                is_dir: false,
                synthetic: false,
                mtime_unix: 1_600_000_000,
                children: Vec::new(),
                parent: Some(NodeId(3)),
            },
            Node {
                name: "dup.log".into(),
                apparent_size: 1_000,
                allocated_size: 1_024,
                file_count: 1,
                is_dir: false,
                synthetic: false,
                mtime_unix: 1_650_000_000,
                children: Vec::new(),
                parent: Some(NodeId(3)),
            },
        ];
        SpaceTree {
            root_path: "/tmp/space-smoke-fixture".into(),
            nodes,
            root: NodeId(0),
            scanned_at_unix: 1_700_000_100,
            item_count: 6,
            skipped_errors: 0,
            hardlink_candidates: 0,
            hardlinks_deduped: 0,
            skipped_roots: Vec::new(),
        }
    }

    impl SpaceApp {
        fn test_instance(tree: SpaceTree) -> Self {
            let (bg_tx, bg_rx) = crossbeam_channel::unbounded();
            let mut largest = LargestFiles::default();
            if let Some(req) = largest.begin(NodeId(0), SizeMode::OnDisk) {
                largest.complete_data(
                    req,
                    InsightData {
                        files: vec![
                            (NodeId(2), "alpha/big.bin".into(), 8_192),
                            (NodeId(4), "beta/old.zip".into(), 3_072),
                        ],
                        folders: vec![
                            (NodeId(1), "alpha".into(), 8_192),
                            (NodeId(3), "beta".into(), 4_096),
                        ],
                        oldest: vec![(NodeId(4), "beta/old.zip".into(), 3_072, 1_600_000_000)],
                        duplicates: vec![("dup.log".into(), 1_024, vec![NodeId(5)])],
                        types: vec![("bin".into(), 8_192, 1), ("zip".into(), 3_072, 1)],
                    },
                );
            }
            Self {
                mode: Mode::Explore,
                tree: Some(Arc::new(tree)),
                focus: NodeId(0),
                selected: Some(NodeId(2)),
                selected_set: vec![NodeId(1), NodeId(2)],
                selection_anchor: Some(NodeId(1)),
                hovered: Some(NodeId(1)),
                view: ViewMode::Sunburst,
                size_mode: SizeMode::OnDisk,
                scan: None,
                progress: None,
                status: String::new(),
                cache: None,
                recent: Vec::new(),
                from_cache: false,
                cache_stale: false,
                cache_offline: false,
                show_settings: false,
                show_about: false,
                show_goto: false,
                goto_draft: "/tmp/space-smoke-fixture/alpha".into(),
                focus_goto: false,
                pending_screenshot: None,
                last_exported_file: None,
                skipped_expanded: false,
                exclude_draft: String::new(),
                filter: ViewFilter::default(),
                history_back: Vec::new(),
                history_fwd: Vec::new(),
                volumes: Vec::new(),
                volumes_loading: false,
                review: vec![NodeId(2)],
                confirm_delete: false,
                delete_progress: None,
                delete_started: None,
                show_tm_panel: false,
                confirm_tm_clean: false,
                tm_snapshots: Vec::new(),
                tm_loading: false,
                junk: JunkPanelState::default(),
                downloads: DownloadsState::default(),
                choosing_target: false,
                rules: RulesPanelState::default(),
                rule_mgr: None,
                rule_mgr_loading: false,
                bg_rx,
                bg_tx,
                busy: false,
                largest,
                insight_tab: InsightTab::Files,
                geom: None,
                settings: AppSettings::default(),
                filter_draft: String::new(),
                ext_draft: String::new(),
                min_mb_draft: String::new(),
                max_mb_draft: String::new(),
                filter_edited_at: None,
                show_shortcuts: false,
                focus_filter: false,
                transition: None,
                pending_open: None,
                show_largest: true,
                expanded: HashSet::from([NodeId(0), NodeId(1)]),
                tree_rows: Vec::new(),
                tree_revealed: None,
                tree_scroll: false,
                compact_map: false,
            }
        }
    }

    fn step_ui(ctx: &egui::Context, app: &mut SpaceApp, width: f32, height: f32) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, height),
            )),
            ..Default::default()
        };
        let output = ctx.run(input, |ctx| app.ui(ctx));
        assert!(
            !output.shapes.is_empty(),
            "expected rendered shapes at {width}x{height}"
        );
    }

    #[test]
    fn headless_ui_renders_all_views_modes_dialogs_and_locales() {
        let _guard = RestoreUiGlobals {
            lang: i18n::get(),
            high_contrast: theme::high_contrast(),
            skin: theme::skin(),
        };

        i18n::set(Lang::En);
        theme::set_high_contrast(false);

        let ctx = egui::Context::default();
        let mut app = SpaceApp::test_instance(synthetic_tree());
        theme::apply(&ctx, app.settings.skin);

        // Wide layout across all 4 visualization modes.
        for mode in ViewMode::ALL {
            app.view = mode;
            step_ui(&ctx, &mut app, 1280.0, 800.0);
        }

        // All 5 Insights tabs in Wide layout.
        app.show_largest = true;
        for tab in InsightTab::ALL {
            app.insight_tab = tab;
            step_ui(&ctx, &mut app, 1280.0, 800.0);
        }

        // Multi-selection + confirm delete modal.
        app.selected_set = vec![NodeId(1), NodeId(3)];
        step_ui(&ctx, &mut app, 1280.0, 800.0);
        app.confirm_delete = true;
        step_ui(&ctx, &mut app, 1280.0, 800.0);
        app.confirm_delete = false;

        // Medium and Narrow layouts (both list and map panes).
        step_ui(&ctx, &mut app, 1000.0, 800.0);
        app.compact_map = false;
        step_ui(&ctx, &mut app, 700.0, 800.0);
        app.compact_map = true;
        step_ui(&ctx, &mut app, 700.0, 800.0);
        app.compact_map = false;

        // Modal dialogs + disk chooser.
        app.show_settings = true;
        step_ui(&ctx, &mut app, 1280.0, 800.0);
        app.show_settings = false;

        app.show_about = true;
        step_ui(&ctx, &mut app, 1280.0, 800.0);
        app.show_about = false;

        app.show_shortcuts = true;
        step_ui(&ctx, &mut app, 1280.0, 800.0);
        app.show_shortcuts = false;

        app.show_goto = true;
        step_ui(&ctx, &mut app, 1280.0, 800.0);
        app.show_goto = false;

        app.choosing_target = true;
        step_ui(&ctx, &mut app, 1280.0, 800.0);
        app.choosing_target = false;

        // Clean mode in Wide & Narrow.
        app.mode = Mode::Clean;
        step_ui(&ctx, &mut app, 1280.0, 800.0);
        step_ui(&ctx, &mut app, 700.0, 800.0);

        // Russian locale + high contrast + Light skin.
        i18n::set(Lang::Ru);
        theme::set_high_contrast(true);
        theme::set_skin(&ctx, Skin::Columns);
        step_ui(&ctx, &mut app, 1280.0, 800.0);
        app.mode = Mode::Explore;
        step_ui(&ctx, &mut app, 700.0, 800.0);
    }

    #[test]
    fn roadmap_done_checks() {
        let plan = include_str!("../../../docs/PLAN.md");
        assert!(
            plan.contains("**200 / 200 DONE** (0 remaining)"),
            "docs/PLAN.md must record 200 / 200 DONE (0 remaining)"
        );
        let section2_rows: Vec<&str> = plan
            .lines()
            .filter(|line| line.starts_with("| `F"))
            .collect();
        assert_eq!(
            section2_rows.len(),
            200,
            "expected 200 P0 feature rows in Section 2.2"
        );
        let done_rows = section2_rows
            .iter()
            .filter(|line| line.contains("| DONE |"))
            .count();
        let todo_rows = section2_rows
            .iter()
            .filter(|line| line.contains("| TODO |"))
            .count();
        assert_eq!(done_rows, 200, "all 200 Section 2.2 rows must be DONE");
        assert_eq!(todo_rows, 0, "no Section 2.2 row may remain TODO");
        assert!(
            !plan
                .lines()
                .any(|line| line.contains("[P0 ·") && line.contains("TODO]")),
            "no [P0 · W* · TODO] checklist items may remain in Section 3"
        );
        assert!(
            plan.contains("- `F1090` [P1 · DONE] roadmap DONE checks"),
            "F1090 must be marked [P1 · DONE]"
        );
    }
}
