//! Overview: how full the disk is, the two ways to get space back, saved scans
//! and other disks. It answers "where do I start" without asking a question first.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align, Layout, RichText, Sense};

use super::shell::{fill_for, short_root};
use crate::app::{Mode, SpaceApp};
use crate::i18n::t;
use crate::icons::{self, Icon};
use crate::theme::metrics::{RADIUS_CARD, S2, S3, S4, S6};
use crate::theme::type_scale;
use crate::theme::{ACCENT, BG, BORDER, DANGER, MUTED, PANEL, PANEL_2, SAFE, TEXT};
use crate::widgets::{
    badge, caption, card, format_age, ghost, meter, page_column, primary, secondary, section_label,
    strong,
};

enum Act {
    Scan(PathBuf, bool),
    Pick,
    Junk,
    Usage,
    RefreshDisks,
}

impl SpaceApp {
    pub(super) fn overview_page(&mut self, ctx: &egui::Context) {
        let enabled = !self.busy && self.scan.is_none();
        let mut act = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG()))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        page_column(ui, 880.0, |ui| {
                            ui.add_space(36.0);
                            self.disk_health(ui);
                            ui.add_space(28.0);
                            self.action_cards(ui, enabled, &mut act);
                            ui.add_space(28.0);
                            self.recent_scans(ui, enabled, &mut act);
                            self.other_disks(ui, enabled, &mut act);
                            ui.add_space(S4);
                            ui.label(caption(
                                "You can also drop a folder anywhere in this window.",
                            ));
                            ui.add_space(36.0);
                        });
                    });
            });
        match act {
            Some(Act::Scan(path, force)) => self.start_scan(path, force),
            Some(Act::Pick) => self.pick_folder_and_scan(),
            Some(Act::Junk) => self.find_junk(),
            Some(Act::Usage) => self.set_mode(Mode::Explore),
            Some(Act::RefreshDisks) => self.refresh_volumes_async(),
            None => {}
        }
    }

    /// "26.1 GB free of 251.9 GB" with the share the home folder takes, if scanned.
    fn disk_health(&self, ui: &mut egui::Ui) {
        let Some(vol) = self.startup_volume() else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(caption("Reading disks…"));
            });
            return;
        };
        let used = vol.used().min(vol.total);
        let frac = used as f32 / vol.total.max(1) as f32;
        ui.horizontal(|ui| {
            ui.label(section_label("Startup disk"));
            ui.label(caption(format!("· {}", vol.name)));
        });
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.label(strong(
                crate::tr!("{} free", crate::i18n::bytes(vol.available)),
                type_scale::DISPLAY,
            ));
            ui.label(
                RichText::new(crate::tr!("of {}", crate::i18n::bytes(vol.total)))
                    .size(type_scale::LEAD)
                    .color(MUTED()),
            );
            let pct = (frac * 100.0).round() as u32;
            if frac >= 0.9 {
                badge(
                    ui,
                    &crate::tr!("Almost full · {}%", pct),
                    DANGER(),
                    "Under 10% free: apps and updates may fail",
                );
            } else {
                badge(
                    ui,
                    &crate::tr!("{}% full", pct),
                    MUTED(),
                    "Share of the disk in use",
                );
            }
        });
        ui.add_space(S3);

        // Home folder from its last scan, the rest of the disk, then free space.
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let home_bytes = home.as_ref().and_then(|h| {
            self.recent
                .iter()
                .find(|m| Path::new(&m.root_path) == h.as_path())
                .map(|m| m.total_size.min(used))
        });
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 10.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 5.0, PANEL_2());
        let total = vol.total.max(1) as f32;
        let w = rect.width();
        let mut x = rect.left();
        let mut seg = |bytes: u64, color: egui::Color32| {
            let sw = (bytes as f32 / total * w).max(if bytes > 0 { 3.0 } else { 0.0 });
            let r =
                egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(sw, rect.height()));
            p.rect_filled(r, 5.0, color);
            x += sw + 2.0;
        };
        match home_bytes {
            Some(hb) => {
                seg(hb, ACCENT());
                seg(used.saturating_sub(hb), MUTED().gamma_multiply(0.7));
            }
            None => seg(used, fill_for(frac)),
        }
        ui.add_space(S2);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 18.0;
            if let Some(hb) = home_bytes {
                legend(
                    ui,
                    ACCENT(),
                    &crate::tr!("Home folder · {} (last scan)", crate::i18n::bytes(hb)),
                );
                legend(
                    ui,
                    MUTED().gamma_multiply(0.7),
                    &crate::tr!(
                        "Rest of the disk · {}",
                        crate::i18n::bytes(used.saturating_sub(hb))
                    ),
                );
            } else {
                legend(
                    ui,
                    fill_for(frac),
                    &crate::tr!("Used · {}", crate::i18n::bytes(used)),
                );
            }
            legend(
                ui,
                PANEL_2(),
                &crate::tr!("Free · {}", crate::i18n::bytes(vol.available)),
            );
        });
    }

    fn action_cards(&self, ui: &mut egui::Ui, enabled: bool, act: &mut Option<Act>) {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let two = ui.available_width() >= 640.0;
        let draw_usage = |ui: &mut egui::Ui, act: &mut Option<Act>| {
            action_card(ui, Icon::Usage, ACCENT(), "See what takes space",
                "Scans your home folder. The first scan takes a minute or so, later ones open instantly.",
                |ui| {
                    if let Some(home) = &home {
                        if ui.add_enabled(enabled, primary("Scan home folder")).clicked() {
                            *act = Some(Act::Scan(home.clone(), false));
                        }
                    }
                    if ui.add_enabled(enabled, ghost("Another folder…  ⌘O")).clicked() {
                        *act = Some(Act::Pick);
                    }
                    if self.tree.is_some() && ui.add(secondary("Open Disk usage")).clicked() {
                        *act = Some(Act::Usage);
                    }
                });
        };
        let draw_junk =
            |ui: &mut egui::Ui, act: &mut Option<Act>| {
                let label = match self.junk.cleanable_bytes() {
                    Some(b) => crate::tr!("Show {} found", crate::i18n::bytes(b)),
                    None => t("Find junk").to_owned(),
                };
                action_card(ui, Icon::Cleanup, SAFE(), "Clear caches and logs",
                "Finds app caches, logs and temporary files. Nothing is removed until you confirm.",
                |ui| {
                    if ui.add(secondary(&label)).clicked() {
                        *act = Some(Act::Junk);
                    }
                });
            };
        if two {
            ui.columns(2, |cols| {
                draw_usage(&mut cols[0], act);
                draw_junk(&mut cols[1], act);
            });
        } else {
            draw_usage(ui, act);
            ui.add_space(S3);
            draw_junk(ui, act);
        }
    }

    fn recent_scans(&self, ui: &mut egui::Ui, enabled: bool, act: &mut Option<Act>) {
        if self.recent.is_empty() {
            return;
        }
        let now = crate::app::now_unix();
        ui.label(section_label("Recent scans"));
        ui.add_space(S2);
        card()
            .inner_margin(egui::Margin::symmetric(S4 as i8, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                for (i, saved) in self.recent.iter().take(6).enumerate() {
                    if i > 0 {
                        crate::widgets::divider(ui);
                    }
                    let stale = std::fs::metadata(&saved.root_path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .is_some_and(|d| d.as_secs() > saved.root_mtime_secs);
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 56.0),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            icons::show(ui, Icon::Folder, 18.0, MUTED());
                            ui.add_space(4.0);
                            let text_w = (ui.available_width() - 250.0).max(120.0);
                            ui.vertical(|ui| {
                                ui.set_width(text_w);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&saved.root_path).monospace().color(TEXT()),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(&saved.root_path);
                                ui.label(caption(format!(
                                    "{} · {}",
                                    crate::i18n::bytes(saved.total_size),
                                    format_age(now.saturating_sub(saved.scanned_at_unix))
                                )));
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let path = PathBuf::from(&saved.root_path);
                                if ui.add_enabled(enabled, secondary("Open")).clicked() {
                                    *act = Some(Act::Scan(path.clone(), false));
                                }
                                if ui
                                    .add_enabled(enabled, ghost("Rescan"))
                                    .on_hover_text(t("Scan again for current numbers"))
                                    .clicked()
                                {
                                    *act = Some(Act::Scan(path, true));
                                }
                                if stale {
                                    badge(
                                        ui,
                                        "Files changed since",
                                        ACCENT(),
                                        "The folder changed after this scan was saved",
                                    );
                                }
                            });
                        },
                    );
                }
            });
        ui.add_space(28.0);
    }

    fn other_disks(&self, ui: &mut egui::Ui, enabled: bool, act: &mut Option<Act>) {
        let startup = self.startup_volume().map(|v| v.mount.clone());
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let disks: Vec<_> = self
            .volumes
            .iter()
            .filter(|v| Some(&v.mount) != startup.as_ref() && Some(&v.mount) != home.as_ref())
            .collect();
        ui.horizontal(|ui| {
            ui.label(section_label("Other disks"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.volumes_loading {
                    ui.spinner();
                } else if ui.add(ghost("Refresh")).clicked() {
                    *act = Some(Act::RefreshDisks);
                }
            });
        });
        ui.add_space(S2);
        if disks.is_empty() {
            ui.label(caption(if self.volumes_loading {
                "Reading disks…"
            } else {
                "No other disks are connected."
            }));
            return;
        }
        let cols = if ui.available_width() >= 640.0 { 2 } else { 1 };
        for row in disks.chunks(cols) {
            ui.columns(cols, |c| {
                for (i, vol) in row.iter().enumerate() {
                    let ui = &mut c[i];
                    card()
                        .inner_margin(egui::Margin::symmetric(S4 as i8, S3 as i8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                icons::show(ui, Icon::Disk, 20.0, MUTED());
                                ui.add_space(4.0);
                                let w = (ui.available_width() - 76.0).max(80.0);
                                ui.vertical(|ui| {
                                    ui.set_width(w);
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&vol.name)
                                                .family(crate::theme::medium())
                                                .color(TEXT()),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(vol.mount.display().to_string());
                                    if vol.total > 0 {
                                        let used = vol.used() as f32 / vol.total as f32;
                                        ui.label(caption(crate::tr!(
                                            "{} free of {}",
                                            crate::i18n::bytes(vol.available),
                                            crate::i18n::bytes(vol.total)
                                        )));
                                        meter(ui, used, fill_for(used), 4.0);
                                    } else {
                                        ui.label(caption(short_root(&vol.mount.to_string_lossy())));
                                    }
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui.add_enabled(enabled, secondary("Scan")).clicked() {
                                        *act = Some(Act::Scan(vol.mount.clone(), false));
                                    }
                                });
                            });
                        });
                }
            });
            ui.add_space(S2);
        }
    }
}

fn legend(ui: &mut egui::Ui, color: egui::Color32, text: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), Sense::hover());
        ui.painter().rect_filled(r, 2.0, color);
        if color == PANEL_2() {
            ui.painter().rect_stroke(
                r,
                2.0,
                egui::Stroke::new(1.0_f32, BORDER().lerp_to_gamma(MUTED(), 0.3)),
                egui::StrokeKind::Inside,
            );
        }
        ui.label(caption(text.to_owned()));
    });
}

/// A card with an icon, a title, one sentence and its buttons.
fn action_card(
    ui: &mut egui::Ui,
    icon: Icon,
    tint: egui::Color32,
    title: &str,
    body: &str,
    buttons: impl FnOnce(&mut egui::Ui),
) {
    egui::Frame::new()
        .fill(PANEL())
        .stroke(egui::Stroke::new(1.0_f32, BORDER()))
        .corner_radius(RADIUS_CARD + 2)
        .inner_margin(egui::Margin::same(20))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(176.0);
            icons::show(ui, icon, 22.0, tint);
            ui.add_space(6.0);
            ui.label(strong(title, type_scale::TITLE));
            ui.add(egui::Label::new(RichText::new(t(body)).color(MUTED())).wrap());
            ui.add_space(S3);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = S2;
                buttons(ui);
            });
        });
    let _ = S6;
}
