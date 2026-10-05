//! Review: the one place deletions are checked. It lists both sources, lets any
//! item be dropped, and is the only route to the Move to Trash confirmation.

use crate::i18n::t;
use std::path::PathBuf;

use eframe::egui::{self, Align, Layout, RichText};
use space_core::NodeId;

use crate::app::{Mode, SpaceApp};
use crate::review;
use crate::theme::metrics::{S2, S3, S4, S6};
use crate::theme::type_scale;
use crate::theme::{BG, BORDER, MUTED, PANEL, SAFE};
use crate::widgets::{caption, empty_state, ghost, page_column, path_size_row, primary, secondary};

/// Row height: the Table skin is a dense grid.
fn row_h() -> f32 {
    if crate::theme::skin() == crate::theme::Skin::Table {
        30.0
    } else {
        40.0
    }
}

enum Row {
    Header(&'static str, usize),
    Disk(NodeId, String, u64),
    Junk(PathBuf, u64),
}

impl SpaceApp {
    pub(super) fn review_page(&mut self, ctx: &egui::Context) {
        let total = self.basket_len();
        let skin = crate::theme::skin();
        if skin == crate::theme::Skin::Columns {
            self.review_side_summary(ctx, total);
        } else {
        egui::TopBottomPanel::bottom("review_footer")
            .frame(
                egui::Frame::new()
                    .fill(PANEL())
                    .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                    .inner_margin(egui::Margin::symmetric(S6 as i8, S3 as i8)),
            )
            .show(ctx, |ui| {
                page_column(ui, 880.0, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(t("Selected size")).color(MUTED()));
                                ui.label(
                                    RichText::new(crate::i18n::bytes(self.basket_bytes()))
                                        .size(type_scale::TITLE)
                                        .strong(),
                                );
                            });
                            ui.label(
                                caption("Space is released after emptying Trash in Finder.")
                                    .color(SAFE()),
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let idle = !self.busy && self.scan.is_none() && !self.junk.cleaning;
                            if ui
                                .add_enabled(total > 0 && idle, primary("Move to Trash…"))
                                .clicked()
                            {
                                self.confirm_delete = true;
                            }
                        });
                    });
                });
            });
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG()))
            .show(ctx, |ui| {
                page_column(ui, 880.0, |ui| {
                    ui.add_space(S6 + S2);
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(t("Review")).size(type_scale::DISPLAY).strong());
                            ui.label(caption(
                                "Check what will move to Trash. Overlapping paths are counted once.",
                            ));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add_enabled(
                                    total > 0 && !self.busy && !self.junk.cleaning,
                                    secondary("Clear selection"),
                                )
                                .clicked()
                            {
                                self.review.clear();
                                self.junk.clear_selection();
                            }
                        });
                    });
                    ui.add_space(S4);

                    if total == 0 {
                        empty_state(
                            ui,
                            "Nothing selected",
                            "Select items in Disk usage or Cleanup, then come back to check them.",
                        );
                        ui.add_space(S4);
                        ui.vertical_centered(|ui| {
                            ui.horizontal(|ui| {
                                if ui.add(secondary("Open Disk usage")).clicked() {
                                    self.set_mode(Mode::Explore);
                                }
                                if ui.add(secondary("Open Cleanup")).clicked() {
                                    self.set_mode(Mode::Clean);
                                }
                            });
                        });
                        return;
                    }
                    match skin {
                        crate::theme::Skin::Map => self.review_hero(ui),
                        crate::theme::Skin::Table => review_table_header(ui),
                        crate::theme::Skin::Columns => {}
                    }
                    self.review_rows(ui);
                });
            });
    }

    fn review_rows(&mut self, ui: &mut egui::Ui) {
        let mut rows: Vec<Row> = Vec::new();
        if let (Some(tree), false) = (self.tree.clone(), self.review.is_empty()) {
            rows.push(Row::Header("From Disk usage", self.review.len()));
            for &id in &self.review {
                if let Some(node) = tree.try_node(id) {
                    rows.push(Row::Disk(
                        id,
                        tree.path_string(id),
                        node.size(self.size_mode),
                    ));
                }
            }
        }
        if self.junk.selected_count() != 0 {
            let findings = self.junk.selected_findings();
            rows.push(Row::Header("From Cleanup", findings.len()));
            rows.extend(findings.into_iter().map(|f| Row::Junk(f.path, f.bytes)));
        }

        let mut drop_disk = None;
        let mut drop_junk = None;
        egui::ScrollArea::vertical()
            .id_salt("review_rows")
            .auto_shrink([false, false])
            .show_rows(ui, row_h(), rows.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for i in range {
                    match &rows[i] {
                        Row::Header(title, count) => {
                            ui.allocate_ui_with_layout(
                                egui::vec2(ui.available_width(), row_h()),
                                Layout::left_to_right(Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(t(title).to_uppercase())
                                            .size(type_scale::CAPTION)
                                            .color(MUTED()),
                                    );
                                    ui.label(caption(format!("· {count}")));
                                },
                            );
                        }
                        Row::Disk(id, path, size) => {
                            if item_row(ui, path, *size) {
                                drop_disk = Some(*id);
                            }
                        }
                        Row::Junk(path, size) => {
                            if item_row(ui, &path.display().to_string(), *size) {
                                drop_junk = Some(path.clone());
                            }
                        }
                    }
                }
            });
        if let Some(id) = drop_disk {
            review::toggle(&mut self.review, id);
        }
        if let Some(path) = drop_junk {
            self.junk.exclude_paths(&[path]);
        }
    }
}

/// One path with its size and a remove button; returns true when removed.
fn item_row(ui: &mut egui::Ui, path: &str, size: u64) -> bool {
    let rect = path_size_row(ui, path, &crate::i18n::bytes(size), row_h(), 40.0);
    let button = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 16.0, rect.center().y),
        egui::vec2(32.0, 32.0),
    );
    ui.painter().hline(
        rect.x_range(),
        rect.bottom(),
        egui::Stroke::new(1.0_f32, BORDER()),
    );
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(button), |ui| {
        crate::icons::button(
            ui,
            crate::icons::Icon::Close,
            32.0,
            "Remove from review",
            true,
        )
        .clicked()
    })
    .inner
}

impl SpaceApp {
    /// Columns skin: the total and the decision live in a panel on the right.
    fn review_side_summary(&mut self, ctx: &egui::Context, total: usize) {
        egui::SidePanel::right("review_summary")
            .exact_width(360.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL())
                    .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                    .inner_margin(egui::Margin::same(28)),
            )
            .show(ctx, |ui| {
                ui.label(caption("Total to free"));
                ui.label(
                    RichText::new(crate::i18n::bytes(self.basket_bytes()))
                        .size(44.0)
                        .strong(),
                );
                ui.label(caption(crate::tr!("{} items", total)));
                ui.add_space(S4);
                for (label, n, bytes) in [
                    ("From Disk usage", self.review.len(), self.review_bytes()),
                    ("From Cleanup", self.junk.selected_count(), self.junk.selected_bytes()),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(t(label)).color(MUTED()));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(crate::tr!("{} · {}", crate::i18n::bytes(bytes), n)).monospace());
                        });
                    });
                }
                ui.add_space(S4);
                ui.label(caption("Files go to the macOS Trash and can be put back until it is emptied.").color(SAFE()));
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    let idle = !self.busy && self.scan.is_none() && !self.junk.cleaning;
                    if ui
                        .add_enabled(total > 0 && idle, secondary("Clear selection").min_size(egui::vec2(ui.available_width(), 34.0)))
                        .clicked()
                    {
                        self.review.clear();
                        self.junk.clear_selection();
                    }
                    ui.add_space(S2);
                    let label = crate::tr!("Move to Trash · {}", crate::i18n::bytes(self.basket_bytes()));
                    if ui
                        .add_enabled(total > 0 && idle, primary(&label).min_size(egui::vec2(ui.available_width(), 44.0)))
                        .clicked()
                    {
                        self.confirm_delete = true;
                    }
                });
            });
    }

    /// Map skin: one big number and the selection drawn as proportional blocks.
    fn review_hero(&mut self, ui: &mut egui::Ui) {
        let mut items = self.basket_items();
        if items.is_empty() {
            return;
        }
        items.sort_by_key(|i| std::cmp::Reverse(i.1));
        let total = items.iter().map(|i| i.1).sum::<u64>().max(1) as f32;
        ui.label(
            RichText::new(crate::i18n::bytes(self.basket_bytes()))
                .size(56.0)
                .monospace()
                .strong(),
        );
        ui.add_space(S2);
        let (bar, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 48.0), egui::Sense::hover());
        let mut x = bar.left();
        let gap = 3.0;
        let shown = items.len().min(8);
        let rest: u64 = items.iter().skip(shown).map(|i| i.1).sum();
        let mut blocks: Vec<(String, u64)> = items
            .iter()
            .take(shown)
            .map(|(p, b)| (p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), *b))
            .collect();
        if rest > 0 {
            blocks.push((String::new(), rest));
        }
        for (name, bytes) in blocks {
            let w = ((bar.width() - gap * shown as f32) * bytes as f32 / total).max(6.0);
            let r = egui::Rect::from_min_size(egui::pos2(x, bar.top()), egui::vec2(w.min(bar.right() - x), bar.height()));
            let col = if name.is_empty() { MUTED() } else { crate::theme::color_for(&name, true) };
            ui.painter().rect_filled(r, 6.0, col);
            if w > 70.0 && !name.is_empty() {
                let mut job = egui::text::LayoutJob::simple_singleline(name, egui::FontId::proportional(type_scale::CAPTION), egui::Color32::from_black_alpha(220));
                job.wrap = egui::text::TextWrapping::truncate_at_width(w - 12.0);
                let g = ui.painter().layout_job(job);
                ui.painter().galley(egui::pos2(r.left() + 6.0, r.bottom() - g.size().y - 5.0), g, egui::Color32::BLACK);
            }
            x += w + gap;
        }
        ui.add_space(S4);
    }
}

/// Table skin: a column header over the dense rows.
fn review_table_header(ui: &mut egui::Ui) {
    let (head, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::hover());
    ui.painter().rect_filled(head, 0.0, crate::theme::PANEL_2());
    let f = egui::FontId::proportional(type_scale::CAPTION);
    ui.painter().text(egui::pos2(head.left() + 8.0, head.center().y), egui::Align2::LEFT_CENTER, t("PATH"), f.clone(), MUTED());
    ui.painter().text(egui::pos2(head.right() - 48.0, head.center().y), egui::Align2::RIGHT_CENTER, t("SIZE"), f, MUTED());
}
