//! Smooth hierarchical tree for the left sidebar.

use std::collections::HashSet;

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Ui};
use space_core::{format_bytes, NodeId, SizeMode, SpaceTree};

use crate::theme::{ACCENT, MUTED, TEXT};
use crate::views::{self, ViewFilter};

const MAX_VISIBLE: usize = 220;
const MAX_CHILDREN: usize = 48;
const MAX_DEPTH: usize = 6;

#[derive(Clone, Copy)]
pub struct TreeRow {
    pub id: NodeId,
    pub depth: usize,
    pub size: u64,
    pub is_dir: bool,
    pub has_kids: bool,
}

pub struct SideTree {
    pub expanded: HashSet<NodeId>,
    rows: Vec<TreeRow>,
    key: Option<(NodeId, SizeMode, u64)>,
}

impl Default for SideTree {
    fn default() -> Self {
        Self {
            expanded: HashSet::new(),
            rows: Vec::new(),
            key: None,
        }
    }
}

impl SideTree {
    pub fn clear(&mut self) {
        self.expanded.clear();
        self.rows.clear();
        self.key = None;
    }

    pub fn ensure_path_expanded(&mut self, tree: &SpaceTree, target: NodeId) {
        let mut changed = false;
        let mut cur = Some(target);
        while let Some(id) = cur {
            if self.expanded.insert(id) {
                changed = true;
            }
            cur = tree.node(id).parent;
        }
        if changed {
            self.invalidate();
        }
    }

    /// While hovering a ring: show peer folders on that circle (open their parent),
    /// and collapse any other expanded branches. When hover ends, keep only the focus path.
    pub fn sync_for_hover(
        &mut self,
        tree: &SpaceTree,
        focus: NodeId,
        hover: Option<NodeId>,
    ) {
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

        let before = self.expanded.clone();
        self.expanded.retain(|id| keep.contains(id));
        for id in &keep {
            self.expanded.insert(*id);
        }
        if self.expanded != before {
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
        flatten(
            tree,
            root,
            0,
            mode,
            filter,
            &self.expanded,
            &mut self.rows,
        );
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
    if depth > 0 && !filter.matches(&node.name, node.is_dir, node.size(mode)) {
        if !(node.is_dir && filter.is_active()) {
            // When filtering, still show matching dirs; skip non-matching files.
            // For dirs with no matching name, peek children.
            if !node.is_dir {
                return;
            }
        }
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
    files.sort_by(|a, b| b.1.cmp(&a.1));
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
    pub toggled_expand: bool,
}

impl TreeAction {
    fn none() -> Self {
        Self {
            select: None,
            navigate: None,
            toggle_review: None,
            toggled_expand: false,
        }
    }
}

/// Draw tree rows (parent provides ScrollArea for smooth scrolling).
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
    let rows: Vec<TreeRow> = side.rows().to_vec();

    let old_spacing = ui.spacing().item_spacing;
    ui.spacing_mut().item_spacing = egui::vec2(4.0, 1.0);

    for row in &rows {
        let node = tree.node(row.id);
        let is_sel = selected == Some(row.id) || row.id == focus;
        let in_q = review.contains(&row.id);
        let active = match hovered {
            Some(h) => views::hover_related(tree, h, focus, row.id),
            None => true,
        };
        let is_hov = active && hovered.is_some();
        let is_exact = hovered == Some(row.id);
        let live = ui.ctx().animate_bool_with_time(
            ui.id().with(("dim", row.id.0)),
            active,
            0.22,
        );
        let fade = |c: Color32| {
            Color32::from_rgba_unmultiplied(
                c.r(),
                c.g(),
                c.b(),
                (26.0 + 229.0 * live) as u8,
            )
        };

        let indent = 2.0 + row.depth as f32 * 10.0;
        ui.horizontal(|ui| {
            ui.set_height(18.0);
            ui.add_space(indent);

            if row.is_dir && row.has_kids {
                let open = side.expanded.contains(&row.id);
                let t = ui
                    .ctx()
                    .animate_bool(ui.id().with(("chev", row.id.0)), open);
                let chev = if t > 0.5 { "▾" } else { "▸" };
                let chev_resp = ui.add(
                    egui::Label::new(
                        RichText::new(chev)
                            .color(fade(MUTED))
                            .monospace()
                            .size(11.0),
                    )
                    .sense(Sense::click()),
                );
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

            let mark = if in_q { "✓ " } else { "" };
            let name = format!("{mark}{}", node.name);
            let name_color = if is_hov {
                fade(ACCENT)
            } else {
                fade(TEXT)
            };
            let resp = ui.selectable_label(
                is_sel || is_hov,
                RichText::new(name).color(name_color).size(12.0),
            );

            if is_hov && !is_sel {
                let r = resp.rect;
                ui.painter().hline(
                    r.x_range(),
                    r.bottom() - 1.0,
                    egui::Stroke::new(if is_exact { 1.4_f32 } else { 1.0_f32 }, ACCENT),
                );
            }
            if is_exact {
                resp.scroll_to_me(Some(Align::Center));
            } else if is_hov && hovered.is_some_and(|h| {
                tree.try_node(h).and_then(|n| n.parent) == Some(row.id)
            }) {
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
                action.select = Some(row.id);
            }
            if resp.double_clicked() && row.is_dir {
                action.navigate = Some(row.id);
            }
            if resp.secondary_clicked() {
                action.toggle_review = Some(row.id);
            }
        });
    }

    ui.spacing_mut().item_spacing = old_spacing;
    action
}

/// Convenience: own scroll area (unused when parent scrolls).
#[allow(dead_code)]
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
