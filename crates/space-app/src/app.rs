use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Receiver};
use eframe::egui::{self, Align, Key, Layout, RichText, Sense};
use space_core::{
    format_bytes, rescan_subtree, scan_path_with_expect, CacheMeta, CacheStore, NodeId, ScanEvent,
    ScanHandle, ScanProgress, SizeMode, SpaceTree,
};

use crate::anim::{TransKind, Transition};
use crate::junk_panel::{self, JunkAction, JunkPanelState};
use crate::review;
use crate::rules_panel::{self, RulesAction, RulesPanelState};
use crate::settings::AppSettings;
use crate::side_tree::SideTree;
use crate::theme::{self, ACCENT, MUTED, TEXT};
use crate::tm_clean::{self, CleanReport, LocalSnapshot};
use crate::views::{self, DrawCtx, EntryKindFilter, GeomCache, ViewFilter, ViewMode};
use crate::volumes::{self, VolumeInfo};

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
        focus: NodeId,
        mode: SizeMode,
        rows: Vec<(NodeId, String, u64)>,
    },
}

pub struct SpaceApp {
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
    largest_key: Option<(NodeId, SizeMode)>,
    largest_rows: Vec<(NodeId, String, u64)>,
    largest_loading: bool,
    /// Hierarchical sidebar tree (smooth expand).
    side_tree: SideTree,
    /// Cached view geometry (sunburst mesh / treemap tiles).
    geom: Option<GeomCache>,
    settings: AppSettings,
    filter_draft: String,
    filter_edited_at: Option<Instant>,
    show_shortcuts: bool,
    focus_filter: bool,
    transition: Option<Transition>,
    pending_open: Option<PathBuf>,
}

impl SpaceApp {
    pub fn new(cc: &eframe::CreationContext<'_>, settings: AppSettings) -> Self {
        theme::apply(&cc.egui_ctx);

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
            status: "Open a folder, drop one here, or press ⌘O.".into(),
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
            largest_key: None,
            largest_rows: Vec::new(),
            largest_loading: false,
            side_tree: SideTree::default(),
            geom: None,
            settings,
            filter_draft: String::new(),
            filter_edited_at: None,
            show_shortcuts: false,
            focus_filter: false,
            transition: None,
            pending_open: folder_arg(std::env::args()),
        }
    }

    fn ensure_side_tree(&mut self) {
        let Some(tree) = self.tree.clone() else {
            self.side_tree.clear();
            return;
        };
        // Drill-in path stays open; hover shows that circle's peers and collapses extras.
        self.side_tree.ensure_path_expanded(&tree, self.focus);
        self.side_tree
            .sync_for_hover(&tree, self.focus, self.hovered);
        let fhash = self.filter.hash64();
        self.side_tree
            .rebuild_if_needed(&tree, tree.root, self.size_mode, &self.filter, fhash);
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
                            self.status = if stale {
                                format!(
                                    "Cached index (stale) · {} items · {} — Rescan recommended",
                                    tree.item_count,
                                    format_bytes(tree.total_size_mode(self.size_mode))
                                )
                            } else {
                                format!(
                                    "Loaded cache · {} items · {}",
                                    tree.item_count,
                                    format_bytes(tree.total_size_mode(self.size_mode))
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
                BgMsg::RescanDone(Ok(tree)) => {
                    self.busy = false;
                    self.focus = tree.root;
                    self.selected = None;
                    self.review.clear();
                    self.history_back.clear();
                    self.history_fwd.clear();
                    self.cache_stale = false;
                    self.from_cache = false;
                    self.status = format!(
                        "Rescanned · {} items · {}",
                        tree.item_count,
                        format_bytes(tree.total_size_mode(self.size_mode))
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
                    self.status = format!("Incremental rescan failed: {e}");
                }
                BgMsg::DeleteProgress(progress) => {
                    self.delete_progress = Some(progress);
                }
                BgMsg::DeleteDone(Ok((mut deleted_ids, errors))) => {
                    self.busy = false;
                    self.delete_progress = None;
                    self.delete_started = None;
                    let n = deleted_ids.len();
                    self.status = if errors.is_empty() {
                        format!("Moved {n} item(s) to Trash")
                    } else {
                        format!("Moved {n} item(s) · {} failed", errors.len())
                    };
                    self.review.clear();
                    self.selected = None;
                    self.confirm_delete = false;
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
                    self.delete_progress = None;
                    self.delete_started = None;
                    self.confirm_delete = false;
                    self.status = format!("Trash failed: {e}");
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
                    self.status = format!(
                        "Junk · {} · {} items",
                        format_bytes(
                            self.junk
                                .report
                                .as_ref()
                                .map(|r| r.total_bytes)
                                .unwrap_or(0)
                        ),
                        self.junk
                            .report
                            .as_ref()
                            .map(|r| r.findings.len())
                            .unwrap_or(0)
                    );
                }
                BgMsg::JunkScanDone(Err(e)) => {
                    self.junk.loading = false;
                    self.junk.scan_started = None;
                    self.junk.error = Some(e.clone());
                    self.status = format!("Junk scan failed: {e}");
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
                    self.status = format!(
                        "Junk · moved {} to Trash · {}",
                        result.deleted.len(),
                        format_bytes(result.processed_bytes)
                    );
                    self.junk.selected.clear();
                    self.start_junk_scan();
                }
                BgMsg::JunkCleanDone(Err(e)) => {
                    self.busy = false;
                    self.junk.cleaning = false;
                    self.junk.clean_started = None;
                    self.junk.error = Some(e.clone());
                    self.status = format!("Junk clean failed: {e}");
                }
                BgMsg::Largest { focus, mode, rows } => {
                    self.largest_loading = false;
                    if self.focus == focus && self.size_mode == mode {
                        self.largest_key = Some((focus, mode));
                        self.largest_rows = rows;
                    }
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
        } else if self.largest_loading {
            // Don't spin the UI at 20fps while a background walk finishes.
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        } else if got {
            ctx.request_repaint();
        }
    }

    fn invalidate_largest(&mut self) {
        self.largest_key = None;
        self.largest_rows.clear();
        self.largest_loading = false;
        self.side_tree.invalidate();
        self.geom = None;
    }

    fn schedule_largest(&mut self) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        let focus = self.focus;
        let mode = self.size_mode;
        if self.largest_key == Some((focus, mode)) {
            return;
        }
        self.largest_loading = true;
        let tx = self.bg_tx.clone();
        thread::Builder::new()
            .name("space-largest".into())
            .spawn(move || {
                let ids = tree.largest_files_bounded(focus, mode, 12, 8_000);
                let rows: Vec<(NodeId, String, u64)> = ids
                    .into_iter()
                    .map(|id| {
                        let n = tree.node(id);
                        (id, n.name.clone(), n.size(mode))
                    })
                    .collect();
                let _ = tx.send(BgMsg::Largest { focus, mode, rows });
            })
            .ok();
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
                self.status = format!("Loading cache for {}…", path.display());
                self.tree = None;
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
        self.from_cache = false;
        self.cache_stale = false;
        self.busy = false;
        self.tree = None;
        self.review.clear();
        self.history_back.clear();
        self.history_fwd.clear();
        self.invalidate_largest();
        self.progress = Some(ScanProgress {
            files_seen: 0,
            bytes_seen: 0,
            current_path: path.display().to_string(),
            skipped_errors: 0,
        });
        self.status = format!("Scanning {}…", path.display());
        self.scan = Some(scan_path_with_expect(path, self.expected_items));
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
        self.busy = true;
        self.status = format!("Incremental rescan of {}…", tree.node(self.focus).name);
        let focus = self.focus;
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
            self.status = format!(
                "Scanning… {} · {}{}{}",
                format_bytes(p.bytes_seen),
                p.files_seen,
                pct,
                if p.skipped_errors > 0 {
                    format!(" · {} skipped", p.skipped_errors)
                } else {
                    String::new()
                }
            );
            self.progress = Some(p);
        }

        if cancelled {
            self.status = "Scan cancelled.".into();
            self.scan = None;
            self.progress = None;
            return;
        }

        if let Some(e) = err {
            self.status = format!("Scan error: {e}");
            self.scan = None;
            self.progress = None;
            return;
        }

        if let Some(tree) = done_tree {
            let skip = tree.skipped_errors;
            let hl = tree.hardlinks_deduped;
            self.status = format!(
                "Indexed {} items · {}{}{}",
                tree.item_count,
                format_bytes(tree.total_size_mode(self.size_mode)),
                if skip > 0 {
                    format!(" · {skip} skipped")
                } else {
                    String::new()
                },
                if hl > 0 {
                    format!(" · {hl} hardlinks deduped")
                } else {
                    String::new()
                }
            );
            self.focus = tree.root;
            self.selected = None;
            self.from_cache = false;
            self.cache_stale = false;
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

    fn copy_focus_path(&mut self) {
        let Some(tree) = &self.tree else { return };
        let id = self.hovered.or(self.selected).unwrap_or(self.focus);
        let path = tree.path_string(id);
        self.status = format!("Copied {path}");
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
        let psize = tree.node(parent).size(self.size_mode).max(1);
        Some((node.size(self.size_mode) as f32 / psize as f32) * 100.0)
    }

    fn apply_filter_debounce(&mut self) {
        let ms = self.settings.flags.debounce_filter_ms;
        if let Some(at) = self.filter_edited_at {
            if at.elapsed() >= Duration::from_millis(ms) {
                if self.filter.query != self.filter_draft {
                    self.filter.query = self.filter_draft.clone();
                    self.side_tree.invalidate();
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
                i.key_pressed(Key::Delete),
                i.key_pressed(Key::Backspace),
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
            key_del,
            key_back,
        ) = input;

        if question || (mods.shift && slash) {
            self.show_shortcuts = !self.show_shortcuts;
        }
        if mods.command || mods.ctrl {
            if self.tree.is_some() {
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
            }
            if key_c && mods.shift {
                self.copy_focus_path();
            }
            if key_f {
                self.focus_filter = true;
            }
            if key_r && self.scan.is_none() {
                if mods.shift {
                    self.incremental_rescan_focus();
                } else {
                    self.rescan_current();
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
            if (key_del || key_back) && !ctx.wants_keyboard_input() {
                let selected_deletable = self.selected.is_some_and(|id| {
                    self.tree
                        .as_ref()
                        .is_some_and(|tree| !tree.node(id).synthetic)
                });
                if !self.review.is_empty() || selected_deletable {
                    if self.review.is_empty() {
                        if let Some(id) = self.selected.filter(|_| selected_deletable) {
                            self.review.push(id);
                        }
                    }
                    self.confirm_delete = true;
                }
            }
            if left {
                self.go_back();
            }
            if right {
                self.go_forward();
            }
            return;
        }

        if slash {
            self.focus_filter = true;
        }
        if esc {
            if self.scan.is_some() {
                if let Some(s) = &self.scan {
                    s.cancel();
                }
            } else if self.show_shortcuts {
                self.show_shortcuts = false;
            } else if self.confirm_delete && self.delete_progress.is_none() {
                self.confirm_delete = false;
            } else if self.junk.confirm {
                self.junk.confirm = false;
            } else if self.junk.open {
                self.junk.open = false;
            } else if self.rules.open {
                self.rules.open = false;
            } else if self.show_tm_panel {
                self.show_tm_panel = false;
            } else if self.filter.is_active() || !self.filter_draft.is_empty() {
                self.filter.clear();
                self.filter_draft.clear();
                self.side_tree.invalidate();
                self.geom = None;
            } else {
                self.go_up();
            }
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
            // Select / focus largest child
            if let Some(tree) = &self.tree {
                if let Some(&first) = tree.children(self.focus).first() {
                    self.selected = Some(first);
                    if down && tree.node(first).is_dir && enter {
                        // no-op; enter handles open
                    }
                }
            }
        }
        if enter {
            if self.confirm_delete && self.delete_progress.is_none() && !self.review.is_empty() {
                self.apply_delete();
                return;
            }
            if self.junk.confirm && !self.junk.cleaning {
                self.junk.confirm = false;
                self.apply_junk_clean();
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
            let id = self.selected.or(self.hovered);
            if let Some(id) = id {
                let is_dir = self
                    .tree
                    .as_ref()
                    .is_some_and(|t| t.try_node(id).is_some_and(|n| n.is_dir));
                if is_dir {
                    self.navigate(id, true);
                } else {
                    self.selected = Some(id);
                    self.reveal_selected();
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

    fn reveal_selected(&self) {
        let Some(tree) = &self.tree else { return };
        let id = self.selected.unwrap_or(self.focus);
        let path = tree.path_string(id);
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("open")
                .args(["-R", &path])
                .spawn();
        }
        #[cfg(target_os = "linux")]
        {
            let p = PathBuf::from(&path);
            let target = p.parent().unwrap_or(p.as_path());
            let _ = std::process::Command::new("xdg-open").arg(target).spawn();
        }
        #[cfg(target_os = "windows")]
        {
            let _ = std::process::Command::new("explorer")
                .args(["/select,", &path])
                .spawn();
        }
    }

    fn ui_primary_file_actions(&mut self, ui: &mut egui::Ui) {
        let selected_deletable = self.selected.is_some_and(|id| {
            self.tree
                .as_ref()
                .is_some_and(|tree| !tree.node(id).synthetic)
        });
        let can_review =
            (!self.review.is_empty() || selected_deletable) && self.scan.is_none() && !self.busy;
        let review_label = if self.review.is_empty() {
            "Move to Trash…".to_string()
        } else {
            format!("Trash queue ({})…", self.review.len())
        };
        let review_hint = if can_review {
            "Opens a list first. Items go to Trash, not gone forever."
        } else if self.tree.is_none() {
            "Scan a folder first"
        } else {
            "Tick Trash in the tree, or select a file on the map"
        };
        if ui
            .add_enabled(
                can_review,
                egui::Button::new(RichText::new(review_label).color(ACCENT)),
            )
            .on_hover_text(review_hint)
            .clicked()
        {
            if self.review.is_empty() {
                if let Some(id) = self.selected.filter(|_| selected_deletable) {
                    self.review.push(id);
                }
            }
            self.confirm_delete = true;
        }
        if ui
            .add_enabled(
                self.selected.is_some() || self.tree.is_some(),
                egui::Button::new("Show in Finder"),
            )
            .on_hover_text("Open the selected item in Finder")
            .clicked()
        {
            self.reveal_selected();
        }
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
            self.status = "Aggregated small-file rows cannot be deleted directly".into();
            return;
        }
        let items = space_clean::coalesce_nested_paths(items, |item| item.1.as_path());
        self.busy = true;
        self.delete_progress = Some(space_clean::CleanProgress {
            items_total: items.len(),
            ..Default::default()
        });
        self.delete_started = Some(Instant::now());
        self.status = "Moving to Trash…".into();
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
        self.status = "Deleting old Time Machine snapshots…".into();
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
        self.junk.open_and_scan();
        self.start_junk_scan();
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
        self.settings.flags.apply_to_rule_prefs(&mut prefs);
        let tx = self.bg_tx.clone();
        thread::Builder::new()
            .name("space-junk-scan".into())
            .spawn(move || {
                let result = (|| {
                    let pack = space_clean::RulePack::load_many_shared(
                        &space_clean::default_rules_dirs(),
                    )?;
                    prefs.migrate_catalog_aliases(&space_clean::default_rules_dirs())?;
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
        self.status = "Moving junk to Trash…".into();
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

    fn ui_welcome(&mut self, ui: &mut egui::Ui) {
        let hovering_drop = ui.input(|i| !i.raw.hovered_files.is_empty());
        if hovering_drop {
            let rect = ui.max_rect();
            ui.painter().rect_filled(
                rect,
                0.0,
                egui::Color32::from_rgba_unmultiplied(232, 168, 72, 28),
            );
        }
        let mut scan_path: Option<PathBuf> = None;
        ui.vertical_centered(|ui| {
            ui.add_space(28.0);
            ui.label(
                RichText::new("Space Scanner")
                    .size(28.0)
                    .color(ACCENT)
                    .strong(),
            );
            ui.label(
                RichText::new("See what fills the disk, then clean what’s safe.")
                    .size(14.0)
                    .color(MUTED),
            );
            ui.add_space(16.0);
            if hovering_drop {
                ui.label(
                    RichText::new("Drop to scan this folder")
                        .size(16.0)
                        .color(ACCENT)
                        .strong(),
                );
                ui.add_space(8.0);
            }
            ui.allocate_ui_with_layout(
                egui::vec2(416.0, 36.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    if ui
                        .add_sized(
                            [200.0, 32.0],
                            egui::Button::new(RichText::new("Scan Home").size(16.0).color(ACCENT)),
                        )
                        .on_hover_text("Your user folder — usually the fastest useful scan")
                        .clicked()
                    {
                        if let Some(home) = std::env::var_os("HOME") {
                            scan_path = Some(PathBuf::from(home));
                        }
                    }
                    if ui
                        .add_sized(
                            [200.0, 32.0],
                            egui::Button::new(RichText::new("Open folder…").size(16.0)),
                        )
                        .on_hover_text("Choose any folder · ⌘O")
                        .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            scan_path = Some(path);
                        }
                    }
                },
            );
            ui.add_space(8.0);
            if ui
                .add_sized(
                    [200.0, 28.0],
                    egui::Button::new(RichText::new("Clean junk").size(14.0)),
                )
                .on_hover_text("Caches, logs, temp — no scan needed")
                .clicked()
            {
                self.open_junk_panel();
            }
            ui.add_space(6.0);
            ui.label(
                RichText::new("or pick a volume below · drop a folder · ? for shortcuts")
                    .small()
                    .color(MUTED),
            );
        });

        ui.add_space(22.0);
        let col_w = ui.available_width().min(520.0);
        ui.horizontal(|ui| {
            let pad = ((ui.available_width() - col_w) * 0.5).max(0.0);
            ui.add_space(pad);
            ui.vertical(|ui| {
                ui.set_width(col_w);
                ui.group(|ui| {
                    ui.label(RichText::new("Volumes").strong().color(TEXT));
                    ui.label(
                        RichText::new("Click a disk to scan the whole volume.")
                            .small()
                            .color(MUTED),
                    );
                    ui.add_space(8.0);
                    for vol in &self.volumes {
                        let used = vol.used();
                        let frac = if vol.total > 0 {
                            used as f32 / vol.total as f32
                        } else {
                            0.0
                        };
                        let caption = if vol.total == 0 {
                            vol.name.clone()
                        } else {
                            format!(
                                "{}  ·  {} free of {}",
                                vol.name,
                                format_bytes(vol.available),
                                format_bytes(vol.total)
                            )
                        };
                        let enabled = !self.busy && self.scan.is_none();
                        let row = egui::Frame::new()
                            .fill(theme::PANEL_2)
                            .corner_radius(8.0)
                            .inner_margin(egui::Margin::symmetric(10, 8))
                            .show(ui, |ui| {
                                ui.add_enabled_ui(enabled, |ui| {
                                    ui.label(RichText::new(caption).size(13.0).color(TEXT));
                                    if vol.total > 0 {
                                        ui.add(
                                            egui::ProgressBar::new(frac.clamp(0.0, 1.0))
                                                .desired_width(ui.available_width())
                                                .fill(ACCENT)
                                                .show_percentage(),
                                        );
                                    }
                                });
                            });
                        let hit = row.response.interact(Sense::click());
                        hit.clone()
                            .on_hover_text(format!("Scan {}", vol.mount.display()));
                        if enabled && hit.clicked() {
                            scan_path = Some(vol.mount.clone());
                        }
                    }
                    if self.volumes_loading {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new("Reading disks…").small().color(MUTED));
                        });
                    }
                });

                if !self.recent.is_empty() {
                    ui.add_space(16.0);
                    ui.group(|ui| {
                        ui.label(RichText::new("Recent scans").strong());
                        ui.label(
                            RichText::new("Opens from cache when the folder hasn’t changed.")
                                .small()
                                .color(MUTED),
                        );
                        ui.add_space(6.0);
                        for meta in &self.recent {
                            let name = std::path::Path::new(&meta.root_path)
                                .file_name()
                                .and_then(|s| s.to_str())
                                .unwrap_or(meta.root_path.as_str());
                            let label = format!(
                                "{name}  ·  {}  ·  {} items",
                                format_bytes(meta.total_size),
                                meta.item_count
                            );
                            if ui
                                .add(
                                    egui::Button::new(RichText::new(label).size(13.0))
                                        .min_size(egui::vec2(ui.available_width(), 26.0)),
                                )
                                .on_hover_text(&meta.root_path)
                                .clicked()
                            {
                                scan_path = Some(PathBuf::from(&meta.root_path));
                            }
                        }
                    });
                }
            });
        });

        if let Some(path) = scan_path {
            self.start_scan(path, false);
        }
    }

    fn ui_scan_placeholder(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(48.0);
            ui.spinner();
            ui.add_space(8.0);
            ui.label(RichText::new(&self.status).color(TEXT));
            if let Some(p) = &self.progress {
                ui.label(
                    RichText::new(format!(
                        "{} files · {}",
                        p.files_seen,
                        format_bytes(p.bytes_seen)
                    ))
                    .color(MUTED),
                );
            }
            ui.add_space(12.0);
            if ui
                .add_sized(
                    [160.0, 32.0],
                    egui::Button::new(RichText::new("Cancel").size(15.0)),
                )
                .on_hover_text("Stop this scan")
                .clicked()
            {
                if let Some(s) = &self.scan {
                    s.cancel();
                }
            }
            ui.add_space(6.0);
            ui.label(
                RichText::new("Wrong folder? Cancel, then Scan Home or drop another folder.")
                    .small()
                    .color(MUTED),
            );
        });
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
                self.status = format!("Folder not found: {}", path.display());
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
        self.persist_settings_from_viewport(ctx);

        if let Some(tree) = &self.tree {
            let path = tree.path_string(self.focus);
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                "Space Scanner — {path}"
            )));
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title("Space Scanner".into()));
        }

        if self.show_shortcuts {
            let mut open = true;
            egui::Window::new("Keyboard shortcuts")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(
                        RichText::new("Shortcuts for the map, plus ⌘J for junk.")
                            .small()
                            .color(MUTED),
                    );
                    ui.add_space(6.0);
                    egui::Grid::new("shortcut_grid")
                        .num_columns(2)
                        .spacing([16.0, 6.0])
                        .show(ui, |ui| {
                            for (keys, action) in [
                                ("⌘O", "Open folder"),
                                ("⌘R", "Rescan everything"),
                                ("⌘⇧R", "Rescan this folder"),
                                ("← →", "Back / forward"),
                                ("↑  Esc", "Go up"),
                                (
                                    "Enter",
                                    "Open folder, show file in Finder, or confirm a dialog",
                                ),
                                ("Home", "Jump to scan root"),
                                ("End / ↓", "Select largest child"),
                                ("/  or  ⌘F", "Focus name filter"),
                                (
                                    "Esc",
                                    "Cancel scan, close panel, then clear filter, then go up",
                                ),
                                ("⌘J", "Clean junk"),
                                ("⌘⌫", "Move selection / queue to Trash"),
                                ("⌘1–4", "Switch map view"),
                                ("Click folder", "Open it on the map"),
                                ("Click file", "Select it"),
                                ("⌘-click", "Add to Trash queue"),
                                ("⌘⇧C", "Copy path"),
                                ("?", "This help"),
                            ] {
                                ui.label(RichText::new(keys).monospace().color(ACCENT));
                                ui.label(action);
                                ui.end_row();
                            }
                        });
                    ui.add_space(10.0);
                    if ui.button("Close").clicked() {
                        self.show_shortcuts = false;
                    }
                });
            if !open {
                self.show_shortcuts = false;
            }
        }

        if self.confirm_delete {
            egui::Window::new("Move to Trash")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    if let Some(progress) = &self.delete_progress {
                        let fraction = if progress.items_total > 0 {
                            progress.items_done as f32 / progress.items_total as f32
                        } else {
                            0.0
                        };
                        let elapsed = self
                            .delete_started
                            .map(|started| started.elapsed().as_secs())
                            .unwrap_or(0);
                        ui.label(
                            RichText::new(format!(
                                "Moving to Trash · {}/{} items",
                                progress.items_done, progress.items_total
                            ))
                            .strong(),
                        );
                        ui.label(
                            RichText::new(format!(
                                "{} files · {} folders · {} · {} errors · {}s",
                                progress.files_done,
                                progress.folders_done,
                                format_bytes(progress.bytes_processed),
                                progress.error_count,
                                elapsed
                            ))
                            .color(MUTED),
                        );
                        ui.add(
                            egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
                                .desired_width(420.0)
                                .show_percentage(),
                        );
                        if !progress.current_path.as_os_str().is_empty() {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(progress.current_path.display().to_string())
                                        .small()
                                        .monospace()
                                        .color(MUTED),
                                )
                                .truncate(),
                            )
                            .on_hover_text(progress.current_path.display().to_string());
                        }
                        return;
                    }

                    let bytes = self
                        .tree
                        .as_ref()
                        .map(|t| review::queue_bytes(t, &self.review, self.size_mode))
                        .unwrap_or(0);
                    ui.label(
                        RichText::new("Files stay in Trash until you empty it. You can restore them in Finder.")
                            .small()
                            .color(MUTED),
                    );
                    ui.add_space(8.0);
                    ui.label(format!(
                        "Move {} item(s) ({}) to Trash?",
                        self.review.len(),
                        format_bytes(bytes)
                    ));
                    if let Some(tree) = &self.tree {
                        egui::ScrollArea::vertical()
                            .max_height(140.0)
                            .show(ui, |ui| {
                                for id in self.review.iter().take(12) {
                                    let n = tree.node(*id);
                                    ui.label(format!(
                                        "· {}  ({})",
                                        n.name,
                                        format_bytes(n.size(self.size_mode))
                                    ));
                                }
                                if self.review.len() > 12 {
                                    ui.label(
                                        RichText::new(format!(
                                            "…and {} more",
                                            self.review.len() - 12
                                        ))
                                        .color(MUTED),
                                    );
                                }
                            });
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.confirm_delete = false;
                        }
                        if ui
                            .button(RichText::new("Move to Trash").color(theme::ACCENT))
                            .clicked()
                        {
                            self.apply_delete();
                        }
                    });
                });
        }

        if self.show_tm_panel {
            let mut open = true;
            egui::Window::new("Time Machine")
                .collapsible(false)
                .resizable(true)
                .open(&mut open)
                .default_size([420.0, 420.0])
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    let tm_n = self.tm_snapshots.iter().filter(|s| s.is_tm()).count();
                    let other_n = self.tm_snapshots.len().saturating_sub(tm_n);

                    ui.label(
                        RichText::new("Local snapshots")
                            .size(18.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.label(
                        RichText::new(
                            "Snapshots use disk space that does not appear as normal files. Deleting old Time Machine copies can free that space.",
                        )
                        .size(12.0)
                        .color(MUTED),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{tm_n}"))
                                .size(22.0)
                                .strong()
                                .color(ACCENT),
                        );
                        ui.label(RichText::new("Time Machine").color(TEXT));
                        if other_n > 0 {
                            ui.separator();
                            ui.label(RichText::new(format!("{other_n} OS / other")).color(MUTED));
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add_enabled(
                                    !self.tm_loading && !self.busy,
                                    egui::Button::new("Refresh"),
                                )
                                .clicked()
                            {
                                self.refresh_snapshots();
                            }
                        });
                    });
                    ui.add_space(8.0);

                    if self.tm_loading {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new("Loading…").color(MUTED));
                        });
                    } else if self.tm_snapshots.is_empty() {
                        ui.label(RichText::new("No local snapshots.").color(MUTED));
                    } else {
                        egui::ScrollArea::vertical()
                            .max_height(240.0)
                            .show(ui, |ui| {
                                for snap in &self.tm_snapshots {
                                    egui::Frame::new()
                                        .fill(theme::PANEL_2)
                                        .corner_radius(6.0)
                                        .inner_margin(egui::Margin::symmetric(10, 8))
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.vertical(|ui| {
                                                    ui.label(
                                                        RichText::new(snap.label())
                                                            .color(TEXT)
                                                            .strong(),
                                                    );
                                                    ui.label(
                                                        RichText::new(snap.kind_label())
                                                            .small()
                                                            .color(MUTED),
                                                    );
                                                });
                                                ui.with_layout(
                                                    Layout::right_to_left(Align::Center),
                                                    |ui| {
                                                        let badge = if !snap.is_tm() {
                                                            "leave this one"
                                                        } else if snap.purgeable {
                                                            "macOS can drop this"
                                                        } else {
                                                            "keeps disk used"
                                                        };
                                                        ui.label(
                                                            RichText::new(badge).small().color(
                                                                if snap.is_tm() {
                                                                    ACCENT
                                                                } else {
                                                                    MUTED
                                                                },
                                                            ),
                                                        );
                                                    },
                                                );
                                            });
                                        });
                                    ui.add_space(4.0);
                                }
                            });
                    }

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Close").clicked() {
                            self.show_tm_panel = false;
                            self.confirm_tm_clean = false;
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let keep = if tm_n > 1 { 1 } else { 0 };
                            let remove = tm_n.saturating_sub(keep);
                            let can_clean = remove > 0 && !self.busy && !self.tm_loading;
                            if ui
                                .add_enabled(
                                    can_clean,
                                    egui::Button::new(
                                        RichText::new(if tm_n <= 1 {
                                            "Delete snapshot".into()
                                        } else {
                                            format!("Delete {remove} old")
                                        })
                                        .color(ACCENT),
                                    ),
                                )
                                .clicked()
                            {
                                self.confirm_tm_clean = true;
                            }
                        });
                    });

                    if self.confirm_tm_clean {
                        ui.add_space(8.0);
                        egui::Frame::new()
                            .fill(theme::PANEL_2)
                            .corner_radius(8.0)
                            .inner_margin(egui::Margin::same(12))
                            .show(ui, |ui| {
                                let keep = if tm_n > 1 { 1 } else { 0 };
                                let remove = tm_n.saturating_sub(keep);
                                ui.label(
                                    RichText::new(if keep == 0 {
                                        "Delete this Time Machine snapshot?".into()
                                    } else {
                                        format!("Delete {remove} oldest, keep the newest?")
                                    })
                                    .strong(),
                                );
                                ui.label(
                                    RichText::new("OS update snapshots are left alone.")
                                        .small()
                                        .color(MUTED),
                                );
                                ui.add_space(6.0);
                                ui.horizontal(|ui| {
                                    if ui.button("Cancel").clicked() {
                                        self.confirm_tm_clean = false;
                                    }
                                    if ui
                                        .button(RichText::new("Delete").color(ACCENT).strong())
                                        .clicked()
                                    {
                                        self.apply_tm_clean();
                                    }
                                });
                            });
                    }
                });
            if !open {
                self.show_tm_panel = false;
            }
        }

        if self.junk.open {
            let mut open = true;
            egui::Window::new("Clean junk")
                .collapsible(false)
                .resizable(true)
                .open(&mut open)
                .default_size([560.0, 520.0])
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    match junk_panel::draw(ui, &mut self.junk, &mut self.settings.flags) {
                        JunkAction::Rescan => self.start_junk_scan(),
                        JunkAction::Clean => self.apply_junk_clean(),
                        JunkAction::Close => {
                            self.junk.open = false;
                            self.junk.confirm = false;
                        }
                        JunkAction::OpenRules => self.open_rules_panel(),
                        JunkAction::PrefsChanged => {
                            self.settings.save();
                            self.start_junk_scan();
                        }
                        JunkAction::None => {}
                    }
                });
            if !open {
                self.junk.open = false;
                self.junk.confirm = false;
            }
        }

        if self.rules.open {
            let mut open = true;
            egui::Window::new("What to clean")
                .collapsible(false)
                .resizable(true)
                .open(&mut open)
                .default_size([820.0, 560.0])
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 12.0])
                .show(ctx, |ui| {
                    if self.rule_mgr.is_none() && self.rules.error.is_none() {
                        self.start_rule_mgr_load();
                    }
                    if let Some(mgr) = self.rule_mgr.as_mut() {
                        match rules_panel::draw(ui, &mut self.rules, mgr, &mut self.settings.flags)
                        {
                            RulesAction::Close => self.rules.open = false,
                            RulesAction::PrefsChanged => {
                                self.settings.save();
                                if self.junk.open {
                                    self.start_junk_scan();
                                }
                            }
                            RulesAction::None => {}
                        }
                    } else if self.rule_mgr_loading {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Loading rules…");
                        });
                        ctx.request_repaint_after(Duration::from_millis(50));
                    } else if let Some(err) = &self.rules.error {
                        ui.colored_label(egui::Color32::from_rgb(220, 90, 90), err);
                        if ui.button("Close").clicked() {
                            self.rules.open = false;
                        }
                    }
                });
            if !open {
                self.rules.open = false;
            }
        }

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.add_space(8.0);
                if self.tree.is_some() {
                    if ui
                        .button("Open folder")
                        .on_hover_text("Scan a folder · ⌘O")
                        .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.start_scan(path, false);
                        }
                    }
                    if ui
                        .add_enabled(
                            self.scan.is_none() && !self.busy,
                            egui::Button::new("Rescan"),
                        )
                        .on_hover_text("Rescan from the root · ⌘R")
                        .clicked()
                    {
                        self.rescan_current();
                    }
                }
                if self.scan.is_some() && ui.button("Cancel").clicked() {
                    if let Some(s) = &self.scan {
                        s.cancel();
                    }
                }

                if self.tree.is_some() {
                    ui.separator();
                    if ui
                        .add_enabled(!self.history_back.is_empty(), egui::Button::new("←"))
                        .on_hover_text("Back")
                        .clicked()
                    {
                        self.go_back();
                    }
                    if ui
                        .add_enabled(!self.history_fwd.is_empty(), egui::Button::new("→"))
                        .on_hover_text("Forward")
                        .clicked()
                    {
                        self.go_forward();
                    }
                    if ui
                        .add_enabled(
                            self.tree
                                .as_ref()
                                .is_some_and(|t| t.node(self.focus).parent.is_some()),
                            egui::Button::new("Up"),
                        )
                        .on_hover_text("Parent folder · ↑ or Esc")
                        .clicked()
                    {
                        self.go_up();
                    }
                }

                if self.cache_stale {
                    ui.label(
                        RichText::new("Cached — files may have changed")
                            .small()
                            .color(ACCENT),
                    )
                    .on_hover_text("Rescan to refresh sizes");
                }

                if let Some(tree) = &self.tree {
                    ui.separator();
                    let crumbs = tree.breadcrumb_names(self.focus);
                    let chain = tree.path_to(self.focus);
                    let mut nav_to = None;
                    let start = crumbs.len().saturating_sub(4);
                    ui.horizontal(|ui| {
                        if start > 0 {
                            ui.label(RichText::new("…").color(MUTED));
                        }
                        for (i, name) in crumbs.iter().enumerate().skip(start) {
                            if i > start {
                                ui.label(RichText::new("/").small().color(MUTED));
                            }
                            let here = i + 1 == crumbs.len();
                            let color = if here { ACCENT } else { TEXT };
                            if ui
                                .add(
                                    egui::Button::new(RichText::new(name).color(color).small())
                                        .frame(false)
                                        .sense(Sense::click()),
                                )
                                .on_hover_text("Go to this folder")
                                .clicked()
                            {
                                nav_to = Some(chain[i]);
                            }
                        }
                    });
                    if let Some(id) = nav_to {
                        self.navigate(id, true);
                    }
                }
            });

            ui.horizontal_wrapped(|ui| {
                ui.add_space(8.0);
                if self.tree.is_some() {
                    let mut view = self.view;
                    egui::ComboBox::from_id_salt("view_mode")
                        .selected_text(format!("Map: {}", view.label()))
                        .show_ui(ui, |ui| {
                            for mode in ViewMode::ALL {
                                let hint = match mode {
                                    ViewMode::Sunburst => "nested rings (default)",
                                    ViewMode::Container => "nested boxes",
                                    ViewMode::Radial => "bubbles",
                                    ViewMode::Cascade => "stacked rows",
                                };
                                ui.selectable_value(
                                    &mut view,
                                    mode,
                                    format!("{} — {hint}", mode.label()),
                                );
                            }
                        });
                    if view != self.view {
                        self.set_view(view);
                    }
                    let label = match self.size_mode {
                        SizeMode::OnDisk => "Size: on disk",
                        SizeMode::Apparent => "Size: files",
                    };
                    if ui
                        .selectable_label(false, label)
                        .on_hover_text(
                            "On disk is space used (APFS clones). Files is the sum of file sizes.",
                        )
                        .clicked()
                    {
                        self.size_mode = match self.size_mode {
                            SizeMode::OnDisk => SizeMode::Apparent,
                            SizeMode::Apparent => SizeMode::OnDisk,
                        };
                        self.invalidate_largest();
                        self.schedule_largest();
                    }
                    ui.separator();
                }
                if ui
                    .button(RichText::new("Junk").color(ACCENT))
                    .on_hover_text("Clean junk: caches, logs, temp — always via Trash · ⌘J")
                    .clicked()
                {
                    self.open_junk_panel();
                }
                if ui
                    .button("Rules")
                    .on_hover_text("Which junk rules are enabled")
                    .clicked()
                {
                    self.open_rules_panel();
                }
                if self.tree.is_some() {
                    ui.separator();
                    self.ui_primary_file_actions(ui);
                    ui.menu_button("More", |ui| {
                        if ui
                            .add_enabled(
                                self.tree.is_some() && self.scan.is_none() && !self.busy,
                                egui::Button::new("Rescan this folder"),
                            )
                            .clicked()
                        {
                            self.incremental_rescan_focus();
                            ui.close_menu();
                        }
                        if ui.button("Copy path").clicked() {
                            self.copy_focus_path();
                            ui.close_menu();
                        }
                        if ui.button("Keyboard shortcuts…").clicked() {
                            self.show_shortcuts = true;
                            ui.close_menu();
                        }
                    });
                }
                if ui
                    .small_button("?")
                    .on_hover_text("Keyboard shortcuts")
                    .clicked()
                {
                    self.show_shortcuts = true;
                }
            });
            ui.add_space(4.0);
        });

        egui::TopBottomPanel::bottom("bottom").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                    ui.add_space(10.0);
                    if let Some(tree) = &self.tree {
                        let indexed = tree.total_size_mode(self.size_mode);
                        ui.label(
                            RichText::new(format!(
                                "This scan {} · {} items{}",
                                format_bytes(indexed),
                                tree.item_count,
                                if self.from_cache { " · from cache" } else { "" }
                            ))
                            .color(MUTED),
                        );

                        let snap_n = if !self.tm_snapshots.is_empty() {
                            self.tm_snapshots.len() as u32
                        } else {
                            volumes::match_volume(&self.volumes, &tree.root_path)
                                .map(|v| v.snapshot_count)
                                .or_else(|| self.volumes.iter().map(|v| v.snapshot_count).max())
                                .unwrap_or(0)
                        };

                        let mut gap: Option<u64> = None;
                        if let Some(vol) = volumes::match_volume(&self.volumes, &tree.root_path) {
                            let vol_used = vol.used();
                            if vol_used > indexed.saturating_add(64 << 20) {
                                gap = Some(vol_used.saturating_sub(indexed));
                            }
                        }

                        if let Some(gap) = gap {
                            ui.separator();
                            let label = if snap_n > 0 {
                                format!(
                                    "{} not in this scan · {snap_n} snapshots",
                                    format_bytes(gap)
                                )
                            } else {
                                format!("{} not in this scan", format_bytes(gap))
                            };
                            if ui
                                .add(
                                    egui::Button::new(RichText::new(label).color(ACCENT).strong())
                                        .frame(false),
                                )
                                .on_hover_text("Disk used on the volume minus this scan — often Time Machine snapshots")
                                .clicked()
                            {
                                self.open_tm_panel();
                            }
                        } else if snap_n > 0 || self.tm_loading {
                            ui.separator();
                            let label = if self.tm_loading {
                                "Snapshots…".to_string()
                            } else {
                                format!("Snapshots · {snap_n}")
                            };
                            if ui
                                .add(
                                    egui::Button::new(RichText::new(label).color(ACCENT))
                                        .frame(false),
                                )
                                .on_hover_text("Local Time Machine snapshots")
                                .clicked()
                            {
                                self.open_tm_panel();
                            }
                        }
                    } else if let Some(p) = &self.progress {
                        ui.label(format!(
                            "{} files · {}",
                            p.files_seen,
                            format_bytes(p.bytes_seen)
                        ));
                        let frac = self
                            .expected_items
                            .filter(|e| *e > 0)
                            .map(|e| (p.files_seen as f32 / e as f32).clamp(0.02, 0.99))
                            .unwrap_or_else(|| {
                                ((p.files_seen % 50_000) as f32 / 50_000.0).clamp(0.05, 0.95)
                            });
                        ui.add(
                            egui::ProgressBar::new(frac)
                                .desired_width(140.0)
                                .fill(ACCENT),
                        );
                    }

                    let status = if self.status.chars().count() > 72 {
                        let t: String = self.status.chars().take(71).collect();
                        format!("{t}…")
                    } else {
                        self.status.clone()
                    };
                    ui.separator();
                    ui.label(RichText::new(status).color(MUTED).small())
                        .on_hover_text(&self.status);

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(10.0);
                        if !self.review.is_empty() {
                            let bytes = self
                                .tree
                                .as_ref()
                                .map(|t| review::queue_bytes(t, &self.review, self.size_mode))
                                .unwrap_or(0);
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(format!(
                                            "Trash {} · {}",
                                            self.review.len(),
                                            format_bytes(bytes)
                                        ))
                                        .small()
                                        .color(ACCENT),
                                    )
                                    .frame(false),
                                )
                                .on_hover_text("Open Trash queue")
                                .clicked()
                            {
                                self.confirm_delete = true;
                            }
                        } else if let (Some(tree), Some(sel)) = (&self.tree, self.selected) {
                            let n = tree.node(sel);
                            ui.label(
                                RichText::new(format!(
                                    "{} · {}",
                                    n.name,
                                    format_bytes(n.size(self.size_mode))
                                ))
                                .small()
                                .color(MUTED),
                            );
                        }
                    });
                });
            });

        if self.tree.is_some() {
            egui::SidePanel::left("sidebar")
            .default_width(280.0)
            .min_width(220.0)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                egui::CollapsingHeader::new(RichText::new("Volumes").small().strong().color(MUTED))
                    .default_open(self.tree.is_none())
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("vols")
                            .max_height(96.0)
                            .show(ui, |ui| {
                                let mut scan_mount = None;
                                for vol in &self.volumes {
                                    let text = if vol.total == 0 {
                                        vol.name.clone()
                                    } else {
                                        vol.label()
                                    };
                                    if ui
                                        .add_enabled(
                                            !self.busy && self.scan.is_none(),
                                            egui::Button::new(RichText::new(text).size(12.0)),
                                        )
                                        .on_hover_text(format!("Scan {}", vol.mount.display()))
                                        .clicked()
                                    {
                                        scan_mount = Some(vol.mount.clone());
                                    }
                                }
                                if let Some(mount) = scan_mount {
                                    self.start_scan(mount, false);
                                }
                                if self.volumes_loading {
                                    ui.spinner();
                                }
                                if ui
                                    .small_button("Refresh")
                                    .on_hover_text("Reload the disk list")
                                    .clicked()
                                {
                                    self.refresh_volumes_async();
                                }
                            });
                    });
                ui.add_space(4.0);

                ui.label(RichText::new("Filter").small().color(MUTED));
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.filter_draft)
                        .desired_width(f32::INFINITY)
                        .hint_text("Folder or file name…"),
                );
                if self.focus_filter {
                    edit.request_focus();
                    self.focus_filter = false;
                }
                if edit.changed() {
                    self.filter_edited_at = Some(Instant::now());
                    ctx.request_repaint_after(Duration::from_millis(
                        self.settings.flags.debounce_filter_ms + 10,
                    ));
                }
                ui.horizontal(|ui| {
                    let kind = self.filter.kind;
                    if ui
                        .selectable_label(kind == EntryKindFilter::All, "All")
                        .on_hover_text("Folders and files")
                        .clicked()
                    {
                        self.filter.kind = EntryKindFilter::All;
                        self.side_tree.invalidate();
                        self.geom = None;
                    }
                    if ui
                        .selectable_label(kind == EntryKindFilter::DirsOnly, "Folders")
                        .on_hover_text("Show folders only")
                        .clicked()
                    {
                        self.filter.kind = EntryKindFilter::DirsOnly;
                        self.side_tree.invalidate();
                        self.geom = None;
                    }
                    if ui
                        .selectable_label(kind == EntryKindFilter::FilesOnly, "Files")
                        .on_hover_text("Show files only")
                        .clicked()
                    {
                        self.filter.kind = EntryKindFilter::FilesOnly;
                        self.side_tree.invalidate();
                        self.geom = None;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let badge = self.filter.active_count();
                        if (badge > 0 || !self.filter_draft.is_empty())
                            && ui.small_button("Clear").clicked()
                        {
                            self.filter.clear();
                            self.filter_draft.clear();
                            self.filter_edited_at = None;
                            self.side_tree.invalidate();
                            self.geom = None;
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Min size").small().color(MUTED));
                    for (label, bytes) in [
                        ("Any", 0u64),
                        ("1 MB", 1 << 20),
                        ("10 MB", 10 << 20),
                        ("100 MB", 100 << 20),
                        ("1 GB", 1 << 30),
                    ] {
                        if ui
                            .selectable_label(self.filter.min_size == bytes, label)
                            .clicked()
                        {
                            self.filter.min_size = bytes;
                            self.side_tree.invalidate();
                            self.geom = None;
                        }
                    }
                });
                if let Some(ext) = self.filter.ext.clone() {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("Extension .{ext}"))
                                .small()
                                .color(ACCENT),
                        );
                        if ui.small_button("Clear").clicked() {
                            self.filter.ext = None;
                            self.side_tree.invalidate();
                            self.geom = None;
                        }
                    });
                }

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);

                if self.tree.is_some() {
                    self.ensure_side_tree();
                    let focus = self.focus;
                    let title = self
                        .tree
                        .as_ref()
                        .map(|t| t.node(focus).name.clone())
                        .unwrap_or_default();
                    let size = self
                        .tree
                        .as_ref()
                        .map(|t| t.node(focus).size(self.size_mode))
                        .unwrap_or(0);

                    ui.label(
                        RichText::new(format!("{title}  ·  {}", format_bytes(size)))
                            .size(12.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.add_space(4.0);

                    if let (Some(tree), Some(hid)) =
                        (self.tree.as_ref(), self.hovered.or(self.selected))
                    {
                        if let Some(n) = tree.try_node(hid) {
                            let pct = self
                                .percent_of_parent(hid)
                                .map(|p| format!(" · {p:.0}%"))
                                .unwrap_or_default();
                            ui.label(
                                RichText::new(format!(
                                    "{} · {}{}",
                                    n.name,
                                    format_bytes(n.size(self.size_mode)),
                                    pct
                                ))
                                .color(ACCENT)
                                .size(11.0),
                            );
                        }
                    }

                    self.schedule_largest();

                    let tree = self.tree.clone().unwrap();
                    let show_pct = self.settings.flags.show_percent_parent;
                    let mode = self.size_mode;
                    let selected = self.selected;
                    let hovered = self.hovered;
                    let focus = self.focus;
                    let tree_h = (ui.available_height() - 156.0).max(80.0);
                    let tree_action = {
                        egui::ScrollArea::vertical()
                            .id_salt("side_tree_wrap")
                            .max_height(tree_h)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                crate::side_tree::draw_rows(
                                    ui,
                                    &tree,
                                    &mut self.side_tree,
                                    selected,
                                    hovered,
                                    &self.review,
                                    focus,
                                    show_pct,
                                    mode,
                                )
                            })
                            .inner
                    };
                    if tree_action.toggled_expand {
                        self.ensure_side_tree();
                    }
                    if let Some(id) = tree_action.select {
                        self.selected = Some(id);
                    }
                    if let Some(id) = tree_action.navigate {
                        self.navigate(id, true);
                    }
                    if let Some(id) = tree_action.toggle_review {
                        if self
                            .tree
                            .as_ref()
                            .is_some_and(|tree| !tree.node(id).synthetic)
                        {
                            review::toggle(&mut self.review, id);
                        }
                    }
                    if let Some(id) = tree_action.reveal {
                        self.selected = Some(id);
                        self.reveal_selected();
                    }

                    ui.add_space(6.0);
                    ui.separator();
                    ui.label(
                        RichText::new("Largest files here")
                            .small()
                            .strong()
                            .color(MUTED),
                    );
                    if self.largest_loading && self.largest_rows.is_empty() {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new("Finding big files…").small().color(MUTED));
                        });
                    } else if self.largest_rows.is_empty() {
                        ui.label(
                            RichText::new("No files in this folder")
                                .small()
                                .color(MUTED),
                        );
                    } else {
                        let rows = self.largest_rows.clone();
                        let mut pick = None;
                        let mut reveal = None;
                        let mut queued = None;
                        let review = self.review.clone();
                        let tree = self.tree.clone();
                        egui::ScrollArea::vertical()
                            .id_salt("largest_files")
                            .max_height(130.0)
                            .show(ui, |ui| {
                                for (id, name, sz) in &rows {
                                    let selected = self.selected == Some(*id);
                                    let in_q = review.contains(id);
                                    let resp = ui.selectable_label(
                                        selected,
                                        RichText::new(format!(
                                            "{}  ·  {}",
                                            name,
                                            format_bytes(*sz)
                                        ))
                                        .size(12.0),
                                    );
                                    resp.clone().on_hover_text(
                                        "Click to select · ⌘-click Trash queue · double-click Finder",
                                    );
                                    resp.context_menu(|ui| {
                                        if ui.button("Select").clicked() {
                                            pick = Some(*id);
                                            ui.close_menu();
                                        }
                                        let q = if in_q {
                                            "Remove from Trash queue"
                                        } else {
                                            "Add to Trash queue"
                                        };
                                        if ui.button(q).clicked() {
                                            queued = Some(*id);
                                            ui.close_menu();
                                        }
                                        if ui.button("Show in Finder").clicked() {
                                            reveal = Some(*id);
                                            ui.close_menu();
                                        }
                                    });
                                    if resp.clicked() {
                                        if ui.input(|i| i.modifiers.command || i.modifiers.ctrl) {
                                            queued = Some(*id);
                                        }
                                        pick = Some(*id);
                                    }
                                    if resp.double_clicked() {
                                        reveal = Some(*id);
                                    }
                                }
                            });
                        if let Some(id) = pick {
                            self.selected = Some(id);
                        }
                        if let Some(id) = reveal {
                            self.selected = Some(id);
                            self.reveal_selected();
                        }
                        if let Some(id) = queued {
                            if tree.is_some_and(|t| !t.node(id).synthetic) {
                                review::toggle(&mut self.review, id);
                            }
                        }
                    }
                } else {
                    ui.label(RichText::new("Recent").small().strong().color(MUTED));
                    if self.recent.is_empty() {
                        ui.label(RichText::new("No cached scans yet").small().color(MUTED));
                    } else {
                        for meta in self.recent.clone() {
                            let name = std::path::Path::new(&meta.root_path)
                                .file_name()
                                .and_then(|s| s.to_str())
                                .unwrap_or(meta.root_path.as_str());
                            let label = format!(
                                "{name}\n{} · {} items",
                                format_bytes(meta.total_size),
                                meta.item_count
                            );
                            if ui
                                .button(RichText::new(label).size(12.0))
                                .on_hover_text(&meta.root_path)
                                .clicked()
                            {
                                self.start_scan(PathBuf::from(&meta.root_path), false);
                            }
                        }
                    }
                }
            });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            if (self.scan.is_some() || self.busy) && self.tree.is_none() {
                self.ui_scan_placeholder(ui);
                return;
            }

            let Some(tree) = self.tree.clone() else {
                egui::ScrollArea::vertical()
                    .id_salt("welcome")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.ui_welcome(ui);
                    });
                return;
            };

            let fhash = self.filter.hash64();
            let review_sig = views::review_sig(&self.review);

            if let Some(tr) = &self.transition {
                if tr.done() {
                    self.transition = None;
                } else {
                    ctx.request_repaint();
                }
            }
            let (opacity, scale) = self
                .transition
                .as_ref()
                .map(|t| (t.opacity(), t.scale()))
                .unwrap_or((1.0, 1.0));

            let files_open = self.settings.flags.show_files_under_map;
            let files_bar_h = 28.0;
            let files_open_t = ctx.animate_bool_with_time(
                egui::Id::new("files_panel_open"),
                files_open,
                0.28,
            );
            let files_list_h = 148.0 * crate::anim::ease_out_cubic(files_open_t);
            let bottom_h = files_bar_h + files_list_h;

            let full = ui.available_rect_before_wrap();
            let map_area = egui::Rect::from_min_max(
                full.min,
                egui::pos2(full.max.x, (full.max.y - bottom_h).max(full.min.y + 80.0)),
            );
            let files_area = egui::Rect::from_min_max(
                egui::pos2(full.min.x, map_area.max.y),
                full.max,
            );

            let draw_rect = {
                let c = map_area.center();
                let sz = map_area.size() * scale;
                egui::Rect::from_center_size(c, sz)
            };
            ui.set_opacity(opacity);
            let action = ui
                .allocate_new_ui(egui::UiBuilder::new().max_rect(draw_rect), |ui| {
                    views::draw(
                        ui,
                        self.view,
                        &mut DrawCtx {
                            tree: &tree,
                            focus: self.focus,
                            selected: self.selected,
                            hovered: self.hovered,
                            review: &self.review,
                            mode: self.size_mode,
                            filter: &self.filter,
                            filter_hash: fhash,
                            review_sig,
                            geom: &mut self.geom,
                        },
                    )
                })
                .inner;
            ui.set_opacity(1.0);

            if self.scan.is_none() && !self.busy && self.settings.flags.show_shortcuts_hint {
                let hint_h = 40.0;
                let hint_rect = egui::Rect::from_min_size(
                    map_area.min,
                    egui::vec2(map_area.width(), hint_h),
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(hint_rect), |ui| {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        egui::Frame::new()
                            .fill(theme::PANEL_2)
                            .corner_radius(8.0)
                            .inner_margin(egui::Margin::symmetric(10, 6))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(self.view.click_hint())
                                        .small()
                                        .color(TEXT),
                                    );
                                    if ui.small_button("Got it").clicked() {
                                        self.settings.flags.show_shortcuts_hint = false;
                                        self.settings.save();
                                    }
                                });
                            });
                    });
                });
            }

            if ctx.input(|i| !i.raw.hovered_files.is_empty()) && self.scan.is_none() && !self.busy {
                ui.painter().rect_filled(
                    map_area,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(232, 168, 72, 40),
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(map_area), |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            RichText::new("Drop to scan this folder")
                                .size(18.0)
                                .color(ACCENT)
                                .strong(),
                        );
                    });
                });
            }

            if self.scan.is_some() || self.busy {
                ui.painter().rect_filled(
                    map_area,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(12, 13, 15, 150),
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(map_area), |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new(&self.status).color(TEXT));
                        });
                    });
                });
            }

            if self.hovered != action.hover {
                self.hovered = action.hover;
                ctx.request_repaint();
            } else {
                self.hovered = action.hover;
            }
            if let Some(id) = action.select {
                self.selected = Some(id);
            }
            if let Some(id) = action.navigate {
                self.navigate(id, true);
                if self.settings.flags.show_shortcuts_hint {
                    self.settings.flags.show_shortcuts_hint = false;
                    self.settings.save();
                }
            }
            if let Some(id) = action.toggle_review {
                if !tree.node(id).synthetic {
                    review::toggle(&mut self.review, id);
                }
            }
            if let Some(id) = action.reveal {
                self.selected = Some(id);
                self.reveal_selected();
            }
            if let Some(ext) = action.filter_ext {
                if self.filter.ext.as_deref() == Some(ext.as_str()) {
                    self.filter.ext = None;
                } else {
                    self.filter.ext = Some(ext);
                }
                self.side_tree.invalidate();
                self.geom = None;
            }
            if action.clear_filter {
                self.filter.clear();
                self.filter_draft.clear();
                self.filter_edited_at = None;
                self.side_tree.invalidate();
                self.geom = None;
            }

            // Files strip under the diagram (collapsible)
            let files_dir = self
                .selected
                .filter(|&id| tree.try_node(id).is_some_and(|n| n.is_dir))
                .or_else(|| {
                    self.selected
                        .and_then(|id| tree.try_node(id).and_then(|n| n.parent))
                })
                .unwrap_or(self.focus);
            let files_label = tree
                .try_node(files_dir)
                .map(|n| n.name.clone())
                .unwrap_or_else(|| "Files".into());
            let file_rows =
                crate::side_tree::folder_files(&tree, files_dir, self.size_mode, &self.filter, 100);

            ui.scope_builder(egui::UiBuilder::new().max_rect(files_area), |ui| {
                ui.painter()
                    .rect_filled(files_area, 0.0, theme::PANEL);
                ui.painter().hline(
                    files_area.x_range(),
                    files_area.top(),
                    egui::Stroke::new(1.0_f32, theme::BORDER),
                );

                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    let title = if files_open {
                        format!("▾  Files in {files_label}  ·  {} shown", file_rows.len())
                    } else {
                        format!("▸  Show files in {files_label}")
                    };
                    if ui
                        .add(egui::Button::new(RichText::new(title).color(ACCENT)).frame(false))
                        .on_hover_text(if files_open {
                            "Hide file list"
                        } else {
                            "Show file list under diagram"
                        })
                        .clicked()
                    {
                        self.settings.flags.show_files_under_map = !files_open;
                        self.settings.save();
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(10.0);
                        if !file_rows.is_empty() {
                            ui.label(
                                RichText::new(format_bytes(
                                    file_rows.iter().map(|(_, s)| *s).sum::<u64>(),
                                ))
                                .small()
                                .color(MUTED),
                            );
                        }
                    });
                });

                if files_open_t > 0.02 {
                    ui.set_opacity(files_open_t);
                    if file_rows.is_empty() {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("  No files in this folder")
                                .small()
                                .color(MUTED),
                        );
                    } else {
                        let mut select_file = None;
                        let mut toggle_file = None;
                        let mut open_file = None;
                        egui::ScrollArea::vertical()
                            .id_salt("map_folder_files")
                            .max_height((files_list_h - 4.0).max(0.0))
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for &(id, sz) in &file_rows {
                                    let node = tree.node(id);
                                    let name = node.name.clone();
                                    let deletable = !node.synthetic;
                                    let selected = self.selected == Some(id);
                                    let in_q = self.review.contains(&id);
                                    let hovered = self.hovered == Some(id);
                                    ui.horizontal(|ui| {
                                        ui.add_space(8.0);
                                        let mut queued = in_q;
                                        if ui
                                            .add_enabled(
                                                deletable,
                                                egui::Checkbox::without_text(&mut queued),
                                            )
                                            .on_hover_text("Add to Trash queue")
                                            .changed()
                                        {
                                            toggle_file = Some(id);
                                        }
                                        let text = RichText::new(name)
                                            .color(if hovered { ACCENT } else { TEXT });
                                        let resp = ui.selectable_label(selected || hovered, text);
                                        resp.clone().on_hover_text(
                                            "Click to select · ⌘-click Trash queue · double-click Finder",
                                        );
                                        let queued = in_q;
                                        let can_del = deletable;
                                        resp.context_menu(|ui| {
                                            if ui.button("Select").clicked() {
                                                select_file = Some(id);
                                                ui.close_menu();
                                            }
                                            if can_del {
                                                let label = if queued {
                                                    "Remove from Trash queue"
                                                } else {
                                                    "Add to Trash queue"
                                                };
                                                if ui.button(label).clicked() {
                                                    toggle_file = Some(id);
                                                    ui.close_menu();
                                                }
                                            }
                                            if ui.button("Show in Finder").clicked() {
                                                open_file = Some(id);
                                                ui.close_menu();
                                            }
                                        });
                                        ui.with_layout(
                                            Layout::right_to_left(Align::Center),
                                            |ui| {
                                                ui.add_space(12.0);
                                                ui.label(
                                                    RichText::new(format_bytes(sz))
                                                        .color(MUTED)
                                                        .monospace()
                                                        .small(),
                                                );
                                            },
                                        );
                                        if resp.hovered() {
                                            self.hovered = Some(id);
                                        }
                                        if resp.clicked() {
                                            if ui.input(|i| i.modifiers.command || i.modifiers.ctrl)
                                                && deletable
                                            {
                                                toggle_file = Some(id);
                                            }
                                            select_file = Some(id);
                                        }
                                        if resp.double_clicked() {
                                            open_file = Some(id);
                                        }
                                    });
                                }
                            });
                        if let Some(id) = select_file {
                            self.selected = Some(id);
                        }
                        if let Some(id) = toggle_file {
                            review::toggle(&mut self.review, id);
                        }
                        if let Some(id) = open_file {
                            self.selected = Some(id);
                            self.reveal_selected();
                        }
                    }
                    ui.set_opacity(1.0);
                }
            });
        });
    }
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
