mod cascade;
mod radial;
mod sunburst;
mod treemap;

use crate::i18n::t;
use std::borrow::Cow;
use std::collections::{BTreeSet, HashSet};
use std::hash::{Hash, Hasher};

use std::sync::Arc;

use egui::epaint::Mesh;
use egui::{Color32, Pos2, Rect, RichText, Stroke, Ui};
use space_core::{NodeId, SizeMode, SpaceTree};

use crate::theme::MUTED;

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
            Self::Radial => "Circles",
            Self::Container => "Rectangles",
            Self::Sunburst => "Rings",
            Self::Cascade => "Bands",
        }
    }

    pub fn click_hint(self) -> &'static str {
        match self {
            Self::Sunburst => {
                "Click selects · double-click opens · ↑ parent · ⌘-click adds to the Trash list"
            }
            Self::Container | Self::Radial | Self::Cascade => {
                "Click selects · double-click opens · ↑ parent · ⌘-click adds to the Trash list"
            }
        }
    }

    pub const ALL: [ViewMode; 4] = [Self::Sunburst, Self::Container, Self::Radial, Self::Cascade];
}

/// Serialize view mode for settings.
impl serde::Serialize for ViewMode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match self {
            Self::Radial => "radial",
            Self::Container => "container",
            Self::Sunburst => "sunburst",
            Self::Cascade => "cascade",
        })
    }
}

impl<'de> serde::Deserialize<'de> for ViewMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = <String as serde::Deserialize>::deserialize(deserializer)?;
        Ok(match s.as_str() {
            "radial" => Self::Radial,
            "container" => Self::Container,
            "cascade" => Self::Cascade,
            _ => Self::Sunburst,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum EntryKindFilter {
    #[default]
    All,
    DirsOnly,
    FilesOnly,
}

#[derive(Debug, Clone, Default)]
pub struct ViewFilter {
    pub query: String,
    /// File extensions to show (lower-case, without the dot); empty = all.
    pub ext: BTreeSet<String>,
    pub min_size: u64,
    pub max_size: Option<u64>,
    pub kind: EntryKindFilter,
}

impl ViewFilter {
    pub fn matches(&self, name: &str, is_dir: bool, size: u64) -> bool {
        match self.kind {
            EntryKindFilter::DirsOnly if !is_dir => return false,
            EntryKindFilter::FilesOnly if is_dir => return false,
            _ => {}
        }
        if size < self.min_size {
            return false;
        }
        if let Some(max) = self.max_size {
            if size > max {
                return false;
            }
        }
        if !self.ext.is_empty() {
            if is_dir {
                return true;
            }
            let e = crate::theme::extension_of(name);
            if !self.has_ext(e) {
                return false;
            }
        }
        if !self.query.is_empty() {
            // Most filenames and queries are ASCII. Compare borrowed bytes rather
            // than allocating two lowercase strings for every candidate.
            if name.is_ascii() && self.query.is_ascii() {
                return name
                    .as_bytes()
                    .windows(self.query.len())
                    .any(|part| part.eq_ignore_ascii_case(self.query.as_bytes()));
            }
            let q = self.query.to_lowercase();
            if !name.to_lowercase().contains(&q) {
                return false;
            }
        }
        true
    }

    /// Is `ext` (any case) one of the selected extensions?
    pub fn has_ext(&self, ext: &str) -> bool {
        self.ext.iter().any(|e| e.eq_ignore_ascii_case(ext))
    }

    /// Add `ext` to the selected extensions, or remove it when present.
    pub fn toggle_ext(&mut self, ext: &str) {
        let key = ext.to_ascii_lowercase();
        if !self.ext.remove(&key) {
            self.ext.insert(key);
        }
    }

    pub fn is_active(&self) -> bool {
        !self.query.is_empty()
            || !self.ext.is_empty()
            || self.min_size > 0
            || self.max_size.is_some()
            || self.kind != EntryKindFilter::All
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn hash64(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.query.hash(&mut h);
        self.ext.hash(&mut h);
        self.min_size.hash(&mut h);
        self.max_size.hash(&mut h);
        (self.kind as u8).hash(&mut h);
        h.finish()
    }
}

pub struct ViewAction {
    pub navigate: Option<NodeId>,
    pub select: Option<NodeId>,
    pub hover: Option<NodeId>,
    pub toggle_review: Option<NodeId>,
    pub filter_ext: Option<String>,
    pub reveal: Option<NodeId>,
    pub clear_filter: bool,
}

/// Same hover card in every map view so gestures stay discoverable.
pub fn node_hover_ui(ui: &mut Ui, tree: &SpaceTree, id: NodeId, focus: NodeId, mode: SizeMode) {
    let n = tree.node(id);
    ui.label(RichText::new(&n.name).family(crate::theme::bold()));
    let kind = t(if n.is_dir { "Folder" } else { "File" });
    let mut line = format!("{kind} · {}", crate::i18n::bytes(n.size(mode)));
    if n.is_dir {
        line.push_str(" · ");
        line.push_str(&crate::i18n::count(n.file_count as usize, "file"));
    }
    if let Some(parent) = n.parent {
        let ps = tree.node(parent).size(mode).max(1) as f32;
        let pct = n.size(mode) as f32 / ps * 100.0;
        line.push_str(&crate::tr!(" · {}% of parent", format!("{pct:.0}")));
    }
    ui.label(line);
    let hint = if n.synthetic {
        "Grouped leftovers · click to select · right-click to filter by type"
    } else if n.is_dir && id == focus {
        "Click to select · ↑ to go up · ⌘-click adds to the Trash list"
    } else if n.is_dir {
        "Click to select · double-click to open · ⌘-click adds to the Trash list"
    } else {
        "Click to select · double-click shows it in Finder · ⌘-click adds to the Trash list"
    };
    ui.label(RichText::new(t(hint)).small().color(MUTED()));
}

pub fn click_map_node(tree: &SpaceTree, id: NodeId, cmd: bool, action: &mut ViewAction) {
    let n = tree.node(id);
    if cmd && !n.synthetic {
        action.toggle_review = Some(id);
    }
    action.select = Some(id);
}

/// Everything a map does with one node's response: click, double-click,
/// context menu, hover card and cursor. Shared by the tile/band/bubble views.
pub fn node_response(ui: &Ui, resp: &egui::Response, ctx: &DrawCtx<'_>, id: NodeId, action: &mut ViewAction) {
    if resp.clicked() {
        click_map_node(
            ctx.tree,
            id,
            ui.input(|i| i.modifiers.command || i.modifiers.ctrl),
            action,
        );
    }
    if resp.double_clicked() {
        double_click_map_node(ctx.tree, id, action);
    }
    node_context_menu(resp, ctx.tree, id, ctx.review.contains(&id), action);
    if resp.hovered() {
        action.hover = Some(id);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        resp.clone()
            .on_hover_ui(|ui| node_hover_ui(ui, ctx.tree, id, ctx.focus, ctx.mode));
    }
}

/// Animated hover/selection weights for a node, keyed by its id (stable
/// across re-layouts, unlike an index into the geometry list).
pub fn highlight(ui: &Ui, view: &'static str, id: Option<NodeId>, is_hov: bool, is_sel: bool) -> (f32, f32) {
    let key = id.map_or(u32::MAX, |id| id.0);
    let hov = ui
        .ctx()
        .animate_bool_with_time(ui.id().with((view, "hov", key)), is_hov, crate::anim::ease_time(0.14));
    let sel = ui
        .ctx()
        .animate_bool_with_time(ui.id().with((view, "sel", key)), is_sel, crate::anim::ease_time(0.18));
    (hov, sel)
}

/// Outline for a map item: accent that strengthens with selection and hover,
/// otherwise the quiet `idle` colour.
pub fn highlight_stroke(hov_t: f32, sel_t: f32, idle: Color32) -> Stroke {
    if sel_t > 0.05 || hov_t > 0.05 {
        let alpha = ((80.0 + sel_t * 160.0 + hov_t * 110.0) / 255.0).min(1.0);
        Stroke::new(
            1.0 + sel_t * 1.2 + hov_t * 0.8,
            crate::theme::ACCENT().gamma_multiply(alpha),
        )
    } else {
        Stroke::new(0.8_f32, idle)
    }
}

/// Shorten a name to `max` characters with an ellipsis (char-aware).
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}

/// Scale a point about `center` (zoom transition drawn from cached geometry).
pub fn zoomed(center: Pos2, p: Pos2, scale: f32) -> Pos2 {
    center + (p - center) * scale
}

pub fn zoomed_rect(center: Pos2, r: Rect, scale: f32) -> Rect {
    Rect::from_min_max(zoomed(center, r.min, scale), zoomed(center, r.max, scale))
}

pub fn double_click_map_node(tree: &SpaceTree, id: NodeId, action: &mut ViewAction) {
    if tree.node(id).synthetic {
        action.select = Some(id);
    } else if tree.node(id).is_dir {
        action.navigate = Some(id);
    } else {
        action.reveal = Some(id);
    }
}

pub fn node_context_menu(
    resp: &egui::Response,
    tree: &SpaceTree,
    id: NodeId,
    in_review: bool,
    action: &mut ViewAction,
) {
    resp.context_menu(|ui| {
        let n = tree.node(id);
        if n.is_dir && ui.button(t("Open on map")).clicked() {
            action.navigate = Some(id);
            ui.close_menu();
        }
        if ui.button(t("Select")).clicked() {
            action.select = Some(id);
            ui.close_menu();
        }
        if !n.synthetic {
            let label = if in_review {
                t("Take out of Trash list")
            } else {
                t("Add to Trash list")
            };
            if ui.button(label).clicked() {
                action.toggle_review = Some(id);
                ui.close_menu();
            }
        }
        if ui.button(t("Show in Finder")).clicked() {
            action.reveal = Some(id);
            ui.close_menu();
        }
    });
}

/// Maps with one hit surface must retain the clicked node while the pointer is
/// over the popup, where the surface no longer reports a hovered node.
pub fn hovered_node_context_menu(
    resp: &egui::Response,
    tree: &SpaceTree,
    hovered: Option<NodeId>,
    review: &HashSet<NodeId>,
    action: &mut ViewAction,
) {
    let target_key = resp.id.with("context_menu_target");
    if resp.secondary_clicked() {
        resp.ctx.data_mut(|data| {
            if let Some(id) = hovered {
                data.insert_temp(target_key, id);
            } else {
                data.remove::<NodeId>(target_key);
            }
        });
    } else if !resp.context_menu_opened() {
        resp.ctx.data_mut(|data| data.remove::<NodeId>(target_key));
        return;
    }
    let target = resp.ctx.data(|data| data.get_temp::<NodeId>(target_key));
    if let Some(id) = target.filter(|&id| tree.try_node(id).is_some()) {
        node_context_menu(resp, tree, id, review.contains(&id), action);
    }
}

impl ViewAction {
    pub fn none() -> Self {
        Self {
            navigate: None,
            select: None,
            hover: None,
            toggle_review: None,
            filter_ext: None,
            reveal: None,
            clear_filter: false,
        }
    }
}

/// Empty map while a filter is on: one click clears it instead of hunting the sidebar.
pub fn empty_filter_prompt(ui: &mut Ui, rect: egui::Rect, action: &mut ViewAction) {
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.centered_and_justified(|ui| {
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(t("Nothing matches this filter"))
                        .size(15.0)
                        .color(MUTED()),
                );
                ui.add_space(8.0);
                if ui.button(t("Clear filter")).clicked() {
                    action.clear_filter = true;
                }
            });
        });
    });
}

/// What the cached geometry was built for. Size and position are quantized
/// to 8 px so sub-pixel layout jitter does not rebuild the mesh every frame.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct GeomKey {
    pub view: u8,
    pub focus: NodeId,
    pub mode: SizeMode,
    pub filter: u64,
    pub depth: u8,
    pub nest: u8,
    pub cascade_depth: u8,
    /// Row scale × 100.
    pub row_scale: u16,
    pub x: i16,
    pub y: i16,
    pub w: u16,
    pub h: u16,
}

impl GeomKey {
    pub fn new(view: u8, ctx: &DrawCtx<'_>, rect: Rect) -> Self {
        let q = |v: f32| (v / 8.0).round() * 8.0;
        Self {
            view,
            focus: ctx.focus,
            mode: ctx.mode,
            filter: ctx.filter_hash,
            depth: ctx.sunburst_depth as u8,
            nest: ctx.treemap_nest as u8,
            cascade_depth: ctx.cascade_depth as u8,
            row_scale: (ctx.cascade_row_scale * 100.0).round() as u16,
            x: q(rect.min.x) as i16,
            y: q(rect.min.y) as i16,
            w: q(rect.width()) as u16,
            h: q(rect.height()) as u16,
        }
    }
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
        legend: Vec<(String, Color32)>,
    },
    Cascade {
        key: GeomKey,
        bands: Vec<(NodeId, egui::Rect, usize, Color32)>,
    },
}

pub struct DrawCtx<'a> {
    pub tree: &'a SpaceTree,
    pub focus: NodeId,
    pub selected: Option<NodeId>,
    pub hovered: Option<NodeId>,
    pub review: &'a HashSet<NodeId>,
    pub mode: SizeMode,
    pub filter: &'a ViewFilter,
    pub filter_hash: u64,
    /// Rings the sunburst draws (1..=4).
    pub sunburst_depth: usize,
    /// Folder levels nested inside treemap tiles (1..=4).
    pub treemap_nest: usize,
    /// Band rows below the cascade focus (2..=8).
    pub cascade_depth: usize,
    /// Cascade band height multiplier.
    pub cascade_row_scale: f32,
    /// Zoom transition: geometry is built for the final rect and drawn
    /// scaled about its centre, so nothing is rebuilt per frame.
    pub scale: f32,
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

#[cfg(test)]
mod click_map_tests {
    use super::*;
    use space_core::Node;

    fn dir(name: &str, parent: Option<NodeId>, children: Vec<NodeId>) -> Node {
        Node {
            name: name.into(),
            apparent_size: 10,
            allocated_size: 10,
            file_count: 1,
            is_dir: true,
            synthetic: false,
            children,
            parent,
            raw_name: None,
        }
    }

    fn file(name: &str, parent: NodeId) -> Node {
        Node {
            name: name.into(),
            apparent_size: 3,
            allocated_size: 3,
            file_count: 1,
            is_dir: false,
            synthetic: false,
            children: Vec::new(),
            parent: Some(parent),
            raw_name: None,
        }
    }

    fn sample() -> SpaceTree {
        SpaceTree {
            root_path: "/tmp".into(),
            nodes: vec![
                dir("root", None, vec![NodeId(1)]),
                dir("child", Some(NodeId(0)), vec![NodeId(2)]),
                file("a.txt", NodeId(1)),
            ],
            root: NodeId(0),
            scanned_at_unix: 0,
            item_count: 3,
            skipped_errors: 0,
            hardlink_candidates: 0,
            hardlinks_deduped: 0,
        }
    }

    #[test]
    fn ring_context_menu_survives_pointer_entering_popup_and_keeps_clicked_target() {
        let tree = sample();
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx, crate::theme::Appearance::default(), false, 350);
        let mut geom = None;
        let filter = ViewFilter::default();
        let review = HashSet::new();
        let mut action = ViewAction::none();
        let mut frame = |events: Vec<egui::Event>| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        action = draw(
                            ui,
                            ViewMode::Sunburst,
                            &mut DrawCtx {
                                tree: &tree,
                                focus: tree.root,
                                selected: None,
                                hovered: None,
                                review: &review,
                                mode: SizeMode::OnDisk,
                                filter: &filter,
                                filter_hash: filter.hash64(),
                                sunburst_depth: 2,
                                treemap_nest: 2,
                                cascade_depth: 5,
                                cascade_row_scale: 1.0,
                                scale: 1.0,
                                geom: &mut geom,
                            },
                        );
                    });
                },
            )
        };
        // The single child occupies the entire first ring.
        frame(vec![]);
        let origin = egui::pos2(520.0, 300.0);
        let button = |pos, button, pressed| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![
            egui::Event::PointerMoved(origin),
            button(origin, egui::PointerButton::Secondary, true),
        ]);
        frame(vec![button(origin, egui::PointerButton::Secondary, false)]);
        let output = frame(vec![]);
        let select = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "Select" => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .expect("right click opens the node menu");
        for _ in 0..3 {
            let output = frame(vec![egui::Event::PointerMoved(select)]);
            assert!(
                output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.job.text == "Select")),
                "menu must stay drawn while the pointer is over its items"
            );
        }
        frame(vec![button(select, egui::PointerButton::Primary, true)]);
        frame(vec![button(select, egui::PointerButton::Primary, false)]);
        assert_eq!(
            action.select,
            Some(NodeId(1)),
            "action uses the right-clicked node, not the node behind the menu"
        );
    }

    #[test]
    fn click_child_folder_selects_without_navigation() {
        let tree = sample();
        let mut action = ViewAction::none();
        click_map_node(&tree, NodeId(1), false, &mut action);
        assert_eq!(action.navigate, None);
        assert_eq!(action.select, Some(NodeId(1)));
    }

    #[test]
    fn click_current_folder_selects_without_navigation() {
        let tree = sample();
        let mut action = ViewAction::none();
        click_map_node(&tree, NodeId(1), false, &mut action);
        assert_eq!(action.navigate, None);
        assert_eq!(action.select, Some(NodeId(1)));
    }

    #[test]
    fn click_file_selects() {
        let tree = sample();
        let mut action = ViewAction::none();
        click_map_node(&tree, NodeId(2), false, &mut action);
        assert_eq!(action.select, Some(NodeId(2)));
        assert_eq!(action.navigate, None);
    }

    #[test]
    fn double_click_file_reveals() {
        let tree = sample();
        let mut action = ViewAction::none();
        double_click_map_node(&tree, NodeId(2), &mut action);
        assert_eq!(action.reveal, Some(NodeId(2)));
        assert_eq!(action.navigate, None);
    }

    #[test]
    fn click_synthetic_does_not_drill_in() {
        let mut tree = sample();
        tree.nodes[1].synthetic = true;
        let mut action = ViewAction::none();
        click_map_node(&tree, NodeId(1), false, &mut action);
        assert_eq!(action.select, Some(NodeId(1)));
        assert_eq!(action.navigate, None);
    }

    #[test]
    fn cmd_click_queues_trash() {
        let tree = sample();
        let mut action = ViewAction::none();
        click_map_node(&tree, NodeId(2), true, &mut action);
        assert_eq!(action.toggle_review, Some(NodeId(2)));
        assert_eq!(action.navigate, None);
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;

    #[test]
    fn query_matching_preserves_lowercase_substring_semantics() {
        let names = [
            "",
            "Cache.BIN",
            "a file.log",
            "abc",
            "abcABC",
            "résumé.TXT",
            "İstanbul",
            "ΣΟΣ",
            "Привет.LOG",
            "😺cache",
            "cache\0temp",
        ];
        let queries = [
            "",
            "a",
            "A",
            "CACHE",
            " file",
            "longer-than-the-name",
            "abcABC",
            "résumé",
            "RÉSUMÉ",
            "i",
            "İ",
            "Σ",
            "σος",
            "ПРИ",
            "😺",
            "\0",
        ];
        for name in names {
            for query in queries {
                let filter = ViewFilter {
                    query: query.into(),
                    ..Default::default()
                };
                assert_eq!(
                    filter.matches(name, false, 10),
                    name.to_lowercase().contains(&query.to_lowercase()),
                    "name={name:?}, query={query:?}",
                );
            }
        }
    }

    #[test]
    fn query_optimization_preserves_kind_size_and_extension_filters() {
        let mut filter = ViewFilter {
            query: "CACHE".into(),
            min_size: 10,
            max_size: Some(20),
            ext: BTreeSet::from(["log".to_string()]),
            kind: EntryKindFilter::FilesOnly,
        };
        assert!(filter.matches("cache.log", false, 10));
        assert!(!filter.matches("cache.log", true, 10));
        assert!(!filter.matches("cache.log", false, 9));
        assert!(!filter.matches("cache.log", false, 21));
        assert!(!filter.matches("cache.bin", false, 15));
        assert!(!filter.matches("other.log", false, 15));
        filter.kind = EntryKindFilter::All;
        // Directory navigation bypasses query matching with an extension filter.
        assert!(filter.matches("other", true, 15));
    }

    #[test]
    fn several_extensions_combine_and_toggle() {
        let mut filter = ViewFilter::default();
        filter.toggle_ext("LOG");
        filter.toggle_ext("bin");
        assert!(filter.has_ext("log") && filter.has_ext("BIN"));
        assert!(filter.matches("a.log", false, 1) && filter.matches("b.Bin", false, 1));
        assert!(!filter.matches("c.txt", false, 1));
        let before = filter.hash64();
        filter.toggle_ext("log");
        assert!(!filter.has_ext("log") && filter.matches("b.bin", false, 1));
        assert_ne!(before, filter.hash64());
        filter.toggle_ext("bin");
        assert!(!filter.is_active());
    }

    #[test]
    #[ignore = "manual release profile: filter_query_profile -- --ignored --nocapture"]
    fn filter_query_profile() {
        use std::{hint::black_box, time::Instant};
        let names: Vec<_> = (0..50_000)
            .map(|index| format!("Cache_File_{index:05}.bin"))
            .collect();
        for query in ["FILE", "missing"] {
            let filter = ViewFilter {
                query: query.into(),
                ..Default::default()
            };
            let expected = names
                .iter()
                .filter(|name| name.to_lowercase().contains(&query.to_lowercase()))
                .count();
            let mut samples = Vec::new();
            for sample in 0..12 {
                let start = Instant::now();
                let count = names
                    .iter()
                    .filter(|name| black_box(&filter).matches(black_box(name), false, 10))
                    .count();
                black_box(count);
                assert_eq!(count, expected);
                if sample > 0 {
                    samples.push(start.elapsed());
                }
            }
            samples.sort();
            println!(
                "query={query}: 50,000 names, {expected} matches, median {:?}",
                samples[5]
            );
        }
    }
}
