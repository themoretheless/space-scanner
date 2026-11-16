//! App bar: two primary tabs (Quick clean | Folders), location bar, and persistent action bar.

use eframe::egui::{self, Align, Color32, Layout, RichText};

use super::Density;
use crate::app::{Mode, SpaceApp};
use crate::i18n::{t, Lang};
use crate::icons::{self, Icon};
use crate::theme::metrics::{APP_BAR_H, CONTROL_H, S2, S3, S4};
use crate::theme::type_scale;
use crate::theme::{ACCENT, BG, BORDER, MUTED, PANEL, PANEL_2, TEXT};
use crate::widgets::{
    caption, count_noun, danger_secondary, ghost, primary, secondary, segmented, text_width,
};

impl SpaceApp {
    pub(super) fn app_bar(&mut self, ctx: &egui::Context, d: Density) {
        let explore = self.mode == Mode::Explore && self.tree.is_some() && !self.choosing_target;

        egui::TopBottomPanel::top("app_bar")
            .exact_height(APP_BAR_H)
            .frame(
                egui::Frame::new()
                    .fill(PANEL())
                    .inner_margin(egui::Margin::symmetric(S4 as i8, 0))
                    .stroke(egui::Stroke::new(1.0_f32, BORDER())),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = S2;
                    brand(ui, d != Density::Narrow);
                    self.mode_tabs(ui);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        self.more_menu(ui);
                        if d == Density::Wide && self.scan.is_none() {
                            self.status_note(ui);
                        }
                    });
                });
            });

        if explore {
            egui::TopBottomPanel::top("sub_bar")
                .exact_height(48.0)
                .frame(
                    egui::Frame::new()
                        .fill(BG())
                        .inner_margin(egui::Margin::symmetric(S3 as i8, 0)),
                )
                .show(ctx, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = S2;
                        if d == Density::Narrow {
                            if let Some(i) =
                                segmented(ui, &["List", "Map"], self.compact_map as usize)
                            {
                                self.compact_map = i == 1;
                            }
                        }
                        let search_w = if d == Density::Narrow { 130.0 } else { 200.0 };
                        let budget = (ui.available_width() - search_w - 140.0 - S4).max(0.0);
                        self.location(ui, budget);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            self.filter_menu(ui);
                            self.search_box(ui, search_w);
                        });
                    });
                });
        }
        if self.scan.is_some() {
            egui::TopBottomPanel::top("scan_progress_bar")
                .frame(
                    egui::Frame::new()
                        .fill(PANEL())
                        .inner_margin(egui::Margin::symmetric(16, 4)),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| self.scan_progress(ui));
                });
        }
    }

    /// Persistent bottom bar whenever items are checked for Trash.
    pub(super) fn basket_bar(&mut self, ctx: &egui::Context, d: Density) {
        let n = self.basket_len();
        if n == 0 {
            return;
        }
        let total_bytes = self.basket_bytes();
        let idle = !self.busy && self.scan.is_none();

        egui::TopBottomPanel::bottom("basket_bar")
            .frame(
                egui::Frame::new()
                    .fill(PANEL())
                    .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                    .inner_margin(egui::Margin::symmetric(S4 as i8, S3 as i8)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = S2;
                    ui.label(
                        RichText::new(crate::i18n::bytes(total_bytes))
                            .size(type_scale::LEAD)
                            .monospace()
                            .strong()
                            .color(ACCENT()),
                    );
                    ui.label(RichText::new(count_noun(n, "item")).color(TEXT()));
                    if d != Density::Narrow {
                        ui.label(caption(
                            "· Space is released after emptying Trash in Finder.",
                        ));
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add_enabled(idle, primary("Move to Trash…")).clicked() {
                            self.request_trash();
                        }
                        if ui
                            .add_enabled(idle, danger_secondary("Delete permanently…"))
                            .clicked()
                        {
                            self.request_permanent_basket();
                        }
                        if ui
                            .add_enabled(!self.busy, secondary("Clear selection"))
                            .clicked()
                        {
                            self.review.clear();
                            self.junk.clear_selection();
                            self.downloads.clear_selection();
                        }
                        if d != Density::Narrow && ui.add(ghost("Export list…")).clicked() {
                            self.export_basket_list();
                        }
                    });
                });
            });
    }

    fn mode_tabs(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(BG())
            .corner_radius(crate::theme::metrics::RADIUS_CONTROL)
            .inner_margin(egui::Margin::same(3))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (mode, icon, label, hint) in [
                    (
                        Mode::Clean,
                        Icon::Cleanup,
                        "Quick clean",
                        "Find & clean caches, logs and downloads · ⌘J",
                    ),
                    (
                        Mode::Explore,
                        Icon::Folder,
                        "Folders",
                        "Explore disk usage by folder",
                    ),
                ] {
                    if tab(ui, icon, label, self.mode == mode, true, hint).clicked() {
                        self.set_mode(mode);
                    }
                }
            });
    }

    /// Disk picker button + ← → ↑ + breadcrumbs, shortened from the left.
    fn location(&mut self, ui: &mut egui::Ui, budget: f32) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        if icons::button(
            ui,
            Icon::Disk,
            30.0,
            "Choose a folder or disk",
            !self.busy && self.scan.is_none(),
        )
        .clicked()
        {
            self.choosing_target = true;
        }
        let nav = |ui: &mut egui::Ui, enabled: bool, icon: Icon, hint: &str| {
            icons::button(ui, icon, 30.0, hint, enabled).clicked()
        };
        if nav(ui, !self.history_back.is_empty(), Icon::Back, "Back") {
            self.go_back();
        }
        if nav(ui, !self.history_fwd.is_empty(), Icon::Forward, "Forward") {
            self.go_forward();
        }
        let can_up = tree
            .try_node(self.focus)
            .is_some_and(|n| n.parent.is_some());
        if nav(ui, can_up, Icon::Up, "Parent folder · Esc") {
            self.go_up();
        }

        let avail = (budget - 4.0 * 30.0 - 4.0 * S2).max(0.0);
        let crumbs = tree.breadcrumb_names(self.focus);
        let chain = tree.path_to(self.focus);
        let font = egui::FontId::proportional(type_scale::BODY);
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
                .add(egui::Button::new(RichText::new("…").color(MUTED())).frame(false))
                .on_hover_text(t("Show the path above"))
                .clicked()
        {
            go = Some(chain[start - 1]);
        }
        let mut remaining = (avail - if start > 0 { 28.0 } else { 0.0 }).max(0.0);
        for i in start..crumbs.len() {
            if i > start {
                icons::show(ui, Icon::ChevronRight, 12.0, MUTED());
            }
            let here = i + 1 == crumbs.len();
            let text = RichText::new(&crumbs[i])
                .size(type_scale::BODY)
                .color(if here { TEXT() } else { MUTED() });
            let text = if here { text.strong() } else { text };
            let width = if here {
                remaining.max(24.0)
            } else {
                f32::INFINITY
            };
            ui.scope(|ui| {
                ui.set_max_width(width.min(remaining.max(24.0)));
                if ui
                    .add(egui::Button::new(text).frame(false).truncate())
                    .on_hover_text(tree.path_string(chain[i]))
                    .clicked()
                    && !here
                {
                    go = Some(chain[i]);
                }
            });
            remaining = (remaining - text_width(ui, &crumbs[i], font.clone()) - 24.0).max(0.0);
        }
        if let Some(id) = go {
            self.navigate(id, true);
        }
    }

    fn search_box(&mut self, ui: &mut egui::Ui, width: f32) {
        if !self.filter_draft.is_empty()
            && icons::button(ui, Icon::Close, 28.0, "Clear search", true).clicked()
        {
            self.filter_draft.clear();
            self.filter.query.clear();
            self.filter.rebuild_regex();
            self.filter_edited_at = None;
            self.geom = None;
        }
        let edit = ui.add_sized(
            [width, CONTROL_H - 2.0],
            egui::TextEdit::singleline(&mut self.filter_draft)
                .margin(egui::vec2(8.0, 5.0))
                .hint_text(
                    RichText::new(t(if width < 190.0 {
                        "Search"
                    } else {
                        "Search this scan  ⌘F"
                    }))
                    .color(MUTED()),
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

    fn filter_menu(&mut self, ui: &mut egui::Ui) {
        let count = self.filter.active_count();
        let label = if count > 0 {
            crate::tr!("Filter ({})", count)
        } else {
            t("Filter").to_string()
        };
        let color = if count > 0 { ACCENT() } else { TEXT() };
        ui.menu_button(RichText::new(label).color(color), |ui| {
            ui.set_min_width(280.0);
            if ui
                .checkbox(&mut self.filter.use_regex, t("Regex"))
                .changed()
            {
                self.filter.query = self.filter_draft.clone();
                self.filter.rebuild_regex();
                self.filter_edited_at = None;
                self.geom = None;
            }
            if self.filter.regex_invalid {
                ui.label(
                    RichText::new(t("Invalid regex syntax"))
                        .size(type_scale::CAPTION)
                        .color(crate::theme::DANGER()),
                );
            }
            if ui
                .checkbox(&mut self.filter.only_dirs, t("Folders only"))
                .changed()
            {
                self.geom = None;
            }
            ui.separator();
            ui.label(caption("File extensions"));
            ui.horizontal_wrapped(|ui| {
                for ext in ["zip", "dmg", "mp4", "mov", "pkg", "iso", "log", "cache"] {
                    let active = self.filter.exts.iter().any(|x| x.eq_ignore_ascii_case(ext));
                    if ui.selectable_label(active, format!(".{ext}")).clicked() {
                        if let Some(pos) = self
                            .filter
                            .exts
                            .iter()
                            .position(|x| x.eq_ignore_ascii_case(ext))
                        {
                            self.filter.exts.remove(pos);
                        } else {
                            self.filter.exts.push(ext.to_string());
                        }
                        self.geom = None;
                    }
                }
            });
            ui.horizontal(|ui| {
                let r = ui.add_sized(
                    [120.0, 26.0],
                    egui::TextEdit::singleline(&mut self.ext_draft)
                        .hint_text(t("Add ext (e.g. dmg)")),
                );
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (enter || ui.button(t("Add")).clicked()) && !self.ext_draft.trim().is_empty() {
                    let clean = self
                        .ext_draft
                        .trim()
                        .trim_start_matches('.')
                        .to_ascii_lowercase();
                    if !clean.is_empty() && !self.filter.exts.iter().any(|x| x == &clean) {
                        self.filter.exts.push(clean);
                        self.geom = None;
                    }
                    self.ext_draft.clear();
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(caption("Min MB"));
                if ui
                    .add_sized(
                        [68.0, 26.0],
                        egui::TextEdit::singleline(&mut self.min_mb_draft),
                    )
                    .changed()
                {
                    let mb: u64 = self.min_mb_draft.trim().parse().unwrap_or(0);
                    self.filter.min_size = mb.saturating_mul(1024 * 1024);
                    self.geom = None;
                }
                ui.label(caption("Max MB"));
                if ui
                    .add_sized(
                        [68.0, 26.0],
                        egui::TextEdit::singleline(&mut self.max_mb_draft),
                    )
                    .changed()
                {
                    self.filter.max_size = self
                        .max_mb_draft
                        .trim()
                        .parse::<u64>()
                        .ok()
                        .map(|mb| mb.saturating_mul(1024 * 1024));
                    self.geom = None;
                }
            });
            ui.separator();
            if ui
                .add_enabled(
                    self.filter.is_active(),
                    egui::Button::new(t("Reset filter")),
                )
                .clicked()
            {
                self.reset_filters();
                ui.close_menu();
            }
        });
    }

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
            Some(p) => crate::tr!(
                "Scanning · {} · {} files",
                crate::i18n::bytes(p.bytes_seen),
                p.files_seen
            ),
            None => t("Starting scan…").to_string(),
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

    fn more_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(RichText::new(t("More")).color(TEXT()), |ui| {
            ui.set_min_width(220.0);
            if ui.button(t("Choose folder…  ⌘O")).clicked() {
                ui.close_menu();
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    self.choosing_target = false;
                    self.start_scan(path, false);
                }
            }
            let idle = self.scan.is_none() && !self.busy;
            let can_rescan = match self.mode {
                Mode::Clean => idle && !self.junk.loading && !self.downloads.loading,
                Mode::Explore => self.tree.is_some() && idle,
            };
            if ui
                .add_enabled(can_rescan, egui::Button::new(t("Rescan everything  ⌘R")))
                .clicked()
            {
                match self.mode {
                    Mode::Clean => self.start_quick_clean_scan(),
                    Mode::Explore => self.rescan_current(),
                }
                ui.close_menu();
            }
            if ui
                .add_enabled(
                    self.mode == Mode::Explore
                        && !self.choosing_target
                        && self.tree.is_some()
                        && idle,
                    egui::Button::new(t("Rescan this folder")),
                )
                .clicked()
            {
                self.incremental_rescan_focus();
                ui.close_menu();
            }
            if ui
                .add_enabled(self.tree.is_some(), egui::Button::new(t("Copy path  ⌘⇧C")))
                .clicked()
            {
                self.copy_focus_path();
                ui.close_menu();
            }
            if ui.button(t("Go to folder…  ⌘L")).clicked() {
                self.open_goto_dialog();
                ui.close_menu();
            }
            ui.menu_button(t("Export"), |ui| {
                let has_tree = self.tree.is_some();
                if ui
                    .add_enabled(has_tree, egui::Button::new(t("Children CSV…")))
                    .clicked()
                {
                    self.export_children_csv();
                    ui.close_menu();
                }
                if ui
                    .add_enabled(has_tree, egui::Button::new(t("Largest files CSV…")))
                    .clicked()
                {
                    self.export_largest_csv();
                    ui.close_menu();
                }
                if ui
                    .add_enabled(has_tree, egui::Button::new(t("Tree TSV…")))
                    .clicked()
                {
                    self.export_tsv();
                    ui.close_menu();
                }
                if ui
                    .add_enabled(has_tree, egui::Button::new(t("Summary JSON…")))
                    .clicked()
                {
                    self.export_json_shallow();
                    ui.close_menu();
                }
                if ui
                    .add_enabled(has_tree, egui::Button::new(t("Full tree JSON…")))
                    .clicked()
                {
                    self.export_json_full();
                    ui.close_menu();
                }
                if ui
                    .add_enabled(has_tree, egui::Button::new(t("Sunburst SVG…")))
                    .clicked()
                {
                    self.export_sunburst_svg();
                    ui.close_menu();
                }
                if ui.button(t("Map PNG screenshot…")).clicked() {
                    self.request_png_screenshot(ui.ctx());
                    ui.close_menu();
                }
                if ui
                    .add_enabled(self.basket_len() > 0, egui::Button::new(t("Basket list…")))
                    .clicked()
                {
                    self.export_basket_list();
                    ui.close_menu();
                }
                #[cfg(target_os = "macos")]
                if ui.button(t("Share…")).clicked() {
                    self.share_report_macos();
                    ui.close_menu();
                }
            });
            ui.separator();
            if ui.button(t("Cleanup rules…")).clicked() {
                self.open_rules_panel();
                ui.close_menu();
            }
            if ui.button(t("Local snapshots…")).clicked() {
                self.open_tm_panel();
                ui.close_menu();
            }
            if ui.button(t("Open Trash")).clicked() {
                crate::widgets::open_user_trash();
                ui.close_menu();
            }
            ui.menu_button(t("Appearance"), |ui| {
                let cur = self.settings.skin.normalized_appearance();
                for skin in crate::theme::Skin::APPEARANCE {
                    if ui
                        .selectable_label(cur == skin, t(skin.appearance_label()))
                        .clicked()
                    {
                        self.settings.skin = skin;
                        crate::theme::set_skin(ui.ctx(), skin);
                        self.geom = None;
                        self.settings.save();
                        ui.close_menu();
                    }
                }
            });
            ui.menu_button(t("Language"), |ui| {
                for (choice, label) in [
                    (None, t("Automatic")),
                    (Some(Lang::En), Lang::En.native_name()),
                    (Some(Lang::Ru), Lang::Ru.native_name()),
                ] {
                    if ui
                        .selectable_label(self.settings.language == choice, label)
                        .clicked()
                    {
                        self.settings.language = choice;
                        crate::i18n::set(choice.unwrap_or_else(crate::i18n::detect));
                        self.settings.save();
                        ui.close_menu();
                    }
                }
            });
            if ui.button(t("Keyboard shortcuts  ?")).clicked() {
                self.show_shortcuts = true;
                ui.close_menu();
            }
            ui.separator();
            if ui.button(t("Settings…  ⌘,")).clicked() {
                self.show_settings = true;
                ui.close_menu();
            }
            if ui.button(t("About Space Scanner")).clicked() {
                self.show_about = true;
                ui.close_menu();
            }
        });
    }
}

/// One destination: icon and label, filled when current.
fn tab(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    on: bool,
    enabled: bool,
    hint: &str,
) -> egui::Response {
    let label = t(label);
    let font = egui::FontId::proportional(type_scale::BODY);
    let text_w = text_width(ui, label, font.clone());
    let size = egui::vec2(12.0 + 16.0 + 8.0 + text_w + 14.0, 32.0);
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let hovered = resp.hovered() && enabled;
    let color = if !enabled {
        MUTED().gamma_multiply(0.5)
    } else if on || hovered {
        TEXT()
    } else {
        MUTED()
    };
    let painter = ui.painter();
    if on {
        painter.rect(
            rect,
            crate::theme::metrics::RADIUS_CONTROL as f32,
            PANEL_2(),
            egui::Stroke::new(1.0_f32, BORDER()),
            egui::StrokeKind::Inside,
        );
    } else if hovered {
        painter.rect_filled(
            rect,
            crate::theme::metrics::RADIUS_CONTROL as f32,
            PANEL_2().gamma_multiply(0.5),
        );
    }
    icons::paint(
        painter,
        icon,
        egui::Rect::from_center_size(
            egui::pos2(rect.left() + 12.0 + 8.0, rect.center().y),
            egui::vec2(16.0, 16.0),
        ),
        if on { ACCENT() } else { color },
    );
    painter.text(
        egui::pos2(rect.left() + 12.0 + 16.0 + 8.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font,
        color,
    );
    resp.on_hover_text(t(hint))
}

/// Two-tone ring + name.
fn brand(ui: &mut egui::Ui, with_name: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
    let c = rect.center();
    let p = ui.painter();
    p.circle_stroke(c, 8.0, egui::Stroke::new(5.0_f32, ACCENT()));
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
