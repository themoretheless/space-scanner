mod cascade;
mod radial;
mod sunburst;
mod treemap;

use crate::i18n::t;
use std::borrow::Cow;
use std::hash::{Hash, Hasher};

use std::sync::Arc;

use egui::epaint::Mesh;
use egui::{Pos2, RichText, Ui};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
    pub ext: Option<String>,
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

    pub fn is_active(&self) -> bool {
        !self.query.is_empty()
            || self.ext.is_some()
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

pub fn click_map_node(
    tree: &SpaceTree,
    id: NodeId,
    focus: NodeId,
    cmd: bool,
    action: &mut ViewAction,
) {
    let n = tree.node(id);
    if cmd && !n.synthetic {
        action.toggle_review = Some(id);
        action.select = Some(id);
        return;
    }
    let _ = focus;
    action.select = Some(id);
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
    review: &[NodeId],
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
    pub hovered: Option<NodeId>,
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
        crate::theme::apply(&ctx, crate::theme::Appearance::default());
        let mut geom = None;
        let filter = ViewFilter::default();
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
                                review: &[],
                                mode: SizeMode::OnDisk,
                                filter: &filter,
                                filter_hash: filter.hash64(),
                                review_sig: 0,
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
        drop(frame);
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
        click_map_node(&tree, NodeId(1), NodeId(0), false, &mut action);
        assert_eq!(action.navigate, None);
        assert_eq!(action.select, Some(NodeId(1)));
    }

    #[test]
    fn click_current_folder_selects_without_navigation() {
        let tree = sample();
        let mut action = ViewAction::none();
        click_map_node(&tree, NodeId(1), NodeId(1), false, &mut action);
        assert_eq!(action.navigate, None);
        assert_eq!(action.select, Some(NodeId(1)));
    }

    #[test]
    fn click_file_selects() {
        let tree = sample();
        let mut action = ViewAction::none();
        click_map_node(&tree, NodeId(2), NodeId(1), false, &mut action);
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
        click_map_node(&tree, NodeId(1), NodeId(0), false, &mut action);
        assert_eq!(action.select, Some(NodeId(1)));
        assert_eq!(action.navigate, None);
    }

    #[test]
    fn cmd_click_queues_trash() {
        let tree = sample();
        let mut action = ViewAction::none();
        click_map_node(&tree, NodeId(2), NodeId(1), true, &mut action);
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
            ext: Some("LOG".into()),
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
