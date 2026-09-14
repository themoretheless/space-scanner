//! Cascade (icicle): stacked horizontal bands by depth, width ∝ size.

use egui::{pos2, Align2, FontId, Rect, Sense, Stroke, Ui, Vec2};
use space_core::{format_bytes, NodeId};

use crate::theme::{self, ACCENT, MUTED, TEXT};
use crate::views::{child_ids, DrawCtx, ViewAction};

struct Band {
    id: NodeId,
    rect: Rect,
    depth: usize,
    color: egui::Color32,
}

pub fn draw(ui: &mut Ui, ctx: &DrawCtx<'_>) -> ViewAction {
    let mut action = ViewAction::none();
    let tree = ctx.tree;
    let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::PANEL);

    let max_depth = 5usize;
    let row_h = ((rect.height() - 8.0) / max_depth as f32).clamp(28.0, 72.0);
    let mut bands = Vec::new();

    let focus_rect = Rect::from_min_size(
        pos2(rect.left() + 4.0, rect.top() + 4.0),
        Vec2::new(rect.width() - 8.0, row_h - 4.0),
    );
    bands.push(Band {
        id: ctx.focus,
        rect: focus_rect,
        depth: 0,
        color: theme::PANEL_2,
    });

    layout_children(
        ctx,
        ctx.focus,
        focus_rect,
        theme::PANEL_2,
        1,
        max_depth,
        row_h,
        &mut bands,
    );

    for band in &bands {
        let node = tree.node(band.id);
        let color = if band.depth == 0 {
            theme::PANEL_2
        } else {
            band.color.gamma_multiply(1.0 - band.depth as f32 * 0.08)
        };
        let is_sel = ctx.selected == Some(band.id)
            || ctx.hovered == Some(band.id)
            || ctx.review.contains(&band.id);
        painter.rect_filled(band.rect.shrink(1.0), 3.0, color);
        painter.rect_stroke(
            band.rect.shrink(1.0),
            3.0,
            Stroke::new(
                if is_sel { 2.0_f32 } else { 1.0_f32 },
                if is_sel { ACCENT } else { theme::BG },
            ),
            egui::StrokeKind::Inside,
        );

        if band.rect.width() > 40.0 {
            painter.text(
                band.rect.left_center() + Vec2::new(8.0, -6.0),
                Align2::LEFT_CENTER,
                truncate(&node.name, ((band.rect.width() / 7.0) as usize).max(4)),
                FontId::proportional(12.0),
                TEXT,
            );
            painter.text(
                band.rect.left_center() + Vec2::new(8.0, 8.0),
                Align2::LEFT_CENTER,
                format_bytes(node.size(ctx.mode)),
                FontId::proportional(11.0),
                MUTED,
            );
        }

        let resp = ui.interact(band.rect, ui.id().with(("cas", band.id.0)), Sense::click());
        if resp.clicked() {
            action.select = Some(band.id);
        }
        if resp.double_clicked() && node.is_dir {
            action.navigate = Some(band.id);
        }
        if resp.secondary_clicked() {
            action.toggle_review = Some(band.id);
        }
        if resp.hovered() {
            action.hover = Some(band.id);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            resp.on_hover_ui(|ui| {
                ui.label(egui::RichText::new(&node.name).strong());
                ui.label(format!(
                    "{} · {} files{}",
                    format_bytes(node.size(ctx.mode)),
                    node.file_count,
                    if node.is_dir { " · folder" } else { "" }
                ));
            });
        }
    }

    action
}

fn layout_children(
    ctx: &DrawCtx<'_>,
    parent: NodeId,
    parent_rect: Rect,
    parent_color: egui::Color32,
    depth: usize,
    max_depth: usize,
    row_h: f32,
    out: &mut Vec<Band>,
) {
    if depth >= max_depth {
        return;
    }
    let kids = child_ids(ctx, parent);
    if kids.is_empty() {
        return;
    }
    let total = kids
        .iter()
        .map(|id| ctx.tree.node(*id).size(ctx.mode))
        .sum::<u64>()
        .max(1);
    let y = parent_rect.top() + row_h;
    let mut x = parent_rect.left();
    let width = parent_rect.width();

    let mut prev_color: Option<egui::Color32> = None;
    for (i, &id) in kids.iter().take(64).enumerate() {
        let frac = ctx.tree.node(id).size(ctx.mode) as f32 / total as f32;
        let w = (width * frac).max(2.0);
        let r = Rect::from_min_size(pos2(x, y), Vec2::new(w - 2.0, row_h - 4.0));
        let node = ctx.tree.node(id);
        let mut avoid = vec![parent_color];
        if let Some(p) = prev_color {
            avoid.push(p);
        }
        let color = theme::color_for_adjacent(&node.name, node.is_dir, i, &avoid);
        prev_color = Some(color);
        out.push(Band {
            id,
            rect: r,
            depth,
            color,
        });
        if node.is_dir && w > 24.0 {
            layout_children(ctx, id, r, color, depth + 1, max_depth, row_h, out);
        }
        x += w;
        if x > parent_rect.right() {
            break;
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}
