use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use crossbeam_channel::{unbounded, Receiver};
use eframe::egui::{self, Align, Layout, RichText, Sense};
use space_core::{
    format_bytes, rescan_subtree, scan_path_with_expect, CacheMeta, CacheStore, NodeId, ScanEvent,
    ScanHandle, ScanProgress, SizeMode, SpaceTree,
};

use crate::review;
use crate::theme::{self, ACCENT, MUTED, TEXT};
use crate::views::{self, DrawCtx, GeomCache, ViewFilter, ViewMode};
use crate::volumes::{self, VolumeInfo};

enum BgMsg {
    Volumes(Vec<VolumeInfo>),
    Recent(Vec<CacheMeta>),
    /// Cache hit (or forced miss path will scan).
    CacheResult {
        path: PathBuf,
        loaded: Option<(Arc<SpaceTree>, bool /*stale*/)>,
    },
    CacheSaved(Vec<CacheMeta>),
    RescanDone(Result<Arc<SpaceTree>, String>),
    DeleteDone(Result<(Arc<SpaceTree>, usize), String>),
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
    bg_rx: Receiver<BgMsg>,
    bg_tx: crossbeam_channel::Sender<BgMsg>,
    /// True while cache load / rescan / delete runs off-thread.
    busy: bool,
    largest_key: Option<(NodeId, SizeMode)>,
    largest_rows: Vec<(NodeId, String, u64)>,
    largest_loading: bool,
    /// Cached sidebar child rows for current focus/filter/mode.
    side_rows: Vec<(NodeId, u64, bool)>,
    side_key: Option<(NodeId, SizeMode, u64 /*filter hash*/)>,
    /// Cached view geometry (sunburst mesh / treemap tiles).
    geom: Option<GeomCache>,
}

impl SpaceApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);

        // Open cache dir only (cheap). List recent + full volume stats off UI thread.
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

        Self {
            tree: None,
            focus: NodeId(0),
            selected: None,
            view: ViewMode::Sunburst,
            size_mode: SizeMode::OnDisk,
            scan: None,
            progress: None,
            expected_items: None,
            status: "Open a folder or pick a volume.".into(),
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
            bg_rx,
            bg_tx,
            busy: false,
            largest_key: None,
            largest_rows: Vec::new(),
            largest_loading: false,
            side_rows: Vec::new(),
            side_key: None,
            geom: None,
        }
    }

    fn ensure_side_rows(&mut self) {
        let Some(tree) = self.tree.clone() else {
            self.side_rows.clear();
            self.side_key = None;
            return;
        };
        let fhash = self.filter.hash64();
        let key = (self.focus, self.size_mode, fhash);
        if self.side_key == Some(key) {
            return;
        }
        let mut geom = None;
        let kids = views::child_ids(
            &DrawCtx {
                tree: &tree,
                focus: self.focus,
                selected: self.selected,
                review: &self.review,
                mode: self.size_mode,
                filter: &self.filter,
                filter_hash: fhash,
                review_sig: 0,
                geom: &mut geom,
            },
            self.focus,
        );
        const MAX_ROWS: usize = 120;
        self.side_rows = kids
            .iter()
            .take(MAX_ROWS)
            .map(|&id| {
                let n = tree.node(id);
                (id, n.size(self.size_mode), n.is_dir)
            })
            .collect();
        self.side_key = Some(key);
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
        if self.volumes_loading || self.busy || self.scan.is_some() || self.largest_loading {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        } else if got {
            ctx.request_repaint();
        }
    }

    fn invalidate_largest(&mut self) {
        self.largest_key = None;
        self.largest_rows.clear();
        self.largest_loading = false;
        self.side_key = None;
        self.side_rows.clear();
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
                let ids = tree.largest_files_bounded(focus, mode, 12, 20_000);
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
            if push_history && id != self.focus {
                self.history_back.push(self.focus);
                self.history_fwd.clear();
            }
            self.focus = id;
            self.selected = None;
            self.invalidate_largest();
            self.schedule_largest();
        }
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
}

impl eframe::App for SpaceApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_bg(ctx);
        self.poll_scan(ctx);

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
                                        egui::Label::new(RichText::new(name).color(TEXT))
                                            .sense(Sense::click()),
                                    )
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

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(8.0);
                        for mode in ViewMode::ALL.iter().rev() {
                            let selected = self.view == *mode;
                            if ui.selectable_label(selected, mode.label()).clicked() {
                                self.view = *mode;
                                self.geom = None;
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
            .exact_height(44.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(12.0);
                    if let Some(tree) = &self.tree {
                        let used = tree.total_size_mode(self.size_mode);
                        ui.label(format!(
                            "{} · {} items{}",
                            format_bytes(used),
                            tree.item_count,
                            if self.from_cache { " · cache" } else { "" }
                        ));
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
                    ui.text_edit_singleline(&mut self.filter.query);
                    if ui.small_button("×").clicked() {
                        self.filter = ViewFilter::default();
                    }
                });
                if let Some(ext) = &self.filter.ext {
                    ui.label(RichText::new(format!("ext: .{ext}")).color(ACCENT));
                }

                ui.separator();

                if self.tree.is_some() {
                    self.ensure_side_rows();
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
                    let child_count = self
                        .tree
                        .as_ref()
                        .map(|t| t.node(focus).children.len())
                        .unwrap_or(0);
                    let truncated = child_count > self.side_rows.len();
                    let row_count = self.side_rows.len();

                    ui.label(RichText::new(title).strong().size(16.0));
                    ui.label(
                        RichText::new(format!(
                            "{} · {} children{}",
                            format_bytes(size),
                            child_count,
                            if truncated {
                                " (showing first 120)"
                            } else {
                                ""
                            }
                        ))
                        .color(MUTED),
                    );
                    ui.add_space(4.0);

                    let mut select_to = None;
                    let mut nav_to = None;
                    let mut toggle_rev = None;
                    egui::ScrollArea::vertical()
                        .id_salt("kids")
                        .max_height(ui.available_height() * 0.45)
                        .show(ui, |ui| {
                            for i in 0..row_count {
                                let (id, child_size, is_dir) = self.side_rows[i];
                                let name = self
                                    .tree
                                    .as_ref()
                                    .map(|t| t.node(id).name.as_str())
                                    .unwrap_or("");
                                let selected = self.selected == Some(id);
                                let in_q = self.review.contains(&id);
                                ui.horizontal(|ui| {
                                    let mark = if is_dir { "▸" } else { "·" };
                                    let prefix = if in_q { "✓ " } else { "" };
                                    let text = format!("{prefix}{mark} {name}");
                                    let resp = ui.selectable_label(selected, text);
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.label(
                                            RichText::new(format_bytes(child_size))
                                                .color(MUTED)
                                                .monospace(),
                                        );
                                    });
                                    if resp.clicked() {
                                        select_to = Some(id);
                                    }
                                    if resp.double_clicked() && is_dir {
                                        nav_to = Some(id);
                                    }
                                    if resp.secondary_clicked() {
                                        toggle_rev = Some(id);
                                    }
                                });
                            }
                        });
                    if let Some(id) = select_to {
                        self.selected = Some(id);
                    }
                    if let Some(id) = nav_to {
                        self.navigate(id, true);
                    }
                    if let Some(id) = toggle_rev {
                        review::toggle(&mut self.review, id);
                    }

                    ui.separator();
                    ui.label(RichText::new("Largest files").strong());
                    if self.largest_loading {
                        ui.spinner();
                    }
                    let largest_n = self.largest_rows.len();
                    egui::ScrollArea::vertical()
                        .id_salt("large")
                        .show(ui, |ui| {
                            for i in 0..largest_n {
                                let (id, ref name, sz) = self.largest_rows[i];
                                if ui
                                    .button(format!("{}  {}", name, format_bytes(sz)))
                                    .clicked()
                                {
                                    self.selected = Some(id);
                                }
                            }
                        });
                    if self.largest_key != Some((self.focus, self.size_mode)) && !self.largest_loading
                    {
                        self.schedule_largest();
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
                                "Sunburst · Container · Radial · Cascade\nIndex once, browse from cache",
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
            let action = views::draw(
                ui,
                self.view,
                &mut DrawCtx {
                    tree: &tree,
                    focus: self.focus,
                    selected: self.selected,
                    review: &self.review,
                    mode: self.size_mode,
                    filter: &self.filter,
                    filter_hash: fhash,
                    review_sig,
                    geom: &mut self.geom,
                },
            );
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
        });
    }
}
