//! Orbital bubbles with LOD (top N children), cached layout geometry.

use egui::{Align2, FontId, Sense, Stroke, Ui, Vec2};

use crate::theme::{self, ACCENT, MUTED, TEXT};
use crate::views::{
    child_ids, quantize_pos, quantize_px, theme_bits, truncate, DrawCtx, GeomCache, GeomKey,
    RadialBubble, ViewAction,
};

const MAX_NODES: usize = 36;

pub fn draw(ui: &mut Ui, ctx: &mut DrawCtx<'_>) -> ViewAction {
    let mut action = ViewAction::none();
    let tree = ctx.tree;
    let focus = ctx.focus;
    let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    let painter = ui.painter_at(rect);

    let key = GeomKey {
        view: 2,
        focus,
        mode: ctx.mode,
        filter: ctx.filter_hash,
        selected: None,
        x: quantize_pos(rect.left()),
        y: quantize_pos(rect.top()),
        w: quantize_px(rect.width()),
        h: quantize_px(rect.height()),
        review_sig: 0,
        view_opts: theme_bits(),
    };

    let need_rebuild = match ctx.geom.as_ref() {
        Some(GeomCache::Radial { key: k, .. }) => *k != key,
        _ => true,
    };

    if need_rebuild {
        let center = rect.center();
        let kids = child_ids(ctx, focus);
        let max_size = kids
            .iter()
            .map(|id| tree.node(*id).size(ctx.mode))
            .max()
            .unwrap_or(1)
            .max(1);

        let orbit_r = (rect.width().min(rect.height()) * 0.32).clamp(80.0, 220.0);
        let center_r = (orbit_r * 0.38).clamp(36.0, 72.0);
        let show = kids.len().min(MAX_NODES);
        let n = show.max(1) as f32;
        let parent_color = theme::PANEL_2();
        let ring_ids: Vec<_> = kids.iter().copied().take(show).collect();
        let mut colors = Vec::with_capacity(ring_ids.len());
        for (i, &id) in ring_ids.iter().enumerate() {
            let node = tree.node(id);
            let mut avoid = vec![parent_color];
            if let Some(&prev) = colors.last() {
                avoid.push(prev);
            }
            colors.push(theme::color_for_adjacent(
                &node.name,
                node.is_dir,
                i,
                &avoid,
            ));
        }
        if colors.len() >= 2 {
            let first = colors[0];
            let prev = colors[colors.len() - 2];
            let last_i = colors.len() - 1;
            let last_id = ring_ids[last_i];
            let nlast = tree.node(last_id);
            colors[last_i] = theme::color_for_adjacent(
                &nlast.name,
                nlast.is_dir,
                last_i,
                &[parent_color, prev, first],
            );
        }

        let mut bubbles = Vec::with_capacity(ring_ids.len());
        for (i, &id) in ring_ids.iter().enumerate() {
            let node = tree.node(id);
            let angle = (i as f32 / n) * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
            let ratio = (node.size(ctx.mode) as f32 / max_size as f32)
                .sqrt()
                .clamp(0.15, 1.0);
            let r = (14.0 + ratio * 34.0).clamp(12.0, 48.0);
            let dist = orbit_r + (1.0 - ratio) * 28.0;
            let pos = center + Vec2::angled(angle) * dist;
            let color = colors.get(i).copied().unwrap_or(parent_color);
            bubbles.push(RadialBubble { id, pos, r, color });
        }

        *ctx.geom = Some(GeomCache::Radial {
            key,
            bubbles,
            center,
            orbit_r,
            center_r,
        });
    }

    let Some(GeomCache::Radial {
        bubbles,
        center,
        orbit_r,
        center_r,
        ..
    }) = ctx.geom.as_ref()
    else {
        return action;
    };

    if bubbles.is_empty() && ctx.filter.is_active() {
        super::empty_filter_prompt(ui, rect, &mut action);
        return action;
    }

    let center = *center;
    let orbit_r = *orbit_r;
    let center_r = *center_r;
    let focus_node = tree.node(focus);

    for i in 1..=3 {
        let r = orbit_r * (0.45 + i as f32 * 0.22);
        painter.circle_stroke(
            center,
            r,
            Stroke::new(1.0_f32, theme::BORDER().gamma_multiply(0.6)),
        );
    }

    painter.circle_filled(center, center_r, theme::PANEL_2());
    painter.circle_stroke(center, center_r, Stroke::new(2.0_f32, ACCENT()));
    painter.text(
        center + Vec2::new(0.0, -8.0),
        Align2::CENTER_CENTER,
        truncate(&focus_node.name, 18),
        FontId::proportional(16.0),
        TEXT(),
    );
    painter.text(
        center + Vec2::new(0.0, 12.0),
        Align2::CENTER_CENTER,
        if focus_node.parent.is_some() {
            format!("{}  ·  ↑", crate::i18n::bytes(focus_node.size(ctx.mode)))
        } else {
            crate::i18n::bytes(focus_node.size(ctx.mode))
        },
        FontId::proportional(13.0),
        MUTED(),
    );

    let t_hov = if theme::reduce_motion() { 0.0 } else { 0.16 };
    let t_sel = if theme::reduce_motion() { 0.0 } else { 0.2 };
    for (i, bubble) in bubbles.iter().enumerate() {
        let id = bubble.id;
        let node = tree.node(id);
        let is_hov = ctx.hovered == Some(id);
        let is_sel =
            ctx.selected == Some(id) || ctx.selected_set.contains(&id) || ctx.review.contains(&id);
        let hov_t = ui
            .ctx()
            .animate_bool_with_time(ui.id().with(("rad_hov", id.0)), is_hov, t_hov);
        let sel_t = ui
            .ctx()
            .animate_bool_with_time(ui.id().with(("rad_sel", id.0)), is_sel, t_sel);
        let r = bubble.r * (1.0 + hov_t * 0.12);
        painter.circle_filled(bubble.pos, r, bubble.color);
        painter.circle_stroke(
            bubble.pos,
            r,
            Stroke::new(
                1.0 + sel_t * 1.5 + hov_t,
                if sel_t > 0.05 || hov_t > 0.05 {
                    crate::theme::ACCENT()
                        .gamma_multiply(((80.0 + sel_t * 160.0 + hov_t * 120.0) / 255.0).min(1.0))
                } else {
                    theme::BORDER()
                },
            ),
        );

        if r >= 22.0 || i < 10 {
            painter.text(
                bubble.pos + Vec2::new(0.0, r + 10.0),
                Align2::CENTER_CENTER,
                format!(
                    "{} {}",
                    truncate(&node.name, 14),
                    crate::i18n::bytes(node.size(ctx.mode))
                ),
                FontId::proportional(theme::type_scale::CAPTION),
                TEXT(),
            );
        }

        let hit = egui::Rect::from_center_size(bubble.pos, Vec2::splat((r * 2.2).max(24.0)));
        let resp = ui.interact(hit, ui.id().with(("radial", id.0)), Sense::click());
        if resp.has_focus() {
            theme::paint_focus_ring(&painter, hit);
        }
        let a11y_name = node.name.clone();
        resp.widget_info(move || {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, a11y_name.as_str())
        });
        if resp.clicked() {
            super::click_map_node(
                tree,
                id,
                focus,
                ui.input(|i| i.modifiers.command || i.modifiers.ctrl),
                ui.input(|i| i.modifiers.shift),
                &mut action,
            );
        }
        if resp.double_clicked() {
            super::double_click_map_node(tree, id, &mut action);
        }
        let in_q = ctx.review.contains(&id);
        super::node_context_menu(&resp, tree, id, in_q, &mut action);
        if resp.hovered() {
            action.hover = Some(id);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            resp.on_hover_ui(|ui| super::node_hover_ui(ui, tree, id, ctx.focus, ctx.mode));
        }
    }

    let center_hit = egui::Rect::from_center_size(center, Vec2::splat((center_r * 2.0).max(24.0)));
    let center_resp = ui.interact(center_hit, ui.id().with("radial-center"), Sense::click());
    if center_resp.has_focus() {
        theme::paint_focus_ring(&painter, center_hit);
    }
    let center_name = focus_node.name.clone();
    center_resp.widget_info(move || {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, center_name.as_str())
    });
    if center_resp.clicked() || center_resp.double_clicked() {
        if let Some(parent) = focus_node.parent {
            action.navigate = Some(parent);
        }
    }
    if center_resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    center_resp.on_hover_text(crate::i18n::t("Click to go up"));

    action
}
