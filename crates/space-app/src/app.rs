use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Receiver};
use eframe::egui::{self, Align, Key, Layout, RichText, Sense};
use space_core::{
    format_bytes, rescan_subtree, scan_path_with_expect, CacheMeta, CacheStore, NodeId, ScanEvent,
    ScanHandle, ScanProgress, SizeMode, SpaceTree, CACHE_VERSION,
};

use crate::anim::{TransKind, Transition};
use crate::review;
use crate::settings::AppSettings;
use crate::side_tree::SideTree;
use crate::theme::{self, ACCENT, MUTED, TEXT};
use crate::tm_clean::{self, CleanReport, LocalSnapshot};
use crate::views::{self, DrawCtx, EntryKindFilter, GeomCache, ViewFilter, ViewMode};
use crate::volumes::{self, VolumeInfo};

enum BgMsg {
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
    DeleteDone(Result<(Arc<SpaceTree>, usize), String>),
    TmCleanDone(CleanReport),
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
    show_tm_panel: bool,
    confirm_tm_clean: bool,
    tm_snapshots: Vec<LocalSnapshot>,
    tm_loading: bool,
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
            status: format!(
                "Open a folder or pick a volume. · cache v{CACHE_VERSION}"
            ),
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
            show_tm_panel: false,
            confirm_tm_clean: false,
            tm_snapshots: Vec::new(),
            tm_loading: true,
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
        self.side_tree.rebuild_if_needed(
            &tree,
            tree.root,
            self.size_mode,
            &self.filter,
            fhash,
        );
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
                }
                BgMsg::RescanDone(Err(e)) => {
                    self.busy = false;
                    self.status = format!("Incremental rescan failed: {e}");
                }
                BgMsg::DeleteDone(Ok((tree, n))) => {
                    self.busy = false;
                    self.status = format!("Moved {n} item(s) to Trash");
                    self.review.clear();
                    self.selected = None;
                    self.confirm_delete = false;
                    self.tree = Some(tree);
                    self.invalidate_largest();
                    self.schedule_largest();
                }
                BgMsg::DeleteDone(Err(e)) => {
                    self.busy = false;
                    self.confirm_delete = false;
                    self.status = format!("Trash failed: {e}");
                }
                BgMsg::TmCleanDone(report) => {
                    self.busy = false;
                    self.confirm_tm_clean = false;
                    if report.deleted == 0 && report.errors.is_empty() {
                        self.status = format!(
                            "TM snapshots: nothing to remove ({}/{} kept)",
                            report.kept, report.before
                        );
                    } else if report.errors.is_empty() {
                        self.status = format!(
                            "Deleted {} of {} old TM snapshot(s), kept {}",
                            report.deleted, report.before, report.kept
                        );
                    } else {
                        self.status = format!(
                            "TM clean: deleted {} of {}, {} error(s)",
                            report.deleted,
                            report.before,
                            report.errors.len()
                        );
                    }
                    self.refresh_volumes();
                    self.refresh_snapshots();
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
        if self.volumes_loading || self.tm_loading || self.busy || self.scan.is_some() {
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
        let Some(tree) = self.tree.clone() else { return };
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
        let Some(cache) = self.cache.clone() else { return };
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
                        let loaded = cache.load(&path_bg).ok().map(|l| {
                            (Arc::new(l.tree), l.stale)
                        });
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
        let Some(tree) = self.tree.clone() else { return };
        if self.busy {
            return;
        }
        self.busy = true;
        self.status = format!("Incremental rescan of {}…", tree.node(self.focus).name);
        let focus = self.focus;
        let cache = self.cache.clone();
        let tx = self.bg_tx.clone();
        thread::Builder::new()
            .name("space-rescan".into())
            .spawn(move || {
                let result = rescan_subtree(tree.as_ref(), focus).map(|t| {
                    let arc = Arc::new(t);
                    if let Some(cache) = &cache {
                        let _ = cache.save(arc.as_ref());
                    }
                    arc
                });
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
            let zoom_in = tree
                .path_to(id)
                .iter()
                .any(|&p| p == self.focus)
                && id != self.focus;
            // Child of current focus → zoom in; otherwise zoom out / sideways.
            let kind = if zoom_in
                || tree
                    .node(id)
                    .parent
                    .is_some_and(|p| p == self.focus)
            {
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
        ) = input;

        if question || (mods.shift && slash) {
            self.show_shortcuts = !self.show_shortcuts;
        }
        if mods.command || mods.ctrl {
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
            if key_r {
                self.rescan_current();
            }
            if key_o {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    self.start_scan(path, false);
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
            if self.show_shortcuts {
                self.show_shortcuts = false;
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
            let id = self.selected.or(self.hovered);
            if let Some(id) = id {
                if self
                    .tree
                    .as_ref()
                    .is_some_and(|t| t.try_node(id).is_some_and(|n| n.is_dir))
                {
                    self.navigate(id, true);
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

    fn apply_delete(&mut self) {
        let Some(tree) = self.tree.clone() else { return };
        if self.review.is_empty() || self.busy {
            return;
        }
        let paths: Vec<PathBuf> = self
            .review
            .iter()
            .map(|id| PathBuf::from(tree.path_string(*id)))
            .collect();
        let ids = self.review.clone();
        self.busy = true;
        self.status = "Moving to Trash…".into();
        let cache = self.cache.clone();
        let tx = self.bg_tx.clone();
        thread::Builder::new()
            .name("space-trash".into())
            .spawn(move || {
                let result = (|| {
                    let n = review::trash_paths(&paths)?;
                    let mut new_tree = (*tree).clone();
                    let mut ids = ids;
                    ids.sort_by_key(|id| std::cmp::Reverse(new_tree.path_to(*id).len()));
                    for id in ids {
                        new_tree.remove_node(id);
                    }
                    let arc = Arc::new(new_tree);
                    if let Some(cache) = &cache {
                        let _ = cache.invalidate(PathBuf::from(&arc.root_path).as_path());
                        let _ = cache.save(arc.as_ref());
                    }
                    Ok((arc, n))
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
}

impl eframe::App for SpaceApp {
    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.settings.default_view = self.view;
        self.settings.default_size_mode = self.size_mode;
        self.settings.save();
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(
                "Space Scanner".into(),
            ));
        }

        if self.show_shortcuts {
            egui::Window::new("Keyboard shortcuts")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("← →  history · ↑ Esc  up · Enter  open");
                    ui.label("Home  root · End/↓  select largest child");
                    ui.label("/ or ⌘F  filter · Esc  clear filter");
                    ui.label("⌘1–4  views · ⌘⇧C  copy path · ⌘O  open · ⌘R  rescan");
                    ui.label("?  toggle this help");
                    if ui.button("Close").clicked() {
                        self.show_shortcuts = false;
                    }
                });
        }

        if self.confirm_delete {
            egui::Window::new("Review and Remove")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    let bytes = self
                        .tree
                        .as_ref()
                        .map(|t| review::queue_bytes(t, &self.review, self.size_mode))
                        .unwrap_or(0);
                    ui.label(format!(
                        "Move {} item(s) ({} ) to Trash?",
                        self.review.len(),
                        format_bytes(bytes)
                    ));
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
            egui::Window::new("Local snapshots")
                .collapsible(false)
                .resizable(true)
                .default_size([420.0, 380.0])
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    let tm_n = self.tm_snapshots.iter().filter(|s| s.is_tm()).count();
                    let other_n = self.tm_snapshots.len().saturating_sub(tm_n);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} TM · {} other",
                                tm_n, other_n
                            ))
                            .strong()
                            .color(ACCENT),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add_enabled(!self.tm_loading && !self.busy, egui::Button::new("Refresh"))
                                .clicked()
                            {
                                self.refresh_snapshots();
                            }
                        });
                    });
                    ui.label(
                        RichText::new(
                            "These hold APFS space outside the file tree (Unaccounted).",
                        )
                        .small()
                        .color(MUTED),
                    );
                    ui.add_space(6.0);

                    if self.tm_loading {
                        ui.label(RichText::new("Loading snapshots…").color(MUTED));
                    } else if self.tm_snapshots.is_empty() {
                        ui.label(RichText::new("No local snapshots found.").color(MUTED));
                    } else {
                        egui::ScrollArea::vertical()
                            .max_height(260.0)
                            .show(ui, |ui| {
                                egui::Grid::new("tm_snap_grid")
                                    .num_columns(3)
                                    .spacing([12.0, 6.0])
                                    .striped(true)
                                    .show(ui, |ui| {
                                        ui.label(RichText::new("When").small().color(MUTED));
                                        ui.label(RichText::new("Kind").small().color(MUTED));
                                        ui.label(RichText::new("").small());
                                        ui.end_row();
                                        for snap in &self.tm_snapshots {
                                            ui.label(RichText::new(snap.label()).color(TEXT));
                                            ui.label(
                                                RichText::new(snap.kind_label()).color(MUTED),
                                            );
                                            let badge = if snap.purgeable {
                                                "purgeable"
                                            } else {
                                                "kept"
                                            };
                                            ui.label(
                                                RichText::new(badge)
                                                    .small()
                                                    .color(if snap.purgeable {
                                                        ACCENT
                                                    } else {
                                                        MUTED
                                                    }),
                                            );
                                            ui.end_row();
                                        }
                                    });
                            });
                    }

                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Close").clicked() {
                            self.show_tm_panel = false;
                            self.confirm_tm_clean = false;
                        }
                        let keep = if tm_n > 1 { 1 } else { 0 };
                        let remove = tm_n.saturating_sub(keep);
                        let can_clean = remove > 0 && !self.busy && !self.tm_loading;
                        let hint = if self.tm_loading {
                            "Loading snapshots…"
                        } else if tm_n == 0 {
                            "No Time Machine snapshots found"
                        } else if tm_n == 1 {
                            "Delete this local Time Machine snapshot"
                        } else {
                            "Delete older snapshots, keep the newest"
                        };
                        if ui
                            .add_enabled(
                                can_clean,
                                egui::Button::new(
                                    RichText::new("Delete old TM").color(ACCENT),
                                ),
                            )
                            .on_hover_text(hint)
                            .clicked()
                        {
                            self.confirm_tm_clean = true;
                        }
                    });

                    if self.confirm_tm_clean {
                        ui.separator();
                        let keep = if tm_n > 1 { 1 } else { 0 };
                        let remove = tm_n.saturating_sub(keep);
                        ui.label(if keep == 0 {
                            format!("Delete this Time Machine snapshot?")
                        } else {
                            format!(
                                "Delete {remove} oldest Time Machine snapshot(s), keep the newest?"
                            )
                        });
                        ui.label(
                            RichText::new("OS update snapshots are left alone.")
                                .small()
                                .color(MUTED),
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                self.confirm_tm_clean = false;
                            }
                            if ui
                                .button(RichText::new("Delete").color(ACCENT))
                                .clicked()
                            {
                                self.apply_tm_clean();
                            }
                        });
                    }
                });
        }

        egui::TopBottomPanel::top("top")
            .exact_height(48.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(8.0);
                    if ui.button("Open…").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.start_scan(path, false);
                        }
                    }
                    if ui
                        .add_enabled(
                            self.tree.is_some() && self.scan.is_none() && !self.busy,
                            egui::Button::new("Rescan"),
                        )
                        .clicked()
                    {
                        self.rescan_current();
                    }
                    if ui
                        .add_enabled(
                            self.tree.is_some() && self.scan.is_none() && !self.busy,
                            egui::Button::new("Rescan folder"),
                        )
                        .clicked()
                    {
                        self.incremental_rescan_focus();
                    }
                    if self.scan.is_some() && ui.button("Cancel").clicked() {
                        if let Some(s) = &self.scan {
                            s.cancel();
                        }
                    }

                    let snap_n = if self.tm_snapshots.is_empty() {
                        self.volumes
                            .iter()
                            .map(|v| v.snapshot_count)
                            .max()
                            .unwrap_or(0)
                    } else {
                        self.tm_snapshots.len() as u32
                    };
                    let tm_label = if self.tm_loading {
                        "Clean TM…".to_string()
                    } else if snap_n > 0 {
                        format!("Clean TM ({snap_n})")
                    } else {
                        "Clean TM".to_string()
                    };
                    if ui
                        .button(RichText::new(tm_label).color(ACCENT))
                        .on_hover_text("Local Time Machine snapshots — list and delete old ones")
                        .clicked()
                    {
                        self.open_tm_panel();
                    }

                    ui.separator();
                    if ui
                        .add_enabled(!self.history_back.is_empty(), egui::Button::new("←"))
                        .clicked()
                    {
                        self.go_back();
                    }
                    if ui
                        .add_enabled(!self.history_fwd.is_empty(), egui::Button::new("→"))
                        .clicked()
                    {
                        self.go_forward();
                    }
                    if ui
                        .add_enabled(
                            self.tree
                                .as_ref()
                                .is_some_and(|t| t.node(self.focus).parent.is_some()),
                            egui::Button::new("↑"),
                        )
                        .clicked()
                    {
                        self.go_up();
                    }

                    ui.separator();
                    ui.label(RichText::new("Space Scanner").strong().color(ACCENT));
                    if self.cache_stale {
                        ui.label(RichText::new("STALE").small().color(theme::ACCENT));
                    }

                    ui.separator();
                    if let Some(tree) = &self.tree {
                        let crumbs = tree.breadcrumb_names(self.focus);
                        let chain = tree.path_to(self.focus);
                        let mut nav_to = None;
                        ui.horizontal(|ui| {
                            for (i, name) in crumbs.iter().enumerate() {
                                if i > 0 {
                                    ui.label(RichText::new("/").color(MUTED));
                                }
                                if ui
                        .add(
                            egui::Label::new(RichText::new(name).color(TEXT)).sense(Sense::click()),
                        )
                        .on_hover_text("Go here")
                        .clicked()
                    {
                        nav_to = Some(chain[i]);
                    }
                            }
                        });
                        if ui.small_button("Copy path").clicked() {
                            self.copy_focus_path();
                        }
                        if ui.small_button("?").on_hover_text("Shortcuts").clicked() {
                            self.show_shortcuts = true;
                        }
                        if let Some(id) = nav_to {
                            self.navigate(id, true);
                        }
                    } else if self.settings.flags.show_shortcuts_hint {
                        ui.label(RichText::new("Press ? for shortcuts").small().color(MUTED));
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(8.0);
                        for mode in ViewMode::ALL.iter().rev() {
                            let selected = self.view == *mode;
                            if ui.selectable_label(selected, mode.label()).clicked() {
                                self.set_view(*mode);
                            }
                        }
                        ui.separator();
                        let label = match self.size_mode {
                            SizeMode::OnDisk => "On disk",
                            SizeMode::Apparent => "Apparent",
                        };
                        if ui.button(label).clicked() {
                            self.size_mode = match self.size_mode {
                                SizeMode::OnDisk => SizeMode::Apparent,
                                SizeMode::Apparent => SizeMode::OnDisk,
                            };
                            self.invalidate_largest();
                            self.schedule_largest();
                        }
                    });
                });
            });

        egui::TopBottomPanel::bottom("bottom")
            .exact_height(52.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(12.0);
                    if let Some(tree) = &self.tree {
                        let indexed = tree.total_size_mode(self.size_mode);
                        ui.label(format!(
                            "Indexed {} · {} items{}",
                            format_bytes(indexed),
                            tree.item_count,
                            if self.from_cache { " · cache" } else { "" }
                        ));
                        if let Some(vol) = volumes::match_volume(&self.volumes, &tree.root_path) {
                            let vol_used = vol.used();
                            if vol_used > 0 {
                                ui.separator();
                                ui.label(
                                    RichText::new(format!(
                                        "Volume {} / {}",
                                        format_bytes(vol_used),
                                        format_bytes(vol.total.max(vol_used))
                                    ))
                                    .color(MUTED),
                                );
                                if vol_used > indexed.saturating_add(64 << 20) {
                                    let gap = vol_used.saturating_sub(indexed);
                                    let snap_n = if !self.tm_snapshots.is_empty() {
                                        self.tm_snapshots.len() as u32
                                    } else {
                                        vol.snapshot_count
                                    };
                                    let snap = if snap_n > 0 {
                                        format!(" · {snap_n} snapshots")
                                    } else {
                                        String::new()
                                    };
                                    ui.label(
                                        RichText::new(format!(
                                            "Unaccounted {}{snap}",
                                            format_bytes(gap)
                                        ))
                                        .color(ACCENT)
                                        .strong(),
                                    );
                                }
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
                                .desired_width(160.0)
                                .fill(ACCENT),
                        );
                    }

                    ui.separator();
                    ui.label(RichText::new(&self.status).color(MUTED));

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(12.0);
                        let snap_n = if !self.tm_snapshots.is_empty() {
                            self.tm_snapshots.len()
                        } else {
                            self.volumes
                                .iter()
                                .map(|v| v.snapshot_count as usize)
                                .max()
                                .unwrap_or(0)
                        };
                        let tm_label = if snap_n > 0 {
                            format!("Clean TM ({snap_n})")
                        } else {
                            "Clean TM".to_string()
                        };
                        if ui
                            .button(RichText::new(tm_label).color(ACCENT))
                            .on_hover_text("Local Time Machine snapshots")
                            .clicked()
                        {
                            self.open_tm_panel();
                        }
                        let can_review = !self.review.is_empty() && self.scan.is_none();
                        if ui
                            .add_enabled(
                                can_review,
                                egui::Button::new(
                                    RichText::new("Review and Remove").color(ACCENT),
                                ),
                            )
                            .clicked()
                        {
                            self.confirm_delete = true;
                        }
                        if ui
                            .add_enabled(self.selected.is_some() || self.tree.is_some(), egui::Button::new("Reveal"))
                            .clicked()
                        {
                            self.reveal_selected();
                        }
                        if let (Some(tree), Some(sel)) = (&self.tree, self.selected) {
                            let n = tree.node(sel);
                            ui.label(format!(
                                "{} · {} · {} files",
                                n.name,
                                format_bytes(n.size(self.size_mode)),
                                n.file_count
                            ));
                        } else if !self.review.is_empty() {
                            let bytes = self
                                .tree
                                .as_ref()
                                .map(|t| review::queue_bytes(t, &self.review, self.size_mode))
                                .unwrap_or(0);
                            ui.label(format!(
                                "Queue {} · {}",
                                self.review.len(),
                                format_bytes(bytes)
                            ));
                        } else {
                            ui.label(RichText::new("No selection").color(MUTED));
                        }
                    });
                });
            });

        egui::SidePanel::left("sidebar")
            .default_width(300.0)
            .min_width(240.0)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.label(RichText::new("Volumes").strong());
                egui::ScrollArea::vertical()
                    .id_salt("vols")
                    .max_height(120.0)
                    .show(ui, |ui| {
                        let mut scan_mount = None;
                        for vol in &self.volumes {
                            let text = if vol.total == 0 {
                                vol.name.clone()
                            } else {
                                vol.label()
                            };
                            if ui
                                .add_enabled(!self.busy && self.scan.is_none(), egui::Button::new(text))
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
                            ui.label(RichText::new("disk stats…").small().color(MUTED));
                        }
                        if ui.small_button("Refresh volumes").clicked() {
                            self.refresh_volumes_async();
                        }
                    });
                ui.separator();

                ui.horizontal(|ui| {
                    ui.label("Filter");
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.filter_draft)
                            .desired_width(140.0)
                            .hint_text("name…"),
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
                    let badge = self.filter.active_count();
                    if badge > 0 {
                        ui.label(RichText::new(format!("({badge})")).color(ACCENT).small());
                    }
                    if ui.small_button("Clear").clicked() {
                        self.filter.clear();
                        self.filter_draft.clear();
                        self.filter_edited_at = None;
                        self.side_tree.invalidate();
                        self.geom = None;
                    }
                });
                ui.horizontal(|ui| {
                    let kind = self.filter.kind;
                    if ui
                        .selectable_label(kind == EntryKindFilter::All, "All")
                        .clicked()
                    {
                        self.filter.kind = EntryKindFilter::All;
                        self.side_tree.invalidate();
                        self.geom = None;
                    }
                    if ui
                        .selectable_label(kind == EntryKindFilter::DirsOnly, "Dirs")
                        .clicked()
                    {
                        self.filter.kind = EntryKindFilter::DirsOnly;
                        self.side_tree.invalidate();
                        self.geom = None;
                    }
                    if ui
                        .selectable_label(kind == EntryKindFilter::FilesOnly, "Files")
                        .clicked()
                    {
                        self.filter.kind = EntryKindFilter::FilesOnly;
                        self.side_tree.invalidate();
                        self.geom = None;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("min").small().color(MUTED));
                    for (label, bytes) in [
                        ("0", 0u64),
                        ("1M", 1 << 20),
                        ("10M", 10 << 20),
                        ("100M", 100 << 20),
                        ("1G", 1 << 30),
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
                if let Some(ext) = &self.filter.ext {
                    ui.label(RichText::new(format!("ext: .{ext}")).color(ACCENT));
                }

                ui.separator();

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

                    ui.label(RichText::new("Tree").strong().color(ACCENT));
                    ui.label(
                        RichText::new(format!("focus: {title} · {}", format_bytes(size)))
                            .small()
                            .color(MUTED),
                    );
                    ui.add_space(4.0);

                    // Live hover / selection info
                    if let (Some(tree), Some(hid)) =
                        (self.tree.as_ref(), self.hovered.or(self.selected))
                    {
                        if let Some(n) = tree.try_node(hid) {
                            let pct = self
                                .percent_of_parent(hid)
                                .map(|p| format!("  ·  {p:.1}% of parent"))
                                .unwrap_or_default();
                            let path = tree.path_string(hid);
                            ui.label(
                                RichText::new(format!(
                                    "{}  ·  {}{}{}",
                                    n.name,
                                    format_bytes(n.size(self.size_mode)),
                                    if n.is_dir {
                                        format!("  ·  {} files", n.file_count)
                                    } else {
                                        String::new()
                                    },
                                    pct
                                ))
                                .color(ACCENT)
                                .size(12.0),
                            );
                            ui.label(RichText::new(path).small().color(MUTED));
                        }
                    }

                    let tree = self.tree.clone().unwrap();
                    let show_pct = self.settings.flags.show_percent_parent;
                    let mode = self.size_mode;
                    let selected = self.selected;
                    let hovered = self.hovered;
                    let focus = self.focus;
                    let tree_action = {
                        egui::ScrollArea::vertical()
                            .id_salt("side_tree_wrap")
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
                        review::toggle(&mut self.review, id);
                    }
                } else {
                    ui.label(RichText::new("Recent scans").strong());
                    if self.recent.is_empty() {
                        ui.label(RichText::new("No cache yet").color(MUTED));
                    } else {
                        for meta in self.recent.clone() {
                            let label = format!(
                                "{}\n{} · {} items",
                                meta.root_path,
                                format_bytes(meta.total_size),
                                meta.item_count
                            );
                            if ui.button(label).clicked() {
                                self.start_scan(PathBuf::from(&meta.root_path), false);
                            }
                        }
                    }
                }
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.scan.is_some() || self.busy {
                ui.centered_and_justified(|ui| {
                    ui.spinner();
                    ui.label(&self.status);
                });
                return;
            }

            let Some(tree) = self.tree.clone() else {
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("Space Scanner")
                                .size(28.0)
                                .color(ACCENT)
                                .strong(),
                        );
                        ui.label(
                            RichText::new(
                                "Sunburst · Container · Radial · Cascade\nIndex once, browse from cache\n\nOpen a folder, pick a volume, or press ⌘O\nPress ? for keyboard shortcuts",
                            )
                            .color(MUTED),
                        );
                        ui.add_space(12.0);
                        if ui.button(RichText::new("Open folder").size(16.0)).clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                self.start_scan(path, false);
                            }
                        }
                    });
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
            let files_list_h = if files_open { 148.0 } else { 0.0 };
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
            }
            if let Some(id) = action.toggle_review {
                review::toggle(&mut self.review, id);
            }
            if let Some(ext) = action.filter_ext {
                if self.filter.ext.as_deref() == Some(ext.as_str()) {
                    self.filter.ext = None;
                } else {
                    self.filter.ext = Some(ext);
                }
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
                    let chev = if files_open { "▾" } else { "▸" };
                    let title = format!(
                        "{chev}  Files in {files_label}  ·  {}",
                        file_rows.len()
                    );
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

                if files_open {
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
                            .max_height(files_list_h - 4.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for &(id, sz) in &file_rows {
                                    let name = tree.node(id).name.clone();
                                    let selected = self.selected == Some(id);
                                    let in_q = self.review.contains(&id);
                                    let hovered = self.hovered == Some(id);
                                    ui.horizontal(|ui| {
                                        ui.add_space(8.0);
                                        let prefix = if in_q { "✓ " } else { "· " };
                                        let text = RichText::new(format!("{prefix}{name}"))
                                            .color(if hovered { ACCENT } else { TEXT });
                                        let resp = ui.selectable_label(selected || hovered, text);
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
                                            select_file = Some(id);
                                        }
                                        if resp.double_clicked() {
                                            open_file = Some(id);
                                        }
                                        if resp.secondary_clicked() {
                                            toggle_file = Some(id);
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
                }
            });
        });
    }
}
