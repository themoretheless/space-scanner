use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::{unbounded, Receiver};
use eframe::egui::{self, Key};
use space_core::{CacheMeta, CacheStore, NodeId, ScanHandle, ScanProgress, SizeMode, SpaceTree};

use crate::anim::Transition;
use crate::downloads::{DownloadItem, DownloadsState};
use crate::junk_panel::JunkPanelState;
use crate::largest_files::{InsightData, InsightTab, LargestFiles, LargestRequest};
use crate::review;
use crate::rules_panel::RulesPanelState;
use crate::settings::AppSettings;
use crate::theme;
use crate::tm_clean::{self, CleanReport, LocalSnapshot};
use crate::views::{GeomCache, ViewFilter, ViewMode};
use crate::volumes::{self, VolumeInfo};

mod actions;
mod caches;
mod ui;
mod workers;

#[derive(Clone, Debug)]
pub(crate) struct BasketItem {
    pub(crate) path: PathBuf,
    pub(crate) bytes: u64,
    pub(crate) is_dir: bool,
    /// `(rule_root, min_age_days)` when the item comes from a Quick clean finding,
    /// used to re-validate rule constraints right before permanent deletion.
    pub(crate) junk_guard: Option<(Option<PathBuf>, u64)>,
}

#[derive(Clone, Debug, Default)]
pub(crate) enum DeleteSource {
    #[default]
    Basket,
    Direct(Vec<BasketItem>),
}

pub(crate) fn basket_total_bytes(items: &[BasketItem]) -> u64 {
    items
        .iter()
        .fold(0_u64, |sum, item| sum.saturating_add(item.bytes))
}

struct TrashOutcome {
    deleted_paths: Vec<PathBuf>,
    processed_bytes: u64,
    freed_bytes: u64,
    method: space_clean::DeleteMethod,
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
    delete_method: space_clean::DeleteMethod,
    delete_source: DeleteSource,
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
    caches: caches::AppCaches,
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
            let _ = workers::spawn_bg("space-vols", move || {
                let _ = tx.send(BgMsg::Volumes(volumes::list_volumes()));
            });
        }
        {
            let tx = bg_tx.clone();
            let _ = workers::spawn_bg("space-snaps", move || {
                let _ = tx.send(BgMsg::Snapshots(tm_clean::list_local_snapshots()));
            });
        }
        if let Some(cache) = cache.clone() {
            let tx = bg_tx.clone();
            let ttl_days = settings.cache_ttl_days;
            let _ = workers::spawn_bg("space-recent", move || {
                if ttl_days > 0 {
                    let _ = cache.prune_ttl(ttl_days);
                }
                let recent = cache.list_recent().unwrap_or_default();
                let _ = tx.send(BgMsg::Recent(recent));
            });
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
            delete_method: space_clean::DeleteMethod::Trash,
            delete_source: DeleteSource::Basket,
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
            caches: caches::AppCaches::default(),
        };
        if app.mode == Mode::Clean {
            app.start_quick_clean_scan();
        }
        app
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
                if mods.alt {
                    let direct_ids = if in_explore {
                        self.selected_deletable_nodes()
                    } else {
                        Vec::new()
                    };
                    if !direct_ids.is_empty() {
                        self.request_delete_nodes(
                            &direct_ids,
                            space_clean::DeleteMethod::Permanent,
                        );
                    } else if self.basket_len() > 0 {
                        self.request_permanent_basket();
                    }
                } else {
                    let selected_deletable = if in_explore {
                        self.selected_deletable_nodes().into_iter().next()
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
                self.delete_source = DeleteSource::Basket;
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
            if self.confirm_delete
                && self.delete_progress.is_none()
                && !self.active_delete_items().is_empty()
            {
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
                delete_method: space_clean::DeleteMethod::Trash,
                delete_source: DeleteSource::Basket,
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
                caches: caches::AppCaches::default(),
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

        // Multi-selection + confirm delete modal (Trash and Permanent).
        app.selected_set = vec![NodeId(1), NodeId(3)];
        step_ui(&ctx, &mut app, 1280.0, 800.0);
        app.confirm_delete = true;
        step_ui(&ctx, &mut app, 1280.0, 800.0);
        app.delete_method = space_clean::DeleteMethod::Permanent;
        step_ui(&ctx, &mut app, 1280.0, 800.0);
        app.delete_method = space_clean::DeleteMethod::Trash;
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
