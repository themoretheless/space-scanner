//! Fast sunburst: 2 rings, single mesh, geometry cached across frames.

use std::sync::Arc;

use egui::{
    epaint::Mesh, Align2, Color32, FontId, Pos2, Sense, Shape, Stroke, Ui, Vec2,
};
use space_core::{format_bytes, NodeId};

use crate::theme::{self, ACCENT, MUTED, TEXT};
use crate::views::{child_ids, DrawCtx, GeomCache, GeomKey, ViewAction};

const MAX_DEPTH: usize = 2;
const MIN_SWEEP: f32 = 0.025;
const MAX_ARCS: usize = 200;
const MAX_TOP: usize = 36;

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

    // Selection/review drawn as overlays — omit from key so we don't rebuild mesh.
    let key = GeomKey {
        view: 0,
        focus,
        mode: ctx.mode,
        filter: ctx.filter_hash,
        selected: None,
        w: rect.width() as u16,
        h: rect.height() as u16,
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
        let total = kids
            .iter()
            .take(MAX_TOP)
            .map(|id| tree.node(*id).size(ctx.mode))
            .sum::<u64>()
            .max(1);

        let mut angle = -std::f32::consts::FRAC_PI_2;
        let mut other = 0.0_f32;
        for &id in kids.iter().take(MAX_TOP) {
            if hits.len() >= MAX_ARCS {
                break;
            }
            let size = tree.node(id).size(ctx.mode);
            let sweep = (size as f32 / total as f32) * std::f32::consts::TAU;
            if sweep < MIN_SWEEP {
                other += sweep;
                continue;
            }
            paint_subtree(
                ctx,
                id,
                0,
                angle,
                angle + sweep,
                hole,
                ring_w,
                center,
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
                hole + ring_w - 1.2,
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

    // Selection / hover outline without rebuilding mesh.
    let pointer = response.hover_pos();
    let mut hover_id = None;
    if let Some(pos) = pointer {
        for hit in hits.iter().rev() {
            if point_in_arc(pos, center, hit) {
                hover_id = hit.id;
                if let Some(id) = hit.id {
                    let _ = id;
                    painter.circle_stroke(
                        center,
                        (hit.inner + hit.outer) * 0.5,
                        Stroke::new(1.5_f32, ACCENT.gamma_multiply(0.6)),
                    );
                }
                break;
            }
        }
    }
    if let Some(sel) = ctx.selected {
        if let Some(hit) = hits.iter().find(|h| h.id == Some(sel)) {
            painter.circle_stroke(
                center,
                (hit.inner + hit.outer) * 0.5,
                Stroke::new(2.0_f32, ACCENT),
            );
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

    let _ = hover_id;
    action
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
    mesh: &mut Mesh,
    hits: &mut Vec<ArcHit>,
) {
    if depth >= MAX_DEPTH || (a1 - a0) < MIN_SWEEP || hits.len() >= MAX_ARCS {
        return;
    }

    let tree = ctx.tree;
    let node = tree.node(id);
    let inner = hole + depth as f32 * ring_w;
    let outer = inner + ring_w - 1.2;
    let mut color = theme::color_for(&node.name, node.is_dir);
    let shade = 1.0 - depth as f32 * 0.08;
    color = Color32::from_rgb(
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
    let total = kids
        .iter()
        .take(28)
        .map(|c| tree.node(*c).size(ctx.mode))
        .sum::<u64>()
        .max(1);
    let span = a1 - a0;
    let mut a = a0;
    let mut other = 0.0_f32;
    for &cid in kids.iter().take(28) {
        if hits.len() >= MAX_ARCS {
            break;
        }
        let cs = tree.node(cid).size(ctx.mode);
        let sweep = (cs as f32 / total as f32) * span;
        if sweep < MIN_SWEEP {
            other += sweep;
            continue;
        }
        paint_subtree(ctx, cid, depth + 1, a, a + sweep, hole, ring_w, center, mesh, hits);
        a += sweep;
    }
    if other >= MIN_SWEEP && hits.len() < MAX_ARCS {
        let inner = hole + (depth + 1) as f32 * ring_w;
        push_segment(
            mesh,
            hits,
            center,
            inner,
            inner + ring_w - 1.2,
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
    let steps = ((sweep / 0.22).ceil() as usize).clamp(2, 10);

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
