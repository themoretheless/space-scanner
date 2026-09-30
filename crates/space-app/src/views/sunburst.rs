//! Sunburst: smooth arcs, geometry cached, cheap overlays.

use std::sync::Arc;

use egui::{epaint::Mesh, Align2, Color32, FontId, Pos2, Sense, Shape, Stroke, Ui, Vec2};
use space_core::{format_bytes, NodeId};

use crate::theme::{self, ACCENT, MUTED, TEXT};
use crate::views::{child_ids, DrawCtx, GeomCache, GeomKey, ViewAction};

const MAX_DEPTH: usize = 2;
/// ~8px at outer ring — hairline wedges become "Other".
const MIN_SWEEP_PX: f32 = 8.0;
const MAX_ARCS: usize = 120;
const MAX_TOP: usize = 18;
/// Gap between rings (px).
const RING_GAP: f32 = 3.0;
/// Angular gap between siblings (radians).
const SIBLING_GAP: f32 = 0.012;
/// ~3px chord length target for smooth outer edges.
const PX_PER_STEP: f32 = 3.0;

#[derive(Clone, Copy)]
pub struct ArcHit {
    pub id: Option<NodeId>,
    pub inner: f32,
    pub outer: f32,
    pub a0: f32,
    pub a1: f32,
}

pub fn draw(ui: &mut Ui, ctx: &mut DrawCtx<'_>) -> ViewAction {
    let mut action = ViewAction::none();
    let tree = ctx.tree;
    let focus = ctx.focus;
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click());
    let painter = ui.painter_at(rect);
    let center = rect.center();
    let max_r = rect.width().min(rect.height()) * 0.46;
    let hole = max_r * 0.22;
    let ring_w = (max_r - hole) / MAX_DEPTH as f32;

    // Quantize size so sub-pixel layout jitter doesn't rebuild the mesh every frame.
    let key = GeomKey {
        view: 0,
        focus,
        mode: ctx.mode,
        filter: ctx.filter_hash,
        selected: None,
        w: ((rect.width() / 8.0).round() as u16).saturating_mul(8),
        h: ((rect.height() / 8.0).round() as u16).saturating_mul(8),
        review_sig: 0,
    };

    let need_rebuild = match ctx.geom.as_ref() {
        Some(GeomCache::Sunburst { key: k, .. }) => *k != key,
        _ => true,
    };

    if need_rebuild {
        let mut mesh = Mesh::default();
        let mut hits: Vec<ArcHit> = Vec::with_capacity(MAX_ARCS);
        let mid_r = hole + ring_w * 0.5;
        let min_sweep = (MIN_SWEEP_PX / mid_r.max(1.0)).max(0.035);

        let kids = child_ids(ctx, focus);
        let top = kids.iter().copied().take(MAX_TOP).collect::<Vec<_>>();
        let total = top
            .iter()
            .map(|id| tree.node(*id).size(ctx.mode))
            .sum::<u64>()
            .max(1);

        let mut visible: Vec<(NodeId, f32)> = Vec::new();
        let mut other = 0.0_f32;
        for &id in &top {
            let size = tree.node(id).size(ctx.mode);
            let sweep = (size as f32 / total as f32) * std::f32::consts::TAU;
            if sweep < min_sweep {
                other += sweep;
            } else {
                visible.push((id, sweep));
            }
        }
        let top_colors = sibling_colors(
            ctx,
            &visible.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            None,
            true,
        );

        let mut angle = -std::f32::consts::FRAC_PI_2;
        let n_vis = visible.len().max(1);
        let gap = if n_vis > 1 { SIBLING_GAP } else { 0.0 };
        for (i, &(id, sweep)) in visible.iter().enumerate() {
            if hits.len() >= MAX_ARCS {
                break;
            }
            let color = top_colors.get(i).copied().unwrap_or_else(|| {
                let n = tree.node(id);
                theme::color_for(&n.name, n.is_dir)
            });
            let a0 = angle + gap * 0.5;
            let a1 = angle + sweep - gap * 0.5;
            if a1 > a0 {
                paint_subtree(
                    ctx, id, 0, a0, a1, hole, ring_w, center, color, min_sweep, &mut mesh,
                    &mut hits,
                );
            }
            angle += sweep;
        }
        if other >= min_sweep && hits.len() < MAX_ARCS {
            push_segment(
                &mut mesh,
                &mut hits,
                center,
                hole,
                hole + ring_w - RING_GAP,
                angle,
                angle + other,
                Color32::from_rgb(52, 56, 62),
                None,
            );
        }

        *ctx.geom = Some(GeomCache::Sunburst {
            key,
            mesh: Arc::new(mesh),
            hits,
            center,
            hole,
            max_r,
        });
    }

    let Some(GeomCache::Sunburst {
        mesh,
        hits,
        center,
        hole,
        max_r,
        ..
    }) = ctx.geom.as_ref()
    else {
        return action;
    };
    let center = *center;
    let hole = *hole;
    let max_r = *max_r;

    painter.circle_filled(center, max_r + 2.0, theme::PANEL);
    painter.add(Shape::Mesh(Arc::clone(mesh)));

    if hits.is_empty() && ctx.filter.is_active() {
        super::empty_filter_prompt(ui, rect, &mut action);
    }

    let pointer = response.hover_pos();
    let mut hover_id = None;
    let mut hover_other = false;
    if let Some(pos) = pointer {
        for hit in hits.iter().rev() {
            if point_in_arc(pos, center, hit) {
                match hit.id {
                    Some(id) => hover_id = Some(id),
                    None => hover_other = true,
                }
                break;
            }
        }
    }

    // Soft animated highlight — stroke only (fill overlay was a broken shard).
    for (hi, hit) in hits.iter().enumerate() {
        let is_hov = match (hover_id, hit.id) {
            (Some(h), Some(id)) => h == id,
            (None, None) if hover_other => true,
            _ => false,
        };
        let is_sel = hit.id.is_some_and(|id| ctx.selected == Some(id));
        let hov_t = ui
            .ctx()
            .animate_bool_with_time(ui.id().with(("sb_hov", hi)), is_hov, 0.14);
        let sel_t = ui
            .ctx()
            .animate_bool_with_time(ui.id().with(("sb_sel", hi)), is_sel, 0.18);
        if hov_t > 0.01 {
            // Proper annular wash (same tessellation as wedges).
            let mut overlay = Mesh::default();
            push_segment_mesh(
                &mut overlay,
                center,
                hit.inner,
                hit.outer,
                hit.a0,
                hit.a1,
                Color32::from_rgba_unmultiplied(255, 210, 120, (hov_t * 70.0) as u8),
            );
            painter.add(Shape::Mesh(Arc::new(overlay)));
            stroke_wedge(
                &painter,
                center,
                hit,
                Stroke::new(
                    1.4 + hov_t,
                    Color32::from_rgba_unmultiplied(255, 200, 100, (hov_t * 230.0) as u8),
                ),
            );
        } else if sel_t > 0.01 {
            stroke_wedge(
                &painter,
                center,
                hit,
                Stroke::new(
                    1.8 + sel_t * 0.6,
                    Color32::from_rgba_unmultiplied(232, 168, 72, (sel_t * 240.0) as u8),
                ),
            );
        }
    }

    let focus_node = tree.node(focus);
    painter.circle_filled(center, hole - 2.0, theme::BG);
    // Fade-in hub label when focus changes (id-keyed animation resets).
    let name_alpha =
        ui.ctx()
            .animate_bool_with_time(ui.id().with(("sb_focus", focus.0)), true, 0.32);
    let label_a = (0.4 + 0.6 * name_alpha).clamp(0.0, 1.0);
    painter.text(
        center + Vec2::new(0.0, -10.0),
        Align2::CENTER_CENTER,
        truncate(&focus_node.name, 18),
        FontId::proportional(15.0),
        Color32::from_rgba_unmultiplied(TEXT.r(), TEXT.g(), TEXT.b(), (label_a * 255.0) as u8),
    );
    painter.text(
        center + Vec2::new(0.0, 10.0),
        Align2::CENTER_CENTER,
        format_bytes(focus_node.size(ctx.mode)),
        FontId::proportional(12.0),
        Color32::from_rgba_unmultiplied(MUTED.r(), MUTED.g(), MUTED.b(), (label_a * 220.0) as u8),
    );
    if focus_node.parent.is_some() {
        painter.text(
            center + Vec2::new(0.0, 26.0),
            Align2::CENTER_CENTER,
            "↑  click to go up",
            FontId::proportional(10.0),
            Color32::from_rgba_unmultiplied(
                ACCENT.r(),
                ACCENT.g(),
                ACCENT.b(),
                (label_a * 200.0) as u8,
            ),
        );
    }

    if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let d = pos - center;
            if d.length() < hole {
                if let Some(parent) = focus_node.parent {
                    action.navigate = Some(parent);
                }
            } else {
                for hit in hits.iter().rev() {
                    if point_in_arc(pos, center, hit) {
                        if let Some(id) = hit.id {
                            super::click_map_node(
                                tree,
                                id,
                                focus,
                                ui.input(|i| i.modifiers.command || i.modifiers.ctrl),
                                &mut action,
                            );
                        }
                        break;
                    }
                }
            }
        }
    }
    if response.double_clicked() {
        if let Some(id) = hover_id {
            super::double_click_map_node(tree, id, &mut action);
        }
    }
    if let Some(id) = hover_id {
        super::node_context_menu(&response, tree, id, ctx.review.contains(&id), &mut action);
    }

    action.hover = hover_id;
    if let Some(id) = hover_id {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        response
            .clone()
            .on_hover_ui(|ui| super::node_hover_ui(ui, tree, id, ctx.focus, ctx.mode));
    } else if hover_other {
        response
            .clone()
            .on_hover_text("Smaller items grouped here. Open a larger folder to see them.");
    }
    let label_t =
        ui.ctx()
            .animate_bool_with_time(ui.id().with("sb_label"), hover_id.is_some(), 0.18);
    if let Some(id) = hover_id {
        if label_t > 0.02 {
            let n = tree.node(id);
            let parent_size = n
                .parent
                .map(|p| tree.node(p).size(ctx.mode).max(1))
                .unwrap_or(1);
            let pct = (n.size(ctx.mode) as f32 / parent_size as f32) * 100.0;
            let label = format!(
                "{}  ·  {}  ·  {pct:.0}%",
                truncate(&n.name, 28),
                format_bytes(n.size(ctx.mode)),
            );
            let bar = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 12.0, rect.bottom() - 28.0),
                egui::vec2(rect.width() - 24.0, 22.0),
            );
            let a = (label_t * 230.0) as u8;
            painter.rect_filled(
                bar,
                4.0,
                Color32::from_rgba_unmultiplied(
                    theme::PANEL_2.r(),
                    theme::PANEL_2.g(),
                    theme::PANEL_2.b(),
                    a,
                ),
            );
            painter.text(
                bar.left_center() + Vec2::new(10.0, 0.0),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(13.0),
                Color32::from_rgba_unmultiplied(TEXT.r(), TEXT.g(), TEXT.b(), a),
            );
            if n.is_dir {
                painter.text(
                    bar.right_center() - Vec2::new(10.0, 0.0),
                    Align2::RIGHT_CENTER,
                    format!("{} files", n.file_count),
                    FontId::proportional(11.0),
                    Color32::from_rgba_unmultiplied(MUTED.r(), MUTED.g(), MUTED.b(), a),
                );
            }
        }
    }

    action
}

fn stroke_wedge(painter: &egui::Painter, center: Pos2, hit: &ArcHit, stroke: Stroke) {
    let sweep = (hit.a1 - hit.a0).abs();
    let steps = ((sweep * hit.outer / 6.0).ceil() as usize).clamp(4, 48);
    let mut outer_pts = Vec::with_capacity(steps + 1);
    let mut inner_pts = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let a = hit.a0 + sweep * t;
        outer_pts.push(center + Vec2::angled(a) * hit.outer);
        inner_pts.push(center + Vec2::angled(a) * hit.inner);
    }
    painter.add(Shape::line(outer_pts, stroke));
    painter.add(Shape::line(inner_pts, stroke));
    painter.line_segment(
        [
            center + Vec2::angled(hit.a0) * hit.inner,
            center + Vec2::angled(hit.a0) * hit.outer,
        ],
        stroke,
    );
    painter.line_segment(
        [
            center + Vec2::angled(hit.a1) * hit.inner,
            center + Vec2::angled(hit.a1) * hit.outer,
        ],
        stroke,
    );
}

fn sibling_colors(
    ctx: &DrawCtx<'_>,
    ids: &[NodeId],
    parent: Option<Color32>,
    circular: bool,
) -> Vec<Color32> {
    if parent.is_none() && circular {
        // Top ring: same palette as the contents list, so swatches match wedges.
        let items: Vec<(&str, bool)> = ids
            .iter()
            .map(|&id| {
                let n = ctx.tree.node(id);
                (n.name.as_str(), n.is_dir)
            })
            .collect();
        return theme::sibling_palette(&items);
    }
    let mut out = Vec::with_capacity(ids.len());
    for (i, &id) in ids.iter().enumerate() {
        let n = ctx.tree.node(id);
        let mut avoid: Vec<Color32> = Vec::with_capacity(3);
        if let Some(&prev) = out.last() {
            avoid.push(prev);
        }
        let c = if let Some(p) = parent {
            // Outer ring: stay in the parent's hue family.
            theme::color_child_of(p, i, &avoid)
        } else if n.is_dir {
            avoid.extend(out.iter().copied().take(2));
            theme::color_for_adjacent(&n.name, true, i, &avoid)
        } else {
            theme::color_for_adjacent(&n.name, false, i, &avoid)
        };
        out.push(c);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn paint_subtree(
    ctx: &DrawCtx<'_>,
    id: NodeId,
    depth: usize,
    a0: f32,
    a1: f32,
    hole: f32,
    ring_w: f32,
    center: Pos2,
    color: Color32,
    min_sweep: f32,
    mesh: &mut Mesh,
    hits: &mut Vec<ArcHit>,
) {
    if depth >= MAX_DEPTH || (a1 - a0) < min_sweep * 0.5 || hits.len() >= MAX_ARCS {
        return;
    }

    let tree = ctx.tree;
    let node = tree.node(id);
    let inner = hole + depth as f32 * ring_w;
    let outer = inner + ring_w - RING_GAP;
    // Outer ring slightly darker so rings stack with depth, not mud.
    let shade = if depth == 0 { 1.0 } else { 0.88 };
    let color = Color32::from_rgb(
        (color.r() as f32 * shade) as u8,
        (color.g() as f32 * shade) as u8,
        (color.b() as f32 * shade) as u8,
    );

    push_segment(mesh, hits, center, inner, outer, a0, a1, color, Some(id));

    if !node.is_dir || depth + 1 >= MAX_DEPTH {
        return;
    }

    let kids = child_ids(ctx, id);
    if kids.is_empty() {
        return;
    }
    let take = kids.iter().copied().take(16).collect::<Vec<_>>();
    let span = a1 - a0;
    let total = take
        .iter()
        .map(|c| tree.node(*c).size(ctx.mode))
        .sum::<u64>()
        .max(1);
    let child_min =
        (min_sweep * (hole + ring_w * 1.5) / (hole + ring_w * 0.5).max(1.0)).max(min_sweep);
    let mut visible: Vec<(NodeId, f32)> = Vec::new();
    let mut other = 0.0_f32;
    for &cid in &take {
        let cs = tree.node(cid).size(ctx.mode);
        let sweep = (cs as f32 / total as f32) * span;
        if sweep < child_min {
            other += sweep;
        } else {
            visible.push((cid, sweep));
        }
    }
    let child_colors = sibling_colors(
        ctx,
        &visible.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        Some(color),
        false,
    );
    let mut a = a0;
    let gap = if visible.len() > 1 {
        SIBLING_GAP * 0.6
    } else {
        0.0
    };
    for (i, &(cid, sweep)) in visible.iter().enumerate() {
        if hits.len() >= MAX_ARCS {
            break;
        }
        let ccol = child_colors.get(i).copied().unwrap_or(color);
        let ca0 = a + gap * 0.5;
        let ca1 = a + sweep - gap * 0.5;
        if ca1 > ca0 {
            paint_subtree(
                ctx,
                cid,
                depth + 1,
                ca0,
                ca1,
                hole,
                ring_w,
                center,
                ccol,
                min_sweep,
                mesh,
                hits,
            );
        }
        a += sweep;
    }
    if other >= child_min && hits.len() < MAX_ARCS {
        let inner = hole + (depth + 1) as f32 * ring_w;
        push_segment(
            mesh,
            hits,
            center,
            inner,
            inner + ring_w - RING_GAP,
            a,
            a + other,
            Color32::from_rgb(48, 52, 58),
            None,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn push_segment(
    mesh: &mut Mesh,
    hits: &mut Vec<ArcHit>,
    center: Pos2,
    inner: f32,
    outer: f32,
    a0: f32,
    a1: f32,
    color: Color32,
    id: Option<NodeId>,
) {
    push_segment_mesh(mesh, center, inner, outer, a0, a1, color);
    hits.push(ArcHit {
        id,
        inner,
        outer,
        a0,
        a1,
    });
}

fn push_segment_mesh(
    mesh: &mut Mesh,
    center: Pos2,
    inner: f32,
    outer: f32,
    a0: f32,
    a1: f32,
    color: Color32,
) {
    let sweep = (a1 - a0).abs();
    let steps = ((sweep * outer / PX_PER_STEP).ceil() as usize).clamp(4, 72);

    for i in 0..steps {
        let t0 = i as f32 / steps as f32;
        let t1 = (i + 1) as f32 / steps as f32;
        let aa = a0 + sweep * t0;
        let ab = a0 + sweep * t1;
        let p0 = center + Vec2::angled(aa) * outer;
        let p1 = center + Vec2::angled(ab) * outer;
        let p2 = center + Vec2::angled(ab) * inner;
        let p3 = center + Vec2::angled(aa) * inner;
        let idx = mesh.vertices.len() as u32;
        mesh.colored_vertex(p0, color);
        mesh.colored_vertex(p1, color);
        mesh.colored_vertex(p2, color);
        mesh.colored_vertex(p3, color);
        mesh.add_triangle(idx, idx + 1, idx + 2);
        mesh.add_triangle(idx, idx + 2, idx + 3);
    }
}

fn point_in_arc(pos: Pos2, center: Pos2, hit: &ArcHit) -> bool {
    let d = pos - center;
    let r = d.length();
    if r < hit.inner || r > hit.outer {
        return false;
    }
    let mut a = d.angle();
    let a0 = hit.a0;
    let mut a1 = hit.a1;
    while a < a0 {
        a += std::f32::consts::TAU;
    }
    while a1 < a0 {
        a1 += std::f32::consts::TAU;
    }
    a <= a1 + 1e-4
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}
