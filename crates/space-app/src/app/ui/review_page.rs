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

const ROW_H: f32 = 40.0;

enum Row {
    Header(&'static str, usize),
    Disk(NodeId, String, u64),
    Junk(PathBuf, u64),
}

impl SpaceApp {
    pub(super) fn review_page(&mut self, ctx: &egui::Context) {
        let total = self.basket_len();
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
            .show_rows(ui, ROW_H, rows.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for i in range {
                    match &rows[i] {
                        Row::Header(title, count) => {
                            ui.allocate_ui_with_layout(
                                egui::vec2(ui.available_width(), ROW_H),
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
    let rect = path_size_row(ui, path, &crate::i18n::bytes(size), ROW_H, 40.0);
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
