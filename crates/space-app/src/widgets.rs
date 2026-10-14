//! Shared widgets. Every screen is built from these so type, spacing, radii and
//! states stay consistent; nothing here is specific to one page.

use eframe::egui::{self, Color32, RichText, Sense, Ui};

use crate::i18n::t;
use crate::theme::metrics::{CHIP_H, CONTROL_H, RADIUS_CONTROL, S3, S4};
use crate::theme::type_scale;
use crate::theme::{ACCENT, ACCENT_INK, BORDER, MUTED, PANEL_2, TEXT};

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
        .color(MUTED())
}

/// Semibold text at `size`, for titles and the numbers people look for first.
pub fn strong(text: impl Into<String>, size: f32) -> RichText {
    let text: String = text.into();
    RichText::new(t(&text).to_owned())
        .size(size)
        .family(crate::theme::bold())
        .color(TEXT())
}

/// Page title (22 pt semibold).
pub fn title(text: &str) -> RichText {
    strong(text, type_scale::PAGE)
}

/// Small uppercase label above a group: "RECENT SCANS".
pub fn section_label(text: &str) -> RichText {
    RichText::new(t(text).to_uppercase())
        .size(type_scale::CAPTION)
        .family(crate::theme::medium())
        .extra_letter_spacing(0.7)
        .color(MUTED())
}

fn label_text(label: &str, color: Color32, family: egui::FontFamily) -> RichText {
    RichText::new(t(label)).color(color).family(family)
}

/// Filled accent button — the one primary action on a screen.
pub fn primary(label: &str) -> egui::Button<'static> {
    egui::Button::new(label_text(label, ACCENT_INK(), crate::theme::bold()))
        .fill(ACCENT())
        .stroke(egui::Stroke::NONE)
        .corner_radius(RADIUS_CONTROL)
        .min_size(egui::vec2(0.0, CONTROL_H))
}

/// Bordered neutral button.
pub fn secondary(label: &str) -> egui::Button<'static> {
    egui::Button::new(label_text(label, TEXT(), crate::theme::medium()))
        .fill(PANEL_2())
        .stroke(egui::Stroke::new(1.0_f32, control_border()))
        .corner_radius(RADIUS_CONTROL)
        .min_size(egui::vec2(0.0, CONTROL_H))
}

/// Destructive, permanent actions only (snapshots). Trash moves stay `primary`.
pub fn danger(label: &str) -> egui::Button<'static> {
    egui::Button::new(label_text(label, Color32::WHITE, crate::theme::bold()))
        .fill(crate::theme::DANGER())
        .stroke(egui::Stroke::NONE)
        .corner_radius(RADIUS_CONTROL)
        .min_size(egui::vec2(0.0, CONTROL_H))
}

/// Frameless text button for quiet actions.
pub fn ghost(label: &str) -> egui::Button<'static> {
    egui::Button::new(label_text(label, MUTED(), crate::theme::medium()))
        .frame(false)
        .min_size(egui::vec2(0.0, CONTROL_H))
}

/// Border of buttons and fields: a step stronger than panel hairlines.
pub fn control_border() -> Color32 {
    BORDER().lerp_to_gamma(MUTED(), 0.22)
}

/// Make a button span the available width.
pub fn block(ui: &Ui, button: egui::Button<'static>) -> egui::Button<'static> {
    button.min_size(egui::vec2(ui.available_width(), CONTROL_H + 4.0))
}

/// Filter pill: filled with the text colour when on, outlined when off.
pub fn chip(ui: &mut Ui, on: bool, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(
            RichText::new(t(label))
                .size(type_scale::MONO)
                .family(crate::theme::medium())
                .color(if on { crate::theme::BG() } else { TEXT() }),
        )
        .fill(if on { TEXT() } else { Color32::TRANSPARENT })
        .stroke(egui::Stroke::new(
            1.0_f32,
            if on { TEXT() } else { BORDER() },
        ))
        .corner_radius(RADIUS_CONTROL)
        .min_size(egui::vec2(0.0, CHIP_H)),
    )
}

/// Mutually exclusive choices in one framed group; the current one is raised.
pub fn segmented(ui: &mut Ui, labels: &[&str], current: usize) -> Option<usize> {
    let mut picked = None;
    group_frame(ui, |ui| {
        for (i, label) in labels.iter().enumerate() {
            let on = i == current;
            let button = egui::Button::new(
                RichText::new(t(label))
                    .size(type_scale::MONO)
                    .family(crate::theme::medium())
                    .color(if on { TEXT() } else { MUTED() }),
            )
            .fill(if on {
                segment_on()
            } else {
                Color32::TRANSPARENT
            })
            .stroke(egui::Stroke::NONE)
            .corner_radius(RADIUS_CONTROL - 1)
            .min_size(egui::vec2(0.0, CHIP_H - 2.0));
            if ui.add(button).clicked() {
                picked = Some(i);
            }
        }
    });
    picked
}

/// The same group with icons; every icon carries a hover label.
pub fn icon_segmented(
    ui: &mut Ui,
    items: &[(crate::icons::Icon, &str)],
    current: usize,
) -> Option<usize> {
    let mut picked = None;
    group_frame(ui, |ui| {
        for (i, (icon, hint)) in items.iter().enumerate() {
            let on = i == current;
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(32.0, CHIP_H - 2.0), Sense::click());
            if on {
                ui.painter()
                    .rect_filled(rect, (RADIUS_CONTROL - 1) as f32, segment_on());
            } else if resp.hovered() {
                ui.painter()
                    .rect_filled(rect, (RADIUS_CONTROL - 1) as f32, PANEL_2());
            }
            crate::icons::paint(
                ui.painter(),
                *icon,
                egui::Rect::from_center_size(rect.center(), egui::vec2(16.0, 16.0)),
                if on || resp.hovered() {
                    TEXT()
                } else {
                    MUTED()
                },
            );
            if resp.on_hover_text(t(hint)).clicked() {
                picked = Some(i);
            }
        }
    });
    picked
}

fn segment_on() -> Color32 {
    if crate::theme::is_dark() {
        BORDER()
    } else {
        crate::theme::PANEL()
    }
}

fn group_frame(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(if crate::theme::is_dark() {
            crate::theme::PANEL()
        } else {
            crate::theme::BG()
        })
        .stroke(egui::Stroke::new(1.0_f32, BORDER()))
        .corner_radius(RADIUS_CONTROL + 1)
        .inner_margin(egui::Margin::same(2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                add(ui);
            });
        });
}

/// Small outlined pill, e.g. a risk class or "Protected". Sized to its text in any layout.
pub fn badge(ui: &mut Ui, text: &str, color: Color32, hint: &str) {
    let g = ui.painter().layout_no_wrap(
        t(text).to_owned(),
        egui::FontId::proportional(type_scale::CAPTION),
        color,
    );
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(g.size().x + 18.0, 22.0), Sense::hover());
    ui.painter().rect_stroke(
        rect,
        11.0,
        egui::Stroke::new(1.0_f32, color.gamma_multiply(0.55)),
        egui::StrokeKind::Inside,
    );
    ui.painter()
        .galley(rect.center() - g.size() * 0.5, g, color);
    resp.on_hover_text(t(hint));
}

/// A painted checkbox, empty or checked.
pub fn paint_check(painter: &egui::Painter, rect: egui::Rect, state: Check) {
    match state {
        Check::On => {
            painter.rect_filled(rect, 4.0, ACCENT());
            crate::icons::paint(
                painter,
                crate::icons::Icon::Check,
                rect.shrink(2.0),
                ACCENT_INK(),
            );
        }
        Check::Off => {
            painter.rect_stroke(
                rect,
                4.0,
                egui::Stroke::new(1.5_f32, MUTED().gamma_multiply(0.8)),
                egui::StrokeKind::Inside,
            );
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Check {
    Off,
    On,
}

/// Big number with a caption underneath.
pub fn stat(ui: &mut Ui, value: &str, caption_text: &str) {
    egui::Frame::new()
        .fill(PANEL_2())
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(S3 as i8, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // `ui.columns` justifies its cells; a plain vertical layout keeps wrapped text left-aligned.
            ui.vertical(|ui| {
                ui.label(strong(value, type_scale::TITLE));
                ui.label(caption(caption_text));
            });
        });
}

/// A card: panel fill, hairline border, 10 pt corners.
pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(crate::theme::PANEL())
        .stroke(egui::Stroke::new(1.0_f32, BORDER()))
        .corner_radius(crate::theme::metrics::RADIUS_CARD)
        .inner_margin(egui::Margin::same(S4 as i8))
}

/// Thin proportion bar.
pub fn meter(ui: &mut Ui, frac: f32, fill: Color32, height: f32) {
    let (bar, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    ui.painter().rect_filled(bar, height * 0.5, PANEL_2());
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

/// Hairline divider.
pub fn divider(ui: &mut Ui) {
    let (line, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(line, 0.0, BORDER());
}

/// Centered message for empty or blocked states, with an optional action.
pub fn empty_state(ui: &mut Ui, title: &str, body: &str) {
    ui.add_space(S4 * 2.0);
    ui.vertical_centered(|ui| {
        ui.label(strong(title, type_scale::LEAD));
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
        MUTED(),
    );
    let size_x = rect.right() - reserve_right - size.size().x;
    let mut job = egui::text::LayoutJob::simple_singleline(
        path.to_owned(),
        egui::FontId::monospace(type_scale::MONO),
        TEXT(),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((size_x - rect.left() - S3).max(40.0));
    let path_g = painter.layout_job(job);
    let y = rect.center().y;
    painter.galley(
        egui::pos2(rect.left(), y - path_g.size().y * 0.5),
        path_g,
        TEXT(),
    );
    painter.galley(egui::pos2(size_x, y - size.size().y * 0.5), size, MUTED());
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
