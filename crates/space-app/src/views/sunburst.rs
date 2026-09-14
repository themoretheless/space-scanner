//! Sunburst: smooth arcs, geometry cached, cheap overlays.

use std::sync::Arc;

use egui::{
    epaint::Mesh, Align2, Color32, FontId, Pos2, Sense, Shape, Stroke, Ui, Vec2,
};
use space_core::{format_bytes, NodeId};

use crate::theme::{self, ACCENT, MUTED, TEXT};
use crate::views::{child_ids, DrawCtx, GeomCache, GeomKey, ViewAction};

const MAX_DEPTH: usize = 2;
const MIN_SWEEP: f32 = 0.02;
const MAX_ARCS: usize = 160;
const MAX_TOP: usize = 28;
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
            if sweep < MIN_SWEEP {
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
        for (i, &(id, sweep)) in visible.iter().enumerate() {
            if hits.len() >= MAX_ARCS {
                break;
            }
            let color = top_colors.get(i).copied().unwrap_or_else(|| {
                let n = tree.node(id);
                theme::color_for(&n.name, n.is_dir)
            });
            paint_subtree(
                ctx,
                id,
                0,
                angle,
                angle + sweep,
                hole,
                ring_w,
                center,
                color,
                &mut mesh,
                &mut hits,
            );
            angle += sweep;
        }
        if other >= MIN_SWEEP && hits.len() < MAX_ARCS {
            push_segment(
                &mut mesh,
                &mut hits,
                center,
                hole,
                hole + ring_w - 1.5,
                angle,
                angle + other,
                Color32::from_rgb(70, 74, 80),
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

    let pointer = response.hover_pos();
    let mut hover_id = None;
    if let Some(pos) = pointer {
        for hit in hits.iter().rev() {
            if point_in_arc(pos, center, hit) {
                hover_id = hit.id;
                stroke_wedge(&painter, center, hit, Stroke::new(1.6_f32, ACCENT));
                break;
            }
        }
    }
    if let Some(sel) = ctx.selected {
        if let Some(hit) = hits.iter().find(|h| h.id == Some(sel)) {
            stroke_wedge(&painter, center, hit, Stroke::new(2.2_f32, ACCENT));
        }
    }

    let focus_node = tree.node(focus);
    painter.circle_filled(center, hole - 2.0, theme::BG);
    painter.text(
        center + Vec2::new(0.0, -10.0),
        Align2::CENTER_CENTER,
        truncate(&focus_node.name, 18),
        FontId::proportional(15.0),
        TEXT,
    );
    painter.text(
        center + Vec2::new(0.0, 10.0),
        Align2::CENTER_CENTER,
        format_bytes(focus_node.size(ctx.mode)),
        FontId::proportional(12.0),
        MUTED,
    );

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
                            action.select = Some(id);
                            if tree.node(id).is_dir {
                                action.navigate = Some(id);
                            }
                        }
                        break;
                    }
                }
            }
        }
    }
    if response.secondary_clicked() {
        if let Some(id) = hover_id {
            action.toggle_review = Some(id);
        }
    }

    action.hover = hover_id;
    if let Some(id) = hover_id {
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
        // Light overlay under the map — avoids a heavy tooltip covering the sunburst.
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 12.0, rect.bottom() - 28.0),
            egui::vec2(rect.width() - 24.0, 22.0),
        );
        painter.rect_filled(bar, 4.0, theme::PANEL_2);
        painter.text(
            bar.left_center() + Vec2::new(10.0, 0.0),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(13.0),
            TEXT,
        );
        if n.is_dir {
            painter.text(
                bar.right_center() - Vec2::new(10.0, 0.0),
                Align2::RIGHT_CENTER,
                format!("{} files", n.file_count),
                FontId::proportional(11.0),
                MUTED,
            );
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
    let mut out = Vec::with_capacity(ids.len());
    for (i, &id) in ids.iter().enumerate() {
        let n = ctx.tree.node(id);
        let mut avoid: Vec<Color32> = Vec::with_capacity(3);
        if let Some(p) = parent {
            avoid.push(p);
        }
        if let Some(&prev) = out.last() {
            avoid.push(prev);
        }
        out.push(theme::color_for_adjacent(&n.name, n.is_dir, i, &avoid));
    }
    if circular && out.len() >= 2 {
        let first = out[0];
        let prev = out[out.len() - 2];
        let last_i = out.len() - 1;
        let last_id = ids[last_i];
        let n = ctx.tree.node(last_id);
        let mut avoid = vec![prev, first];
        if let Some(p) = parent {
            avoid.push(p);
        }
        out[last_i] = theme::color_for_adjacent(&n.name, n.is_dir, last_i, &avoid);
    }
    out
}

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
    mesh: &mut Mesh,
    hits: &mut Vec<ArcHit>,
) {
    if depth >= MAX_DEPTH || (a1 - a0) < MIN_SWEEP || hits.len() >= MAX_ARCS {
        return;
    }

    let tree = ctx.tree;
    let node = tree.node(id);
    let inner = hole + depth as f32 * ring_w;
    let outer = inner + ring_w - 1.5;
    let shade = 1.0 - depth as f32 * 0.08;
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
    let take = kids.iter().copied().take(20).collect::<Vec<_>>();
    let span = a1 - a0;
    let total = take
        .iter()
        .map(|c| tree.node(*c).size(ctx.mode))
        .sum::<u64>()
        .max(1);
    let mut visible: Vec<(NodeId, f32)> = Vec::new();
    let mut other = 0.0_f32;
    for &cid in &take {
        let cs = tree.node(cid).size(ctx.mode);
        let sweep = (cs as f32 / total as f32) * span;
        if sweep < MIN_SWEEP {
            other += sweep;
        } else {
            visible.push((cid, sweep));
        }
    }
    let circular = span > std::f32::consts::TAU * 0.92;
    let child_colors = sibling_colors(
        ctx,
        &visible.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        Some(color),
        circular,
    );
    let mut a = a0;
    for (i, &(cid, sweep)) in visible.iter().enumerate() {
        if hits.len() >= MAX_ARCS {
            break;
        }
        let ccol = child_colors.get(i).copied().unwrap_or(color);
        paint_subtree(
            ctx,
            cid,
            depth + 1,
            a,
            a + sweep,
            hole,
            ring_w,
            center,
            ccol,
            mesh,
            hits,
        );
        a += sweep;
    }
    if other >= MIN_SWEEP && hits.len() < MAX_ARCS {
        let inner = hole + (depth + 1) as f32 * ring_w;
        push_segment(
            mesh,
            hits,
            center,
            inner,
            inner + ring_w - 1.5,
            a,
            a + other,
            Color32::from_rgb(70, 74, 80),
            None,
        );
    }
}

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
    let sweep = (a1 - a0).abs();
    // Tessellate by outer arc length so large wedges stay round (old cap of 10 made them polygonal).
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

    hits.push(ArcHit {
        id,
        inner,
        outer,
        a0,
        a1,
    });
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
