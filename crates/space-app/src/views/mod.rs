mod cascade;
mod radial;
mod sunburst;
mod treemap;

use std::borrow::Cow;
use std::hash::{Hash, Hasher};

use std::sync::Arc;

use egui::epaint::Mesh;
use egui::Pos2;
use egui::Ui;
use space_core::{NodeId, SizeMode, SpaceTree};

use sunburst::ArcHit;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Radial,
    Container,
    Sunburst,
    Cascade,
}

impl ViewMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Radial => "Radial",
            Self::Container => "Container",
            Self::Sunburst => "Sunburst",
            Self::Cascade => "Cascade",
        }
    }

    pub const ALL: [ViewMode; 4] = [
        Self::Sunburst,
        Self::Container,
        Self::Radial,
        Self::Cascade,
    ];
}

#[derive(Debug, Clone, Default)]
pub struct ViewFilter {
    pub query: String,
    pub ext: Option<String>,
    pub min_size: u64,
}

impl ViewFilter {
    pub fn matches(&self, name: &str, is_dir: bool, size: u64) -> bool {
        if size < self.min_size {
            return false;
        }
        if let Some(ext) = &self.ext {
            if is_dir {
                return true;
            }
            let e = crate::theme::extension_of(name);
            if !e.eq_ignore_ascii_case(ext) {
                return false;
            }
        }
        if !self.query.is_empty() {
            let q = self.query.to_lowercase();
            if !name.to_lowercase().contains(&q) {
                return false;
            }
        }
        true
    }

    pub fn is_active(&self) -> bool {
        !self.query.is_empty() || self.ext.is_some() || self.min_size > 0
    }

    pub fn hash64(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.query.hash(&mut h);
        self.ext.hash(&mut h);
        self.min_size.hash(&mut h);
        h.finish()
    }
}

pub struct ViewAction {
    pub navigate: Option<NodeId>,
    pub select: Option<NodeId>,
    pub toggle_review: Option<NodeId>,
    pub filter_ext: Option<String>,
}

impl ViewAction {
    pub fn none() -> Self {
        Self {
            navigate: None,
            select: None,
            toggle_review: None,
            filter_ext: None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct GeomKey {
    pub view: u8,
    pub focus: NodeId,
    pub mode: SizeMode,
    pub filter: u64,
    pub selected: Option<NodeId>,
    pub w: u16,
    pub h: u16,
    pub review_sig: u64,
}

pub enum GeomCache {
    Sunburst {
        key: GeomKey,
        mesh: Arc<Mesh>,
        hits: Vec<ArcHit>,
        center: Pos2,
        hole: f32,
        max_r: f32,
    },
    Treemap {
        key: GeomKey,
        tiles: Vec<(NodeId, egui::Rect, usize)>,
    },
}

pub struct DrawCtx<'a> {
    pub tree: &'a SpaceTree,
    pub focus: NodeId,
    pub selected: Option<NodeId>,
    pub review: &'a [NodeId],
    pub mode: SizeMode,
    pub filter: &'a ViewFilter,
    pub filter_hash: u64,
    #[allow(dead_code)]
    pub review_sig: u64,
    pub geom: &'a mut Option<GeomCache>,
}

pub fn draw(ui: &mut Ui, view: ViewMode, ctx: &mut DrawCtx<'_>) -> ViewAction {
    match view {
        ViewMode::Radial => radial::draw(ui, ctx),
        ViewMode::Container => treemap::draw(ui, ctx),
        ViewMode::Sunburst => sunburst::draw(ui, ctx),
        ViewMode::Cascade => cascade::draw(ui, ctx),
    }
}

/// Prefer borrowed children when filter is idle (no alloc).
pub fn child_ids<'a>(ctx: &DrawCtx<'a>, id: NodeId) -> Cow<'a, [NodeId]> {
    if !ctx.filter.is_active() {
        return Cow::Borrowed(ctx.tree.children(id));
    }
    Cow::Owned(
        ctx.tree
            .children(id)
            .iter()
            .copied()
            .filter(|&cid| {
                let n = ctx.tree.node(cid);
                ctx.filter.matches(&n.name, n.is_dir, n.size(ctx.mode))
            })
            .collect(),
    )
}

pub fn review_sig(review: &[NodeId]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    review.len().hash(&mut h);
    if let Some(first) = review.first() {
        first.0.hash(&mut h);
    }
    if let Some(last) = review.last() {
        last.0.hash(&mut h);
    }
    h.finish()
}
