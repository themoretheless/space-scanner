//! Shared widgets. Every screen is built from these so type, spacing, radii and
//! states stay consistent; nothing here is specific to one page.

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Ui};

use crate::i18n::t;
use crate::theme::metrics::{CONTROL_H, RADIUS_CARD, RADIUS_CONTROL, S2, S3, S4};
use crate::theme::type_scale;
use crate::theme::{ACCENT, ACCENT_INK, BORDER, MUTED, PANEL, PANEL_2, TEXT};

/// Width of `text` in `font`, for fitting breadcrumbs and labels without wrapping.
pub fn text_width(ui: &Ui, text: &str, font: egui::FontId) -> f32 {
    ui.fonts(|f| {
        f.layout_no_wrap(text.to_owned(), font, Color32::WHITE)
            .size()
            .x
    })
}

/// Secondary text at caption size.
pub fn caption(text: impl Into<String>) -> RichText {
    let text: String = text.into();
    RichText::new(t(&text).to_owned())
        .size(type_scale::CAPTION)
        .color(MUTED)
}

/// Filled accent button — the one primary action on a screen.
pub fn primary(label: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(t(label)).color(ACCENT_INK).strong())
        .fill(ACCENT)
        .corner_radius(RADIUS_CONTROL)
        .min_size(egui::vec2(0.0, CONTROL_H))
}

/// Bordered neutral button.
pub fn secondary(label: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(t(label)).color(TEXT))
        .fill(PANEL_2)
        .stroke(egui::Stroke::new(1.0_f32, BORDER))
        .corner_radius(RADIUS_CONTROL)
        .min_size(egui::vec2(0.0, CONTROL_H))
}

/// Frameless text button for quiet actions.
pub fn ghost(label: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(t(label)).color(MUTED))
        .frame(false)
        .min_size(egui::vec2(0.0, CONTROL_H))
}

/// Make a button span the available width.
pub fn block(ui: &Ui, button: egui::Button<'static>) -> egui::Button<'static> {
    button.min_size(egui::vec2(ui.available_width(), CONTROL_H))
}

/// Filter pill: filled when on, outlined when off.
pub fn chip(ui: &mut Ui, on: bool, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(
            RichText::new(t(label))
                .size(type_scale::CAPTION)
                .color(if on { crate::theme::BG } else { MUTED }),
        )
        .fill(if on { TEXT } else { Color32::TRANSPARENT })
        .stroke(if on {
            egui::Stroke::NONE
        } else {
            egui::Stroke::new(1.0_f32, BORDER)
        })
        .corner_radius(14)
        .min_size(egui::vec2(0.0, 28.0)),
    )
}

/// Mutually exclusive choices in one outlined group. Returns the picked index.
pub fn segmented(ui: &mut Ui, labels: &[&str], current: usize) -> Option<usize> {
    let mut picked = None;
    egui::Frame::new()
        .fill(PANEL)
        .stroke(egui::Stroke::new(1.0_f32, BORDER))
        .corner_radius(9)
        .inner_margin(egui::Margin::same(3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                for (i, label) in labels.iter().enumerate() {
                    let on = i == current;
                    let button = egui::Button::new(RichText::new(t(label)).color(if on {
                        TEXT
                    } else {
                        MUTED
                    }))
                    .fill(if on { PANEL_2 } else { Color32::TRANSPARENT })
                    .corner_radius(7)
                    .min_size(egui::vec2(0.0, 28.0));
                    if ui.add(button).clicked() {
                        picked = Some(i);
                    }
                }
            });
        });
    picked
}

/// Bordered surface used for cards on the Scan page and Review.
pub fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(egui::Stroke::new(1.0_f32, BORDER))
        .corner_radius(RADIUS_CARD)
        .inner_margin(egui::Margin::same(S4 as i8 + 2))
}

/// A card whose content is laid out top-down regardless of the parent's layout
/// (frames inherit the layout of the ui they are added to).
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    card_frame().show(ui, |ui| ui.vertical(add).inner).inner
}

/// Small outlined label, e.g. a risk class.
pub fn badge(ui: &mut Ui, text: &str, color: Color32, hint: &str) {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0_f32, color.gamma_multiply(0.5)))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(
                RichText::new(t(text))
                    .size(type_scale::CAPTION)
                    .color(color),
            );
        })
        .response
        .on_hover_text(t(hint));
}

/// Big number with a caption underneath.
pub fn stat(ui: &mut Ui, value: &str, caption_text: &str) {
    egui::Frame::new()
        .fill(PANEL_2)
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(S3 as i8, S3 as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // `ui.columns` justifies its cells; a plain vertical layout keeps wrapped text left-aligned.
            ui.vertical(|ui| {
                ui.label(RichText::new(value).size(type_scale::TITLE).strong());
                ui.label(caption(caption_text));
            });
        });
}

/// Thin proportion bar.
pub fn meter(ui: &mut Ui, frac: f32, fill: Color32, height: f32) {
    let (bar, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    ui.painter().rect_filled(bar, height * 0.5, PANEL_2);
    let mut part = bar;
    part.set_width((bar.width() * frac.clamp(0.0, 1.0)).max(if frac > 0.0 { height } else { 0.0 }));
    ui.painter().rect_filled(part, height * 0.5, fill);
}

/// A centered column no wider than `max_w`, with a side gutter.
pub fn page_column<R>(ui: &mut Ui, max_w: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let avail = ui.available_width();
    let w = (avail - 2.0 * S4).min(max_w).max(0.0);
    let pad = ((avail - w) * 0.5).max(0.0);
    ui.horizontal_top(|ui| {
        ui.add_space(pad);
        ui.vertical(|ui| {
            ui.set_width(w);
            add(ui)
        })
        .inner
    })
    .inner
}

/// Section heading with an optional right-aligned action.
pub fn section_header(ui: &mut Ui, title: &str, right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(t(title)).size(type_scale::LEAD).strong());
        ui.with_layout(Layout::right_to_left(Align::Center), right);
    });
    ui.add_space(S2);
}

/// Hairline divider.
pub fn divider(ui: &mut Ui) {
    let (line, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(line, 0.0, BORDER);
}

/// Centered message for empty or blocked states, with an optional action.
pub fn empty_state(ui: &mut Ui, title: &str, body: &str) {
    ui.add_space(S4 * 2.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(t(title)).size(type_scale::LEAD).strong());
        ui.add_space(S1_5);
        ui.label(caption(body));
    });
}

const S1_5: f32 = 6.0;

/// "1 item" / "3 items" (or the Russian forms).
pub fn count_noun(n: usize, noun: &str) -> String {
    crate::i18n::count(n, noun)
}

/// A full-width row: path on the left (shortened with an ellipsis, full path on hover),
/// size on the right. `reserve_right` leaves room for a trailing control.
/// Returns the row rect.
pub fn path_size_row(
    ui: &mut Ui,
    path: &str,
    size_text: &str,
    height: f32,
    reserve_right: f32,
) -> egui::Rect {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter();
    let size = painter.layout_no_wrap(
        size_text.to_owned(),
        egui::FontId::monospace(type_scale::MONO),
        MUTED,
    );
    let size_x = rect.right() - reserve_right - size.size().x;
    let mut job = egui::text::LayoutJob::simple_singleline(
        path.to_owned(),
        egui::FontId::monospace(type_scale::MONO),
        TEXT,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((size_x - rect.left() - S3).max(40.0));
    let path_g = painter.layout_job(job);
    let y = rect.center().y;
    painter.galley(
        egui::pos2(rect.left(), y - path_g.size().y * 0.5),
        path_g,
        TEXT,
    );
    painter.galley(egui::pos2(size_x, y - size.size().y * 0.5), size, MUTED);
    ui.interact(
        egui::Rect::from_min_max(rect.min, egui::pos2(size_x, rect.max.y)),
        ui.id().with(("path_row", path)),
        Sense::hover(),
    )
    .on_hover_text(path);
    rect
}

/// Age as a short phrase: "3 h ago", "yesterday".
pub fn format_age(secs: u64) -> String {
    crate::i18n::age(secs)
}
