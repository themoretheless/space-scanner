//! Modal windows: shortcuts, the Move to Trash confirmation, local snapshots, rules.

use crate::i18n::t;
use std::time::Duration;

use eframe::egui::{self, Align, Layout, RichText};

use crate::app::SpaceApp;
use crate::rules_panel::{self, RulesAction};
use crate::theme::metrics::{S2, S3, S4, S6};
use crate::theme::type_scale;
use crate::theme::{self, ACCENT, DANGER, MUTED, PANEL_2};
use crate::widgets::{caption, count_noun, divider, ghost, path_size_row, primary, secondary};

fn window(title: &'static str, ctx: &egui::Context) -> egui::Window<'static> {
    egui::Window::new(title)
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(
            egui::Frame::window(&ctx.style())
                .corner_radius(8)
                .inner_margin(egui::Margin::same(S6 as i8)),
        )
}

impl SpaceApp {
    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        self.shortcuts_dialog(ctx);
        self.trash_dialog(ctx);
        self.snapshots_dialog(ctx);
        self.rules_dialog(ctx);
        self.settings_dialog(ctx);
    }

    fn shortcuts_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let mut close = false;
        window("Keyboard shortcuts", ctx).show(ctx, |ui| {
            ui.label(
                RichText::new(t("Keyboard shortcuts"))
                    .size(type_scale::TITLE)
                    .strong(),
            );
            ui.set_width((ctx.screen_rect().width() - 80.0).min(620.0));
            ui.add_space(S3);
            egui::ScrollArea::vertical()
                .max_height((ctx.screen_rect().height() - 220.0).max(120.0))
                .show(ui, |ui| {
            egui::Grid::new("shortcut_grid")
                .num_columns(2)
                .spacing([S6, S2])
                .show(ui, |ui| {
                    for (keys, action) in [
                        ("⌘O", "Choose a folder to scan"),
                        ("⌘R", "Rescan everything"),
                        ("⌘⇧R", "Rescan this folder"),
                        ("⌘F  or  /", "Filter this folder"),
                        ("↑ ↓", "Move through the tree"),
                        ("→ ←", "Expand or collapse a folder; ← on a closed one goes to its parent"),
                        ("Space", "Add to review or take out"),
                        ("Enter", "Open a folder, show a file in Finder, or confirm"),
                        ("Esc", "Go to the parent folder"),
                        ("⌘← ⌘→", "Back / forward"),
                        ("Home", "Jump to the scan root"),
                        ("⌥-click ▸", "Expand a whole branch"),
                        ("Double-click", "Open a folder"),
                        ("⌘-click", "Add to review"),
                        ("⌘1–4", "Switch the map style"),
                        ("⌘J", "Cleanup candidates"),
                        ("⌘⇧C", "Copy the folder path"),
                        ("⌘,", "Settings"),
                        ("?", "This list"),
                    ] {
                        ui.label(RichText::new(t(keys)).monospace().color(ACCENT()));
                        ui.label(t(action));
                        ui.end_row();
                    }
                });
                });
            ui.add_space(S3);
            ui.label(caption(
                "Esc cancels a scan, closes a panel, clears search, then goes up. It never switches tabs.",
            ));
            ui.add_space(S3);
            if ui.add(secondary("Close")).clicked() {
                close = true;
            }
        });
        if close {
            self.show_shortcuts = false;
        }
    }

    /// Confirmation lists exactly what moves, with sizes and where it goes.
    fn trash_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_delete {
            return;
        }
        window("Move to Trash", ctx).show(ctx, |ui| {
            ui.set_width((ctx.screen_rect().width() - 80.0).min(520.0));
            if let Some(progress) = &self.delete_progress {
                let fraction = if progress.items_total > 0 {
                    progress.items_done as f32 / progress.items_total as f32
                } else {
                    0.0
                };
                let elapsed = self
                    .delete_started
                    .map(|t| t.elapsed().as_secs())
                    .unwrap_or(0);
                ui.label(
                    RichText::new(crate::tr!(
                        "Moving to Trash · {}/{} items",
                        progress.items_done,
                        progress.items_total
                    ))
                    .size(type_scale::LEAD)
                    .strong(),
                );
                ui.label(caption(crate::tr!(
                    "{} files · {} folders · {} · {} errors · {}s",
                    progress.files_done,
                    progress.folders_done,
                    crate::i18n::bytes(progress.bytes_processed),
                    progress.error_count,
                    elapsed
                )));
                ui.add_space(S3);
                ui.add(egui::ProgressBar::new(fraction.clamp(0.0, 1.0)).show_percentage());
                if !progress.current_path.as_os_str().is_empty() {
                    let current = progress.current_path.display().to_string();
                    ui.add(
                        egui::Label::new(
                            RichText::new(&current)
                                .size(type_scale::CAPTION)
                                .monospace()
                                .color(MUTED()),
                        )
                        .truncate(),
                    )
                    .on_hover_text(current);
                }
                return;
            }

            let mut items: Vec<(std::path::PathBuf, u64)> = self.basket_items().as_ref().clone();
            items.sort_by_key(|item| std::cmp::Reverse(item.1));
            ui.label(
                RichText::new(crate::tr!(
                    "Move {} ({}) to Trash?",
                    count_noun(items.len(), "item"),
                    crate::i18n::bytes(self.basket_bytes())
                ))
                .size(type_scale::LEAD)
                .strong(),
            );
            ui.label(caption(
                "Items go to the macOS Trash and can be restored from there. Disk space is only released after you empty Trash in Finder.",
            ));
            ui.add_space(S2);
            ui.horizontal(|ui| {
                for (label, n, bytes) in [
                    ("From Disk usage", self.review.len(), self.review_bytes()),
                    ("From Cleanup", self.junk.selected_count(), self.junk.selected_bytes()),
                ] {
                    if n > 0 {
                        ui.label(caption(crate::tr!("{}: {} · {}", t(label), n, crate::i18n::bytes(bytes))));
                    }
                }
            });
            ui.add_space(S2);
            egui::Frame::new()
                .fill(PANEL_2())
                .corner_radius(10)
                .inner_margin(egui::Margin::symmetric(S3 as i8, S2 as i8))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height((ctx.screen_rect().height() - 280.0).clamp(100.0, 260.0))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for (path, size) in items.iter().take(40) {
                                path_size_row(
                                    ui,
                                    &path.display().to_string(),
                                    &crate::i18n::bytes(*size),
                                    28.0,
                                    0.0,
                                );
                            }
                            if items.len() > 40 {
                                ui.label(caption(crate::tr!("…and {} more", items.len() - 40)));
                            }
                        });
                });
            ui.add_space(S4);
            ui.horizontal(|ui| {
                if ui.add(secondary("Cancel")).clicked() {
                    self.confirm_delete = false;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            !items.is_empty(),
                            primary(&crate::tr!(
                                "Move {} to Trash",
                                count_noun(items.len(), "item")
                            )),
                        )
                        .clicked()
                    {
                        self.apply_basket();
                    }
                });
            });
        });
    }

    fn snapshots_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_tm_panel {
            return;
        }
        let mut open = true;
        window("Local snapshots", ctx)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width((ctx.screen_rect().width() - 80.0).min(460.0));
                ui.label(
                    RichText::new(t("Local snapshots"))
                        .size(type_scale::TITLE)
                        .strong(),
                );
                ui.add_space(S2);
                let tm_n = self.tm_snapshots.iter().filter(|s| s.is_tm()).count();
                let other_n = self.tm_snapshots.len().saturating_sub(tm_n);
                ui.label(caption(
                    "Snapshots hold disk space that never appears as ordinary files. Deleting old Time Machine copies can release it.",
                ));
                ui.add_space(S3);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(tm_n.to_string())
                            .size(type_scale::TITLE)
                            .strong()
                            .color(ACCENT()),
                    );
                    ui.label("Time Machine");
                    if other_n > 0 {
                        ui.label(caption(crate::tr!("· {} macOS / other", other_n)));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled(!self.tm_loading && !self.busy, ghost("Refresh"))
                            .clicked()
                        {
                            self.refresh_snapshots();
                        }
                    });
                });
                ui.add_space(S2);
                if self.tm_loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(caption("Loading…"));
                    });
                } else if self.tm_snapshots.is_empty() {
                    ui.label(caption("No local snapshots."));
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(240.0)
                        .show(ui, |ui| {
                            for snap in &self.tm_snapshots {
                                egui::Frame::new()
                                    .fill(PANEL_2())
                                    .corner_radius(8)
                                    .inner_margin(egui::Margin::symmetric(S3 as i8, S2 as i8))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.horizontal_wrapped(|ui| {
                                            ui.vertical(|ui| {
                                                ui.label(RichText::new(snap.label()).strong());
                                                ui.label(caption(snap.kind_label()));
                                            });
                                            ui.with_layout(
                                                Layout::right_to_left(Align::Center),
                                                |ui| {
                                                    let (text, color) = if !snap.is_tm() {
                                                        ("left alone", MUTED())
                                                    } else if snap.purgeable {
                                                        ("macOS can drop this", MUTED())
                                                    } else {
                                                        ("keeps disk used", ACCENT())
                                                    };
                                                    ui.label(caption(text).color(color));
                                                },
                                            );
                                        });
                                    });
                                ui.add_space(4.0);
                            }
                        });
                }
                ui.add_space(S3);
                divider(ui);
                ui.add_space(S3);
                let keep = usize::from(tm_n > 1);
                let remove = tm_n.saturating_sub(keep);
                ui.horizontal(|ui| {
                    if ui.add(secondary("Close")).clicked() {
                        self.show_tm_panel = false;
                        self.confirm_tm_clean = false;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let can = remove > 0 && !self.busy && !self.tm_loading;
                        let label = if tm_n <= 1 {
                            "Delete snapshot…".to_string()
                        } else {
                            crate::tr!("Delete {} old…", remove)
                        };
                        if ui.add_enabled(can, secondary(&label)).clicked() {
                            self.confirm_tm_clean = true;
                        }
                    });
                });
                if self.confirm_tm_clean {
                    ui.add_space(S3);
                    egui::Frame::new()
                        .fill(PANEL_2())
                        .stroke(egui::Stroke::new(1.0_f32, DANGER().gamma_multiply(0.6)))
                        .corner_radius(10)
                        .inner_margin(egui::Margin::same(S3 as i8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new(if keep == 0 {
                                    "Permanently delete this snapshot?".to_string()
                                } else {
                                    crate::tr!("Permanently delete {} oldest, keep the newest?", remove)
                                })
                                .strong(),
                            );
                            ui.label(caption(
                                "Snapshots do not go to Trash — this cannot be undone. macOS update snapshots are left alone.",
                            ));
                            ui.add_space(S2);
                            ui.horizontal(|ui| {
                                if ui.add(secondary("Cancel")).clicked() {
                                    self.confirm_tm_clean = false;
                                }
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(t("Delete permanently"))
                                                .color(egui::Color32::WHITE)
                                                .strong(),
                                        )
                                        .fill(DANGER())
                                        .corner_radius(theme::metrics::RADIUS_CONTROL)
                                        .min_size(egui::vec2(0.0, theme::metrics::CONTROL_H)),
                                    )
                                    .clicked()
                                {
                                    self.apply_tm_clean();
                                }
                            });
                        });
                }
            });
        if !open {
            self.show_tm_panel = false;
        }
    }

    fn rules_dialog(&mut self, ctx: &egui::Context) {
        if !self.rules.open {
            return;
        }
        let mut open = true;
        let mut reveal: Option<String> = None;
        egui::Window::new(t("Cleanup rules"))
            .collapsible(false)
            .resizable(true)
            .open(&mut open)
            .default_size([
                (ctx.screen_rect().width() - 48.0).min(1040.0),
                (ctx.screen_rect().height() - 80.0).min(640.0),
            ])
            .min_width(640.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 12.0])
            .show(ctx, |ui| {
                if self.rule_mgr.is_none() && self.rules.error.is_none() {
                    self.start_rule_mgr_load();
                }
                if let Some(mgr) = self.rule_mgr.as_mut() {
                    match rules_panel::draw(ui, &mut self.rules, mgr, &mut self.settings.flags) {
                        RulesAction::Close => self.rules.open = false,
                        RulesAction::PrefsChanged => {
                            self.mark_settings_dirty();
                            if self.junk.open {
                                self.start_junk_scan();
                            }
                        }
                        RulesAction::Reveal(path) => reveal = Some(path),
                        RulesAction::None => {}
                    }
                } else if self.rule_mgr_loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(t("Loading rules…"));
                    });
                    ctx.request_repaint_after(Duration::from_millis(50));
                } else if let Some(err) = &self.rules.error {
                    ui.colored_label(theme::DANGER(), err);
                    if ui.add(secondary("Close")).clicked() {
                        self.rules.open = false;
                    }
                }
            });
        if !open {
            self.rules.open = false;
        }
        if let Some(path) = reveal {
            self.reveal_rule_path(&path);
        }
    }
}
