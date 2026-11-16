//! Quick Clean: simple goal-oriented disk cleanup screen.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align, Layout, RichText};

use super::Density;
use crate::app::{Mode, SpaceApp};
use crate::downloads::DownloadKind;
use crate::i18n::t;
use crate::icons::{self, Icon};
use crate::junk_panel::{self, JunkGroup};
use crate::theme::metrics::{S2, S3, S4};
use crate::theme::type_scale;
use crate::theme::{ACCENT, BG, BORDER, DANGER, MUTED, PANEL, TEXT};
use crate::volumes::VolumeInfo;
use crate::widgets::{caption, divider, empty_state, format_age, meter, page_column, secondary};

impl SpaceApp {
    pub(super) fn cleanup_page(&mut self, ctx: &egui::Context, d: Density) {
        let margin = if d == Density::Narrow { S3 } else { S4 };
        let mut rescan = false;
        let mut open_rules = false;
        let mut open_tm = false;
        let mut open_folders = false;
        let mut reveal: Option<PathBuf> = None;
        let mut delete_now_findings: Option<Vec<space_clean::Finding>> = None;
        let mut delete_now_download: Option<crate::app::BasketItem> = None;

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG()).inner_margin(egui::Margin {
                left: margin as i8,
                right: margin as i8,
                top: 12,
                bottom: 12,
            }))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("quick_clean_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        page_column(ui, 860.0, |ui| {
                            ui.add_space(4.0);
                            self.quick_clean_header_card(ui, &mut rescan, &mut open_rules);
                            ui.add_space(S3);

                            if let Some(err) = &self.junk.error {
                                ui.label(RichText::new(err).color(DANGER()));
                                ui.add_space(S2);
                            }

                            if let Some(report) = self.junk.report() {
                                if !report.skipped_roots.is_empty() {
                                    let skipped_id = ui.id().with("junk_skipped_expanded");
                                    let mut open = ui.data(|d| d.get_temp::<bool>(skipped_id)).unwrap_or(false);
                                    let arrow = if open { "▾" } else { "▸" };
                                    let label = crate::tr!(
                                        "Skipped / inaccessible ({})",
                                        report.skipped_roots.len()
                                    );
                                    let resp = ui
                                        .add(
                                            egui::Button::new(
                                                crate::widgets::caption(format!("{arrow} {label}"))
                                                    .color(DANGER()),
                                            )
                                            .frame(false)
                                            .min_size(egui::vec2(24.0, 24.0)),
                                        )
                                        .on_hover_text(t(
                                            "Paths skipped during quick clean scan due to permissions or safety rules.",
                                        ));
                                    if resp.has_focus() {
                                        crate::theme::paint_focus_ring(ui.painter(), resp.rect);
                                    }
                                    if resp.clicked() {
                                        open = !open;
                                        ui.data_mut(|d| d.insert_temp(skipped_id, open));
                                    }
                                    if open {
                                        egui::ScrollArea::vertical()
                                            .id_salt("junk_skipped_list")
                                            .max_height(100.0)
                                            .show(ui, |ui| {
                                                for path in &report.skipped_roots {
                                                    ui.add(
                                                        egui::Label::new(
                                                            RichText::new(path)
                                                                .monospace()
                                                                .size(crate::theme::type_scale::CAPTION)
                                                                .color(MUTED()),
                                                        )
                                                        .truncate(),
                                                    );
                                                }
                                            });
                                    }
                                    ui.add_space(S2);
                                }
                            }

                            junk_panel::draw_group_card(
                                ui,
                                &mut self.junk,
                                JunkGroup::Safe,
                                &mut reveal,
                                &mut delete_now_findings,
                            );
                            if self.junk.group_stats(JunkGroup::Safe).0 > 0 {
                                ui.add_space(S3);
                            }

                            let downloads_shown =
                                !self.downloads.items().is_empty() || self.downloads.loading;
                            self.downloads_card(ui, &mut reveal, &mut delete_now_download);
                            if downloads_shown {
                                ui.add_space(S3);
                            }

                            junk_panel::draw_group_card(
                                ui,
                                &mut self.junk,
                                JunkGroup::Review,
                                &mut reveal,
                                &mut delete_now_findings,
                            );
                            if let Some(findings) = delete_now_findings {
                                let items = findings
                                    .iter()
                                    .map(crate::app::SpaceApp::basket_item_from_finding)
                                    .collect();
                                self.request_delete_direct(
                                    items,
                                    space_clean::DeleteMethod::Permanent,
                                );
                            }
                            if let Some(item) = delete_now_download {
                                self.request_delete_direct(
                                    vec![item],
                                    space_clean::DeleteMethod::Permanent,
                                );
                            }
                            if self.junk.group_stats(JunkGroup::Review).0 > 0 {
                                ui.add_space(S3);
                            }

                            if self.snapshots_card(ui) {
                                open_tm = true;
                            }

                            let all_empty = !self.junk.loading
                                && !self.downloads.loading
                                && !self.junk.has_cleanable_items()
                                && self.downloads.items().is_empty();
                            if all_empty {
                                empty_state(
                                    ui,
                                    "Nothing to clean right now.",
                                    "Caches and temporary folders are already clean. Open Folders to inspect large files.",
                                );
                                ui.add_space(S3);
                                ui.vertical_centered(|ui| {
                                    if ui.add(secondary("Browse folders")).clicked() {
                                        open_folders = true;
                                    }
                                });
                            }
                            ui.add_space(S4);
                        });
                    });
            });

        if rescan {
            self.start_quick_clean_scan();
        }
        if open_rules {
            self.open_rules_panel();
        }
        if open_tm {
            self.open_tm_panel();
        }
        if open_folders {
            self.mode = Mode::Explore;
        }
        if let Some(path) = reveal {
            crate::widgets::reveal_in_os(&path);
        }
    }

    fn quick_clean_header_card(&self, ui: &mut egui::Ui, rescan: &mut bool, open_rules: &mut bool) {
        let primary_vol = primary_volume(&self.volumes);
        let found_total = self
            .junk
            .total_cleanable_bytes()
            .saturating_add(self.downloads.total_bytes());
        let selected_total = self.basket_bytes();
        let selected_len = self.basket_len();
        let scanning = self.junk.loading || self.downloads.loading;

        egui::Frame::new()
            .fill(PANEL())
            .stroke(egui::Stroke::new(1.0_f32, BORDER()))
            .corner_radius(10)
            .inner_margin(egui::Margin::same(S4 as i8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());

                if let Some(vol) = primary_vol {
                    ui.horizontal(|ui| {
                        icons::show(ui, Icon::Disk, 18.0, ACCENT());
                        ui.label(RichText::new(&vol.name).size(type_scale::LEAD).strong());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if vol.total > 0 {
                                ui.label(caption(crate::tr!(
                                    "{} free of {}",
                                    crate::i18n::bytes(vol.available),
                                    crate::i18n::bytes(vol.total)
                                )));
                            } else if self.volumes_loading {
                                ui.spinner();
                            }
                        });
                    });
                    if vol.total > 0 {
                        ui.add_space(6.0);
                        let frac = vol.used() as f32 / vol.total.max(1) as f32;
                        meter(ui, frac, ACCENT(), 5.0);
                    }
                    ui.add_space(S3);
                    divider(ui);
                    ui.add_space(S3);
                }

                ui.horizontal_wrapped(|ui| {
                    ui.vertical(|ui| {
                        if scanning {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(
                                    RichText::new(t("Looking through caches, logs and temp…"))
                                        .size(type_scale::LEAD)
                                        .strong(),
                                );
                            });
                            let p = &self.junk.progress;
                            let elapsed = self
                                .junk
                                .scan_started
                                .map(|s| s.elapsed().as_secs())
                                .unwrap_or(0);
                            let stage = if p.rules_total == 0 {
                                t("Loading rules…").to_string()
                            } else {
                                crate::tr!("Scanning {}/{} rules", p.rules_done, p.rules_total)
                            };
                            ui.label(caption(crate::tr!(
                                "{} · {} ({}) · {}s",
                                stage,
                                crate::i18n::bytes(p.bytes_found),
                                p.findings_found,
                                elapsed
                            )));
                        } else {
                            let label = if selected_len > 0 {
                                t("Ready to clean")
                            } else {
                                t("Cleanup candidates")
                            };
                            ui.label(caption(label));
                            ui.horizontal_wrapped(|ui| {
                                let headline = if selected_len > 0 {
                                    selected_total
                                } else {
                                    found_total
                                };
                                ui.label(
                                    RichText::new(crate::i18n::bytes(headline))
                                        .size(type_scale::DISPLAY)
                                        .strong()
                                        .color(if selected_len > 0 { ACCENT() } else { TEXT() }),
                                );
                                if selected_len > 0 && found_total > selected_total {
                                    ui.label(caption(crate::tr!(
                                        "· found {} total",
                                        crate::i18n::bytes(found_total)
                                    )));
                                }
                            });
                        }
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(secondary("Rules…"))
                            .on_hover_text(t("Turn individual rules on or off"))
                            .clicked()
                        {
                            *open_rules = true;
                        }
                        if ui
                            .add_enabled(!scanning && !self.busy, secondary("Refresh"))
                            .clicked()
                        {
                            *rescan = true;
                        }
                    });
                });

                if self.junk.loading {
                    ui.add_space(S2);
                    let p = &self.junk.progress;
                    let fraction = if p.roots_total > 0 {
                        p.roots_done as f32 / p.roots_total as f32
                    } else if p.rules_total > 0 {
                        p.rules_done as f32 / p.rules_total as f32
                    } else {
                        0.0
                    };
                    ui.add(
                        egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
                            .desired_width(ui.available_width())
                            .animate(p.rules_total == 0),
                    );
                    if !p.current_path.as_os_str().is_empty() {
                        ui.add(
                            egui::Label::new(
                                RichText::new(p.current_path.display().to_string())
                                    .size(type_scale::CAPTION)
                                    .monospace()
                                    .color(MUTED()),
                            )
                            .truncate(),
                        );
                    }
                }
            });
    }

    fn downloads_card(
        &mut self,
        ui: &mut egui::Ui,
        reveal: &mut Option<PathBuf>,
        delete_now: &mut Option<crate::app::BasketItem>,
    ) {
        if self.downloads.items().is_empty() {
            if self.downloads.loading {
                egui::Frame::new()
                    .fill(PANEL())
                    .stroke(egui::Stroke::new(1.0_f32, BORDER()))
                    .corner_radius(10)
                    .inner_margin(egui::Margin::same(S4 as i8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new(t("Downloads")).strong());
                            ui.label(caption("Scanning ~/Downloads…"));
                        });
                    });
            }
            return;
        }

        let total_count = self.downloads.items().len();
        let total_bytes = self.downloads.total_bytes();
        let selected_count = self.downloads.selected_count();
        let has_installers = self.downloads.has_installers();
        let mut open = self.downloads.open;
        let mut toggle_all = None;
        let mut select_installers = false;
        let mut toggle_item: Option<(usize, bool)> = None;

        junk_panel::group_card(
            ui,
            t("Downloads"),
            t("Installers (.dmg, .pkg) and large files in ~/Downloads"),
            t("Check"),
            ACCENT(),
            t("Review files before moving to Trash"),
            total_count,
            total_bytes,
            selected_count,
            &mut open,
            |on| toggle_all = Some(on),
            |ui| {
                if has_installers {
                    ui.horizontal(|ui| {
                        ui.label(caption(
                            "Files in Downloads are never preselected automatically.",
                        ));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add(secondary("Select installers (.dmg, .pkg)"))
                                .clicked()
                            {
                                select_installers = true;
                            }
                        });
                    });
                    ui.add_space(6.0);
                }

                ui.spacing_mut().item_spacing.y = 2.0;
                for (index, item) in self.downloads.items().iter().enumerate() {
                    let mut checked = self.downloads.is_selected(index);
                    let kind_text = match (item.kind, item.is_dir) {
                        (DownloadKind::Installer, _) => t("Installer"),
                        (DownloadKind::LargeItem, true) => t("Large folder"),
                        (DownloadKind::LargeItem, false) => t("Large file"),
                    };
                    let meta = match item.age_secs {
                        Some(secs) => format!("{kind_text} · {}", format_age(secs)),
                        None => kind_text.to_owned(),
                    };

                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut checked, "").changed() {
                            toggle_item = Some((index, checked));
                        }
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let name_resp = ui
                                .add(
                                    egui::Label::new(RichText::new(&item.name).color(TEXT()))
                                        .truncate(),
                                )
                                .on_hover_text(item.path.display().to_string());
                            name_resp.context_menu(|ui| {
                                if ui
                                    .button(
                                        RichText::new(t("Delete permanently…"))
                                            .color(crate::theme::DANGER()),
                                    )
                                    .clicked()
                                {
                                    *delete_now = Some(crate::app::BasketItem {
                                        path: item.path.clone(),
                                        bytes: item.bytes,
                                        is_dir: item.is_dir,
                                        junk_guard: None,
                                    });
                                    ui.close_menu();
                                }
                                if ui.button(t("Show in Finder")).clicked() {
                                    *reveal = Some(item.path.clone());
                                    ui.close_menu();
                                }
                            });
                            ui.label(caption(meta));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add(
                                    egui::Button::new(caption("Finder"))
                                        .frame(false)
                                        .min_size(egui::vec2(0.0, 24.0)),
                                )
                                .on_hover_text(t("Show in Finder"))
                                .clicked()
                            {
                                *reveal = Some(item.path.clone());
                            }
                            ui.label(
                                RichText::new(crate::i18n::bytes(item.bytes))
                                    .monospace()
                                    .size(type_scale::MONO)
                                    .color(if checked { ACCENT() } else { TEXT() }),
                            );
                        });
                    });
                }
            },
        );

        self.downloads.open = open;
        if let Some(on) = toggle_all {
            self.downloads.set_all(on);
        }
        if select_installers {
            self.downloads.select_installers();
        }
        if let Some((idx, on)) = toggle_item {
            self.downloads.set_selected(idx, on);
        }
    }

    fn snapshots_card(&self, ui: &mut egui::Ui) -> bool {
        let tm_n = self.tm_snapshots.iter().filter(|s| s.is_tm()).count();
        if tm_n == 0 {
            return false;
        }
        let mut open = false;
        egui::Frame::new()
            .fill(PANEL())
            .stroke(egui::Stroke::new(1.0_f32, BORDER()))
            .corner_radius(10)
            .inner_margin(egui::Margin::same(S4 as i8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(t("Local Time Machine snapshots"))
                                    .size(type_scale::LEAD)
                                    .strong(),
                            );
                            ui.label(
                                RichText::new(crate::tr!("{} local snapshots", tm_n))
                                    .size(type_scale::CAPTION)
                                    .color(ACCENT()),
                            );
                        });
                        ui.label(caption(
                            "Snapshots hold disk space that never appears as ordinary files. Deleting old Time Machine copies can release it.",
                        ));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(secondary("Manage snapshots…")).clicked() {
                            open = true;
                        }
                    });
                });
            });
        open
    }
}

fn primary_volume(volumes: &[VolumeInfo]) -> Option<&VolumeInfo> {
    if let Some(home) = std::env::var_os("HOME") {
        let home_str = home.to_string_lossy();
        // Prefer real mounted volumes (with `total > 0`) over the `$HOME` fast placeholder.
        let home_path = Path::new(home_str.as_ref());
        if let Some(v) = volumes
            .iter()
            .filter(|v| v.total > 0)
            .filter(|v| {
                home_path.starts_with(&v.mount) || v.mount == Path::new("/System/Volumes/Data")
            })
            .max_by_key(|v| v.mount.as_os_str().len())
        {
            return Some(v);
        }
    }
    volumes
        .iter()
        .find(|v| v.total > 0)
        .or_else(|| volumes.first())
}
