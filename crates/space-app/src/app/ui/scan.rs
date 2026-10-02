//! Scan page: one primary choice, a specific-folder alternative, disk targets and
//! saved results. Cleanup is a quiet secondary entry (docs/UX.md).

use crate::i18n::t;
use std::path::PathBuf;

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense};
use space_core::CacheMeta;

use crate::app::SpaceApp;
use crate::icons::{self, Icon};
use crate::theme::metrics::{S2, S3, S4, S6, S8};
use crate::theme::type_scale;
use crate::theme::{ACCENT, BG, MUTED, PANEL_2, TEXT};
use crate::volumes::VolumeInfo;
use crate::widgets::{
    block, caption, card, divider, format_age, ghost, meter, page_column, primary, secondary,
    section_header,
};

const CARD_MIN_W: f32 = 250.0;
const GAP: f32 = 16.0;

impl SpaceApp {
    pub(super) fn scan_page(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG))
            .show(ctx, |ui| {
                if ui.input(|i| !i.raw.hovered_files.is_empty()) {
                    ui.painter().rect_filled(
                        ui.max_rect(),
                        0.0,
                        Color32::from_rgba_unmultiplied(240, 173, 78, 28),
                    );
                }
                egui::ScrollArea::vertical()
                    .id_salt("scan_page")
                    .auto_shrink([false, false])
                    .show(ui, |ui| page_column(ui, 1120.0, |ui| self.scan_content(ui)));
            });
    }

    fn scan_content(&mut self, ui: &mut egui::Ui) {
        let hovering_drop = ui.input(|i| !i.raw.hovered_files.is_empty());
        let enabled = !self.busy && self.scan.is_none();
        let mut target: Option<PathBuf> = None;
        let mut cleanup = false;

        ui.add_space(S8);
        ui.horizontal_wrapped(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(t("What do you want to scan?")).size(type_scale::DISPLAY).strong());
                ui.label(
                    RichText::new(t(if hovering_drop {
                        "Drop the folder to start scanning."
                    } else {
                        "Pick your home folder or a disk — or drop a folder anywhere in this window."
                    }))
                    .size(type_scale::BODY)
                    .color(if hovering_drop { ACCENT } else { MUTED }),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(enabled, secondary("Choose another folder…  ⌘O"))
                    .clicked()
                {
                    target = rfd::FileDialog::new().pick_folder();
                }
            });
        });
        ui.add_space(S6);

        // Cards: the first is the recommended starting point.
        let avail = ui.available_width();
        let cols = (((avail + GAP) / (CARD_MIN_W + GAP)).floor() as usize).clamp(1, 3);
        let card_w = ((avail - GAP * (cols as f32 - 1.0)) / cols as f32).floor();
        // Rows of equal-width cards; each row is top-aligned so heights never stretch.
        enum Target<'a> {
            Home,
            Disk(&'a VolumeInfo),
        }
        let mut targets: Vec<Target> = vec![Target::Home];
        targets.extend(self.volumes.iter().map(Target::Disk));
        for row in targets.chunks(cols) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                for t in row {
                    match t {
                        Target::Home => {
                            if home_card(ui, card_w, enabled) {
                                target = std::env::var_os("HOME").map(PathBuf::from);
                            }
                        }
                        Target::Disk(vol) => {
                            if disk_card(ui, vol, card_w, enabled) {
                                target = Some(vol.mount.clone());
                            }
                        }
                    }
                }
            });
            ui.add_space(GAP);
        }
        ui.horizontal(|ui| {
            if self.volumes_loading {
                ui.spinner();
                ui.label(caption("Reading disks…"));
            } else if ui
                .add(ghost("Refresh disks").min_size(egui::vec2(0.0, 28.0)))
                .clicked()
            {
                self.refresh_volumes_async();
            }
        });
        ui.add_space(S6);

        section_header(ui, "Saved results", |ui| {
            ui.label(caption("Historical. Opening is instant; rescan to update."));
        });
        card(ui, |ui| {
            ui.set_width(ui.available_width());
            if self.recent.is_empty() {
                ui.label(caption("Scans you run appear here."));
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            for meta in self.recent.iter().take(6) {
                if recent_row(ui, meta, now, enabled) {
                    target = Some(PathBuf::from(&meta.root_path));
                }
            }
        });
        ui.add_space(S6);

        divider(ui);
        ui.add_space(S3);
        ui.horizontal_wrapped(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(t("Looking for disposable files?")).size(type_scale::LEAD).strong());
                ui.label(caption(
                    "Caches, logs and temporary files, found without a full scan. Nothing is selected until you choose it.",
                ));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add_enabled(enabled, secondary("Review cleanup candidates")).clicked() {
                    cleanup = true;
                }
            });
        });
        ui.add_space(S8);

        if cleanup {
            self.open_junk_panel();
        }
        if let Some(path) = target {
            self.start_scan(path, false);
        }
    }
}

fn home_card(ui: &mut egui::Ui, width: f32, enabled: bool) -> bool {
    let mut clicked = false;
    card(ui, |ui| {
        ui.set_width(width - 36.0);
        ui.set_min_height(168.0);
        ui.horizontal(|ui| {
            icons::show(ui, Icon::Home, 20.0, ACCENT);
            ui.label(
                RichText::new(t("Home folder"))
                    .size(type_scale::LEAD)
                    .strong(),
            );
        });
        ui.label(
            RichText::new("~/")
                .size(type_scale::MONO)
                .monospace()
                .color(MUTED),
        );
        ui.add_space(S3);
        ui.label(caption(
            "Downloads, Documents and personal app data. Usually the fastest useful scan.",
        ));
        ui.add_space(S3);
        clicked = ui
            .add_enabled(enabled, block(ui, primary("Scan home folder")))
            .clicked();
    });
    clicked
}

fn disk_card(ui: &mut egui::Ui, vol: &VolumeInfo, width: f32, enabled: bool) -> bool {
    let mut clicked = false;
    let frac = if vol.total > 0 {
        (vol.used() as f32 / vol.total as f32).clamp(0.0, 1.0)
    } else {
        0.0
    };
    card(ui, |ui| {
        ui.set_width(width - 36.0);
        ui.set_min_height(168.0);
        ui.horizontal(|ui| {
            icons::show(ui, Icon::Disk, 20.0, MUTED);
            ui.add(
                egui::Label::new(RichText::new(&vol.name).size(type_scale::LEAD).strong())
                    .truncate(),
            );
        });
        ui.add(
            egui::Label::new(
                RichText::new(vol.mount.display().to_string())
                    .size(type_scale::MONO)
                    .monospace()
                    .color(MUTED),
            )
            .truncate(),
        );
        ui.add_space(S3);
        if vol.total > 0 {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(crate::i18n::bytes(vol.available))
                        .size(type_scale::TITLE)
                        .strong(),
                );
                ui.label(caption(crate::tr!(
                    "free of {}",
                    crate::i18n::bytes(vol.total)
                )));
            });
            meter(
                ui,
                frac,
                if frac > 0.85 {
                    ACCENT
                } else {
                    Color32::from_rgb(106, 169, 255)
                },
                8.0,
            );
        } else {
            ui.label(caption("Capacity unknown"));
        }
        if vol.snapshot_count > 0 {
            ui.label(caption(crate::tr!(
                "{} local snapshots",
                vol.snapshot_count
            )));
        }
        ui.add_space(S3);
        clicked = ui
            .add_enabled(enabled, block(ui, secondary("Scan disk")))
            .on_hover_text(crate::tr!("Scan everything on {}", vol.mount.display()))
            .clicked();
    });
    clicked
}

/// One saved result: path, age, size. Click reopens it from cache.
fn recent_row(ui: &mut egui::Ui, meta: &CacheMeta, now: u64, enabled: bool) -> bool {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 44.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() && enabled {
        p.rect_filled(rect, 8.0, PANEL_2);
    }
    let y = rect.center().y;
    let size = p.layout_no_wrap(
        crate::i18n::bytes(meta.total_size),
        egui::FontId::proportional(type_scale::BODY),
        TEXT,
    );
    let age = p.layout_no_wrap(
        format_age(now.saturating_sub(meta.scanned_at_unix)),
        egui::FontId::proportional(type_scale::BODY),
        MUTED,
    );
    let size_x = rect.right() - S2 - size.size().x;
    let age_x = size_x - S4 - age.size().x.max(96.0);
    let mut job = egui::text::LayoutJob::simple_singleline(
        meta.root_path.clone(),
        egui::FontId::monospace(type_scale::MONO),
        TEXT,
    );
    job.wrap =
        egui::text::TextWrapping::truncate_at_width((age_x - rect.left() - S4 - S2).max(40.0));
    let path = p.layout_job(job);
    p.galley(
        egui::pos2(rect.left() + S2, y - path.size().y * 0.5),
        path,
        TEXT,
    );
    p.galley(egui::pos2(age_x, y - age.size().y * 0.5), age, MUTED);
    p.galley(egui::pos2(size_x, y - size.size().y * 0.5), size, TEXT);
    enabled && resp.on_hover_text(&meta.root_path).clicked()
}
