//! App bar: one row of global navigation, location, progress and the Review entry.

use eframe::egui::{self, Align, Color32, Layout, RichText};

use super::Density;
use crate::app::{Mode, SpaceApp};
use crate::theme::metrics::{APP_BAR_H, CONTROL_H, S2, S3, S4};
use crate::theme::type_scale;
use crate::theme::{ACCENT, ACCENT_INK, BG, BORDER, MUTED, PANEL, PANEL_2, TEXT};
use crate::widgets::{caption, ghost, secondary, segmented, text_width};
use space_core::format_bytes;

/// Space reserved for the controls on the right of the bar when fitting breadcrumbs.
const MORE_W: f32 = 76.0;
const SCAN_W: f32 = 300.0;

impl SpaceApp {
    pub(super) fn app_bar(&mut self, ctx: &egui::Context, d: Density) {
        let explore = self.mode == Mode::Explore && self.tree.is_some();
        let search_w = match d {
            Density::Wide => 240.0,
            _ => 160.0,
        };
        let inline_location = explore && d != Density::Narrow;
        let mut reserve = MORE_W + S4;
        if self.basket_len() > 0 {
            let label = format!(
                "Review · {} · {}",
                self.basket_len(),
                format_bytes(self.basket_bytes())
            );
            let font = egui::FontId::proportional(type_scale::BODY);
            reserve += text_width_ctx(ctx, &label, font) + 40.0 + S2;
        }
        if self.scan.is_some() {
            reserve += SCAN_W + S2;
        }
        if inline_location {
            reserve += search_w + S3;
        }

        egui::TopBottomPanel::top("app_bar")
            .exact_height(APP_BAR_H)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(S4 as i8, 0))
                    .stroke(egui::Stroke::new(1.0_f32, BORDER)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = S2;
                    brand(ui, d != Density::Narrow);
                    self.mode_tabs(ui);
                    if inline_location {
                        ui.add_space(S3);
                        let budget = (ui.available_width() - reserve).max(0.0);
                        self.location(ui, budget);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        self.more_menu(ui);
                        self.review_pill(ui);
                        self.scan_progress(ui);
                        if inline_location {
                            self.search_box(ui, search_w);
                        }
                        if d == Density::Wide
                            && self.scan.is_none()
                            && matches!(self.mode, Mode::Review | Mode::Clean)
                        {
                            self.status_note(ui);
                        }
                    });
                });
            });

        if explore && d == Density::Narrow {
            egui::TopBottomPanel::top("sub_bar")
                .exact_height(48.0)
                .frame(
                    egui::Frame::new()
                        .fill(BG)
                        .inner_margin(egui::Margin::symmetric(S3 as i8, 0)),
                )
                .show(ctx, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = S2;
                        if let Some(i) = segmented(ui, &["List", "Map"], self.compact_map as usize)
                        {
                            self.compact_map = i == 1;
                        }
                        let budget = (ui.available_width() - 150.0 - S4).max(0.0);
                        self.location(ui, budget);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            self.search_box(ui, 140.0);
                        });
                    });
                });
        }
    }

    fn mode_tabs(&mut self, ui: &mut egui::Ui) {
        let has_scan = self.tree.is_some() || self.scan.is_some();
        egui::Frame::new()
            .fill(BG)
            .corner_radius(10)
            .inner_margin(egui::Margin::same(3))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (mode, label, hint, enabled) in [
                    (Mode::Home, "Scan", "Choose a folder or disk", true),
                    (
                        Mode::Explore,
                        "Disk usage",
                        "Explore the current scan",
                        has_scan,
                    ),
                    (Mode::Clean, "Cleanup", "Find cleanup candidates · ⌘J", true),
                ] {
                    let on = self.mode == mode;
                    let button = egui::Button::new(
                        RichText::new(label).size(type_scale::BODY).color(if on {
                            TEXT
                        } else {
                            MUTED
                        }),
                    )
                    .fill(if on { PANEL_2 } else { Color32::TRANSPARENT })
                    .stroke(if on {
                        egui::Stroke::new(1.0_f32, BORDER)
                    } else {
                        egui::Stroke::NONE
                    })
                    .corner_radius(8)
                    .min_size(egui::vec2(0.0, 32.0));
                    if ui
                        .add_enabled(enabled, button)
                        .on_hover_text(hint)
                        .clicked()
                    {
                        self.set_mode(mode);
                    }
                }
            });
    }

    /// ← → ↑ and breadcrumbs, shortened from the left so they never wrap.
    fn location(&mut self, ui: &mut egui::Ui, budget: f32) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        let nav = |ui: &mut egui::Ui, enabled: bool, glyph: &str, hint: &str| {
            ui.add_enabled(
                enabled,
                egui::Button::new(RichText::new(glyph).size(type_scale::LEAD))
                    .frame(false)
                    .min_size(egui::vec2(28.0, CONTROL_H)),
            )
            .on_hover_text(hint)
            .clicked()
        };
        if nav(ui, !self.history_back.is_empty(), "←", "Back") {
            self.go_back();
        }
        if nav(ui, !self.history_fwd.is_empty(), "→", "Forward") {
            self.go_forward();
        }
        let can_up = tree.node(self.focus).parent.is_some();
        if nav(ui, can_up, "↑", "Parent folder · Esc") {
            self.go_up();
        }

        let avail = budget - 3.0 * 28.0 - 3.0 * S2;
        let crumbs = tree.breadcrumb_names(self.focus);
        let chain = tree.path_to(self.focus);
        let font = egui::FontId::proportional(type_scale::BODY);
        // Keep as many trailing segments as fit; the current folder always stays.
        let mut used = 0.0;
        let mut start = crumbs.len();
        for i in (0..crumbs.len()).rev() {
            let w = text_width(ui, &crumbs[i], font.clone()) + 24.0;
            if start < crumbs.len() && used + w > avail - 28.0 {
                break;
            }
            used += w;
            start = i;
        }
        let mut go = None;
        if start > 0
            && ui
                .add(egui::Button::new(RichText::new("…").color(MUTED)).frame(false))
                .on_hover_text("Show the path above")
                .clicked()
        {
            go = Some(chain[start - 1]);
        }
        for i in start..crumbs.len() {
            if i > start {
                ui.label(RichText::new("/").color(MUTED));
            }
            let here = i + 1 == crumbs.len();
            let text = RichText::new(&crumbs[i])
                .size(type_scale::BODY)
                .color(if here { TEXT } else { MUTED });
            let text = if here { text.strong() } else { text };
            let width = if here {
                (avail - used + 24.0).max(60.0)
            } else {
                f32::INFINITY
            };
            ui.scope(|ui| {
                ui.set_max_width(width.min(avail.max(60.0)));
                if ui
                    .add(egui::Button::new(text).frame(false).truncate())
                    .on_hover_text(tree.path_string(chain[i]))
                    .clicked()
                    && !here
                {
                    go = Some(chain[i]);
                }
            });
        }
        if let Some(id) = go {
            self.navigate(id, true);
        }
    }

    fn search_box(&mut self, ui: &mut egui::Ui, width: f32) {
        if !self.filter_draft.is_empty()
            && ui
                .add(ghost("×").min_size(egui::vec2(28.0, CONTROL_H)))
                .on_hover_text("Clear search")
                .clicked()
        {
            self.filter_draft.clear();
            self.filter.query.clear();
            self.filter_edited_at = None;
            self.geom = None;
        }
        let edit = ui.add_sized(
            [width, CONTROL_H - 2.0],
            egui::TextEdit::singleline(&mut self.filter_draft)
                .margin(egui::vec2(10.0, 7.0))
                .hint_text(
                    RichText::new(if width < 190.0 {
                        "Search"
                    } else {
                        "Search this scan  ⌘F"
                    })
                    .color(MUTED),
                ),
        );
        if self.focus_filter {
            edit.request_focus();
            self.focus_filter = false;
        }
        if edit.changed() {
            self.filter_edited_at = Some(std::time::Instant::now());
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(
                    self.settings.flags.debounce_filter_ms + 10,
                ));
        }
    }

    /// Indeterminate progress: a measured percentage would be a guess.
    fn scan_progress(&mut self, ui: &mut egui::Ui) {
        if self.scan.is_none() {
            return;
        }
        if ui
            .add(secondary("Cancel").min_size(egui::vec2(0.0, 30.0)))
            .clicked()
        {
            if let Some(s) = &self.scan {
                s.cancel();
            }
        }
        let text = match &self.progress {
            Some(p) => format!(
                "Scanning · {} · {} files",
                format_bytes(p.bytes_seen),
                p.files_seen
            ),
            None => "Starting scan…".to_string(),
        };
        ui.add(egui::Label::new(caption(text)).truncate());
        ui.add(
            egui::ProgressBar::new(0.0)
                .desired_width(56.0)
                .desired_height(4.0)
                .animate(true),
        );
    }

    fn status_note(&mut self, ui: &mut egui::Ui) {
        if self.status.is_empty() || ui.available_width() < 120.0 {
            return;
        }
        ui.add(egui::Label::new(caption(&self.status)).truncate())
            .on_hover_text(&self.status);
    }

    /// Review entry — only present once something is selected.
    fn review_pill(&mut self, ui: &mut egui::Ui) {
        let n = self.basket_len();
        if n == 0 {
            return;
        }
        let label = format!("Review · {n} · {}", format_bytes(self.basket_bytes()));
        let pill = egui::Button::new(RichText::new(label).color(ACCENT_INK).strong())
            .fill(ACCENT)
            .corner_radius(18)
            .min_size(egui::vec2(0.0, 34.0));
        if ui
            .add(pill)
            .on_hover_text("Check what will move to Trash")
            .clicked()
        {
            self.set_mode(Mode::Review);
        }
    }

    fn more_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(RichText::new("More").color(TEXT), |ui| {
            ui.set_min_width(220.0);
            if ui.button("Choose folder…  ⌘O").clicked() {
                ui.close_menu();
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    self.start_scan(path, false);
                }
            }
            let idle = self.scan.is_none() && !self.busy;
            if ui
                .add_enabled(
                    self.tree.is_some() && idle,
                    egui::Button::new("Rescan everything  ⌘R"),
                )
                .clicked()
            {
                self.rescan_current();
                ui.close_menu();
            }
            if ui
                .add_enabled(
                    self.tree.is_some() && idle,
                    egui::Button::new("Rescan this folder"),
                )
                .clicked()
            {
                self.incremental_rescan_focus();
                ui.close_menu();
            }
            if ui
                .add_enabled(self.tree.is_some(), egui::Button::new("Copy path  ⌘⇧C"))
                .clicked()
            {
                self.copy_focus_path();
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Cleanup rules…").clicked() {
                self.open_rules_panel();
                ui.close_menu();
            }
            if ui.button("Keyboard shortcuts  ?").clicked() {
                self.show_shortcuts = true;
                ui.close_menu();
            }
        });
    }
}

fn text_width_ctx(ctx: &egui::Context, text: &str, font: egui::FontId) -> f32 {
    ctx.fonts(|f| {
        f.layout_no_wrap(text.to_owned(), font, Color32::WHITE)
            .size()
            .x
    })
}

/// Two-tone ring + name.
fn brand(ui: &mut egui::Ui, with_name: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
    let c = rect.center();
    let p = ui.painter();
    p.circle_stroke(c, 8.0, egui::Stroke::new(5.0_f32, ACCENT));
    let blue = Color32::from_rgb(106, 169, 255);
    let arc: Vec<egui::Pos2> = (-40..=40)
        .step_by(8)
        .map(|deg| {
            let a = (deg as f32).to_radians();
            c + egui::vec2(a.cos(), a.sin()) * 8.0
        })
        .collect();
    p.add(egui::Shape::line(arc, egui::Stroke::new(5.0_f32, blue)));
    if with_name {
        ui.label(RichText::new("Space").size(type_scale::LEAD).strong());
    }
}
