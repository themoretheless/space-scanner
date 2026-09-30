//! Smooth hierarchical tree for the left sidebar.

use std::collections::HashSet;
use std::sync::{Arc, Weak};

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Ui};
use space_core::{format_bytes, NodeId, SizeMode, SpaceTree};

use crate::theme::{ACCENT, MUTED, TEXT};
use crate::views::{self, ViewFilter};

const MAX_VISIBLE: usize = 220;
const MAX_CHILDREN: usize = 48;
const MAX_DEPTH: usize = 6;

/// Derived rows belong to an immutable tree snapshot and a specific filter.
#[derive(Default)]
pub struct FolderFiles {
    tree: Weak<SpaceTree>,
    key: Option<(NodeId, SizeMode, u64, usize)>,
    rows: Vec<(NodeId, u64)>,
}

impl FolderFiles {
    pub fn get(
        &mut self,
        tree: &Arc<SpaceTree>,
        dir: NodeId,
        mode: SizeMode,
        filter: &ViewFilter,
        limit: usize,
    ) -> &[(NodeId, u64)] {
        let key = (dir, mode, filter.hash64(), limit);
        let snapshot = Arc::downgrade(tree);
        if self.key != Some(key) || !self.tree.ptr_eq(&snapshot) {
            self.rows = folder_files(tree, dir, mode, filter, limit);
            self.tree = snapshot;
            self.key = Some(key);
        }
        &self.rows
    }
}

#[derive(Clone, Copy)]
pub struct TreeRow {
    pub id: NodeId,
    pub depth: usize,
    pub size: u64,
    pub is_dir: bool,
    pub has_kids: bool,
}

#[derive(Default)]
pub struct SideTree {
    pub expanded: HashSet<NodeId>,
    rows: Vec<TreeRow>,
    key: Option<(NodeId, SizeMode, u64)>,
}

impl SideTree {
    pub fn clear(&mut self) {
        self.expanded.clear();
        self.rows.clear();
        self.key = None;
    }

    /// While hovering a ring: show peer folders on that circle (open their parent),
    /// and collapse any other expanded branches. When hover ends, keep only the focus path.
    pub fn sync_for_hover(&mut self, tree: &SpaceTree, focus: NodeId, hover: Option<NodeId>) {
        let mut keep = HashSet::new();
        let mut cur = Some(focus);
        while let Some(id) = cur {
            keep.insert(id);
            cur = tree.node(id).parent;
        }
        keep.insert(tree.root);

        if let Some(hover) = hover {
            // Parent of the hovered wedge — open so the whole circle is listed.
            // Do not expand the hover itself (next ring stays collapsed).
            if let Some(parent) = tree.node(hover).parent {
                keep.insert(parent);
                let mut cur = tree.node(parent).parent;
                while let Some(id) = cur {
                    keep.insert(id);
                    cur = tree.node(id).parent;
                }
            }
        }

        if self.expanded != keep {
            self.expanded = keep;
            self.invalidate();
        }
    }

    pub fn rebuild_if_needed(
        &mut self,
        tree: &SpaceTree,
        root: NodeId,
        mode: SizeMode,
        filter: &ViewFilter,
        filter_hash: u64,
    ) {
        let key = (root, mode, filter_hash);
        if self.key == Some(key) && !self.rows.is_empty() {
            return;
        }
        self.expanded.insert(root);
        self.rows.clear();
        flatten(tree, root, 0, mode, filter, &self.expanded, &mut self.rows);
        self.key = Some(key);
    }

    /// Force rebuild after expand/collapse.
    pub fn invalidate(&mut self) {
        self.key = None;
    }

    pub fn rows(&self) -> &[TreeRow] {
        &self.rows
    }
}

fn flatten(
    tree: &SpaceTree,
    id: NodeId,
    depth: usize,
    mode: SizeMode,
    filter: &ViewFilter,
    expanded: &HashSet<NodeId>,
    out: &mut Vec<TreeRow>,
) {
    if out.len() >= MAX_VISIBLE || depth > MAX_DEPTH {
        return;
    }
    let node = tree.node(id);
    // Root of panel always shown; deeper nodes must match filter (dirs kept for navigation).
    if depth > 0 && !node.is_dir && !filter.matches(&node.name, false, node.size(mode)) {
        return;
    }

    let kids: Vec<NodeId> = if node.is_dir {
        let dirs: Vec<NodeId> = tree
            .children(id)
            .iter()
            .copied()
            .filter(|&cid| {
                let c = tree.node(cid);
                if !c.is_dir {
                    return false; // files live in the bottom panel
                }
                if !filter.is_active() {
                    return true;
                }
                filter.matches(&c.name, true, c.size(mode))
                    || dir_has_match(tree, cid, mode, filter, 0)
            })
            .take(MAX_CHILDREN)
            .collect();
        // Already size-sorted from scan; keep order.
        dirs
    } else {
        Vec::new()
    };

    // Tree panel: directories only (skip file leaves).
    if depth > 0 && !node.is_dir {
        return;
    }

    out.push(TreeRow {
        id,
        depth,
        size: node.size(mode),
        is_dir: node.is_dir,
        has_kids: !kids.is_empty(),
    });

    if node.is_dir && expanded.contains(&id) {
        for cid in kids {
            if out.len() >= MAX_VISIBLE {
                break;
            }
            flatten(tree, cid, depth + 1, mode, filter, expanded, out);
        }
    }
}

/// Files directly under `dir` (not recursive), filter-aware, size-desc.
pub fn folder_files(
    tree: &SpaceTree,
    dir: NodeId,
    mode: SizeMode,
    filter: &ViewFilter,
    limit: usize,
) -> Vec<(NodeId, u64)> {
    let mut files: Vec<(NodeId, u64)> = tree
        .children(dir)
        .iter()
        .copied()
        .filter_map(|cid| {
            let n = tree.node(cid);
            if n.is_dir {
                return None;
            }
            let sz = n.size(mode);
            if filter.is_active() && !filter.matches(&n.name, false, sz) {
                return None;
            }
            Some((cid, sz))
        })
        .collect();
    files.sort_by_key(|row| std::cmp::Reverse(row.1));
    files.truncate(limit);
    files
}

fn dir_has_match(
    tree: &SpaceTree,
    id: NodeId,
    mode: SizeMode,
    filter: &ViewFilter,
    depth: usize,
) -> bool {
    if depth > 4 {
        return false;
    }
    for &cid in tree.children(id).iter().take(64) {
        let c = tree.node(cid);
        if filter.matches(&c.name, c.is_dir, c.size(mode)) {
            return true;
        }
        if c.is_dir && dir_has_match(tree, cid, mode, filter, depth + 1) {
            return true;
        }
    }
    false
}

pub struct TreeAction {
    pub select: Option<NodeId>,
    pub navigate: Option<NodeId>,
    pub toggle_review: Option<NodeId>,
    pub reveal: Option<NodeId>,
    pub toggled_expand: bool,
}

impl TreeAction {
    fn none() -> Self {
        Self {
            select: None,
            navigate: None,
            toggle_review: None,
            reveal: None,
            toggled_expand: false,
        }
    }
}

/// Draw tree rows (parent provides ScrollArea for smooth scrolling).
#[allow(clippy::too_many_arguments)]
pub fn draw_rows(
    ui: &mut Ui,
    tree: &SpaceTree,
    side: &mut SideTree,
    selected: Option<NodeId>,
    hovered: Option<NodeId>,
    review: &[NodeId],
    focus: NodeId,
    show_pct: bool,
    mode: SizeMode,
) -> TreeAction {
    let mut action = TreeAction::none();

    let old_spacing = ui.spacing().item_spacing;
    ui.spacing_mut().item_spacing = egui::vec2(4.0, 1.0);

    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.label(RichText::new("Trash").small().color(MUTED));
        ui.label(RichText::new("Name").small().color(MUTED));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if show_pct {
                ui.label(RichText::new("%").small().color(MUTED).monospace());
            }
            ui.label(RichText::new("Size").small().color(MUTED));
        });
    });

    for index in 0..side.rows().len() {
        // Expanding a row invalidates the next rebuild; this frame's rows stay fixed.
        let row = side.rows()[index];
        let node = tree.node(row.id);
        let is_sel = selected == Some(row.id) || row.id == focus;
        let in_q = review.contains(&row.id);
        let active = match hovered {
            Some(h) => views::hover_related(tree, h, focus, row.id),
            None => true,
        };
        let is_hov = active && hovered.is_some();
        let is_exact = hovered == Some(row.id);
        let live = ui
            .ctx()
            .animate_bool_with_time(ui.id().with(("dim", row.id.0)), active, 0.28);
        let fade = |c: Color32| {
            Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (26.0 + 229.0 * live) as u8)
        };

        let indent = 2.0 + row.depth as f32 * 10.0;
        ui.horizontal(|ui| {
            ui.set_height(22.0);
            ui.add_space(indent);
            let mut queued = in_q;
            if ui
                .add_enabled(!node.synthetic, egui::Checkbox::without_text(&mut queued))
                .on_hover_text("Add to Trash queue")
                .changed()
            {
                action.toggle_review = Some(row.id);
            }

            if row.is_dir && row.has_kids {
                let open = side.expanded.contains(&row.id);
                let t =
                    ui.ctx()
                        .animate_bool_with_time(ui.id().with(("chev", row.id.0)), open, 0.18);
                // Rotate ▸ toward ▾ via opacity blend of two glyphs.
                let chev_resp = ui
                    .allocate_ui(egui::vec2(12.0, 14.0), |ui| {
                        let (rect, resp) =
                            ui.allocate_exact_size(egui::vec2(12.0, 14.0), Sense::click());
                        let painter = ui.painter();
                        let closed_a = ((1.0 - t) * 255.0) as u8;
                        let open_a = (t * 255.0) as u8;
                        if closed_a > 8 {
                            painter.text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "▸",
                                egui::FontId::monospace(11.0),
                                Color32::from_rgba_unmultiplied(
                                    MUTED.r(),
                                    MUTED.g(),
                                    MUTED.b(),
                                    ((closed_a as f32) * (26.0 + 229.0 * live) / 255.0) as u8,
                                ),
                            );
                        }
                        if open_a > 8 {
                            painter.text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "▾",
                                egui::FontId::monospace(11.0),
                                Color32::from_rgba_unmultiplied(
                                    MUTED.r(),
                                    MUTED.g(),
                                    MUTED.b(),
                                    ((open_a as f32) * (26.0 + 229.0 * live) / 255.0) as u8,
                                ),
                            );
                        }
                        resp
                    })
                    .inner;
                if chev_resp.clicked() {
                    if open {
                        side.expanded.remove(&row.id);
                    } else {
                        side.expanded.insert(row.id);
                    }
                    side.invalidate();
                    action.toggled_expand = true;
                }
            } else {
                ui.label(
                    RichText::new(if row.is_dir { " " } else { "·" })
                        .color(fade(MUTED))
                        .size(11.0),
                );
            }

            let name = node.name.clone();
            let name_color = if is_hov { fade(ACCENT) } else { fade(TEXT) };
            let resp = ui.selectable_label(
                is_sel || is_hov,
                RichText::new(name).color(name_color).size(12.5),
            );
            resp.clone().on_hover_text(if row.is_dir {
                "Click to open on the map · ⌘-click Trash queue"
            } else {
                "Click to select · ⌘-click Trash queue"
            });

            resp.context_menu(|ui| {
                if row.is_dir && ui.button("Open on map").clicked() {
                    action.navigate = Some(row.id);
                    ui.close_menu();
                }
                if ui.button("Select").clicked() {
                    action.select = Some(row.id);
                    ui.close_menu();
                }
                if !node.synthetic {
                    let label = if in_q {
                        "Remove from Trash queue"
                    } else {
                        "Add to Trash queue"
                    };
                    if ui.button(label).clicked() {
                        action.toggle_review = Some(row.id);
                        ui.close_menu();
                    }
                }
                if ui.button("Show in Finder").clicked() {
                    action.reveal = Some(row.id);
                    ui.close_menu();
                }
            });

            if is_hov && !is_sel {
                let r = resp.rect;
                ui.painter().hline(
                    r.x_range(),
                    r.bottom() - 1.0,
                    egui::Stroke::new(if is_exact { 1.4_f32 } else { 1.0_f32 }, ACCENT),
                );
            }
            if is_exact
                || (is_hov
                    && hovered
                        .is_some_and(|h| tree.try_node(h).and_then(|n| n.parent) == Some(row.id)))
            {
                resp.scroll_to_me(Some(Align::Center));
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if show_pct {
                    if let Some(parent) = node.parent {
                        let ps = tree.node(parent).size(mode).max(1) as f32;
                        let pct = row.size as f32 / ps * 100.0;
                        ui.label(
                            RichText::new(format!("{pct:.0}%"))
                                .size(10.0)
                                .color(fade(MUTED))
                                .monospace(),
                        );
                    }
                }
                ui.label(
                    RichText::new(format_bytes(row.size))
                        .color(fade(if is_hov || is_sel { ACCENT } else { MUTED }))
                        .monospace()
                        .size(10.0),
                );
            });

            if resp.clicked() {
                if ui.input(|i| i.modifiers.command || i.modifiers.ctrl) && !node.synthetic {
                    action.toggle_review = Some(row.id);
                    action.select = Some(row.id);
                } else if row.is_dir {
                    action.navigate = Some(row.id);
                } else {
                    action.select = Some(row.id);
                }
            }
            if resp.double_clicked() && row.is_dir {
                action.navigate = Some(row.id);
            }
        });
    }

    ui.spacing_mut().item_spacing = old_spacing;
    action
}

/// Convenience: own scroll area (unused when parent scrolls).
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
pub fn draw(
    ui: &mut Ui,
    tree: &SpaceTree,
    side: &mut SideTree,
    selected: Option<NodeId>,
    hovered: Option<NodeId>,
    review: &[NodeId],
    focus: NodeId,
    show_pct: bool,
    mode: SizeMode,
) -> TreeAction {
    egui::ScrollArea::vertical()
        .id_salt("side_tree")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            draw_rows(
                ui, tree, side, selected, hovered, review, focus, show_pct, mode,
            )
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use space_core::Node;

    fn fixture(count: usize) -> Arc<SpaceTree> {
        let mut nodes = vec![Node {
            name: "root".into(),
            apparent_size: 0,
            allocated_size: 0,
            file_count: count as u64,
            is_dir: true,
            synthetic: false,
            children: (1..=count).map(|i| NodeId(i as u32)).collect(),
            parent: None,
        }];
        for i in 1..=count {
            nodes.push(Node {
                name: format!("file-{i}.log"),
                apparent_size: (i * 7919 % (count + 1)) as u64,
                allocated_size: (count - i) as u64,
                file_count: 1,
                is_dir: false,
                synthetic: false,
                children: vec![],
                parent: Some(NodeId(0)),
            });
        }
        Arc::new(SpaceTree {
            root_path: "/fixture".into(),
            nodes,
            root: NodeId(0),
            scanned_at_unix: 0,
            item_count: count as u64,
            skipped_errors: 0,
            hardlink_candidates: 0,
            hardlinks_deduped: 0,
        })
    }

    #[test]
    fn hover_sync_keeps_focus_ancestors_and_rebuilds_visible_peers() {
        let mut tree = fixture(4);
        let nodes = &mut Arc::make_mut(&mut tree).nodes;
        nodes[0].children = vec![NodeId(1), NodeId(3), NodeId(4)];
        nodes[1].is_dir = true;
        nodes[1].children = vec![NodeId(2)];
        nodes[2].is_dir = true;
        nodes[2].parent = Some(NodeId(1));
        nodes[3].is_dir = true;
        let mut side = SideTree::default();
        let filter = ViewFilter::default();

        side.sync_for_hover(&tree, tree.root, Some(NodeId(2)));
        side.rebuild_if_needed(&tree, tree.root, SizeMode::OnDisk, &filter, filter.hash64());
        assert_eq!(
            side.rows().iter().map(|row| row.id).collect::<Vec<_>>(),
            [NodeId(0), NodeId(1), NodeId(2), NodeId(3)],
        );
        let rows = side.rows().as_ptr();
        side.sync_for_hover(&tree, tree.root, Some(NodeId(2)));
        side.rebuild_if_needed(&tree, tree.root, SizeMode::OnDisk, &filter, filter.hash64());
        assert_eq!(side.rows().as_ptr(), rows);

        side.sync_for_hover(&tree, tree.root, None);
        side.rebuild_if_needed(&tree, tree.root, SizeMode::OnDisk, &filter, filter.hash64());
        assert_eq!(
            side.rows().iter().map(|row| row.id).collect::<Vec<_>>(),
            [NodeId(0), NodeId(1), NodeId(3)],
        );
        side.sync_for_hover(&tree, NodeId(2), Some(NodeId(3)));
        assert_eq!(
            side.expanded,
            HashSet::from([NodeId(0), NodeId(1), NodeId(2)])
        );
    }

    #[test]
    fn cached_files_follow_filter_mode_limit_and_directory() {
        let mut tree = fixture(20);
        let empty = Node {
            name: "empty".into(),
            is_dir: true,
            children: vec![],
            ..tree.nodes[0].clone()
        };
        Arc::make_mut(&mut tree).nodes.push(empty);
        let mut cache = FolderFiles::default();
        for mode in [SizeMode::Apparent, SizeMode::OnDisk] {
            for limit in [0, 1, 10, 100] {
                for query in ["", "file-1", "missing"] {
                    let filter = ViewFilter {
                        query: query.into(),
                        ..Default::default()
                    };
                    for dir in [NodeId(0), NodeId(21)] {
                        let expected = folder_files(&tree, dir, mode, &filter, limit);
                        assert_eq!(cache.get(&tree, dir, mode, &filter, limit), expected);
                        let previous = cache.rows.as_ptr();
                        assert_eq!(cache.get(&tree, dir, mode, &filter, limit), expected);
                        assert_eq!(cache.rows.as_ptr(), previous);
                    }
                }
            }
        }
    }

    #[test]
    fn cached_files_follow_tree_replacement_and_in_place_edits() {
        let mut tree = fixture(10);
        let mut cache = FolderFiles::default();
        let filter = ViewFilter::default();
        let original = cache
            .get(&tree, tree.root, SizeMode::Apparent, &filter, 100)
            .to_vec();
        // Arc::make_mut detaches the cached Weak even when there is no other strong owner.
        Arc::make_mut(&mut tree).nodes[1].apparent_size = 999;
        let edited = cache.get(&tree, tree.root, SizeMode::Apparent, &filter, 100);
        assert_eq!(edited[0], (NodeId(1), 999));
        assert_ne!(edited, original);
        tree = fixture(10);
        assert_eq!(
            cache.get(&tree, tree.root, SizeMode::Apparent, &filter, 100),
            original
        );
    }

    #[test]
    #[ignore = "manual release profile: --release folder_files_profile -- --ignored --nocapture"]
    fn folder_files_profile() {
        use std::hint::black_box;
        use std::time::Instant;
        let tree = fixture(50_000);
        let filter = ViewFilter::default();
        let mut cache = FolderFiles::default();
        let expected = folder_files(&tree, tree.root, SizeMode::Apparent, &filter, 100);
        assert_eq!(
            cache.get(&tree, tree.root, SizeMode::Apparent, &filter, 100),
            expected
        );
        let mut uncached = Vec::new();
        let mut cached = Vec::new();
        for _ in 0..31 {
            let start = Instant::now();
            black_box(folder_files(
                black_box(&tree),
                tree.root,
                SizeMode::Apparent,
                &filter,
                100,
            ));
            uncached.push(start.elapsed());
            let start = Instant::now();
            for _ in 0..1_000 {
                black_box(cache.get(
                    black_box(&tree),
                    tree.root,
                    SizeMode::Apparent,
                    &filter,
                    100,
                ));
            }
            cached.push(start.elapsed() / 1_000);
        }
        uncached.sort();
        cached.sort();
        println!(
            "50,000 files, top 100: uncached {:?}, unchanged cached {:?}",
            uncached[15], cached[15]
        );
    }
}
