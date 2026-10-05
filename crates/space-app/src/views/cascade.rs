//! Cascade (icicle): stacked horizontal bands by depth, width ∝ size.

use egui::{pos2, Align2, FontId, Rect, Sense, Stroke, Ui, Vec2};
use space_core::NodeId;

use crate::theme::{self, MUTED, TEXT};
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
    painter.rect_filled(rect, 4.0, theme::PANEL());

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
        color: theme::PANEL_2(),
    });

    layout_children(
        ctx,
        ctx.focus,
        focus_rect,
        theme::PANEL_2(),
        1,
        max_depth,
        row_h,
        &mut bands,
    );

    if bands.len() == 1 && ctx.filter.is_active() {
        super::empty_filter_prompt(ui, rect, &mut action);
        return action;
    }

    for band in &bands {
        let node = tree.node(band.id);
        let color = if band.depth == 0 {
            theme::PANEL_2()
        } else {
            band.color.gamma_multiply(1.0 - band.depth as f32 * 0.08)
        };
        let is_hov = ctx.hovered == Some(band.id);
        let is_sel = ctx.selected == Some(band.id) || ctx.review.contains(&band.id);
        let hov_t =
            ui.ctx()
                .animate_bool_with_time(ui.id().with(("cas_hov", band.id.0)), is_hov, 0.14);
        let sel_t =
            ui.ctx()
                .animate_bool_with_time(ui.id().with(("cas_sel", band.id.0)), is_sel, 0.18);
        let draw = band.rect.shrink(1.0);
        painter.rect_filled(draw, 3.0, color);
        if hov_t > 0.01 {
            painter.rect_filled(
                draw,
                3.0,
                crate::theme::ACCENT().gamma_multiply(hov_t * 0.14),
            );
        }
        painter.rect_stroke(
            draw,
            3.0,
            Stroke::new(
                1.0 + sel_t + hov_t * 0.8,
                if sel_t > 0.05 || hov_t > 0.05 {
                    crate::theme::ACCENT().gamma_multiply(((70.0 + sel_t * 170.0 + hov_t * 120.0) / 255.0).min(1.0))
                } else {
                    theme::BG()
                },
            ),
            egui::StrokeKind::Inside,
        );

        if band.rect.width() > 40.0 {
            painter.text(
                band.rect.left_center() + Vec2::new(8.0, -6.0),
                Align2::LEFT_CENTER,
                truncate(&node.name, ((band.rect.width() / 7.0) as usize).max(4)),
                FontId::proportional(12.0),
                TEXT(),
            );
            painter.text(
                band.rect.left_center() + Vec2::new(8.0, 8.0),
                Align2::LEFT_CENTER,
                if band.depth == 0 && node.parent.is_some() {
                    format!(
                        "{}  ·  click to go up",
                        crate::i18n::bytes(node.size(ctx.mode))
                    )
                } else {
                    crate::i18n::bytes(node.size(ctx.mode))
                },
                FontId::proportional(11.0),
                MUTED(),
            );
        }

        let resp = ui.interact(band.rect, ui.id().with(("cas", band.id.0)), Sense::click());
        if resp.clicked() {
            super::click_map_node(
                ctx.tree,
                band.id,
                ctx.focus,
                ui.input(|i| i.modifiers.command || i.modifiers.ctrl),
                &mut action,
            );
        }
        if resp.double_clicked() {
            super::double_click_map_node(ctx.tree, band.id, &mut action);
        }
        let in_q = ctx.review.contains(&band.id);
        super::node_context_menu(&resp, ctx.tree, band.id, in_q, &mut action);
        if resp.hovered() {
            action.hover = Some(band.id);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            resp.on_hover_ui(|ui| super::node_hover_ui(ui, ctx.tree, band.id, ctx.focus, ctx.mode));
        }
    }

    action
}

#[allow(clippy::too_many_arguments)]
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
