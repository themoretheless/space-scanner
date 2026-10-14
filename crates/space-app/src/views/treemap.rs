//! Nested squarified treemap with extension legend filter.

use egui::{pos2, Align2, FontId, Rect, Sense, Ui, Vec2};
use space_core::NodeId;

use crate::theme::type_scale::CAPTION;
use crate::theme::{self, ACCENT, MUTED, TEXT};
use crate::views::{child_ids, truncate, DrawCtx, GeomCache, GeomKey, ViewAction};

const MAX_NEST: usize = 2;
const MIN_PX: f32 = 10.0;
const MAX_TILES: usize = 180;

struct Tile {
    id: NodeId,
    rect: Rect,
    depth: usize,
}

pub fn draw(ui: &mut Ui, ctx: &mut DrawCtx<'_>) -> ViewAction {
    let mut action = ViewAction::none();
    let tree = ctx.tree;
    let full = ui.available_size();
    let legend_h = 28.0;
    let (map_rect, _) = ui.allocate_exact_size(
        egui::vec2(full.x, (full.y - legend_h).max(40.0)),
        Sense::hover(),
    );
    let (legend_rect, _) = ui.allocate_exact_size(egui::vec2(full.x, legend_h), Sense::hover());
    let painter = ui.painter_at(map_rect);
    painter.rect_filled(map_rect, 4.0, theme::PANEL());

    let key = GeomKey::new(1, ctx, map_rect);
    let need = match ctx.geom.as_ref() {
        Some(GeomCache::Treemap { key: k, .. }) => *k != key,
        _ => true,
    };

    if need {
        let kids = child_ids(ctx, ctx.focus);
        let mut tiles = Vec::new();
        if !kids.is_empty() {
            layout_nested(ctx, &kids, map_rect.shrink(3.0), 0, &mut tiles);
        }
        let legend = theme::legend_for_names(tiles.iter().map(|t| {
            let n = tree.node(t.id);
            (n.name.as_str(), n.is_dir)
        }));
        *ctx.geom = Some(GeomCache::Treemap {
            key,
            tiles: tiles.into_iter().map(|t| (t.id, t.rect, t.depth)).collect(),
            legend,
        });
    }

    let Some(GeomCache::Treemap { tiles, legend, .. }) = ctx.geom.as_ref() else {
        return action;
    };

    if tiles.is_empty() {
        if ctx.filter.is_active() {
            super::empty_filter_prompt(ui, map_rect, &mut action);
        } else {
            painter.text(
                map_rect.center(),
                Align2::CENTER_CENTER,
                crate::i18n::t("This folder has no children to show"),
                FontId::proportional(14.0),
                MUTED(),
            );
        }
        return action;
    }

    {
        let mut x = legend_rect.left() + 8.0;
        let y = legend_rect.center().y;
        ui.painter_at(legend_rect).text(
            egui::pos2(x, y),
            Align2::LEFT_CENTER,
            crate::i18n::t("Types"),
            FontId::proportional(CAPTION),
            MUTED(),
        );
        x += 44.0;
        for (ext, color) in legend {
            let chip = Rect::from_min_size(
                egui::pos2(x, y - 10.0),
                Vec2::new(12.0 + ext.len() as f32 * 7.0 + 16.0, 20.0),
            );
            let selected = ctx.filter.ext.as_deref() == Some(ext.as_str());
            let resp = ui.interact(chip, ui.id().with(("leg", ext.as_str())), Sense::click());
            if resp.clicked() {
                action.filter_ext = Some(ext.clone());
            }
            resp.on_hover_text(crate::i18n::t(if selected {
                "Showing this type — click to show all"
            } else {
                "Show only this file type on the map"
            }));
            ui.painter().rect_filled(
                Rect::from_center_size(egui::pos2(x + 6.0, y), Vec2::splat(10.0)),
                2.0,
                *color,
            );
            ui.painter().text(
                egui::pos2(x + 16.0, y),
                Align2::LEFT_CENTER,
                format!(".{ext}"),
                FontId::proportional(CAPTION),
                if selected { ACCENT() } else { TEXT() },
            );
            x += chip.width() + 4.0;
            if x > legend_rect.right() - 40.0 {
                break;
            }
        }
    }

    let center = map_rect.center();
    let scale = ctx.scale;
    for &(id, rect, depth) in tiles {
        let node = tree.node(id);
        let rect = super::zoomed_rect(center, rect, scale);
        if rect.width() < 2.0 || rect.height() < 2.0 {
            continue;
        }
        let color = if node.is_dir && depth < MAX_NEST.saturating_sub(1) {
            theme::DIR()
        } else {
            theme::color_for(&node.name, node.is_dir)
        };
        let is_hov = ctx.hovered == Some(id);
        let is_sel = ctx.selected == Some(id) || ctx.review.contains(&id);
        let (hov_t, sel_t) = super::highlight(ui, "tm", Some(id), is_hov, is_sel);
        let shrink = if node.is_dir { 1.5 } else { 0.8 };
        let draw = rect.shrink(shrink);
        // Soft grow on hover.
        let grow = hov_t * 1.2;
        let draw = Rect::from_center_size(draw.center(), draw.size() + egui::vec2(grow, grow));
        painter.rect_filled(draw, 2.0, color);
        painter.rect_stroke(
            draw,
            2.0,
            super::highlight_stroke(hov_t, sel_t, theme::BG()),
            egui::StrokeKind::Inside,
        );

        if depth == 0 && rect.width() > 52.0 && rect.height() > 26.0 {
            painter.text(
                rect.left_top() + Vec2::new(5.0, 4.0),
                Align2::LEFT_TOP,
                truncate(&node.name, 20),
                FontId::proportional(CAPTION),
                TEXT(),
            );
            if rect.height() > 40.0 {
                painter.text(
                    rect.left_top() + Vec2::new(5.0, 19.0),
                    Align2::LEFT_TOP,
                    crate::i18n::bytes(node.size(ctx.mode)),
                    FontId::proportional(CAPTION),
                    MUTED(),
                );
            }
        }

        let resp = ui.interact(rect, ui.id().with(("tm", id.0, depth)), Sense::click());
        super::node_response(ui, &resp, ctx, id, &mut action);
    }

    action
}

fn layout_nested(ctx: &DrawCtx<'_>, ids: &[NodeId], rect: Rect, depth: usize, out: &mut Vec<Tile>) {
    if ids.is_empty() || rect.width() < MIN_PX || rect.height() < MIN_PX || out.len() >= MAX_TILES {
        return;
    }
    let sizes: Vec<f64> = ids
        .iter()
        .map(|id| ctx.tree.node(*id).size(ctx.mode).max(1) as f64)
        .collect();
    let flat = squarify(ids, &sizes, rect);

    for tile in flat {
        if out.len() >= MAX_TILES {
            break;
        }
        let node = ctx.tree.node(tile.id);
        out.push(Tile {
            id: tile.id,
            rect: tile.rect,
            depth,
        });
        if node.is_dir && depth + 1 < MAX_NEST {
            let kids = child_ids(ctx, tile.id);
            if !kids.is_empty()
                && tile.rect.width() > MIN_PX * 2.0
                && tile.rect.height() > MIN_PX * 2.0
            {
                layout_nested(ctx, &kids, tile.rect.shrink(2.0), depth + 1, out);
            }
        }
    }
}

struct FlatTile {
    id: NodeId,
    rect: Rect,
}

/// A row being filled: items plus the running sum/min/max of their areas, so
/// the aspect test is O(1) per candidate instead of a clone-and-rescan.
#[derive(Default)]
struct Row {
    items: Vec<(NodeId, f64)>,
    sum: f64,
    min: f64,
    max: f64,
}

impl Row {
    fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn push(&mut self, id: NodeId, area: f64) {
        if self.items.is_empty() {
            self.min = area;
            self.max = area;
        } else {
            self.min = self.min.min(area);
            self.max = self.max.max(area);
        }
        self.sum += area;
        self.items.push((id, area));
    }

    fn clear(&mut self) {
        self.items.clear();
        self.sum = 0.0;
        self.min = 0.0;
        self.max = 0.0;
    }

    /// Worst aspect ratio of the row laid along a side of length `side`
    /// (Bruls, Huizing & van Wijk): max(side²·max/sum², sum²/(side²·min)).
    fn worst(&self, side: f64) -> f64 {
        if self.items.is_empty() {
            return f64::MAX;
        }
        let s2 = (side * side).max(1e-9);
        let sum2 = (self.sum * self.sum).max(1e-18);
        (s2 * self.max / sum2).max(sum2 / (s2 * self.min.max(1e-9)))
    }

    /// Would adding `area` keep the row at least as square?
    fn accepts(&self, area: f64, side: f64) -> bool {
        if self.items.is_empty() {
            return true;
        }
        let sum = self.sum + area;
        let min = self.min.min(area);
        let max = self.max.max(area);
        let s2 = (side * side).max(1e-9);
        let sum2 = (sum * sum).max(1e-18);
        let worst = (s2 * max / sum2).max(sum2 / (s2 * min.max(1e-9)));
        worst <= self.worst(side)
    }
}

/// Squarified layout in absolute pixel areas: every item gets
/// `size / total × rect area`, rows are laid along the shorter side of what
/// remains, and the last tile of each row snaps to the edge so the tiles
/// cover the rect exactly.
fn squarify(ids: &[NodeId], sizes: &[f64], rect: Rect) -> Vec<FlatTile> {
    let total: f64 = sizes.iter().sum::<f64>().max(1e-9);
    let rect_area = rect.width() as f64 * rect.height() as f64;
    let mut items: Vec<(NodeId, f64)> = ids
        .iter()
        .zip(sizes.iter())
        .map(|(&id, &s)| (id, s / total * rect_area))
        .collect();
    items.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut out = Vec::with_capacity(items.len());
    let mut remaining = rect;
    let mut row = Row::default();
    let mut i = 0;

    while i < items.len() {
        let (id, area) = items[i];
        let side = remaining.width().min(remaining.height()) as f64;
        if row.is_empty() || row.accepts(area, side) {
            row.push(id, area);
            i += 1;
        } else {
            layout_row(&row, &mut remaining, &mut out, false);
            row.clear();
        }
    }
    if !row.is_empty() {
        layout_row(&row, &mut remaining, &mut out, true);
    }
    out
}

/// Place one row along the shorter side of `remaining`; its thickness is
/// `row area / that side`. The final row takes whatever is left.
fn layout_row(row: &Row, remaining: &mut Rect, out: &mut Vec<FlatTile>, last_row: bool) {
    let sum = row.sum.max(1e-12);
    let vertical = remaining.width() >= remaining.height();
    if vertical {
        // Column at the left, tiles stacked top to bottom.
        let side = remaining.height().max(1.0) as f64;
        let w = if last_row {
            remaining.width()
        } else {
            (sum / side) as f32
        }
        .min(remaining.width());
        let mut y = remaining.top();
        for (k, &(id, area)) in row.items.iter().enumerate() {
            let h = if k + 1 == row.items.len() {
                remaining.bottom() - y
            } else {
                (area / w.max(1e-6) as f64) as f32
            };
            out.push(FlatTile {
                id,
                rect: Rect::from_min_size(pos2(remaining.left(), y), Vec2::new(w, h)),
            });
            y += h;
        }
        *remaining = Rect::from_min_max(pos2(remaining.left() + w, remaining.top()), remaining.max);
    } else {
        // Strip along the top, tiles left to right.
        let side = remaining.width().max(1.0) as f64;
        let h = if last_row {
            remaining.height()
        } else {
            (sum / side) as f32
        }
        .min(remaining.height());
        let mut x = remaining.left();
        for (k, &(id, area)) in row.items.iter().enumerate() {
            let w = if k + 1 == row.items.len() {
                remaining.right() - x
            } else {
                (area / h.max(1e-6) as f64) as f32
            };
            out.push(FlatTile {
                id,
                rect: Rect::from_min_size(pos2(x, remaining.top()), Vec2::new(w, h)),
            });
            x += w;
        }
        *remaining = Rect::from_min_max(pos2(remaining.left(), remaining.top() + h), remaining.max);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiles_for(sizes: &[f64], rect: Rect) -> Vec<FlatTile> {
        let ids: Vec<NodeId> = (0..sizes.len() as u32).map(NodeId).collect();
        squarify(&ids, sizes, rect)
    }

    fn check_cover(sizes: &[f64], rect: Rect) {
        let tiles = tiles_for(sizes, rect);
        assert_eq!(tiles.len(), sizes.len());
        let rect_area = rect.width() as f64 * rect.height() as f64;
        let sum: f64 = tiles
            .iter()
            .map(|t| t.rect.width() as f64 * t.rect.height() as f64)
            .sum();
        assert!(
            (sum - rect_area).abs() <= 1.0,
            "tiles cover {sum} of {rect_area} px² for {sizes:?}"
        );
        for t in &tiles {
            assert!(t.rect.width() >= 0.0 && t.rect.height() >= 0.0);
            assert!(rect.expand(0.01).contains_rect(t.rect), "{:?} outside {rect:?}", t.rect);
        }
        for (a, ta) in tiles.iter().enumerate() {
            for tb in &tiles[a + 1..] {
                let i = ta.rect.intersect(tb.rect);
                let overlap = if i.is_positive() {
                    i.width() as f64 * i.height() as f64
                } else {
                    0.0
                };
                assert!(overlap <= 0.01, "{:?} overlaps {:?}", ta.rect, tb.rect);
            }
        }
        // Areas are proportional to sizes.
        let total: f64 = sizes.iter().sum();
        for t in &tiles {
            let s = sizes[t.id.0 as usize];
            let want = s / total * rect_area;
            let got = t.rect.width() as f64 * t.rect.height() as f64;
            assert!((got - want).abs() <= 1.0, "size {s}: {got} vs {want}");
        }
    }

    #[test]
    fn tiles_cover_the_rect_without_overlap() {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), Vec2::new(800.0, 600.0));
        check_cover(&[6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0], rect);
        check_cover(&[1.0], rect);
        check_cover(&[1.0, 1.0], rect);
        check_cover(&[1000.0, 1.0, 1.0, 1.0], rect);
        let many: Vec<f64> = (1..=60).map(|i| (i * i) as f64).collect();
        check_cover(&many, rect);
        check_cover(&many, Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(300.0, 900.0)));
    }

    #[test]
    fn rows_are_laid_along_the_shorter_side() {
        // Wide rect: the first row must be a column at the left.
        let rect = Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(800.0, 400.0));
        let tiles = tiles_for(&[1.0, 1.0, 1.0, 1.0], rect);
        assert!(tiles[0].rect.left() == rect.left());
        assert!(tiles[0].rect.height() <= rect.height());
        assert!((tiles[0].rect.width() - 400.0).abs() < 1.0, "{:?}", tiles[0].rect);
    }

    #[test]
    fn incremental_worst_matches_recomputation() {
        let mut row = Row::default();
        for a in [40.0, 30.0, 25.0] {
            row.push(NodeId(0), a);
        }
        let side = 10.0;
        let sum = 95.0_f64;
        let expect = (side * side * 40.0 / (sum * sum)).max(sum * sum / (side * side * 25.0));
        assert!((row.worst(side) - expect).abs() < 1e-9);
    }
}
