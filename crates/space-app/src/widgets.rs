//! Shared widgets. Every screen is built from these so type, spacing, radii and
//! states stay consistent; nothing here is specific to one page.

use eframe::egui::{self, Color32, RichText, Sense, Ui};

use crate::i18n::t;
use crate::theme::metrics::{CONTROL_H, RADIUS_CONTROL, S3, S4};
use crate::theme::type_scale;
use crate::theme::{ACCENT, ACCENT_INK, BORDER, DANGER, MUTED, PANEL_2, TEXT};

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

/// Filled accent button — the one primary action on a screen.
pub fn primary(label: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(t(label)).color(ACCENT_INK()).strong())
        .fill(ACCENT())
        .corner_radius(RADIUS_CONTROL)
        .min_size(egui::vec2(0.0, CONTROL_H))
}

/// Bordered neutral button.
pub fn secondary(label: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(t(label)).color(TEXT()))
        .fill(PANEL_2())
        .stroke(egui::Stroke::new(1.0_f32, BORDER()))
        .corner_radius(RADIUS_CONTROL)
        .min_size(egui::vec2(0.0, CONTROL_H))
}

/// Filled destructive button for irreversible actions.
pub fn danger(label: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(t(label)).color(Color32::WHITE).strong())
        .fill(DANGER())
        .corner_radius(RADIUS_CONTROL)
        .min_size(egui::vec2(0.0, CONTROL_H))
}

/// Bordered destructive button alongside primary actions.
pub fn danger_secondary(label: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(t(label)).color(DANGER()))
        .fill(PANEL_2())
        .stroke(egui::Stroke::new(1.0_f32, DANGER().gamma_multiply(0.55)))
        .corner_radius(RADIUS_CONTROL)
        .min_size(egui::vec2(0.0, CONTROL_H))
}

/// Frameless text button for quiet actions.
pub fn ghost(label: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(t(label)).color(MUTED()))
        .frame(false)
        .min_size(egui::vec2(0.0, CONTROL_H))
}

/// Make a button span the available width.
pub fn block(ui: &Ui, button: egui::Button<'static>) -> egui::Button<'static> {
    button.min_size(egui::vec2(ui.available_width(), CONTROL_H))
}

/// Filter pill: filled when on, outlined when off.
pub fn chip(ui: &mut Ui, on: bool, label: &str) -> egui::Response {
    let text = t(label);
    let resp = ui.add(
        egui::Button::new(RichText::new(text).size(type_scale::CAPTION).color(if on {
            crate::theme::BG()
        } else {
            MUTED()
        }))
        .fill(if on { TEXT() } else { Color32::TRANSPARENT })
        .stroke(if on {
            egui::Stroke::NONE
        } else {
            egui::Stroke::new(1.0_f32, BORDER())
        })
        .corner_radius(RADIUS_CONTROL)
        .min_size(egui::vec2(24.0, 28.0)),
    );
    if resp.has_focus() {
        crate::theme::paint_focus_ring(ui.painter(), resp.rect);
    }
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, on, text)
    });
    resp
}

/// Flat mutually exclusive choices; only the current choice has a background.
pub fn segmented(ui: &mut Ui, labels: &[&str], current: usize) -> Option<usize> {
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (i, label) in labels.iter().enumerate() {
            let on = i == current;
            let text = t(label);
            let button =
                egui::Button::new(RichText::new(text).color(if on { TEXT() } else { MUTED() }))
                    .fill(if on { PANEL_2() } else { Color32::TRANSPARENT })
                    .stroke(egui::Stroke::NONE)
                    .corner_radius(RADIUS_CONTROL)
                    .min_size(egui::vec2(24.0, 28.0));
            let resp = ui.add(button);
            if resp.has_focus() {
                crate::theme::paint_focus_ring(ui.painter(), resp.rect);
            }
            resp.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, on, text)
            });
            if resp.clicked() {
                picked = Some(i);
            }
        }
    });
    picked
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
        .fill(PANEL_2())
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

/// Visual state for unified custom checkboxes across Quick Clean and Folder Explorer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckVisual {
    Unchecked,
    Checked,
    Partial,
    CoveredByParent,
}

/// Draw a consistent rounded checkbox inside `rect`.
pub fn paint_checkbox(
    painter: &egui::Painter,
    rect: egui::Rect,
    state: CheckVisual,
    hovered: bool,
) {
    match state {
        CheckVisual::Checked => {
            painter.rect_filled(rect, 4.0, ACCENT());
            crate::icons::paint(
                painter,
                crate::icons::Icon::Check,
                rect.shrink(2.5),
                ACCENT_INK(),
            );
        }
        CheckVisual::CoveredByParent => {
            painter.rect_filled(rect, 4.0, ACCENT().gamma_multiply(0.38));
            crate::icons::paint(
                painter,
                crate::icons::Icon::Check,
                rect.shrink(2.5),
                ACCENT_INK(),
            );
        }
        CheckVisual::Partial => {
            let stroke_color = if hovered { TEXT() } else { MUTED() };
            painter.rect_stroke(
                rect,
                4.0,
                egui::Stroke::new(1.5_f32, stroke_color),
                egui::StrokeKind::Inside,
            );
            painter.rect_filled(rect.shrink(4.0), 1.5, ACCENT());
        }
        CheckVisual::Unchecked => {
            let stroke_color = if hovered { TEXT() } else { MUTED() };
            painter.rect_stroke(
                rect,
                4.0,
                egui::Stroke::new(1.5_f32, stroke_color),
                egui::StrokeKind::Inside,
            );
        }
    }
}

/// Reveal `path` in the OS file manager (Finder on macOS).
pub fn reveal_in_os(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .args(["-R", &path.to_string_lossy()])
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let target = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        let _ = std::process::Command::new("xdg-open").arg(target).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .args(["/select,", &path.to_string_lossy()])
            .spawn();
    }
}

/// Preview `path` in macOS Quick Look (`qlmanage -p`), or fall back on other platforms.
pub fn quick_look(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("qlmanage")
            .arg("-p")
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
    #[cfg(not(target_os = "macos"))]
    {
        reveal_in_os(path);
    }
}

/// Open Terminal at `path` (or its parent directory if `path` is a file).
pub fn open_in_terminal(path: &std::path::Path) {
    let dir = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .args(["-a", "Terminal"])
            .arg(dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(dir).spawn();
    }
}

/// Open the user's OS Trash folder (`~/.Trash` on macOS).
pub fn open_user_trash() {
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            let trash = std::path::PathBuf::from(home).join(".Trash");
            let _ = std::process::Command::new("open")
                .arg(trash)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg("trash:///")
            .spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg("shell:RecycleBinFolder")
            .spawn();
    }
}
