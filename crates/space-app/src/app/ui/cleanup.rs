//! Cleanup: discover candidates. Scanning and selecting are separate actions; the
//! queue lives on the Review page, this page only shows a summary beside the list.

use crate::i18n::t;
use eframe::egui::{self, Align, Layout, RichText};

use super::Density;
use crate::app::{Mode, SpaceApp};
use crate::junk_panel::{self, JunkAction};
use crate::theme::metrics::{S2, S3, S4, SIDE_W};
use crate::theme::type_scale;
use crate::theme::{BG, BORDER, MUTED, PANEL, SAFE};
use crate::widgets::{block, caption, ghost, primary, secondary, stat};

impl SpaceApp {
    pub(super) fn cleanup_page(&mut self, ctx: &egui::Context, d: Density) {
        // Below 1000 pt the summary would crowd the list; Review stays one click away.
        let show_summary = ctx.screen_rect().width() >= 1000.0 && d != Density::Narrow;
        if show_summary {
            egui::SidePanel::right("cleanup_summary")
                .exact_width(SIDE_W + 10.0)
                .resizable(false)
                .frame(
                    egui::Frame::new()
                        .fill(PANEL())
                        .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                        .inner_margin(egui::Margin::same(S4 as i8 + 2)),
                )
                .show(ctx, |ui| self.selection_summary(ui));
        }
        let margin = if d == Density::Narrow { S3 } else { 32.0 };
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG()).inner_margin(egui::Margin {
                left: margin as i8,
                right: margin as i8,
                top: 24,
                bottom: 12,
            }))
            .show(ctx, |ui| {
                match junk_panel::draw(ui, &mut self.junk, &mut self.settings.flags) {
                    JunkAction::Rescan => self.start_junk_scan(),
                    JunkAction::Close => self.leave_clean(),
                    JunkAction::OpenRules => self.open_rules_panel(),
                    JunkAction::PrefsChanged => {
                        self.settings.save();
                        self.start_junk_scan();
                    }
                    JunkAction::None => {}
                }
                ui.add_space(S2);
                ui.horizontal_wrapped(|ui| {
                    ui.label(caption(
                        "Selected files go to Trash. Snapshot removal is separate and permanent.",
                    ));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(ghost("Local snapshots…"))
                            .on_hover_text(t("Snapshots hold space a scan can't see"))
                            .clicked()
                        {
                            self.open_tm_panel();
                        }
                        if !show_summary && self.basket_len() > 0 {
                            let label = crate::tr!(
                                "Review · {} · {}",
                                self.basket_len(),
                                crate::i18n::bytes(self.basket_bytes())
                            );
                            if ui.add(primary(&label)).clicked() {
                                self.set_mode(Mode::Review);
                            }
                        }
                    });
                });
            });
    }

    /// Where the selection stands, with the way into Review.
    fn selection_summary(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new(t("Selected for review"))
                .size(type_scale::LEAD)
                .strong(),
        );
        ui.label(caption("Nothing is removed until you confirm in Review."));
        ui.add_space(S3);
        ui.columns(2, |cols| {
            stat(
                &mut cols[0],
                &self.junk.selected_count().to_string(),
                &crate::tr!(
                    "cleanup · {}",
                    crate::i18n::bytes(self.junk.selected_bytes())
                ),
            );
            stat(
                &mut cols[1],
                &self.review.len().to_string(),
                &crate::tr!("disk usage · {}", crate::i18n::bytes(self.review_bytes())),
            );
        });
        ui.add_space(S3);
        ui.horizontal(|ui| {
            ui.label(RichText::new(t("Selected size")).color(MUTED()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(crate::i18n::bytes(self.basket_bytes()))
                        .size(type_scale::TITLE)
                        .strong(),
                );
            });
        });
        ui.add_space(S3);
        let n = self.basket_len();
        if ui
            .add_enabled(n > 0, block(ui, primary(&crate::tr!("Review {} items", n))))
            .clicked()
        {
            self.set_mode(Mode::Review);
        }
        ui.add_space(S2);
        if ui
            .add_enabled(
                n > 0 && !self.busy && !self.junk.cleaning,
                block(ui, secondary("Clear selection")),
            )
            .clicked()
        {
            self.review.clear();
            self.junk.clear_selection();
        }
        ui.add_space(S3);
        ui.label(caption("Space is released after emptying Trash in Finder.").color(SAFE()));
    }
}
