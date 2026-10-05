//! Nested squarified treemap with extension legend filter.

use egui::{pos2, Align2, FontId, Rect, Sense, Stroke, Ui, Vec2};
use space_core::NodeId;

use crate::theme::{self, ACCENT, MUTED, TEXT};
use crate::views::{child_ids, DrawCtx, ViewAction};

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

    let key = crate::views::GeomKey {
        view: 1,
        focus: ctx.focus,
        mode: ctx.mode,
        filter: ctx.filter_hash,
        selected: None,
        w: ((map_rect.width() / 8.0).round() as u16).saturating_mul(8),
        h: ((map_rect.height() / 8.0).round() as u16).saturating_mul(8),
        review_sig: 0,
    };

    let need = match ctx.geom.as_ref() {
        Some(crate::views::GeomCache::Treemap { key: k, .. }) => *k != key,
        _ => true,
    };

    if need {
        let kids = child_ids(ctx, ctx.focus);
        if kids.is_empty() {
            *ctx.geom = Some(crate::views::GeomCache::Treemap {
                key,
                tiles: Vec::new(),
            });
        } else {
            let mut tiles = Vec::new();
            layout_nested(ctx, &kids, map_rect.shrink(3.0), 0, &mut tiles);
            *ctx.geom = Some(crate::views::GeomCache::Treemap {
                key,
                tiles: tiles.into_iter().map(|t| (t.id, t.rect, t.depth)).collect(),
            });
        }
    }

    let tiles: &[(NodeId, egui::Rect, usize)] = match ctx.geom.as_ref() {
        Some(crate::views::GeomCache::Treemap { tiles, .. }) => tiles.as_slice(),
        _ => &[],
    };

    if tiles.is_empty() {
        if ctx.filter.is_active() {
            super::empty_filter_prompt(ui, map_rect, &mut action);
        } else {
            painter.text(
                map_rect.center(),
                Align2::CENTER_CENTER,
                "This folder has no children to show",
                FontId::proportional(14.0),
                MUTED(),
            );
        }
        return action;
    }

    let legend = theme::legend_for_names(tiles.iter().map(|(id, _, _)| {
        let n = tree.node(*id);
        (n.name.as_str(), n.is_dir)
    }));
    {
        let mut x = legend_rect.left() + 8.0;
        let y = legend_rect.center().y;
        ui.painter_at(legend_rect).text(
            egui::pos2(x, y),
            Align2::LEFT_CENTER,
            "Types",
            FontId::proportional(11.0),
            MUTED(),
        );
        x += 44.0;
        for (ext, color) in &legend {
            let chip = egui::Rect::from_min_size(
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
                egui::Rect::from_center_size(egui::pos2(x + 6.0, y), Vec2::splat(10.0)),
                2.0,
                *color,
            );
            ui.painter().text(
                egui::pos2(x + 16.0, y),
                Align2::LEFT_CENTER,
                format!(".{ext}"),
                FontId::proportional(11.0),
                if selected { ACCENT() } else { TEXT() },
            );
            x += chip.width() + 4.0;
            if x > legend_rect.right() - 40.0 {
                break;
            }
        }
    }

    for &(id, rect, depth) in tiles {
        let node = tree.node(id);
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
        let hov_t =
            ui.ctx()
                .animate_bool_with_time(ui.id().with(("tm_hov", id.0, depth)), is_hov, 0.14);
        let sel_t =
            ui.ctx()
                .animate_bool_with_time(ui.id().with(("tm_sel", id.0, depth)), is_sel, 0.18);
        let shrink = if node.is_dir { 1.5 } else { 0.8 };
        let draw = rect.shrink(shrink);
        // Soft grow on hover.
        let grow = hov_t * 1.2;
        let draw =
            egui::Rect::from_center_size(draw.center(), draw.size() + egui::vec2(grow, grow));
        painter.rect_filled(draw, 2.0, color);
        let stroke_w = 0.6 + sel_t * 1.4 + hov_t * 1.0;
        let stroke_a = (90.0 + sel_t * 150.0 + hov_t * 100.0).min(255.0) as u8;
        painter.rect_stroke(
            draw,
            2.0,
            Stroke::new(
                stroke_w,
                if sel_t > 0.05 || hov_t > 0.05 {
                    crate::theme::ACCENT().gamma_multiply(stroke_a as f32 / 255.0)
                } else {
                    theme::BG()
                },
            ),
            egui::StrokeKind::Inside,
        );

        if depth == 0 && rect.width() > 52.0 && rect.height() > 26.0 {
            painter.text(
                rect.left_top() + Vec2::new(5.0, 4.0),
                Align2::LEFT_TOP,
                truncate(&node.name, 20),
                FontId::proportional(11.0),
                TEXT(),
            );
            if rect.height() > 40.0 {
                painter.text(
                    rect.left_top() + Vec2::new(5.0, 18.0),
                    Align2::LEFT_TOP,
                    crate::i18n::bytes(node.size(ctx.mode)),
                    FontId::proportional(10.0),
                    MUTED(),
                );
            }
        }

        let resp = ui.interact(rect, ui.id().with(("tm", id.0, depth)), Sense::click());
        if resp.clicked() {
            super::click_map_node(
                ctx.tree,
                id,
                ctx.focus,
                ui.input(|i| i.modifiers.command || i.modifiers.ctrl),
                &mut action,
            );
        }
        if resp.double_clicked() {
            super::double_click_map_node(ctx.tree, id, &mut action);
        }
        let in_q = ctx.review.contains(&id);
        super::node_context_menu(&resp, ctx.tree, id, in_q, &mut action);
        if resp.hovered() {
            action.hover = Some(id);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            resp.on_hover_ui(|ui| super::node_hover_ui(ui, ctx.tree, id, ctx.focus, ctx.mode));
        }
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

fn squarify(ids: &[NodeId], sizes: &[f64], rect: Rect) -> Vec<FlatTile> {
    let total: f64 = sizes.iter().sum::<f64>().max(1.0);
    let mut items: Vec<(NodeId, f64)> = ids
        .iter()
        .zip(sizes.iter())
        .map(|(&id, &s)| (id, s / total))
        .collect();
    items.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut out = Vec::new();
    let mut remaining = rect;
    let mut row: Vec<(NodeId, f64)> = Vec::new();
    let mut i = 0;

    while i < items.len() {
        let next = items[i];
        let mut trial = row.clone();
        trial.push(next);
        if row.is_empty() || worst_aspect(&trial, remaining) <= worst_aspect(&row, remaining) {
            row.push(next);
            i += 1;
        } else {
            layout_row(&row, &mut remaining, &mut out);
            row.clear();
        }
    }
    if !row.is_empty() {
        layout_row(&row, &mut remaining, &mut out);
    }
    out
}

fn layout_row(row: &[(NodeId, f64)], remaining: &mut Rect, out: &mut Vec<FlatTile>) {
    let sum: f64 = row.iter().map(|(_, f)| *f).sum::<f64>().max(1e-12);
    let horizontal = remaining.width() >= remaining.height();
    if horizontal {
        let h = (remaining.height() as f64 * sum) as f32;
        let mut x = remaining.left();
        for &(id, frac) in row {
            let w = remaining.width() * (frac / sum) as f32;
            out.push(FlatTile {
                id,
                rect: Rect::from_min_size(pos2(x, remaining.top()), Vec2::new(w, h)),
            });
            x += w;
        }
        *remaining = Rect::from_min_max(pos2(remaining.left(), remaining.top() + h), remaining.max);
    } else {
        let w = (remaining.width() as f64 * sum) as f32;
        let mut y = remaining.top();
        for &(id, frac) in row {
            let h = remaining.height() * (frac / sum) as f32;
            out.push(FlatTile {
                id,
                rect: Rect::from_min_size(pos2(remaining.left(), y), Vec2::new(w, h)),
            });
            y += h;
        }
        *remaining = Rect::from_min_max(pos2(remaining.left() + w, remaining.top()), remaining.max);
    }
}

fn worst_aspect(row: &[(NodeId, f64)], rect: Rect) -> f64 {
    if row.is_empty() {
        return f64::MAX;
    }
    let sum: f64 = row.iter().map(|(_, f)| *f).sum();
    let horizontal = rect.width() >= rect.height();
    let side = if horizontal {
        rect.height() as f64
    } else {
        rect.width() as f64
    }
    .max(1.0);
    let other = if horizontal {
        rect.width() as f64 * sum
    } else {
        rect.height() as f64 * sum
    }
    .max(1.0);

    row.iter()
        .map(|(_, f)| {
            let len = other * (*f / sum);
            side.max(len) / side.min(len).max(1e-9)
        })
        .fold(0.0_f64, f64::max)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}
