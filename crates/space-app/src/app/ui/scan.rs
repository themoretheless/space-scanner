//! Scan states while there is no result to show yet: scanning, opening a saved
//! result, stopped, failed. Every state has a way forward; none is a dead end.

use eframe::egui::{self, RichText};

use crate::app::{Mode, SpaceApp};
use crate::i18n::t;
use crate::icons::{self, Icon};
use crate::theme::metrics::{S2, S3, S4};
use crate::theme::type_scale;
use crate::theme::{ACCENT, BG, DANGER, MUTED, PANEL_2, TEXT};
use crate::widgets::{caption, ghost, primary, secondary, strong};

enum Next {
    Cancel,
    Again,
    Pick,
    Overview,
}

impl SpaceApp {
    /// Accent wash while a folder is dragged over the window; dropping it scans it.
    pub(super) fn drop_tint(&self, ctx: &egui::Context) {
        if !ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            return;
        }
        let rect = ctx.screen_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("drop_tint"),
        ));
        painter.rect_filled(rect, 0.0, ACCENT().gamma_multiply(0.12));
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            t("Drop to scan this folder"),
            egui::FontId::new(type_scale::TITLE, crate::theme::bold()),
            ACCENT(),
        );
    }

    pub(super) fn scan_state_page(&mut self, ctx: &egui::Context) {
        let mut next = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG()))
            .show(ctx, |ui| {
                let h = ui.available_height();
                ui.vertical_centered(|ui| {
                    ui.add_space((h * 0.5 - 150.0).max(S4));
                    ui.set_max_width(520.0);
                    if let Some(outcome) = self.scan_outcome.clone() {
                        let failed = outcome.error.is_some();
                        badge_icon(
                            ui,
                            if failed { Icon::Alert } else { Icon::Pause },
                            if failed { DANGER() } else { MUTED() },
                        );
                        ui.add_space(S3);
                        if failed {
                            ui.label(strong(
                                crate::tr!("Couldn't scan {}", outcome.path.display()),
                                type_scale::TITLE,
                            ));
                            ui.label(
                                RichText::new(t(
                                    "The folder may have moved, or the app needs permission to read it.",
                                ))
                                .color(MUTED()),
                            );
                            if let Some(e) = &outcome.error {
                                ui.add_space(4.0);
                                ui.label(RichText::new(e).monospace().color(DANGER()));
                            }
                        } else {
                            ui.label(strong("Scan stopped", type_scale::TITLE));
                            ui.label(
                                RichText::new(crate::tr!(
                                    "You stopped it after {} in {} files. Nothing was saved.",
                                    crate::i18n::bytes(outcome.bytes_seen),
                                    outcome.files_seen
                                ))
                                .color(MUTED()),
                            );
                        }
                        ui.add_space(S4);
                        ui.horizontal(|ui| {
                            let w = 140.0 + 150.0 + 100.0 + 2.0 * S2;
                            ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
                            if ui
                                .add(primary(if failed { "Try again" } else { "Scan again" }))
                                .clicked()
                            {
                                next = Some(Next::Again);
                            }
                            if ui.add(secondary("Another folder…")).clicked() {
                                next = Some(Next::Pick);
                            }
                            if ui.add(ghost("Overview")).clicked() {
                                next = Some(Next::Overview);
                            }
                        });
                        return;
                    }

                    ui.add(egui::Spinner::new().size(36.0).color(ACCENT()));
                    ui.add_space(S3);
                    if self.scan.is_none() {
                        // Loading an index from disk: quick, and nothing to cancel.
                        ui.label(strong("Opening the saved result", type_scale::TITLE));
                        ui.label(caption(
                            "Takes a second or two. Rescan updates anything that changed.",
                        ));
                        return;
                    }
                    let target = self
                        .scan_target
                        .as_ref()
                        .map(|p| super::shell::short_root(&p.to_string_lossy()))
                        .unwrap_or_default();
                    ui.label(strong(crate::tr!("Scanning {}", target), type_scale::TITLE));
                    if let Some(p) = &self.progress {
                        ui.label(
                            RichText::new(crate::tr!(
                                "{} files · {} so far",
                                p.files_seen,
                                crate::i18n::bytes(p.bytes_seen)
                            ))
                            .color(MUTED()),
                        );
                        ui.add_space(S2);
                        ui.add(
                            egui::ProgressBar::new(0.0)
                                .desired_width(320.0)
                                .desired_height(4.0)
                                .animate(true),
                        );
                        ui.add_space(S2);
                        ui.add(
                            egui::Label::new(
                                RichText::new(&p.current_path)
                                    .monospace()
                                    .size(type_scale::CAPTION)
                                    .color(MUTED()),
                            )
                            .truncate(),
                        );
                    }
                    ui.add_space(S4);
                    if ui.add(secondary("Cancel scan")).clicked() {
                        next = Some(Next::Cancel);
                    }
                    ui.add_space(S2);
                    ui.label(caption("Esc cancels."));
                });
            });
        match next {
            Some(Next::Cancel) => {
                if let Some(s) = &self.scan {
                    s.cancel();
                }
            }
            Some(Next::Again) => {
                if let Some(path) = self.scan_outcome.take().map(|o| o.path) {
                    self.start_scan(path, true);
                }
            }
            Some(Next::Pick) => self.pick_folder_and_scan(),
            Some(Next::Overview) => {
                self.scan_outcome = None;
                self.set_mode(Mode::Home);
            }
            None => {}
        }
    }
}

fn badge_icon(ui: &mut egui::Ui, icon: Icon, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 22.0, PANEL_2());
    icons::paint(
        ui.painter(),
        icon,
        egui::Rect::from_center_size(rect.center(), egui::vec2(22.0, 22.0)),
        color,
    );
    let _ = TEXT();
}
