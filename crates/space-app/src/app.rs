use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Receiver};
use eframe::egui::{self, Key};
use space_core::{
    rescan_subtree, scan_path_with_expect, CacheMeta, CacheStore, NodeId, ScanEvent, ScanHandle,
    ScanProgress, SizeMode, SpaceTree,
};

use crate::anim::{TransKind, Transition};
use crate::junk_panel::JunkPanelState;
use crate::largest_files::{LargestFiles, LargestRequest, LargestRow};
use crate::review;
use crate::rules_panel::RulesPanelState;
use crate::settings::AppSettings;
use crate::theme;
use crate::tm_clean::{self, CleanReport, LocalSnapshot};
use crate::views::{GeomCache, ViewFilter, ViewMode};
use crate::volumes::{self, VolumeInfo};

mod ui;

enum BgMsg {
    RuleManagerLoaded(Result<space_clean::RuleManager, String>),
    Volumes(Vec<VolumeInfo>),
    Snapshots(Vec<LocalSnapshot>),
    Recent(Vec<CacheMeta>),
    /// Cache hit (or forced miss path will scan).
    CacheResult {
        path: PathBuf,
        loaded: Option<(Arc<SpaceTree>, bool /*stale*/)>,
    },
    CacheSaved(Vec<CacheMeta>),
    RescanDone(Result<Arc<SpaceTree>, String>),
    DeleteProgress(space_clean::CleanProgress),
    DeleteDone(Result<(Vec<NodeId>, Vec<String>), String>),
    TmCleanDone(CleanReport),
    JunkScanProgress(space_clean::JunkScanProgress),
    JunkScanDone(Result<space_clean::ScanReport, String>),
    JunkCleanProgress(space_clean::CleanProgress),
    JunkCleanDone(Result<space_clean::CleanResult, String>),
    Largest {
        request: LargestRequest,
        rows: Vec<LargestRow>,
    },
}

/// Top-level screens: overview, explore a scan, find junk, the Trash list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    /// Overview: disk health, what to do next, saved scans.
    Home,
    Explore,
    Clean,
    /// The Trash list: the only route to Move to Trash.
    Review,
}

/// How a scan that produced no result ended, so the page can offer a way on.
#[derive(Clone, Debug)]
pub(crate) struct ScanOutcome {
    pub path: PathBuf,
    pub bytes_seen: u64,
    pub files_seen: u64,
    /// `None` when the user cancelled.
    pub error: Option<String>,
}

/// What the last Move to Trash did, shown on the Trash list page afterwards.
#[derive(Clone, Debug, Default)]
pub(crate) struct TrashResult {
    pub moved: usize,
    pub bytes: u64,
    pub failures: Vec<String>,
}

/// Whole-scan search results for one query.
pub(crate) struct SearchHits {
    pub key: (String, u64, SizeMode, usize),
    pub ids: Vec<NodeId>,
    pub bytes: u64,
    /// More matches exist than are listed.
    pub truncated: bool,
}

/// Matches listed by a whole-scan search; the rest are counted.
pub(crate) const MAX_SEARCH_HITS: usize = 2_000;

pub struct SpaceApp {
    mode: Mode,
    /// Junk selection to trash once the Explore queue delete finishes.
    junk_after_delete: bool,
    tree: Option<Arc<SpaceTree>>,
    focus: NodeId,
    selected: Option<NodeId>,
    /// Live hover from the central view (tooltip + sidebar highlight).
    hovered: Option<NodeId>,
    view: ViewMode,
    size_mode: SizeMode,
    scan: Option<ScanHandle>,
    progress: Option<ScanProgress>,
    expected_items: Option<u64>,
    status: String,
    cache: Option<CacheStore>,
    recent: Vec<CacheMeta>,
    from_cache: bool,
    cache_stale: bool,
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
    rules: RulesPanelState,
    rule_mgr: Option<space_clean::RuleManager>,
    rule_mgr_loading: bool,
    bg_rx: Receiver<BgMsg>,
    bg_tx: crossbeam_channel::Sender<BgMsg>,
    /// True while cache load / rescan / delete runs off-thread.
    busy: bool,
    largest: LargestFiles,
    /// Cached view geometry (sunburst mesh / treemap tiles).
    geom: Option<GeomCache>,
    settings: AppSettings,
    filter_draft: String,
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
    /// Transient messages in the corner: results, errors, undo.
    toasts: ui::Toasts,
    /// Set when a scan ended without a result (cancelled or failed).
    scan_outcome: Option<ScanOutcome>,
    /// Folder of the scan in progress, for Try again.
    scan_target: Option<PathBuf>,
    /// Trash list paths to restore after a rescan of the same folder.
    keep_review: Option<Vec<PathBuf>>,
    trash_result: Option<TrashResult>,
    show_settings: bool,
    /// Search the whole scan instead of the folder in view.
    search_everywhere: bool,
    search_hits: Option<SearchHits>,
    /// Last row clicked in a list; shift-click adds the range from here.
    range_anchor: Option<NodeId>,
    /// Text to put on the clipboard on the next frame.
    pending_copy: Option<String>,
    /// The search field had keyboard focus last frame.
    search_focused: bool,
}

impl SpaceApp {
    pub fn new(cc: &eframe::CreationContext<'_>, settings: AppSettings) -> Self {
        theme::apply(&cc.egui_ctx, settings.appearance);
        crate::i18n::set(settings.language.unwrap_or_else(crate::i18n::detect));

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
            thread::Builder::new()
                .name("space-recent".into())
                .spawn(move || {
                    let recent = cache.list_recent().unwrap_or_default();
                    let _ = tx.send(BgMsg::Recent(recent));
                })
                .ok();
        }

        let view = settings.default_view;
        let size_mode = settings.default_size_mode;

        Self {
            tree: None,
            focus: NodeId(0),
            selected: None,
            hovered: None,
            view,
            size_mode,
            scan: None,
            progress: None,
            expected_items: None,
            status: String::new(),
            cache,
            recent: Vec::new(),
            from_cache: false,
            cache_stale: false,
            filter: ViewFilter::default(),
            history_back: Vec::new(),
            history_fwd: Vec::new(),
            volumes: volumes::list_volumes_fast(),
            volumes_loading: true,
            review: Vec::new(),
            mode: Mode::Home,
            junk_after_delete: false,
            confirm_delete: false,
            delete_progress: None,
            delete_started: None,
            show_tm_panel: false,
            confirm_tm_clean: false,
            tm_snapshots: Vec::new(),
            tm_loading: true,
            junk: JunkPanelState::default(),
            rules: RulesPanelState::default(),
            rule_mgr: None,
            rule_mgr_loading: false,
            bg_rx,
            bg_tx,
            busy: false,
            largest: LargestFiles::default(),
            geom: None,
            settings,
            filter_draft: String::new(),
            filter_edited_at: None,
            show_shortcuts: false,
            focus_filter: false,
            transition: None,
            pending_open: folder_arg(std::env::args()),
            show_largest: false,
            expanded: HashSet::new(),
            tree_rows: Vec::new(),
            tree_revealed: None,
            tree_scroll: false,
            compact_map: false,
            toasts: ui::Toasts::default(),
            scan_outcome: None,
            scan_target: None,
            keep_review: None,
            trash_result: None,
            show_settings: false,
            search_everywhere: false,
            search_hits: None,
            range_anchor: None,
            pending_copy: None,
            search_focused: false,
        }
    }

    /// Show a result or error to the user and keep it as the status line.
    fn notify(&mut self, level: ui::ToastLevel, text: String) {
        self.toasts.push(level, text.clone(), None);
        self.status = text;
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
                        Some((tree, stale)) => {
                            self.focus = tree.root;
                            self.selected = None;
                            self.review.clear();
                            self.history_back.clear();
                            self.history_fwd.clear();
                            self.cache_stale = stale;
                            self.from_cache = true;
                            self.scan_outcome = None;
                            self.search_hits = None;
                            if stale {
                                self.toasts.push(
                                    ui::ToastLevel::Info,
                                    crate::tr!(
                                        "Opened the saved result from {}. Files changed since then.",
                                        crate::i18n::age(now_unix().saturating_sub(tree.scanned_at_unix))
                                    ),
                                    Some(ui::ToastAction::Rescan),
                                );
                            }
                            self.status = crate::tr!(
                                "Loaded cache · {} items · {}",
                                tree.item_count,
                                crate::i18n::bytes(tree.total_size_mode(self.size_mode))
                            );
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
                BgMsg::RescanDone(Ok(tree)) => {
                    self.busy = false;
                    // Node ids change; carry the location and Trash list over by path.
                    let (focus_path, review_paths) = self.paths_to_keep();
                    self.history_back.clear();
                    self.history_fwd.clear();
                    self.cache_stale = false;
                    self.from_cache = false;
                    self.search_hits = None;
                    self.selected = None;
                    self.focus = focus_path
                        .and_then(|p| find_path(&tree, &p))
                        .unwrap_or(tree.root);
                    self.review = review_paths
                        .iter()
                        .filter_map(|p| find_path(&tree, p))
                        .collect();
                    self.notify(
                        ui::ToastLevel::Success,
                        crate::tr!(
                            "Rescanned · {} items · {}",
                            tree.item_count,
                            crate::i18n::bytes(tree.total_size_mode(self.size_mode))
                        ),
                    );
                    self.tree = Some(tree);
                    self.invalidate_largest();
                    self.schedule_largest();
                    if let Some(tree) = self.tree.clone() {
                        self.save_cache_async(tree);
                    }
                }
                BgMsg::RescanDone(Err(e)) => {
                    self.busy = false;
                    self.notify(
                        ui::ToastLevel::Error,
                        crate::tr!("Incremental rescan failed: {}", e),
                    );
                }
                BgMsg::DeleteProgress(progress) => {
                    self.delete_progress = Some(progress);
                }
                BgMsg::DeleteDone(Ok((mut deleted_ids, errors))) => {
                    self.busy = false;
                    self.delete_progress = None;
                    self.delete_started = None;
                    let n = deleted_ids.len();
                    let bytes = self
                        .tree
                        .as_ref()
                        .map(|t| review::queue_bytes(t, &deleted_ids, self.size_mode))
                        .unwrap_or(0);
                    let result = self.trash_result.get_or_insert_with(TrashResult::default);
                    result.moved += n;
                    result.bytes = result.bytes.saturating_add(bytes);
                    result.failures.extend(errors.iter().cloned());
                    self.status = if errors.is_empty() {
                        crate::tr!("Moved {} items to Trash", n)
                    } else {
                        crate::tr!("Moved {} items · {} failed", n, errors.len())
                    };
                    self.refresh_volumes();
                    if let Some(tree) = &self.tree {
                        let removed_paths: Vec<_> = deleted_ids
                            .iter()
                            .map(|&id| PathBuf::from(tree.path_string(id)))
                            .collect();
                        self.junk.exclude_paths(&removed_paths);
                    }
                    self.review.clear();
                    self.selected = None;
                    self.confirm_delete = false;
                    self.search_hits = None;
                    // Folders that went to the Trash must not be reachable by Back.
                    self.history_back.retain(|id| !deleted_ids.contains(id));
                    self.history_fwd.retain(|id| !deleted_ids.contains(id));
                    if std::mem::take(&mut self.junk_after_delete) {
                        self.apply_junk_clean();
                    }
                    if let Some(tree) = &mut self.tree {
                        deleted_ids.sort_by_key(|id| std::cmp::Reverse(tree.path_to(*id).len()));
                        let tree = Arc::make_mut(tree);
                        for id in deleted_ids {
                            tree.remove_node(id);
                        }
                    }
                    self.invalidate_largest();
                    self.schedule_largest();
                    if let Some(tree) = self.tree.clone() {
                        self.save_cache_async(tree);
                    }
                }
                BgMsg::DeleteDone(Err(e)) => {
                    self.busy = false;
                    self.junk_after_delete = false;
                    self.delete_progress = None;
                    self.delete_started = None;
                    self.confirm_delete = false;
                    let result = self.trash_result.get_or_insert_with(TrashResult::default);
                    result.failures.extend(e.lines().map(str::to_owned));
                    self.notify(
                        ui::ToastLevel::Error,
                        crate::tr!(
                            "Nothing was moved: {}",
                            e.lines().next().unwrap_or_default()
                        ),
                    );
                }
                BgMsg::TmCleanDone(report) => {
                    self.busy = false;
                    self.confirm_tm_clean = false;
                    if report.deleted == 0 && report.errors.is_empty() {
                        self.notify(
                            ui::ToastLevel::Info,
                            crate::tr!(
                                "Snapshots: nothing to remove ({}/{} kept)",
                                report.kept,
                                report.before
                            ),
                        );
                    } else if report.errors.is_empty() {
                        self.notify(
                            ui::ToastLevel::Success,
                            crate::tr!(
                                "Removed {} snapshots, kept {}. macOS is releasing the space.",
                                report.deleted,
                                report.kept
                            ),
                        );
                        self.show_tm_panel = false;
                    } else {
                        self.notify(
                            ui::ToastLevel::Error,
                            crate::tr!(
                                "Deleted {} of {} snapshots, {} failed",
                                report.deleted,
                                report.before,
                                report.errors.len()
                            ),
                        );
                    }
                    self.refresh_volumes();
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
                    self.notify(ui::ToastLevel::Error, crate::tr!("Junk scan failed: {}", e));
                }
                BgMsg::JunkCleanProgress(progress) => {
                    if self.junk.cleaning {
                        self.junk.clean_progress = progress;
                    }
                }
                BgMsg::JunkCleanDone(Ok(result)) => {
                    self.busy = false;
                    self.junk.cleaning = false;
                    self.junk.clean_started = None;
                    let summary = self.trash_result.get_or_insert_with(TrashResult::default);
                    summary.moved += result.deleted.len();
                    summary.bytes = summary.bytes.saturating_add(result.processed_bytes);
                    summary.failures.extend(result.errors.iter().cloned());
                    self.status = crate::tr!(
                        "Junk · moved {} to Trash · {}",
                        result.deleted.len(),
                        crate::i18n::bytes(result.processed_bytes)
                    );
                    self.confirm_delete = false;
                    self.junk.clear_selection();
                    self.refresh_volumes();
                    self.start_junk_scan();
                }
                BgMsg::JunkCleanDone(Err(e)) => {
                    self.busy = false;
                    self.junk.cleaning = false;
                    self.junk.clean_started = None;
                    self.junk.error = Some(e.clone());
                    self.confirm_delete = false;
                    self.trash_result
                        .get_or_insert_with(TrashResult::default)
                        .failures
                        .push(e.clone());
                    self.notify(
                        ui::ToastLevel::Error,
                        crate::tr!("Junk clean failed: {}", e),
                    );
                }
                BgMsg::Largest { request, rows } => {
                    self.largest.complete(request, rows);
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
        let tx = self.bg_tx.clone();
        let result = thread::Builder::new()
            .name("space-largest".into())
            .spawn(move || {
                let ids = tree.largest_files_bounded(focus, mode, 100, 8_000);
                let rows: Vec<(NodeId, String, u64)> = ids
                    .into_iter()
                    .map(|id| {
                        let n = tree.node(id);
                        (id, n.name.clone(), n.size(mode))
                    })
                    .collect();
                let _ = tx.send(BgMsg::Largest { request, rows });
            });
        if let Err(error) = result {
            self.largest.failed(request);
            self.status = crate::tr!("Could not start largest-file search: {}", error);
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
        self.scan_outcome = None;
        self.scan_target = Some(path.clone());
        if let Some(scan) = self.scan.take() {
            scan.cancel();
        }

        self.expected_items = self
            .cache
            .as_ref()
            .and_then(|c| c.peek_meta(&path))
            .map(|m| m.item_count);

        if !force {
            if let Some(cache) = self.cache.clone() {
                self.busy = true;
                self.status = crate::tr!("Loading cache for {}…", path.display());
                self.tree = None;
                self.search_hits = None;
                self.progress = None;
                self.invalidate_largest();
                let tx = self.bg_tx.clone();
                let path_bg = path.clone();
                thread::Builder::new()
                    .name("space-cache-load".into())
                    .spawn(move || {
                        let loaded = cache
                            .load(&path_bg)
                            .ok()
                            .map(|l| (Arc::new(l.tree), l.stale));
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
        self.busy = false;
        self.scan_outcome = None;
        self.scan_target = Some(path.clone());
        let same_root = self
            .tree
            .as_ref()
            .is_some_and(|t| std::path::Path::new(&t.root_path) == path);
        if same_root {
            // Rescan in place: the old result stays open (dimmed) until the new one lands,
            // and the Trash list follows by path.
            let (_, review_paths) = self.paths_to_keep();
            self.keep_review = Some(review_paths);
        } else {
            self.from_cache = false;
            self.cache_stale = false;
            self.tree = None;
            self.review.clear();
            self.history_back.clear();
            self.history_fwd.clear();
            self.keep_review = None;
        }
        self.search_hits = None;
        self.invalidate_largest();
        self.progress = Some(ScanProgress {
            files_seen: 0,
            bytes_seen: 0,
            current_path: path.display().to_string(),
            skipped_errors: 0,
        });
        self.status = crate::tr!("Scanning {}…", path.display());
        self.scan = Some(scan_path_with_expect(path, self.expected_items));
    }

    fn rescan_current(&mut self) {
        if self.busy {
            return;
        }
        let Some(tree) = &self.tree else { return };
        let path = PathBuf::from(&tree.root_path);
        self.start_scan(path, true);
    }

    /// Focus path and Trash list paths, to find them again in a rebuilt tree.
    fn paths_to_keep(&self) -> (Option<PathBuf>, Vec<PathBuf>) {
        let Some(tree) = &self.tree else {
            return (None, Vec::new());
        };
        let focus = Some(PathBuf::from(tree.path_string(self.focus)));
        let review = self
            .review
            .iter()
            .filter(|id| tree.try_node(**id).is_some())
            .map(|&id| PathBuf::from(tree.path_string(id)))
            .collect();
        (focus, review)
    }

    /// Rebuild the index under one folder; the rest of the tree is kept.
    fn incremental_rescan(&mut self, folder: NodeId) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        if self.busy || self.scan.is_some() {
            return;
        }
        let Some(node) = tree.try_node(folder) else {
            return;
        };
        let folder = if node.is_dir && !node.synthetic {
            folder
        } else {
            node.parent.unwrap_or(tree.root)
        };
        self.busy = true;
        self.status = crate::tr!("Incremental rescan of {}…", tree.node(folder).name);
        let focus = folder;
        let tx = self.bg_tx.clone();
        thread::Builder::new()
            .name("space-rescan".into())
            .spawn(move || {
                let result = rescan_subtree(tree.as_ref(), focus).map(Arc::new);
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
            let pct = self
                .expected_items
                .filter(|e| *e > 0)
                .map(|e| {
                    format!(
                        " ({:.0}%)",
                        (p.files_seen as f64 / e as f64 * 100.0).min(99.0)
                    )
                })
                .unwrap_or_default();
            self.status = crate::tr!(
                "Scanning… {} · {}{}{}",
                crate::i18n::bytes(p.bytes_seen),
                p.files_seen,
                pct,
                if p.skipped_errors > 0 {
                    crate::tr!(" · {} skipped", p.skipped_errors)
                } else {
                    String::new()
                }
            );
            self.progress = Some(p);
        }

        if cancelled || err.is_some() {
            let seen = self.progress.take();
            self.scan = None;
            self.keep_review = None;
            match &err {
                Some(e) => self.status = crate::tr!("Scan error: {}", e),
                None => self.status = crate::i18n::t("Scan cancelled.").into(),
            }
            if self.tree.is_some() {
                // A rescan was stopped: the previous result is still open.
                let text = self.status.clone();
                self.toasts.push(ui::ToastLevel::Info, text, None);
            } else {
                self.scan_outcome = Some(ScanOutcome {
                    path: self.scan_target.clone().unwrap_or_default(),
                    bytes_seen: seen.as_ref().map(|p| p.bytes_seen).unwrap_or(0),
                    files_seen: seen.as_ref().map(|p| p.files_seen).unwrap_or(0),
                    error: err,
                });
            }
            return;
        }

        if let Some(tree) = done_tree {
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
            let (focus_path, _) = self.paths_to_keep();
            let kept = self.keep_review.take();
            self.focus = focus_path
                .filter(|_| kept.is_some())
                .and_then(|p| find_path(&tree, &p))
                .unwrap_or(tree.root);
            self.review = kept
                .unwrap_or_default()
                .iter()
                .filter_map(|p| find_path(&tree, p))
                .collect();
            self.history_back.clear();
            self.history_fwd.clear();
            self.selected = None;
            self.from_cache = false;
            self.cache_stale = false;
            self.scan_outcome = None;
            self.search_hits = None;
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
        if tree.try_node(id).is_some_and(|n| n.is_dir) {
            if id == self.focus {
                return;
            }
            let zoom_in = tree.path_to(id).contains(&self.focus) && id != self.focus;
            // Child of current focus → zoom in; otherwise zoom out / sideways.
            let kind = if zoom_in || tree.node(id).parent.is_some_and(|p| p == self.focus) {
                TransKind::ZoomIn
            } else {
                TransKind::ZoomOut
            };
            if push_history {
                self.history_back.push(self.focus);
                self.history_fwd.clear();
            }
            self.focus = id;
            self.selected = None;
            self.hovered = None;
            self.transition = Some(Transition::start(kind));
            self.invalidate_largest();
            self.schedule_largest();
        }
    }

    fn set_view(&mut self, mode: ViewMode) {
        if self.view != mode {
            self.view = mode;
            self.geom = None;
            self.transition = Some(Transition::start(TransKind::ViewSwitch));
        }
    }

    /// Copy the selected item's path (or the folder in view).
    fn copy_focus_path(&mut self) {
        let Some(tree) = &self.tree else { return };
        let id = self.selected.unwrap_or(self.focus);
        let path = tree.path_string(id);
        self.copy_text(path);
    }

    fn copy_text(&mut self, text: String) {
        self.notify(ui::ToastLevel::Success, crate::tr!("Copied {}", text));
        self.pending_copy = Some(text);
    }

    /// Add or take out items of the Trash list, with an undo in the corner.
    fn queue_toggle(&mut self, ids: &[NodeId]) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        let mut protected = None;
        let ids: Vec<NodeId> = ids
            .iter()
            .copied()
            .filter(|&id| {
                tree.try_node(id)
                    .is_some_and(|n| !n.synthetic && n.parent.is_some())
            })
            .filter(|&id| {
                // System and personal-data folders never enter the list, by any route.
                let path = PathBuf::from(tree.path_string(id));
                if !self.review.contains(&id) && space_clean::safety::is_never_touch(&path) {
                    protected.get_or_insert(tree.node(id).name.clone());
                    return false;
                }
                true
            })
            .collect();
        if let Some(name) = protected {
            self.toasts.push(
                ui::ToastLevel::Info,
                crate::tr!("{} is protected and stays where it is", name),
                None,
            );
        }
        if ids.is_empty() {
            return;
        }
        let adding = ids.iter().any(|id| !self.review.contains(id));
        let mut changed = Vec::new();
        for id in ids {
            let has = self.review.contains(&id);
            if adding && !has {
                self.review.push(id);
                changed.push(id);
            } else if !adding && has {
                self.review.retain(|x| *x != id);
                changed.push(id);
            }
        }
        if changed.is_empty() {
            return;
        }
        let bytes = review::queue_bytes(&tree, &changed, self.size_mode);
        let what = if changed.len() == 1 {
            tree.node(changed[0]).name.clone()
        } else {
            crate::i18n::count(changed.len(), "item")
        };
        let text = if adding {
            crate::tr!(
                "Added {} to the Trash list · {}",
                what,
                crate::i18n::bytes(bytes)
            )
        } else {
            crate::tr!("Took {} out of the Trash list", what)
        };
        self.toasts.push(
            ui::ToastLevel::Success,
            text,
            Some(ui::ToastAction::UndoQueue(changed)),
        );
        self.trash_result = None;
    }

    /// Undo a Trash list change: flip each id back.
    fn undo_queue(&mut self, ids: &[NodeId]) {
        for &id in ids {
            review::toggle(&mut self.review, id);
        }
    }

    fn percent_of_parent(&self, id: NodeId) -> Option<f32> {
        let tree = self.tree.as_ref()?;
        let node = tree.try_node(id)?;
        let parent = node.parent?;
        let psize = tree.node(parent).size(self.size_mode).max(1);
        Some((node.size(self.size_mode) as f32 / psize as f32) * 100.0)
    }

    fn apply_filter_debounce(&mut self) {
        let ms = self.settings.flags.debounce_filter_ms;
        if let Some(at) = self.filter_edited_at {
            if at.elapsed() >= Duration::from_millis(ms) {
                if self.filter.query != self.filter_draft {
                    self.filter.query = self.filter_draft.clone();
                    self.geom = None;
                }
                self.filter_edited_at = None;
            }
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let input = ctx.input(|i| {
            (
                [
                    i.key_pressed(Key::ArrowUp),
                    i.key_pressed(Key::ArrowDown),
                    i.key_pressed(Key::ArrowLeft),
                    i.key_pressed(Key::ArrowRight),
                ],
                [
                    i.key_pressed(Key::Enter),
                    i.key_pressed(Key::Escape),
                    i.key_pressed(Key::Home),
                    i.key_pressed(Key::End),
                    i.key_pressed(Key::Slash),
                    i.key_pressed(Key::Questionmark),
                    i.key_pressed(Key::Space),
                ],
                i.modifiers,
                [
                    i.key_pressed(Key::Num1),
                    i.key_pressed(Key::Num2),
                    i.key_pressed(Key::Num3),
                    i.key_pressed(Key::Num4),
                ],
                [
                    i.key_pressed(Key::C),
                    i.key_pressed(Key::F),
                    i.key_pressed(Key::R),
                    i.key_pressed(Key::O),
                    i.key_pressed(Key::J),
                    i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace),
                    i.key_pressed(Key::Comma),
                ],
            )
        });
        let (
            [up, down, left, right],
            [enter, esc, home, end, slash, question, space],
            mods,
            views,
            letters,
        ) = input;
        let [key_c, key_f, key_r, key_o, key_j, key_del, key_comma] = letters;
        // While a text field has focus, keys belong to it: no navigation behind the caret.
        // (egui also reports focus for a clicked row, so ask about text fields only.)
        let typing = self.search_focused && ctx.wants_keyboard_input();
        // A focused button takes Space and Enter as a click (egui does that), so the
        // app must not act on them a second time. Rows and the map never take focus.
        let button_focus = ctx.wants_keyboard_input() && !typing;
        let (enter, space) = if button_focus {
            (false, false)
        } else {
            (enter, space)
        };
        let modal = self.confirm_delete
            || self.show_settings
            || self.show_shortcuts
            || self.show_tm_panel
            || self.rules.open;

        // Escape closes the innermost thing first. It never switches pages.
        if esc {
            if self.scan.is_some() {
                if let Some(s) = &self.scan {
                    s.cancel();
                }
            } else if self.show_shortcuts {
                self.show_shortcuts = false;
            } else if self.show_settings {
                self.show_settings = false;
            } else if self.confirm_delete {
                if self.delete_progress.is_none() && !self.junk.cleaning {
                    self.confirm_delete = false;
                }
            } else if self.junk.confirm {
                self.junk.confirm = false;
            } else if self.rules.open {
                self.rules.open = false;
            } else if self.show_tm_panel {
                self.show_tm_panel = false;
                self.confirm_tm_clean = false;
            } else if typing {
                // The text field gives up focus on its own.
            } else if self.mode == Mode::Explore && self.selected.is_some() {
                self.selected = None;
            } else if self.filter.is_active() || !self.filter_draft.is_empty() {
                self.filter.clear();
                self.filter_draft.clear();
                self.search_hits = None;
                self.geom = None;
            } else if self.mode == Mode::Explore {
                self.go_up();
            }
            return;
        }

        if mods.command || mods.ctrl {
            if self.tree.is_some() && !modal {
                for (pressed, view) in views.into_iter().zip(ViewMode::ALL) {
                    if pressed {
                        self.set_view(view);
                    }
                }
            }
            if key_comma {
                self.show_settings = !self.show_settings;
            }
            if modal {
                return;
            }
            if key_c && mods.shift {
                self.copy_focus_path();
            }
            if key_f {
                if self.tree.is_some() {
                    self.mode = Mode::Explore;
                }
                self.focus_filter = true;
            }
            if key_r && self.scan.is_none() {
                if mods.shift {
                    let folder = self.selected.unwrap_or(self.focus);
                    self.incremental_rescan(folder);
                } else {
                    self.rescan_current();
                }
            }
            if key_o && self.scan.is_none() && !self.busy {
                self.pick_folder_and_scan();
            }
            if key_j {
                self.find_junk();
            }
            if key_del && !typing {
                // Add the selection if it is not listed yet, then open the Trash list.
                // The confirmation itself is always one more explicit step.
                if let Some(id) = self.selected {
                    if self.mode == Mode::Explore && !self.review.contains(&id) {
                        self.queue_toggle(&[id]);
                    }
                }
                self.set_mode(Mode::Review);
            }
            if !typing && left {
                self.go_back();
            }
            if !typing && right {
                self.go_forward();
            }
            return;
        }

        if enter && !typing && self.confirm_delete {
            if self.delete_progress.is_none() && !self.junk.cleaning && self.basket_len() > 0 {
                self.apply_basket();
            }
            return;
        }
        if typing || modal {
            return;
        }

        if question || (mods.shift && slash) {
            self.show_shortcuts = true;
            return;
        }
        if slash {
            self.focus_filter = true;
            return;
        }
        if self.mode != Mode::Explore {
            return;
        }
        if self.tree_keys(up, down, left, right, space) {
            return;
        }
        if left {
            self.go_back();
        }
        if right {
            self.go_forward();
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
            // Select the largest item in view.
            if let Some(tree) = &self.tree {
                if let Some(&first) = tree.children(self.focus).first() {
                    self.selected = Some(first);
                }
            }
        }
        if enter {
            if let Some(id) = self.selected {
                let is_dir = self
                    .tree
                    .as_ref()
                    .is_some_and(|t| t.try_node(id).is_some_and(|n| n.is_dir && !n.synthetic));
                if is_dir {
                    self.navigate(id, true);
                } else {
                    self.reveal_selected();
                }
            }
        }
    }

    fn pick_folder_and_scan(&mut self) {
        if let Some(path) = rfd::FileDialog::new().pick_folder() {
            self.start_scan(path, false);
        }
    }

    /// Whole-scan search: every indexed item that matches the filter, largest first.
    fn refresh_search_hits(&mut self) {
        let Some(tree) = self.tree.clone() else {
            self.search_hits = None;
            return;
        };
        if !self.search_everywhere || self.filter.query.is_empty() {
            self.search_hits = None;
            return;
        }
        let key = (
            self.filter.query.clone(),
            self.filter.hash64(),
            self.size_mode,
            Arc::as_ptr(&tree) as usize,
        );
        if self.search_hits.as_ref().is_some_and(|h| h.key == key) {
            return;
        }
        let mode = self.size_mode;
        let mut hits = Vec::new();
        // Walk from the root so items already moved to the Trash are skipped.
        let mut stack = tree.children(tree.root).to_vec();
        while let Some(id) = stack.pop() {
            let n = tree.node(id);
            if self.filter.matches(&n.name, n.is_dir, n.size(mode)) && !n.synthetic {
                hits.push(id);
            }
            stack.extend_from_slice(tree.children(id));
        }
        hits.sort_by_key(|&id| std::cmp::Reverse(tree.node(id).size(mode)));
        let truncated = hits.len() > MAX_SEARCH_HITS;
        hits.truncate(MAX_SEARCH_HITS);
        let set: HashSet<NodeId> = hits.iter().copied().collect();
        let bytes = hits
            .iter()
            .filter(|&&id| {
                let mut p = tree.node(id).parent;
                while let Some(a) = p {
                    if set.contains(&a) {
                        return false;
                    }
                    p = tree.node(a).parent;
                }
                true
            })
            .fold(0_u64, |sum, &id| {
                sum.saturating_add(tree.node(id).size(mode))
            });
        self.search_hits = Some(SearchHits {
            key,
            ids: hits,
            bytes,
            truncated,
        });
    }

    /// Show the system Trash in the file manager.
    fn open_trash(&mut self) {
        #[cfg(target_os = "macos")]
        let result = std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(".Trash"))
            .ok_or_else(|| std::io::Error::other("no HOME"))
            .and_then(|p| std::process::Command::new("open").arg(p).spawn());
        #[cfg(target_os = "windows")]
        let result = std::process::Command::new("explorer")
            .arg("shell:RecycleBinFolder")
            .spawn();
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let result = std::process::Command::new("xdg-open")
            .arg("trash:///")
            .spawn();
        if let Err(e) = result {
            self.notify(
                ui::ToastLevel::Error,
                crate::tr!("Could not open the Trash: {}", e),
            );
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
            if let Some(parent) = tree.node(self.focus).parent {
                self.navigate(parent, true);
            }
        }
    }

    fn go_back(&mut self) {
        if let Some(prev) = self.history_back.pop() {
            self.history_fwd.push(self.focus);
            self.focus = prev;
            self.selected = None;
            self.invalidate_largest();
            self.schedule_largest();
        }
    }

    fn go_forward(&mut self) {
        if let Some(next) = self.history_fwd.pop() {
            self.history_back.push(self.focus);
            self.focus = next;
            self.selected = None;
            self.invalidate_largest();
            self.schedule_largest();
        }
    }

    /// A rule path (may start with `~` or contain a glob): select it in Disk
    /// usage when the open scan indexed it, otherwise show it in Finder.
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
            let parent = self.tree.as_ref().and_then(|t| t.node(id).parent);
            if let Some(parent) = parent {
                self.navigate(parent, true);
            }
            self.selected = Some(id);
            self.rules.open = false;
            self.mode = Mode::Explore;
            return;
        }
        let shown = path.display().to_string();
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("open")
                .args(["-R", &shown])
                .spawn();
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = std::process::Command::new("xdg-open").arg(&shown).spawn();
        }
        self.notify(
            ui::ToastLevel::Info,
            crate::tr!("Not in this scan — opened {} in Finder", shown),
        );
    }

    fn reveal_selected(&self) {
        let Some(tree) = &self.tree else { return };
        let id = self.selected.unwrap_or(self.focus);
        reveal_path(&tree.path_string(id));
    }

    fn apply_delete(&mut self) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        if self.review.is_empty() || self.busy {
            return;
        }
        let mode = self.size_mode;
        let items: Vec<(NodeId, PathBuf, u64, bool)> = self
            .review
            .iter()
            .filter(|id| !tree.node(**id).synthetic)
            .map(|id| {
                let node = tree.node(*id);
                (
                    *id,
                    PathBuf::from(tree.path_string(*id)),
                    node.size(mode),
                    node.is_dir,
                )
            })
            .collect();
        if items.is_empty() {
            self.status =
                crate::i18n::t("Aggregated small-file rows cannot be deleted directly").into();
            return;
        }
        let items = space_clean::coalesce_nested_paths(items, |item| item.1.as_path());
        self.busy = true;
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
                    let mut deleted_ids = Vec::new();
                    let mut errors = Vec::new();
                    let mut progress = space_clean::CleanProgress {
                        items_total: items.len(),
                        ..Default::default()
                    };
                    let _ = tx.send(BgMsg::DeleteProgress(progress.clone()));
                    let mut candidates = Vec::new();
                    for item in items {
                        progress.current_path = item.1.clone();
                        if space_clean::safety::is_never_touch(&item.1) {
                            errors.push(format!("Protected: {}", item.1.display()));
                            progress.items_done += 1;
                            progress.error_count = errors.len();
                            let _ = tx.send(BgMsg::DeleteProgress(progress.clone()));
                            continue;
                        }
                        if std::fs::symlink_metadata(&item.1).is_err() {
                            progress.items_done += 1;
                            let _ = tx.send(BgMsg::DeleteProgress(progress.clone()));
                            continue;
                        }
                        candidates.push(item);
                    }
                    candidates.sort_by(|a, b| {
                        a.1.parent().cmp(&b.1.parent()).then_with(|| a.1.cmp(&b.1))
                    });
                    for chunk in candidates.chunks(space_clean::TRASH_BATCH_SIZE) {
                        if let Some((_, path, _, _)) = chunk.first() {
                            progress.current_path = path.clone();
                            let _ = tx.send(BgMsg::DeleteProgress(progress.clone()));
                        }
                        let paths: Vec<PathBuf> =
                            chunk.iter().map(|(_, path, _, _)| path.clone()).collect();
                        let outcomes = space_clean::trash_paths_batch(&paths);
                        for ((id, path, bytes, is_dir), outcome) in
                            chunk.iter().cloned().zip(outcomes)
                        {
                            progress.current_path = path.clone();
                            match outcome {
                                Ok(()) => {
                                    deleted_ids.push(id);
                                    progress.items_done += 1;
                                    progress.bytes_processed =
                                        progress.bytes_processed.saturating_add(bytes);
                                    if is_dir {
                                        progress.folders_done += 1;
                                    } else {
                                        progress.files_done += 1;
                                    }
                                }
                                Err(e) => {
                                    errors.push(format!("{}: {e}", path.display()));
                                    progress.items_done += 1;
                                    progress.error_count = errors.len();
                                }
                            }
                        }
                        let _ = tx.send(BgMsg::DeleteProgress(progress.clone()));
                    }
                    if deleted_ids.is_empty() && !errors.is_empty() {
                        return Err(errors.join("\n"));
                    }

                    Ok((deleted_ids, errors))
                })();
                let _ = tx.send(BgMsg::DeleteDone(result));
            })
            .ok();
    }

    fn refresh_volumes(&mut self) {
        self.volumes_loading = true;
        let tx = self.bg_tx.clone();
        thread::Builder::new()
            .name("space-vols".into())
            .spawn(move || {
                let _ = tx.send(BgMsg::Volumes(volumes::list_volumes()));
            })
            .ok();
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

    /// Open Cleanup and check for junk unless a result is already there.
    fn find_junk(&mut self) {
        self.set_mode(Mode::Clean);
        if self.junk.report().is_none() && !self.junk.loading {
            self.check_junk();
        }
    }

    /// Start a fresh junk check (the user asked for it).
    fn check_junk(&mut self) {
        self.junk.open_and_scan();
        self.start_junk_scan();
    }

    /// Switch pages. Opening Cleanup does not start a check by itself.
    fn set_mode(&mut self, mode: Mode) {
        if mode == Mode::Clean {
            self.junk.open = true;
        }
        if mode != Mode::Review && self.basket_len() == 0 {
            self.trash_result = None;
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

    fn apply_junk_clean(&mut self) {
        if self.busy {
            return;
        }
        let findings = self.junk.selected_findings();
        if findings.is_empty() {
            return;
        }
        self.busy = true;
        self.junk.cleaning = true;
        self.junk.clean_progress = space_clean::CleanProgress {
            items_total: findings.len(),
            ..Default::default()
        };
        self.junk.clean_started = Some(Instant::now());
        self.junk.confirm = false;
        self.status = crate::i18n::t("Moving junk to Trash…").into();
        let tx = self.bg_tx.clone();
        thread::Builder::new()
            .name("space-junk-clean".into())
            .spawn(move || {
                let report = space_clean::ScanReport {
                    findings,
                    total_bytes: 0,
                    rules_matched: 0,
                    rules_scanned: 0,
                    skipped_roots: Vec::new(),
                };
                let progress_tx = tx.clone();
                let result = space_clean::clean_with_progress(&report, false, move |progress| {
                    let _ = progress_tx.send(BgMsg::JunkCleanProgress(progress));
                })
                .map_err(|e| e.to_string());
                let _ = tx.send(BgMsg::JunkCleanDone(result));
            })
            .ok();
    }

    fn review_bytes(&self) -> u64 {
        self.tree
            .as_ref()
            .map(|t| review::queue_bytes(t, &self.review, self.size_mode))
            .unwrap_or(0)
    }

    /// Items in the Trash queue: Explore picks plus selected junk.
    fn basket_items(&self) -> Vec<(PathBuf, u64)> {
        let mut items = Vec::new();
        if let Some(tree) = &self.tree {
            items.extend(self.review.iter().map(|&id| {
                (
                    PathBuf::from(tree.path_string(id)),
                    tree.node(id).size(self.size_mode),
                )
            }));
        }
        items.extend(
            self.junk
                .selected_findings()
                .into_iter()
                .map(|finding| (finding.path, finding.bytes)),
        );
        space_clean::coalesce_nested_paths(items, |item| item.0.as_path())
    }

    fn basket_len(&self) -> usize {
        self.basket_items().len()
    }

    fn basket_bytes(&self) -> u64 {
        if self.junk.selected_count() == 0 {
            return self.review_bytes();
        }
        self.basket_items()
            .iter()
            .fold(0_u64, |sum, item| sum.saturating_add(item.1))
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

    /// One confirmation trashes both sources; junk runs after the Explore batch.
    fn apply_basket(&mut self) {
        // A fresh result for this move; both workers add to it.
        self.trash_result = Some(TrashResult::default());
        let has_junk = self.junk.selected_count() != 0;
        if !self.review.is_empty() && self.tree.is_some() {
            self.junk_after_delete = has_junk;
            self.apply_delete();
        } else if has_junk {
            self.confirm_delete = false;
            self.apply_junk_clean();
        }
    }
}

impl eframe::App for SpaceApp {
    /// Pixel seams between panels show the clear colour; make it the page ground.
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }

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
                self.notify(
                    ui::ToastLevel::Error,
                    crate::tr!("Folder not found: {}", path.display()),
                );
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
        self.apply_filter_debounce();
        self.handle_keys(ctx);
        self.refresh_search_hits();
        if let Some(text) = self.pending_copy.take() {
            ctx.copy_text(text);
        }
        self.persist_settings_from_viewport(ctx);

        if let Some(tree) = &self.tree {
            let path = tree.path_string(self.focus);
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                "Space Scanner — {path}"
            )));
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title("Space Scanner".into()));
        }

        let before = (self.selected, self.mode, self.focus, self.review.len());
        self.ui(ctx);
        // State changed by a click in this frame: draw it now, not on the next input.
        if before != (self.selected, self.mode, self.focus, self.review.len()) {
            ctx.request_repaint();
        }
    }
}

/// Show a file or folder in the system file manager.
pub(crate) fn reveal_path(path: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .args(["-R", path])
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let p = PathBuf::from(path);
        let target = if p.is_dir() {
            p.as_path()
        } else {
            p.parent().unwrap_or(p.as_path())
        };
        let _ = std::process::Command::new("xdg-open").arg(target).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .args(["/select,", path])
            .spawn();
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn folder_arg(args: impl IntoIterator<Item = String>) -> Option<PathBuf> {
    args.into_iter()
        .skip(1)
        .find(|a| !a.starts_with('-'))
        .map(PathBuf::from)
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

/// Node for an absolute path inside the scanned tree, walking by name.
fn find_path(tree: &space_core::SpaceTree, path: &std::path::Path) -> Option<NodeId> {
    let rel = path.strip_prefix(&tree.root_path).ok()?;
    let mut id = tree.root;
    for comp in rel.components() {
        let name = comp.as_os_str().to_string_lossy();
        id = *tree
            .children(id)
            .iter()
            .find(|&&c| tree.node(c).name == name)?;
    }
    Some(id)
}
