//! Target & saved-result picker shown inside the Folders tab before or between scans.

use std::path::PathBuf;

use eframe::egui::{self, RichText};

use crate::app::{Mode, SpaceApp};
use crate::i18n::t;
use crate::icons::{self, Icon};
use crate::theme::metrics::{S2, S3, S4};
use crate::theme::type_scale;
use crate::theme::{ACCENT, BG, BORDER, MUTED, PANEL};
use crate::widgets::{caption, format_age, ghost, page_column, primary, secondary};

impl SpaceApp {
    pub(super) fn start_page(&mut self, ctx: &egui::Context) {
        let enabled = !self.busy && self.scan.is_none();
        let mut target = None;
        let mut choose_folder = false;
        let mut resume = false;
        let mut refresh = false;
        let mut toggle_pin: Option<PathBuf> = None;
        let mut remove_saved: Option<PathBuf> = None;
        let mut clear_saved = false;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG()))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        page_column(ui, 860.0, |ui| {
                            ui.add_space(S4);
                            ui.horizontal(|ui| {
                                icons::show(ui, Icon::Scan, 20.0, ACCENT());
                                ui.label(
                                    RichText::new(t("Where to look for space?"))
                                        .size(type_scale::TITLE)
                                        .strong(),
                                );
                            });
                            ui.label(caption("The last result opens instantly. Rescan when you want fresh numbers."));
                            ui.add_space(S3);
                            ui.horizontal_wrapped(|ui| {
                                if let Some(home) = std::env::var_os("HOME") {
                                    icons::show(ui, Icon::Home, 16.0, MUTED());
                                    if ui.add_enabled(enabled, primary("Home folder")).clicked() {
                                        target = Some(PathBuf::from(home));
                                    }
                                }
                                if ui.add_enabled(enabled, secondary("Folder…  ⌘O")).clicked() {
                                    choose_folder = true;
                                }
                                if self.tree.is_some() && ui.add(secondary("Continue")).clicked() {
                                    resume = true;
                                }
                            });
                            ui.add_space(S4);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(t("DISKS")).size(type_scale::CAPTION).color(MUTED()));
                                if self.volumes_loading {
                                    ui.spinner();
                                } else if ui.add(ghost("Refresh disks")).clicked() {
                                    refresh = true;
                                }
                            });
                            let home = std::env::var_os("HOME").map(PathBuf::from);
                            for volume in &self.volumes {
                                if home.as_ref() == Some(&volume.mount) {
                                    continue;
                                }
                                egui::Frame::new()
                                    .fill(PANEL())
                                    .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                                    .corner_radius(6)
                                    .inner_margin(egui::Margin::same(12))
                                    .show(ui, |ui| {
                                        let info_w = (ui.available_width() - 100.0).max(80.0);
                                        ui.horizontal_top(|ui| {
                                            ui.vertical(|ui| {
                                                ui.set_width(info_w);
                                                ui.add(egui::Label::new(RichText::new(&volume.name).strong()).truncate());
                                                ui.add(egui::Label::new(caption(volume.mount.display().to_string())).truncate())
                                                    .on_hover_text(volume.mount.display().to_string());
                                                if volume.total > 0 {
                                                    crate::widgets::meter(ui, volume.used() as f32 / volume.total as f32, MUTED(), 4.0);
                                                    ui.label(caption(crate::tr!(
                                                        "{} used · {} free of {}",
                                                        crate::i18n::bytes(volume.used()),
                                                        crate::i18n::bytes(volume.available),
                                                        crate::i18n::bytes(volume.total)
                                                    )));
                                                }
                                            });
                                            if ui.add_enabled(enabled, secondary("Scan")).clicked() {
                                                target = Some(volume.mount.clone());
                                            }
                                        });
                                    });
                                ui.add_space(S2);
                            }
                            if !self.recent.is_empty() {
                                ui.add_space(S3);
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(t("Saved result")).size(type_scale::LEAD).strong());
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui.add_enabled(enabled, ghost("Clear all")).clicked() {
                                            clear_saved = true;
                                        }
                                    });
                                });
                                let now = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                                let mut ordered: Vec<&space_core::CacheMeta> = self.recent.iter().collect();
                                ordered.sort_by_key(|m| {
                                    let pinned = self.settings.pinned_roots.contains(&PathBuf::from(&m.root_path));
                                    (!pinned, std::cmp::Reverse(m.scanned_at_unix))
                                });
                                for saved in ordered {
                                    let saved_path = PathBuf::from(&saved.root_path);
                                    let pinned = self.settings.pinned_roots.contains(&saved_path);
                                    ui.horizontal_top(|ui| {
                                        ui.vertical(|ui| {
                                            ui.set_width((ui.available_width() - 180.0).max(80.0));
                                            ui.add(egui::Label::new(&saved.root_path).truncate()).on_hover_text(&saved.root_path);
                                            ui.label(caption(format!("{} · {}", crate::i18n::bytes(saved.total_size), format_age(now.saturating_sub(saved.scanned_at_unix)))));
                                        });
                                        if ui.add_enabled(enabled, secondary("Open")).clicked() {
                                            target = Some(saved_path.clone());
                                        }
                                        if ui.add(ghost(if pinned { "Unpin" } else { "Pin" })).clicked() {
                                            toggle_pin = Some(saved_path.clone());
                                        }
                                        if icons::button(ui, Icon::Close, 24.0, "Remove cached result", enabled).clicked() {
                                            remove_saved = Some(saved_path);
                                        }
                                    });
                                    ui.separator();
                                }
                            }
                        });
                    });
            });
        if refresh {
            self.refresh_volumes_async();
        }
        if clear_saved {
            self.clear_all_cache();
        }
        if let Some(path) = remove_saved {
            self.remove_cache_entry(&path);
        }
        if let Some(path) = toggle_pin {
            if let Some(pos) = self.settings.pinned_roots.iter().position(|p| p == &path) {
                self.settings.pinned_roots.remove(pos);
            } else {
                self.settings.pinned_roots.push(path);
            }
            self.settings.save();
        }
        if resume {
            self.choosing_target = false;
            self.mode = Mode::Explore;
        }
        if choose_folder {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                target = Some(path);
            }
        }
        if let Some(path) = target {
            self.choosing_target = false;
            self.start_scan(path, false);
        }
    }
}
