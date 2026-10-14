//! Cascade (icicle): stacked horizontal bands by depth, width ∝ size.

use egui::{pos2, Align2, Color32, FontId, Rect, Sense, Ui, Vec2};
use space_core::NodeId;

use crate::theme::type_scale::CAPTION;
use crate::theme::{self, MUTED, TEXT};
use crate::views::{child_ids, truncate, DrawCtx, GeomCache, GeomKey, ViewAction};

const MAX_DEPTH: usize = 5;

struct Band {
    id: NodeId,
    rect: Rect,
    depth: usize,
    color: Color32,
}

pub fn draw(ui: &mut Ui, ctx: &mut DrawCtx<'_>) -> ViewAction {
    let mut action = ViewAction::none();
    let tree = ctx.tree;
    let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::PANEL());

    let key = GeomKey::new(3, ctx, rect);
    let need = match ctx.geom.as_ref() {
        Some(GeomCache::Cascade { key: k, .. }) => *k != key,
        _ => true,
    };
    if need {
        let bands = layout(ctx, rect);
        *ctx.geom = Some(GeomCache::Cascade {
            key,
            bands: bands
                .into_iter()
                .map(|b| (b.id, b.rect, b.depth, b.color))
                .collect(),
        });
    }
    let bands: &[(NodeId, Rect, usize, Color32)] = match ctx.geom.as_ref() {
        Some(GeomCache::Cascade { bands, .. }) => bands.as_slice(),
        _ => &[],
    };

    if bands.len() <= 1 && ctx.filter.is_active() {
        super::empty_filter_prompt(ui, rect, &mut action);
        return action;
    }

    let center = rect.center();
    let scale = ctx.scale;
    for &(id, band_rect, depth, base) in bands {
        let node = tree.node(id);
        let band_rect = super::zoomed_rect(center, band_rect, scale);
        let color = if depth == 0 {
            theme::PANEL_2()
        } else {
            base.gamma_multiply(1.0 - depth as f32 * 0.08)
        };
        let is_hov = ctx.hovered == Some(id);
        let is_sel = ctx.selected == Some(id) || ctx.review.contains(&id);
        let (hov_t, sel_t) = super::highlight(ui, "cas", Some(id), is_hov, is_sel);
        let draw = band_rect.shrink(1.0);
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
            super::highlight_stroke(hov_t, sel_t, theme::BG()),
            egui::StrokeKind::Inside,
        );

        if band_rect.width() > 40.0 {
            painter.text(
                band_rect.left_center() + Vec2::new(8.0, -7.0),
                Align2::LEFT_CENTER,
                truncate(&node.name, ((band_rect.width() / 7.0) as usize).max(4)),
                FontId::proportional(CAPTION),
                TEXT(),
            );
            painter.text(
                band_rect.left_center() + Vec2::new(8.0, 8.0),
                Align2::LEFT_CENTER,
                if depth == 0 && node.parent.is_some() {
                    format!(
                        "{}  ·  {}",
                        crate::i18n::bytes(node.size(ctx.mode)),
                        crate::i18n::t("click to go up")
                    )
                } else {
                    crate::i18n::bytes(node.size(ctx.mode))
                },
                FontId::proportional(CAPTION),
                MUTED(),
            );
        }

        let resp = ui.interact(band_rect, ui.id().with(("cas", id.0)), Sense::click());
        let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
        if depth == 0 && resp.clicked() && !cmd {
            // The focus band is the way up, as its label promises.
            if let Some(parent) = node.parent {
                action.navigate = Some(parent);
            } else {
                action.select = Some(id);
            }
            continue;
        }
        super::node_response(ui, &resp, ctx, id, &mut action);
    }

    action
}

fn layout(ctx: &DrawCtx<'_>, rect: Rect) -> Vec<Band> {
    let row_h = ((rect.height() - 8.0) / MAX_DEPTH as f32).clamp(28.0, 72.0);
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
    layout_children(ctx, ctx.focus, focus_rect, theme::PANEL_2(), 1, row_h, &mut bands);
    bands
}

fn layout_children(
    ctx: &DrawCtx<'_>,
    parent: NodeId,
    parent_rect: Rect,
    parent_color: Color32,
    depth: usize,
    row_h: f32,
    out: &mut Vec<Band>,
) {
    if depth >= MAX_DEPTH {
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

    let mut prev_color: Option<Color32> = None;
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
            layout_children(ctx, id, r, color, depth + 1, row_h, out);
        }
        x += w;
        if x > parent_rect.right() {
            break;
        }
    }
}
